#![allow(dead_code)]
use std::{
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use spec_viewer_daemon::{Daemon, Options};
use spec_viewer_protocol::{Client, Runtime, Watch};

pub const BIN: &str = env!("CARGO_BIN_EXE_spec-viewer");
pub const WAIT: Duration = Duration::from_secs(15);

/// One machine's directories: runtime, state, and user config, all below `dir`.
pub struct Host {
    pub dir: PathBuf,
    pub runtime: Runtime,
}

impl Host {
    pub fn new(base: &Path, name: &str) -> Self {
        let dir = std::fs::canonicalize(base).unwrap().join(name);
        std::fs::create_dir_all(dir.join("config/spec-viewer")).unwrap();
        let host = Self {
            runtime: Runtime::at(dir.join("run/spec-viewer")),
            dir,
        };
        host.write_user("");
        host
    }
    pub fn user_file(&self) -> PathBuf {
        self.dir.join("config/spec-viewer/config.toml")
    }
    pub fn known_file(&self) -> PathBuf {
        self.dir.join("state/spec-viewer/projects.toml")
    }
    /// Writes the user config; the web port is always any free one.
    pub fn write_user(&self, source: &str) {
        std::fs::write(self.user_file(), format!("[web]\nport = 0\n{source}")).unwrap();
    }
    pub fn env(&self) -> [(String, String); 3] {
        let path = |child: &str| self.dir.join(child).display().to_string();
        [
            ("XDG_RUNTIME_DIR".into(), path("run")),
            ("XDG_STATE_HOME".into(), path("state")),
            ("XDG_CONFIG_HOME".into(), path("config")),
        ]
    }
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BIN);
        command.args(args).envs(self.env());
        command
    }
    /// The `[[host]]` command reaching this host's daemon over stdio.
    pub fn host_entry(&self, name: &str) -> String {
        let mut argv: Vec<String> = vec!["env".into()];
        argv.extend(self.env().iter().map(|(key, value)| format!("{key}={value}")));
        argv.extend([BIN.into(), "daemon".into(), "--stdio".into()]);
        let argv = argv.iter().map(|arg| format!("{arg:?}")).collect::<Vec<_>>().join(", ");
        format!("[[host]]\nname = {name:?}\ncommand = [{argv}]\n")
    }
    /// An in-process daemon of this host.
    pub fn daemon(&self) -> Arc<Daemon> {
        Daemon::new(&Options {
            runtime: self.runtime.clone(),
            user_config: Some(self.user_file()),
            known: Some(self.known_file()),
            idle_exit: None,
        })
        .unwrap()
    }
    /// A client of this host's running daemon process.
    pub fn client(&self) -> Client {
        Client::over_socket(UnixStream::connect(self.runtime.socket()).unwrap(), None).unwrap()
    }
    pub fn pid(&self) -> Option<i32> {
        std::fs::read_to_string(self.runtime.lock()).ok()?.trim().parse().ok()
    }
    /// Kills this host's daemon process, if one ran.
    pub fn kill(&self) {
        if let Some(pid) = self.pid() {
            let _ = Command::new("kill")
                .args(["-9", &pid.to_string()])
                .stderr(std::process::Stdio::null())
                .status();
            let start = Instant::now();
            let alive = || {
                Command::new("kill")
                    .args(["-0", &pid.to_string()])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success()
            };
            while alive() && start.elapsed() < WAIT {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
    pub fn set_runtime_mode(&self, mode: u32) {
        std::fs::set_permissions(self.runtime.dir(), std::fs::Permissions::from_mode(mode)).unwrap();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.kill();
    }
}

pub fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

pub fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// A Git worktree with one plan, and `origin` as its `remote.origin.url`.
pub fn repository(path: &Path, origin: Option<&str>) -> PathBuf {
    std::fs::create_dir_all(path).unwrap();
    let path = std::fs::canonicalize(path).unwrap();
    git(&path, &["init", "-qb", "main"]);
    git(&path, &["config", "core.excludesFile", "/dev/null"]);
    if let Some(origin) = origin {
        git(&path, &["remote", "add", "origin", origin]);
    }
    write(&path, ".plan/a.md", "---\ntitle: Alpha\nstatus: shipped\n---\n# Alpha\n");
    path
}

pub fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(start.elapsed() < WAIT, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn wait_connected(client: &Client, host: &str) {
    wait_until(host, || {
        client.hosts().unwrap().iter().any(|state| state.name == host && state.connected)
    });
}

/// Lets the watch start, then drops anything already queued.
pub fn settle(watch: &Watch) {
    std::thread::sleep(Duration::from_millis(400));
    while watch.try_recv().is_ok() {}
}
