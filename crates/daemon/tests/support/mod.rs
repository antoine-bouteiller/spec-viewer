#![allow(dead_code)]
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use spec_viewer_daemon::{Daemon, Options};
use spec_viewer_protocol::Runtime;

pub fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

pub fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A Git worktree at `<base>/<name>` with one plan and one spec.
pub fn repository(base: &Path, name: &str) -> PathBuf {
    let root = std::fs::canonicalize(base).unwrap().join(name);
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-qb", "main"]);
    git(&root, &["config", "core.excludesFile", "/dev/null"]);
    write(
        &root,
        ".plan/a.md",
        "---\ntitle: Alpha\nstatus: shipped\n---\n# Alpha\n\n## Why\n\nBody.\n",
    );
    write(&root, "spec/b.spec.md", "---\ntitle: Beta\nstatus: done\n---\n# Beta\n");
    root
}

/// A daemon whose user config, known projects, and runtime directory live in `base`.
pub struct Fixture {
    pub base: tempfile::TempDir,
    pub user: PathBuf,
    pub known: PathBuf,
}

impl Fixture {
    pub fn new(user: &str) -> Self {
        let base = tempfile::tempdir().unwrap();
        let fixture = Self {
            user: base.path().join("config.toml"),
            known: base.path().join("state/projects.toml"),
            base,
        };
        std::fs::write(&fixture.user, format!("[web]\nport = 0\n{user}")).unwrap();
        fixture
    }
    pub fn options(&self, idle_exit: Option<std::time::Duration>) -> Options {
        Options {
            runtime: Runtime::at(self.base.path().join("run/spec-viewer")),
            user_config: Some(self.user.clone()),
            known: Some(self.known.clone()),
            idle_exit,
        }
    }
    pub fn daemon(&self) -> Arc<Daemon> {
        Daemon::new(&self.options(None)).unwrap()
    }
}
