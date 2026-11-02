//! The daemon process: a single instance per host, started on demand, exiting when idle.
mod support;

use std::{
    process::Command,
    time::{Duration, Instant},
};

use spec_viewer_protocol::Client;
use support::{Host, repository, wait_until};

#[test]
fn vc_1_concurrent_clients_start_one_daemon_and_share_it() {
    let base = support::base();
    let host = Host::new(base.path(), "host");
    let root = repository(&base.path().join("r"), None);
    // A distinctive idle time finds this test's daemons among other processes.
    let marker = "daemon --idle-exit 3001";
    let clients: Vec<_> = (0..2)
        .map(|_| {
            let (runtime, spawn, root) = (host.runtime.clone(), host.command(&["daemon", "--idle-exit", "3001"]), root.clone());
            std::thread::spawn(move || Client::connect_with(&runtime, spawn).unwrap().resolve(None, &root).unwrap())
        })
        .collect();
    let resolved: Vec<_> = clients.into_iter().map(|client| client.join().unwrap()).collect();
    assert_eq!(resolved[0], resolved[1]);
    let count = || {
        let output = Command::new("pgrep").args(["-f", marker]).output().unwrap();
        String::from_utf8_lossy(&output.stdout).lines().count()
    };
    wait_until("one daemon", || count() == 1);
}

#[test]
fn vc_2_an_idle_exit_daemon_lives_while_a_client_does() {
    let base = support::base();
    let host = Host::new(base.path(), "host");
    let mut daemon = host.command(&["daemon", "--idle-exit", "1"]).spawn().unwrap();
    wait_until("the socket", || host.runtime.socket().exists());
    let client = host.client();
    std::thread::sleep(Duration::from_secs(2));
    assert!(daemon.try_wait().unwrap().is_none(), "a client keeps it up");
    drop(client);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = daemon.try_wait().unwrap() {
            break status;
        }
        assert!(start.elapsed() < Duration::from_secs(3), "still running 3 s after the last client");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(status.success());
    assert!(!host.runtime.socket().exists());

    let mut daemon = host.command(&["daemon"]).spawn().unwrap();
    wait_until("the socket", || host.runtime.socket().exists());
    std::thread::sleep(Duration::from_secs(3));
    assert!(daemon.try_wait().unwrap().is_none(), "without --idle-exit it keeps serving");
    daemon.kill().unwrap();
    daemon.wait().unwrap();
}
