//! Debounced change notifications for plan, spec, and Git metadata paths.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::spec_paths;

const DEBOUNCE: Duration = Duration::from_millis(100);

/// Yields one `()` per burst of changes, 100 ms after the last one. Dropping it stops watching.
pub struct Watch {
    pub reloads: Receiver<()>,
    _watcher: Arc<Mutex<RecommendedWatcher>>,
}

pub fn watch(worktree: &Path) -> notify::Result<Watch> {
    let (event_sender, event_receiver) = mpsc::channel::<Event>();
    let watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if let Ok(event) = result {
            let _ = event_sender.send(event);
        }
    })?;
    let watcher = Arc::new(Mutex::new(watcher));
    let mut set = WatchSet::new(worktree);
    set.start(&mut watcher.lock().expect("watcher"));
    let (reload_sender, reload_receiver) = mpsc::channel();
    let weak = Arc::downgrade(&watcher);
    std::thread::spawn(move || debounce(set, &weak, &event_receiver, &reload_sender));
    Ok(Watch {
        reloads: reload_receiver,
        _watcher: watcher,
    })
}

// Ends when the watcher is dropped, which drops the notify callback and disconnects `events`.
fn debounce(mut set: WatchSet, watcher: &Weak<Mutex<RecommendedWatcher>>, events: &Receiver<Event>, reloads: &mpsc::Sender<()>) {
    let mut pending = false;
    loop {
        let next = if pending {
            events.recv_timeout(DEBOUNCE)
        } else {
            events.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match next {
            Ok(event) if set.relevant(&event) => {
                if let Some(watcher) = watcher.upgrade() {
                    set.watch_document_directories(&mut watcher.lock().expect("watcher"));
                }
                pending = true;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                pending = false;
                if reloads.send(()).is_err() {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

struct WatchSet {
    worktree: PathBuf,
    plan_dir: PathBuf,
    git_dir: Option<PathBuf>,
    spec_dirs: HashSet<PathBuf>,
}

impl WatchSet {
    fn new(worktree: &Path) -> Self {
        let git_dir = git_index_path(worktree)
            .and_then(|index| index.parent().map(Path::to_path_buf))
            .filter(|directory| directory.exists());
        Self {
            worktree: worktree.to_path_buf(),
            plan_dir: worktree.join(".plan"),
            git_dir,
            spec_dirs: HashSet::new(),
        }
    }

    fn start(&mut self, watcher: &mut RecommendedWatcher) {
        if self.plan_dir.exists() {
            let _ = watcher.watch(&self.plan_dir, RecursiveMode::Recursive);
        }
        if let Some(directory) = &self.git_dir {
            let _ = watcher.watch(directory, RecursiveMode::NonRecursive);
        }
        self.watch_document_directories(watcher);
    }

    fn relevant(&self, event: &Event) -> bool {
        // inotify reports opens, and the `git ls-files` run per relevant event opens the index.
        if event.kind.is_access() {
            return false;
        }
        event.paths.iter().any(|path| {
            if path.starts_with(&self.plan_dir) {
                return true;
            }
            let parent = path.parent();
            if self.git_dir.is_some() && parent == self.git_dir.as_deref() {
                return path.file_name().is_some_and(|name| name == "index" || name == "index.lock");
            }
            parent.is_some_and(|directory| self.spec_dirs.contains(directory))
        })
    }

    fn watch_document_directories(&mut self, watcher: &mut RecommendedWatcher) {
        for path in spec_paths(&self.worktree) {
            let Some(directory) = self.worktree.join(path).parent().map(Path::to_path_buf) else {
                continue;
            };
            if directory.exists() && !self.spec_dirs.contains(&directory) && watcher.watch(&directory, RecursiveMode::NonRecursive).is_ok()
            {
                self.spec_dirs.insert(directory);
            }
        }
    }
}

fn git_index_path(worktree: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["rev-parse", "--git-path", "index"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| worktree.join(String::from_utf8_lossy(&output.stdout).trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .arg("-C")
                    .arg(&root)
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            )
        };
        git(&["init", "-qb", "main"]);
        std::fs::create_dir_all(root.join(".plan")).unwrap();
        std::fs::create_dir_all(root.join("spec")).unwrap();
        std::fs::write(root.join("spec/a.spec.md"), "# A\n").unwrap();
        git(&["add", "."]);
        directory
    }

    fn root(directory: &tempfile::TempDir) -> PathBuf {
        std::fs::canonicalize(directory.path()).unwrap()
    }

    fn settle(watch: &Watch) {
        std::thread::sleep(Duration::from_millis(300));
        while watch.reloads.try_recv().is_ok() {}
    }

    #[test]
    fn a_new_plan_file_sends_one_reload() {
        let directory = repository();
        let watch = watch(&root(&directory)).unwrap();
        settle(&watch);
        std::fs::write(root(&directory).join(".plan/new.md"), "# New\n").unwrap();
        std::fs::write(root(&directory).join(".plan/new.md"), "# New\n\nmore\n").unwrap();
        assert_eq!(watch.reloads.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert!(watch.reloads.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn an_edited_spec_sends_a_reload() {
        let directory = repository();
        let watch = watch(&root(&directory)).unwrap();
        settle(&watch);
        std::fs::write(root(&directory).join("spec/a.spec.md"), "# A edited\n").unwrap();
        assert_eq!(watch.reloads.recv_timeout(Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn unrelated_files_do_not_reload() {
        let directory = repository();
        let watch = watch(&root(&directory)).unwrap();
        settle(&watch);
        std::fs::write(root(&directory).join("README.md"), "x\n").unwrap();
        assert!(watch.reloads.recv_timeout(Duration::from_millis(500)).is_err());
    }
}
