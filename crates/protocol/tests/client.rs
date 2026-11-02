use std::{
    io::{BufReader, Write},
    os::unix::net::UnixStream,
    path::Path,
    sync::mpsc,
    time::Duration,
};

use spec_viewer_protocol::{
    Client, ClientMessage, DaemonMessage, Error, Event, HostState, Layer, PROTOCOL, Request, Response, receive, send,
};

/// A daemon end that answers the handshake, then hands its streams to `script`.
fn client(script: impl FnOnce(&mut BufReader<UnixStream>, &mut UnixStream) + Send + 'static) -> (Client, mpsc::Receiver<ClientMessage>) {
    let (near, far) = UnixStream::pair().unwrap();
    let (seen, messages) = mpsc::channel();
    std::thread::spawn(move || {
        let mut writer = far.try_clone().unwrap();
        let mut reader = BufReader::new(far);
        let hello: ClientMessage = receive(&mut reader).unwrap().unwrap();
        seen.send(hello).unwrap();
        let welcome = DaemonMessage::Welcome {
            protocol: PROTOCOL,
            version: "test".into(),
            web: Some("http://localhost:4747".into()),
        };
        send(&mut writer, &welcome).unwrap();
        script(&mut reader, &mut writer);
    });
    let client = Client::over(near.try_clone().unwrap(), near, Some(Layer::default())).unwrap();
    (client, messages)
}

fn next_request(reader: &mut BufReader<UnixStream>) -> (u64, Option<String>, Request) {
    match receive(reader).unwrap().unwrap() {
        ClientMessage::Request { id, host, request } => (id, host, request),
        other => panic!("expected a request, got {other:?}"),
    }
}

fn respond(writer: &mut UnixStream, id: u64, result: Result<Response, Error>) {
    send(writer, &DaemonMessage::Response { id, result }).unwrap();
}

#[test]
fn the_handshake_sends_hello_and_keeps_the_web_url() {
    let (client, messages) = client(|_, _| {});
    assert_eq!(
        messages.recv().unwrap(),
        ClientMessage::Hello {
            protocol: PROTOCOL,
            user: Some(Layer::default())
        }
    );
    assert_eq!(client.web_url(), Some("http://localhost:4747"));
}

#[test]
fn a_welcome_of_another_protocol_fails_the_handshake() {
    let (near, far) = UnixStream::pair().unwrap();
    std::thread::spawn(move || {
        let mut writer = far.try_clone().unwrap();
        let _: Option<ClientMessage> = receive(&mut BufReader::new(far)).unwrap();
        let welcome = DaemonMessage::Welcome {
            protocol: PROTOCOL + 1,
            version: "next".into(),
            web: None,
        };
        send(&mut writer, &welcome).unwrap();
    });
    let error = Client::over(near.try_clone().unwrap(), near, None).err().unwrap();
    assert_eq!(
        error,
        Error::Protocol {
            daemon: PROTOCOL + 1,
            client: PROTOCOL
        }
    );
}

#[test]
fn responses_route_by_id_and_carry_the_host() {
    let (client, _) = client(|reader, writer| {
        let (first, host, request) = next_request(reader);
        assert_eq!((host.as_deref(), request), (Some("box"), Request::Projects));
        let (second, host, request) = next_request(reader);
        assert_eq!((host, request), (None, Request::Hosts));
        // Answer out of order.
        respond(
            writer,
            second,
            Ok(Response::Hosts(vec![HostState {
                name: "box".into(),
                connected: true,
                error: None,
            }])),
        );
        respond(writer, first, Ok(Response::Projects(vec![])));
        let (third, _, _) = next_request(reader);
        respond(writer, third, Err(Error::NotAProject));
    });
    let client = std::sync::Arc::new(client);
    let first = {
        let client = std::sync::Arc::clone(&client);
        std::thread::spawn(move || client.projects(Some("box")))
    };
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(client.hosts().unwrap()[0].name, "box");
    assert_eq!(first.join().unwrap().unwrap(), vec![]);
    assert_eq!(client.resolve(None, Path::new("/x")).unwrap_err(), Error::NotAProject);
}

#[test]
fn watches_receive_their_changes_and_host_down_and_unwatch_on_drop() {
    let (done, finished) = mpsc::channel();
    let (client, _) = client(move |reader, writer| {
        let (local, host, request) = next_request(reader);
        assert_eq!((host, request), (None, Request::Watch { worktree: "/a".into() }));
        respond(writer, local, Ok(Response::Watching));
        let (remote, host, _) = next_request(reader);
        assert_eq!(host.as_deref(), Some("box"));
        respond(writer, remote, Ok(Response::Watching));
        send(writer, &DaemonMessage::Changed { id: local }).unwrap();
        send(writer, &DaemonMessage::HostDown { host: "box".into() }).unwrap();
        send(writer, &DaemonMessage::Changed { id: remote }).unwrap();
        let (_, host, request) = next_request(reader);
        assert_eq!((host, request), (None, Request::Unwatch { id: local }));
        done.send(()).unwrap();
        writer.flush().unwrap();
    });
    let local = client.watch(None, Path::new("/a")).unwrap();
    let remote = client.watch(Some("box"), Path::new("/b")).unwrap();
    let wait = Duration::from_secs(2);
    assert_eq!(local.recv_timeout(wait), Ok(Event::Changed));
    assert_eq!(remote.recv_timeout(wait), Ok(Event::HostDown));
    assert_eq!(remote.recv_timeout(wait), Ok(Event::Changed));
    assert!(local.try_recv().is_err(), "HostDown reaches only the watches on that host");
    drop(local);
    finished.recv_timeout(wait).unwrap();
}

#[test]
fn a_lost_connection_fails_pending_and_later_calls() {
    let (client, _) = client(|reader, _| {
        let _ = next_request(reader);
    });
    assert!(!client.is_lost());
    let error = client.corpus(None, Path::new("/a")).unwrap_err();
    assert!(matches!(error, Error::HostUnreachable { .. }), "{error:?}");
    assert!(client.is_lost());
    let error = client.hosts().unwrap_err();
    assert!(matches!(error, Error::HostUnreachable { .. }), "{error:?}");
}

#[test]
fn user_and_override_messages_reach_the_daemon_in_order() {
    let (seen, received) = mpsc::channel();
    let (client, _) = client(move |reader, _| {
        for _ in 0..2 {
            seen.send(receive::<ClientMessage>(reader).unwrap().unwrap()).unwrap();
        }
    });
    let layer = Layer {
        chips: Some(vec![]),
        ..Layer::default()
    };
    client.send_user(layer.clone()).unwrap();
    client.send_override(Path::new("/b"), None).unwrap();
    let wait = Duration::from_secs(2);
    assert_eq!(received.recv_timeout(wait).unwrap(), ClientMessage::User { user: layer });
    assert_eq!(
        received.recv_timeout(wait).unwrap(),
        ClientMessage::Override {
            worktree: "/b".into(),
            layer: None
        }
    );
}
