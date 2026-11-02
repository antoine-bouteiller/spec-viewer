//! The daemon process: the single-instance lock, the socket, the gateway port, and idle exit.
use std::{
    fs,
    io::{self, Write},
    net::{Ipv4Addr, Shutdown, TcpListener},
    os::{fd::AsRawFd, unix::net::UnixListener},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use spec_viewer_protocol::Runtime;

use crate::{Daemon, Options};

/// Runs a daemon until it idles out, or forever without `idle_exit`. Returns `Ok` at once when
/// another daemon holds the lock. `gateway` receives the bound web port; a port held by another
/// process leaves the daemon without one.
pub fn run(options: Options, gateway: impl FnOnce(&Arc<Daemon>, TcpListener)) -> Result<(), String> {
    let runtime = &options.runtime;
    runtime.prepare().map_err(|error| error.to_string())?;
    let lock_path = runtime.lock();
    let mut lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("{}: {error}", lock_path.display()))?;
    // SAFETY: the descriptor stays open for the life of the process, which holds the lock.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let _ = lock.set_len(0).and_then(|()| writeln!(lock, "{}", std::process::id()));
    let daemon = Daemon::new(&options).map_err(|error| error.to_string())?;
    let socket = runtime.socket();
    // Left by a crashed daemon; the lock proves no live one uses it.
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket).map_err(|error| format!("{}: {error}", socket.display()))?;
    match TcpListener::bind((Ipv4Addr::LOCALHOST, daemon.config().web.port)) {
        Ok(web) => {
            let port = web.local_addr().map_err(|error| error.to_string())?.port();
            daemon.set_web_url(Some(format!("http://localhost:{port}")));
            gateway(&daemon, web);
        }
        Err(error) => eprintln!("spec-viewer: no web app on port {}: {error}", daemon.config().web.port),
    }
    let accepting = Arc::clone(&daemon);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let Ok(writer) = stream.try_clone() else { continue };
            let daemon = Arc::clone(&accepting);
            let hold = daemon.hold();
            std::thread::spawn(move || daemon.serve(stream, writer, Some(hold)));
        }
    });
    let Some(idle_exit) = options.idle_exit else {
        loop {
            std::thread::park();
        }
    };
    let start = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(100));
        if daemon.idle(start).is_some_and(|idle| idle >= idle_exit) {
            let _ = fs::remove_file(&socket);
            return Ok(());
        }
    }
}

/// Relays stdin and stdout to this host's daemon socket, spawning `spawn` when none answers.
pub fn relay(runtime: &Runtime, spawn: Command) -> io::Result<()> {
    let stream = runtime.connect(spawn)?;
    let mut upstream = stream.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut upstream);
        let _ = upstream.shutdown(Shutdown::Write);
    });
    let mut downstream = stream;
    io::copy(&mut downstream, &mut io::stdout().lock())?;
    Ok(())
}
