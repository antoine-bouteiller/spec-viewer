mod support;

use std::{io::BufReader, os::unix::net::UnixStream, path::Path, time::Duration};

use spec_viewer_daemon::Daemon;
use spec_viewer_protocol::{ClientMessage, DaemonMessage, Error, Event, Layer, PROTOCOL, StatusConfig, receive, send};
use support::{Fixture, git, repository, write};

const WAIT: Duration = Duration::from_secs(5);

fn tab_names(corpus: &spec_viewer_protocol::Corpus) -> Vec<&str> {
    corpus.tabs.iter().map(|tab| tab.name.as_str()).collect()
}

fn settle(watch: &spec_viewer_protocol::Watch) {
    std::thread::sleep(Duration::from_millis(300));
    while watch.try_recv().is_ok() {}
}

#[test]
fn vc_3_a_client_of_another_protocol_gets_a_protocol_error() {
    let fixture = Fixture::new("");
    let daemon = fixture.daemon();
    let (near, far) = UnixStream::pair().unwrap();
    let writer = far.try_clone().unwrap();
    let served = std::thread::spawn(move || daemon.serve(far, writer, None));
    let mut writer = near.try_clone().unwrap();
    send(&mut writer, &ClientMessage::Hello { protocol: 0, user: None }).unwrap();
    let mut reader = BufReader::new(near);
    assert!(matches!(
        receive(&mut reader).unwrap(),
        Some(DaemonMessage::Welcome { protocol: PROTOCOL, .. })
    ));
    assert_eq!(
        receive::<DaemonMessage>(&mut reader).unwrap(),
        Some(DaemonMessage::Response {
            id: 0,
            result: Err(Error::Protocol { daemon: 1, client: 0 })
        })
    );
    served.join().unwrap();
    assert_eq!(receive::<DaemonMessage>(&mut reader).unwrap(), None);
}

#[test]
fn requests_resolve_list_and_read_a_worktree() {
    let fixture = Fixture::new("[[status]]\nname='shipped'\ncolor='green'\n");
    let root = repository(fixture.base.path(), "a");
    let client = fixture.daemon().connect().unwrap();
    std::fs::create_dir(root.join("nested")).unwrap();
    let resolved = client.resolve(None, &root.join("nested")).unwrap();
    assert_eq!(resolved.project.worktrees[resolved.worktree].path, root);
    assert_eq!(
        resolved.statuses.iter().map(|status| status.name.as_str()).collect::<Vec<_>>(),
        ["shipped"]
    );
    assert_eq!(client.project(None, &resolved.project.key).unwrap(), resolved.project);
    let corpus = client.corpus(None, &root).unwrap();
    assert_eq!(tab_names(&corpus), ["plans", "specs"]);
    assert_eq!(corpus.tabs[0].nodes[0].status.as_ref().map(|status| status.known), Some(true));
    let opened = client.open(None, &root, ".plan/a.md").unwrap();
    assert_eq!(opened.document.meta.title, "Alpha");
    assert_eq!(client.open(None, &root, "../a.md").unwrap_err(), Error::Forbidden);
    assert_eq!(client.open(None, &root, ".plan/missing.md").unwrap_err(), Error::NotFound);
    assert_eq!(client.corpus(None, &root.join("nested")).unwrap_err(), Error::UnlistedWorktree);
    assert_eq!(client.resolve(None, fixture.base.path()).unwrap_err(), Error::NotAProject);
    assert_eq!(client.corpus(None, Path::new("/nonexistent")).unwrap_err(), Error::NotAProject);
    assert_eq!(client.corpus(Some("box"), &root).unwrap_err(), Error::UnknownHost);
    assert!(client.hosts().unwrap().is_empty());
}

#[test]
fn vc_7_a_new_tab_in_the_worktree_file_reaches_open_watches() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "a");
    let client = fixture.daemon().connect().unwrap();
    let watch = client.watch(None, &root).unwrap();
    settle(&watch);
    write(
        &root,
        ".spec-viewer.toml",
        "[[tab]]\nname='plans'\nfolder='.plan'\n[[tab]]\nname='notes'\nfolder='notes'\n",
    );
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed));
    assert_eq!(tab_names(&client.corpus(None, &root).unwrap()), ["plans", "notes"]);
    settle(&watch);
    write(&root, "notes/n.md", "# Note\n");
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed), "the new tab's folder is watched");
}

#[test]
fn vc_8_a_worktree_file_with_user_sections_fails_resolve_naming_the_file() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "a");
    let client = fixture.daemon().connect().unwrap();
    for section in [
        "[[host]]\nname='box'\nssh='box'",
        "[web]\nport=1",
        "[keys]\nquit='x'",
        "[theme]\nmode='dark'",
        "[tui]\ndiff=true",
    ] {
        write(&root, ".spec-viewer.toml", section);
        let Error::Config { message } = client.resolve(None, &root).unwrap_err() else {
            panic!("{section}: expected a config error");
        };
        assert!(message.contains(&root.join(".spec-viewer.toml").display().to_string()), "{message}");
    }
}

#[test]
fn vc_12_known_projects_survive_a_restart_while_they_resolve() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "b");
    let resolved = {
        let client = fixture.daemon().connect().unwrap();
        assert!(client.projects(None).unwrap().is_empty());
        client.resolve(None, &root).unwrap()
    };
    let client = fixture.daemon().connect().unwrap();
    assert_eq!(client.projects(None).unwrap(), [resolved.project]);
    std::fs::remove_dir_all(&root).unwrap();
    assert!(client.projects(None).unwrap().is_empty());
}

#[test]
fn watchers_of_one_worktree_share_one_watch_dropped_with_the_last() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "a");
    let daemon = fixture.daemon();
    let (first, second) = (daemon.connect().unwrap(), daemon.connect().unwrap());
    let (one, two) = (first.watch(None, &root).unwrap(), second.watch(None, &root).unwrap());
    assert_eq!(daemon.watched_worktrees(), 1);
    settle(&one);
    settle(&two);
    write(&root, ".plan/c.md", "# C\n");
    assert_eq!(one.recv_timeout(WAIT), Ok(Event::Changed));
    assert_eq!(two.recv_timeout(WAIT), Ok(Event::Changed));
    drop(one);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(daemon.watched_worktrees(), 1);
    drop(two);
    wait_until(|| daemon.watched_worktrees() == 0);
    let third = daemon.connect().unwrap();
    let watch = third.watch(None, &root).unwrap();
    assert_eq!(daemon.watched_worktrees(), 1);
    drop((third, watch));
    wait_until(|| daemon.watched_worktrees() == 0);
    drop((first, second));
}

#[test]
fn a_closed_connection_drops_its_watches() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "a");
    let daemon = fixture.daemon();
    let (mut near, far) = UnixStream::pair().unwrap();
    let writer = far.try_clone().unwrap();
    let served = std::sync::Arc::clone(&daemon);
    std::thread::spawn(move || served.serve(far, writer, None));
    send(
        &mut near,
        &ClientMessage::Hello {
            protocol: PROTOCOL,
            user: None,
        },
    )
    .unwrap();
    let request = spec_viewer_protocol::Request::Watch { worktree: root };
    send(
        &mut near,
        &ClientMessage::Request {
            id: 1,
            host: None,
            request,
        },
    )
    .unwrap();
    wait_until(|| daemon.watched_worktrees() == 1);
    near.shutdown(std::net::Shutdown::Both).unwrap();
    wait_until(|| daemon.watched_worktrees() == 0);
}

fn wait_until(condition: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while !condition() {
        assert!(start.elapsed() < WAIT, "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn status(name: &str) -> StatusConfig {
    StatusConfig {
        name: name.into(),
        aliases: vec![],
        color: "green".into(),
    }
}

#[test]
fn user_and_override_layers_apply_per_connection_and_reach_its_watches() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "a");
    write(&root, ".spec-viewer.toml", "[[tab]]\nname='own'\nfolder='.plan'\n");
    let daemon = fixture.daemon();
    let (layered, (near, far)) = (daemon.connect().unwrap(), UnixStream::pair().unwrap());
    let writer = far.try_clone().unwrap();
    let served = std::sync::Arc::clone(&daemon);
    std::thread::spawn(move || served.serve(far, writer, None));
    let user = Layer {
        statuses: Some(vec![status("shipped")]),
        ..Layer::default()
    };
    let hub = spec_viewer_protocol::Client::over(near.try_clone().unwrap(), near, Some(user)).unwrap();
    assert_eq!(hub.resolve(None, &root).unwrap().statuses, [status("shipped")]);
    assert!(
        layered
            .resolve(None, &root)
            .unwrap()
            .statuses
            .iter()
            .all(|status| status.name != "shipped")
    );

    let watch = hub.watch(None, &root).unwrap();
    let other = layered.watch(None, &root).unwrap();
    settle(&watch);
    settle(&other);
    let notes = Layer {
        tabs: Some(vec![spec_viewer_config::TabConfig {
            name: "notes".into(),
            folder: Some(".plan".into()),
            kinds: None,
            companions: vec![],
        }]),
        ..Layer::default()
    };
    hub.send_override(&root, Some(notes)).unwrap();
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed));
    assert_eq!(tab_names(&hub.corpus(None, &root).unwrap()), ["notes"]);
    assert_eq!(tab_names(&layered.corpus(None, &root).unwrap()), ["own"]);
    assert!(
        other.recv_timeout(Duration::from_millis(300)).is_err(),
        "other connections keep their layers"
    );
    hub.send_override(&root, None).unwrap();
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed));
    assert_eq!(tab_names(&hub.corpus(None, &root).unwrap()), ["own"]);
    hub.send_user(Layer::default()).unwrap();
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed));
    assert!(
        hub.resolve(None, &root)
            .unwrap()
            .statuses
            .iter()
            .all(|status| status.name != "shipped")
    );
}

#[test]
fn a_user_config_edit_reaches_connections_on_the_daemon_layer() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "a");
    let daemon = fixture.daemon();
    let client = daemon.connect().unwrap();
    let watch = client.watch(None, &root).unwrap();
    settle(&watch);
    std::fs::write(&fixture.user, "[web]\nport = 0\n[[status]]\nname='shipped'\ncolor='green'\n").unwrap();
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed));
    assert_eq!(client.resolve(None, &root).unwrap().statuses, [status("shipped")]);
    std::fs::write(&fixture.user, "[web]\nport = 0\n[theme\n").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        client.resolve(None, &root).unwrap().statuses,
        [status("shipped")],
        "an invalid edit keeps the last layer"
    );
    git(&root, &["status"]);
    assert!(
        Daemon::new(&fixture.options(None)).is_err(),
        "a daemon does not start on an invalid user config"
    );
}

#[test]
fn a_user_config_tab_for_an_existing_folder_is_watched_at_once() {
    let fixture = Fixture::new("");
    let root = repository(fixture.base.path(), "a");
    write(&root, "notes/n.md", "# Note\n");
    let client = fixture.daemon().connect().unwrap();
    let watch = client.watch(None, &root).unwrap();
    settle(&watch);
    std::fs::write(&fixture.user, "[web]\nport = 0\n[[tab]]\nname = 'notes'\nfolder = 'notes'\n").unwrap();
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed));
    assert_eq!(tab_names(&client.corpus(None, &root).unwrap()), ["notes"]);
    settle(&watch);
    write(&root, "notes/n.md", "# Note edited\n");
    assert_eq!(watch.recv_timeout(WAIT), Ok(Event::Changed));
}
