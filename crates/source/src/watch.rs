//! Debounced change notifications for tab documents and Git metadata paths.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::git_paths;
use spec_viewer_config::ParseConfig;

const DEBOUNCE: Duration = Duration::from_millis(100);

/// Yields one `()` per burst of changes, 100 ms after the last one. Dropping it stops watching.
pub struct Watch {
    pub reloads: Receiver<()>,
    _watcher: Watcher,
}

/// A running watch; dropping it stops watching.
pub struct Watcher {
    watcher: Arc<Mutex<RecommendedWatcher>>,
    set: Option<Arc<Mutex<WatchSet>>>,
}

impl Watcher {
    /// Watches the tab folders and document directories of the current `configs` now, for a
    /// config change that made no event in the worktree.
    pub fn refresh(&self) {
        if let Some(set) = &self.set {
            set.lock()
                .expect("watch set")
                .watch_document_directories(&mut self.watcher.lock().expect("watcher"));
        }
    }
}

/// Tab folders are watched recursively; whole-worktree tab directories are found with `git_paths`.
pub fn watch(worktree: &Path, config: &ParseConfig) -> notify::Result<Watch> {
    let (sender, reloads) = mpsc::channel();
    let config = config.clone();
    let watcher = watch_with(worktree, move || vec![config.clone()], move || sender.send(()).is_ok())?;
    Ok(Watch {
        reloads,
        _watcher: watcher,
    })
}

/// Calls `notify` once per burst of changes to the documents of the tabs `configs` lists, the Git
/// index, or the worktree's `.spec-viewer.toml`, until it returns `false`. `configs` is read again
/// after every burst, so folders of tabs added since start are watched too.
pub fn watch_with(
    worktree: &Path,
    configs: impl Fn() -> Vec<ParseConfig> + Send + 'static,
    notify: impl FnMut() -> bool + Send + 'static,
) -> notify::Result<Watcher> {
    let (event_sender, event_receiver) = mpsc::channel::<Event>();
    let watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if let Ok(event) = result {
            let _ = event_sender.send(event);
        }
    })?;
    let watcher = Arc::new(Mutex::new(watcher));
    let mut set = WatchSet::new(worktree, Box::new(configs));
    set.start(&mut watcher.lock().expect("watcher"));
    let set = Arc::new(Mutex::new(set));
    let (weak, shared) = (Arc::downgrade(&watcher), Arc::clone(&set));
    std::thread::spawn(move || {
        debounce(&event_receiver, notify, |relevant| {
            let mut set = shared.lock().expect("watch set");
            match relevant {
                Some(event) => set.relevant(event),
                None => {
                    if let Some(watcher) = weak.upgrade() {
                        set.watch_document_directories(&mut watcher.lock().expect("watcher"));
                    }
                    true
                }
            }
        })
    });
    Ok(Watcher { watcher, set: Some(set) })
}

/// Calls `notify` once per burst of changes to any of `files`, which need not exist, until it
/// returns `false`.
pub fn watch_files(files: Vec<PathBuf>, notify: impl FnMut() -> bool + Send + 'static) -> notify::Result<Watcher> {
    let (event_sender, event_receiver) = mpsc::channel::<Event>();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if let Ok(event) = result {
            let _ = event_sender.send(event);
        }
    })?;
    // Events name canonical paths, such as /private/var for /var on macOS.
    let files: Vec<PathBuf> = files
        .into_iter()
        .map(|file| {
            match (
                file.parent().and_then(|parent| std::fs::canonicalize(parent).ok()),
                file.file_name(),
            ) {
                (Some(parent), Some(name)) => parent.join(name),
                _ => file,
            }
        })
        .collect();
    for directory in files.iter().filter_map(|file| file.parent()).collect::<HashSet<_>>() {
        if directory.exists() {
            watcher.watch(directory, RecursiveMode::NonRecursive)?;
        }
    }
    std::thread::spawn(move || {
        debounce(&event_receiver, notify, |event| {
            event.is_none_or(|event| !event.kind.is_access() && event.paths.iter().any(|path| files.contains(path)))
        })
    });
    Ok(Watcher {
        watcher: Arc::new(Mutex::new(watcher)),
        set: None,
    })
}

/// `filter(Some(event))` decides whether an event counts; `filter(None)` runs once per burst
/// before `notify`. Ends when `notify` returns `false` or the watcher is dropped, which drops the
/// notify callback and disconnects `events`.
fn debounce(events: &Receiver<Event>, mut notify: impl FnMut() -> bool, mut filter: impl FnMut(Option<&Event>) -> bool) {
    let mut pending = false;
    loop {
        let next = if pending {
            events.recv_timeout(DEBOUNCE)
        } else {
            events.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match next {
            Ok(event) if filter(Some(&event)) => pending = true,
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) => {
                pending = false;
                filter(None);
                if !notify() {
                    return;
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

struct WatchSet {
    worktree: PathBuf,
    configs: Box<dyn Fn() -> Vec<ParseConfig> + Send>,
    folder_dirs: HashSet<PathBuf>,
    /// Tab folders, watched or not yet created.
    wanted: HashSet<PathBuf>,
    git_dir: Option<PathBuf>,
    spec_dirs: HashSet<PathBuf>,
    config_file: PathBuf,
}

impl WatchSet {
    fn new(worktree: &Path, configs: Box<dyn Fn() -> Vec<ParseConfig> + Send>) -> Self {
        let git_dir = git_index_path(worktree)
            .and_then(|index| index.parent().map(Path::to_path_buf))
            .filter(|directory| directory.exists());
        Self {
            worktree: worktree.to_path_buf(),
            configs,
            folder_dirs: HashSet::new(),
            wanted: HashSet::new(),
            git_dir,
            spec_dirs: HashSet::new(),
            config_file: worktree.join(".spec-viewer.toml"),
        }
    }

    fn start(&mut self, watcher: &mut RecommendedWatcher) {
        if let Some(directory) = &self.git_dir {
            let _ = watcher.watch(directory, RecursiveMode::NonRecursive);
        }
        let _ = watcher.watch(&self.worktree, RecursiveMode::NonRecursive);
        self.watch_document_directories(watcher);
    }

    fn relevant(&self, event: &Event) -> bool {
        // inotify reports opens, and the `git ls-files` run per burst opens the index.
        if event.kind.is_access() {
            return false;
        }
        event.paths.iter().any(|path| {
            if *path == self.config_file || self.wanted.iter().any(|directory| path.starts_with(directory)) {
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
        for config in (self.configs)() {
            for folder in config.tabs.iter().filter_map(|tab| tab.folder.as_ref()) {
                let directory = self.worktree.join(folder);
                self.wanted.insert(directory.clone());
                if !self.folder_dirs.contains(&directory)
                    && directory.exists()
                    && watcher.watch(&directory, RecursiveMode::Recursive).is_ok()
                {
                    self.folder_dirs.insert(directory);
                }
            }
            for path in git_paths(&self.worktree, &config) {
                let Some(directory) = self.worktree.join(path).parent().map(Path::to_path_buf) else {
                    continue;
                };
                if directory.exists()
                    && !self.spec_dirs.contains(&directory)
                    && watcher.watch(&directory, RecursiveMode::NonRecursive).is_ok()
                {
                    self.spec_dirs.insert(directory);
                }
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
        let watch = watch(&root(&directory), &ParseConfig::default()).unwrap();
        settle(&watch);
        std::fs::write(root(&directory).join(".plan/new.md"), "# New\n").unwrap();
        std::fs::write(root(&directory).join(".plan/new.md"), "# New\n\nmore\n").unwrap();
        assert_eq!(watch.reloads.recv_timeout(Duration::from_secs(1)), Ok(()));
        assert!(watch.reloads.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn an_edited_spec_sends_a_reload() {
        let directory = repository();
        let watch = watch(&root(&directory), &ParseConfig::default()).unwrap();
        settle(&watch);
        std::fs::write(root(&directory).join("spec/a.spec.md"), "# A edited\n").unwrap();
        assert_eq!(watch.reloads.recv_timeout(Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn the_worktree_config_file_sends_a_reload() {
        let directory = repository();
        let watch = watch(&root(&directory), &ParseConfig::default()).unwrap();
        settle(&watch);
        std::fs::write(root(&directory).join(".spec-viewer.toml"), "chip = []\n").unwrap();
        assert_eq!(watch.reloads.recv_timeout(Duration::from_secs(5)), Ok(()));
    }

    #[test]
    fn a_folder_of_a_tab_added_later_is_watched_after_the_next_burst() {
        let directory = repository();
        let root = root(&directory);
        let tabs = Arc::new(Mutex::new(ParseConfig::default()));
        let (sender, reloads) = mpsc::channel();
        let configs = Arc::clone(&tabs);
        let _watcher = watch_with(
            &root,
            move || vec![configs.lock().unwrap().clone()],
            move || sender.send(()).is_ok(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::create_dir(root.join("notes")).unwrap();
        tabs.lock().unwrap().tabs[0].folder = Some("notes".into());
        std::fs::write(root.join(".spec-viewer.toml"), "").unwrap();
        assert_eq!(reloads.recv_timeout(Duration::from_secs(5)), Ok(()));
        std::fs::write(root.join("notes/a.md"), "# A\n").unwrap();
        assert_eq!(reloads.recv_timeout(Duration::from_secs(5)), Ok(()));
    }

    #[test]
    fn refresh_watches_the_folder_of_a_tab_added_without_a_worktree_event() {
        let directory = repository();
        let root = root(&directory);
        std::fs::create_dir(root.join("notes")).unwrap();
        let tabs = Arc::new(Mutex::new(ParseConfig::default()));
        let (sender, reloads) = mpsc::channel();
        let configs = Arc::clone(&tabs);
        let watcher = watch_with(
            &root,
            move || vec![configs.lock().unwrap().clone()],
            move || sender.send(()).is_ok(),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        tabs.lock().unwrap().tabs[0].folder = Some("notes".into());
        watcher.refresh();
        std::fs::write(root.join("notes/a.md"), "# A\n").unwrap();
        assert_eq!(reloads.recv_timeout(Duration::from_secs(5)), Ok(()));
    }

    #[test]
    fn watched_files_need_not_exist() {
        let directory = tempfile::tempdir().unwrap();
        let file = std::fs::canonicalize(directory.path()).unwrap().join("config.toml");
        let (sender, reloads) = mpsc::channel();
        let _watcher = watch_files(vec![file.clone()], move || sender.send(()).is_ok()).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        std::fs::write(file.with_file_name("other.toml"), "").unwrap();
        assert!(reloads.recv_timeout(Duration::from_millis(400)).is_err());
        std::fs::write(&file, "").unwrap();
        assert_eq!(reloads.recv_timeout(Duration::from_secs(5)), Ok(()));
    }

    #[test]
    fn unrelated_files_do_not_reload() {
        let directory = repository();
        let watch = watch(&root(&directory), &ParseConfig::default()).unwrap();
        settle(&watch);
        std::fs::write(root(&directory).join("README.md"), "x\n").unwrap();
        assert!(watch.reloads.recv_timeout(Duration::from_millis(500)).is_err());
    }
}
