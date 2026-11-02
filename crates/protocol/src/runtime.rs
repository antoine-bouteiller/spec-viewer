//! The per-user runtime directory holding the daemon's lock, socket, and log.
use std::{
    fs, io,
    os::unix::{
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::PROTOCOL;

const SPAWN_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    dir: PathBuf,
}

impl Runtime {
    /// `$XDG_RUNTIME_DIR/spec-viewer`, else `$TMPDIR/spec-viewer-<uid>`.
    pub fn from_env() -> Self {
        let var = |name| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
        let dir = match var("XDG_RUNTIME_DIR") {
            Some(dir) => dir.join("spec-viewer"),
            None => var("TMPDIR")
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(format!("spec-viewer-{}", uid())),
        };
        Self { dir }
    }
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
    pub fn dir(&self) -> &Path {
        &self.dir
    }
    pub fn socket(&self) -> PathBuf {
        self.dir.join(format!("daemon-v{PROTOCOL}.sock"))
    }
    pub fn lock(&self) -> PathBuf {
        self.dir.join(format!("daemon-v{PROTOCOL}.lock"))
    }
    pub fn log(&self) -> PathBuf {
        self.dir.join("daemon.log")
    }

    /// Creates the directory with mode `0700`; an existing one must have that mode and be owned by
    /// this user.
    pub fn prepare(&self) -> io::Result<()> {
        match fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.dir) {
            Ok(()) => {}
            Err(error) => return Err(io::Error::new(error.kind(), format!("{}: {error}", self.dir.display()))),
        }
        let metadata = fs::metadata(&self.dir)?;
        if metadata.uid() != uid() || metadata.permissions().mode() & 0o777 != 0o700 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{}: expected a directory of mode 0700 owned by this user", self.dir.display()),
            ));
        }
        Ok(())
    }

    /// Connects to the daemon's socket. When none answers, runs `spawn` detached, with its output
    /// appended to the log, and retries for 2 s.
    pub fn connect(&self, mut spawn: Command) -> io::Result<UnixStream> {
        self.prepare()?;
        if let Ok(stream) = UnixStream::connect(self.socket()) {
            return Ok(stream);
        }
        let log = fs::OpenOptions::new().create(true).append(true).open(self.log())?;
        spawn.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
        // SAFETY: setsid is async-signal-safe; it detaches the daemon from the terminal session.
        unsafe {
            spawn.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
        let mut child = spawn.spawn()?;
        let started = Instant::now();
        let result = loop {
            if let Ok(stream) = UnixStream::connect(self.socket()) {
                break Ok(stream);
            }
            // A daemon losing the lock to another one exits 0; only a failure ends the wait early.
            let failed = child.try_wait()?.is_some_and(|status| !status.success());
            if failed || started.elapsed() > SPAWN_WAIT {
                let last = fs::read_to_string(self.log())
                    .ok()
                    .and_then(|log| log.lines().last().map(str::to_owned))
                    .unwrap_or_default();
                break Err(io::Error::other(format!(
                    "no daemon answered on {}: {last}",
                    self.socket().display()
                )));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        std::thread::spawn(move || child.wait());
        result
    }
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_creates_a_private_directory_and_rejects_a_shared_one() {
        let base = tempfile::tempdir().unwrap();
        let runtime = Runtime::at(base.path().join("spec-viewer"));
        runtime.prepare().unwrap();
        assert_eq!(fs::metadata(runtime.dir()).unwrap().permissions().mode() & 0o777, 0o700);
        fs::set_permissions(runtime.dir(), fs::Permissions::from_mode(0o755)).unwrap();
        let error = runtime.prepare().unwrap_err();
        assert!(error.to_string().contains(&runtime.dir().display().to_string()), "{error}");
        assert_eq!(runtime.socket(), runtime.dir().join("daemon-v1.sock"));
        assert_eq!(runtime.lock(), runtime.dir().join("daemon-v1.lock"));
    }

    #[test]
    fn a_failing_spawn_reports_the_last_log_line() {
        let base = tempfile::tempdir().unwrap();
        let runtime = Runtime::at(base.path().join("spec-viewer"));
        let mut spawn = Command::new("sh");
        spawn.args(["-c", "echo 'spec-viewer: broken config' >&2; exit 1"]);
        let error = runtime.connect(spawn).unwrap_err();
        assert!(error.to_string().ends_with("spec-viewer: broken config"), "{error}");
    }
}
