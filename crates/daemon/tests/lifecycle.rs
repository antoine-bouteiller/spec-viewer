mod support;

use std::{
    net::TcpListener,
    os::unix::net::UnixStream,
    sync::mpsc,
    time::{Duration, Instant},
};

use spec_viewer_daemon::run;
use spec_viewer_protocol::{Client, Runtime};
use support::Fixture;

fn wait_for_socket(runtime: &Runtime) -> UnixStream {
    let start = Instant::now();
    loop {
        if let Ok(stream) = UnixStream::connect(runtime.socket()) {
            return stream;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "no socket");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn vc_2_an_idle_exit_daemon_serves_while_a_client_is_open_then_exits() {
    let fixture = Fixture::new("");
    let options = fixture.options(Some(Duration::from_secs(1)));
    let runtime = options.runtime.clone();
    runtime.prepare().unwrap();
    std::fs::write(runtime.socket(), "left by a crashed daemon").unwrap();
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || done.send(run(options, |_, _| {})).unwrap());
    let client = Client::over_socket(wait_for_socket(&runtime), None).unwrap();
    assert_eq!(
        run(fixture.options(Some(Duration::from_secs(1))), |_, _| panic!(
            "a second daemon serves"
        )),
        Ok(()),
        "a daemon losing the lock exits 0"
    );
    let pid = std::fs::read_to_string(runtime.lock()).unwrap();
    assert_eq!(pid.trim(), std::process::id().to_string());
    assert!(finished.recv_timeout(Duration::from_secs(2)).is_err(), "a client keeps it up");
    assert!(client.hosts().is_ok());
    drop(client);
    assert_eq!(finished.recv_timeout(Duration::from_secs(3)).unwrap(), Ok(()));
    assert!(!runtime.socket().exists());
}

#[test]
fn vc_2_a_daemon_without_idle_exit_keeps_serving() {
    let fixture = Fixture::new("");
    let options = fixture.options(None);
    let runtime = options.runtime.clone();
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || done.send(run(options, |_, _| {})).unwrap());
    drop(wait_for_socket(&runtime));
    assert!(finished.recv_timeout(Duration::from_secs(3)).is_err());
    assert!(Client::over_socket(wait_for_socket(&runtime), None).is_ok());
}

#[test]
fn vc_6_a_taken_port_leaves_the_daemon_without_a_gateway() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let fixture = Fixture::new("");
    std::fs::write(&fixture.user, format!("[web]\nport = {port}\n")).unwrap();
    let options = fixture.options(None);
    let runtime = options.runtime.clone();
    std::thread::spawn(move || run(options, |_, _| panic!("no gateway on a taken port")));
    let client = Client::over_socket(wait_for_socket(&runtime), None).unwrap();
    assert_eq!(client.web_url(), None);
    assert!(client.hosts().is_ok());

    let fixture = Fixture::new("");
    let options = fixture.options(None);
    let runtime = options.runtime.clone();
    let (bound, gateway) = mpsc::channel();
    std::thread::spawn(move || {
        run(options, move |_, listener| {
            bound.send(listener.local_addr().unwrap().port()).unwrap()
        })
    });
    let port = gateway.recv_timeout(Duration::from_secs(5)).unwrap();
    let client = Client::over_socket(wait_for_socket(&runtime), None).unwrap();
    assert_eq!(client.web_url(), Some(format!("http://localhost:{port}").as_str()));
}

#[test]
fn a_shared_runtime_directory_is_an_error_naming_it() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new("");
    let options = fixture.options(None);
    std::fs::create_dir_all(options.runtime.dir()).unwrap();
    std::fs::set_permissions(options.runtime.dir(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let dir = options.runtime.dir().display().to_string();
    let error = run(options, |_, _| {}).unwrap_err();
    assert!(error.contains(&dir), "{error}");
}
