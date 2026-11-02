use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use spec_viewer_config::Config;
use spec_viewer_daemon::{Daemon, Options};
use spec_viewer_protocol::Runtime;
use spec_viewer_render_web::Backend;

fn repository() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-qb", "main"])
            .status()
            .unwrap()
            .success()
    );
    std::fs::create_dir(root.join(".plan")).unwrap();
    std::fs::write(
        root.join(".plan/a.md"),
        "---\ntitle: Alpha\nstatus: done\n---\n# Alpha\n\n## Why\n\nSee KD-1.\n\n## Acceptance criteria\n\n- [x] one\n- [ ] two\n",
    )
    .unwrap();
    (directory, root)
}

/// A gateway on an in-process daemon whose user config is `user`, if any.
struct Gateway {
    address: SocketAddr,
    daemon: Arc<Daemon>,
    _base: tempfile::TempDir,
}

fn start(user: Option<&str>) -> Gateway {
    let base = tempfile::tempdir().unwrap();
    let user_config = user.map(|source| {
        let file = base.path().join("config.toml");
        std::fs::write(&file, source).unwrap();
        file
    });
    let daemon = Daemon::new(&Options {
        runtime: Runtime::at(base.path().join("run")),
        user_config,
        known: None,
        idle_exit: None,
    })
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let held = Arc::clone(&daemon);
    let backend = Backend {
        client: daemon.connect().unwrap(),
        hold: Box::new(move || Box::new(held.hold())),
    };
    spec_viewer_render_web::start(listener, &Config::default(), backend).unwrap();
    Gateway {
        address,
        daemon,
        _base: base,
    }
}

fn get(address: SocketAddr, target: &str) -> (u16, String) {
    get_as(address, target, &format!("localhost:{}", address.port()))
}

fn get_as(address: SocketAddr, target: &str, host: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(stream, "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").map(|(_, body)| body.to_owned()).unwrap_or_default();
    (status, body)
}

#[test]
fn serves_client_api_and_confines_reads() {
    let (_directory, root) = repository();
    let address = start(None).address;
    let wt = root.display();

    let (status, index) = get(address, "/");
    assert_eq!(status, 200);
    assert!(index.contains("src=\"/app.js\""));
    let (status, theme) = get(address, "/theme.css");
    assert_eq!(status, 200);
    assert!(theme.contains("--tone-0: var(--ansi-yellow);"), "{theme}");
    assert!(theme.contains(":root[data-theme='dark']"));
    assert!(theme.contains(".hl-keyword {\n color: var(--ansi-magenta);"), "{theme}");
    assert!(!theme.contains("#abcd"), "{theme}");
    assert!(theme.contains("--code-bg: #1f1f22;"), "{theme}");

    let (_, keys) = get(address, "/api/keys");
    let keys: serde_json::Value = serde_json::from_str(&keys).unwrap();
    assert_eq!(
        keys["main"]["search"][1],
        serde_json::json!({ "key": "k", "ctrl": true, "alt": false, "shift": false, "meta": false, "label": "C-k" })
    );
    assert_eq!(keys["main"]["up"][0]["key"], "ArrowUp");
    assert_eq!(keys["main"]["switch_tab"], serde_json::json!([]));
    assert_eq!(keys["search_dialog"]["close"][0]["key"], "Escape");

    let (_, docs) = get(address, &format!("/api/docs?wt={wt}"));
    let docs: serde_json::Value = serde_json::from_str(&docs).unwrap();
    assert_eq!(docs["tabs"][0]["name"], "plans");
    assert_eq!(docs["tabs"][0]["nodes"][0]["name"], "Alpha");
    assert_eq!(docs["tabs"][0]["nodes"][0]["status"]["name"], "done");
    assert_eq!(docs["tabs"][0]["nodes"][0]["status"]["known"], true);

    // Opening a worktree adds its project to the known ones.
    let (_, projects) = get(address, "/api/projects");
    let projects: serde_json::Value = serde_json::from_str(&projects).unwrap();
    assert_eq!(projects["projects"][0]["worktrees"][0]["path"], wt.to_string());

    let (_, search) = get(address, &format!("/api/search?wt={wt}&q=alp"));
    let search: serde_json::Value = serde_json::from_str(&search).unwrap();
    assert_eq!(
        search,
        serde_json::json!({ "total": 1, "hits": [{
            "tab": "plans", "name": "Alpha", "path": ".plan/a.md", "archived": false,
            "status": docs["tabs"][0]["nodes"][0]["status"], "titleMatches": [0, 1, 2], "pathMatches": [],
        }] })
    );
    assert_eq!(get(address, &format!("/api/search?wt={wt}&q=zzz")).1, r#"{"total":1,"hits":[]}"#);
    assert_eq!(get(address, "/api/search?wt=/&q=a").0, 403);

    let (status, doc) = get(address, &format!("/api/doc?wt={wt}&path=.plan%2Fa.md"));
    assert_eq!(status, 200);
    let doc: serde_json::Value = serde_json::from_str(&doc).unwrap();
    assert_eq!(doc["rootPath"], ".plan/a.md");
    assert_eq!(doc["meta"]["title"], "Alpha");
    assert_eq!(
        doc["meta"]["chips"][0],
        serde_json::json!({ "kind": "checklist", "label": "AC", "done": 1, "total": 2 })
    );
    assert_eq!(doc["toc"][0]["id"], "why");
    assert!(doc["html"].as_str().unwrap().contains("data-ref=\"KD-1\""));

    assert_eq!(get(address, &format!("/api/doc?wt={wt}&path=../a.md")).0, 403);
    assert_eq!(get(address, &format!("/api/doc?wt={wt}&path=.plan/missing.md")).0, 404);
    assert_eq!(get(address, "/api/docs?wt=/").0, 403);
    assert_eq!(get(address, "/missing").0, 404);
}

#[test]
fn events_stream_a_reload_when_a_plan_changes() {
    let (_directory, root) = repository();
    let gateway = start(None);
    let address = gateway.address;
    let since = Instant::now();
    assert!(gateway.daemon.idle(since).is_some());
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        stream,
        "GET /api/events?wt={} HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
        root.display(),
        address.port()
    )
    .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.starts_with("HTTP/1.1 200"), "{line}");
    while line != "\r\n" {
        line.clear();
        reader.read_line(&mut line).unwrap();
    }
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(root.join(".plan/b.md"), "# Beta\n").unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(line, "data: reload\n");
    assert_eq!(gateway.daemon.idle(since), None, "vc_2: an open events stream keeps the daemon up");
    drop(reader);
    let closed = Instant::now();
    while gateway.daemon.idle(since).is_none() {
        assert!(
            closed.elapsed() < Duration::from_secs(3),
            "vc_2: a closed stream releases the daemon within 3 s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn vc_6_requests_naming_another_host_are_forbidden() {
    let (_directory, root) = repository();
    let address = start(None).address;
    let port = address.port();
    let target = format!("/api/docs?wt={}", root.display());
    for host in [format!("127.0.0.1:{port}"), format!("[::1]:{port}"), format!("LOCALHOST:{port}")] {
        assert_eq!(get_as(address, &target, &host).0, 200, "{host}");
    }
    for host in [
        format!("evil.example:{port}"),
        "localhost".into(),
        format!("localhost:{}", port.wrapping_add(1)),
    ] {
        assert_eq!(get_as(address, &target, &host), (403, "forbidden".into()), "{host}");
    }
}

#[test]
fn vc_6_an_unreachable_host_is_a_bad_gateway() {
    let (_directory, root) = repository();
    let address = start(Some("[web]\nport = 0\n[[host]]\nname = 'box'\ncommand = ['false']\n")).address;
    let (status, body) = get(address, &format!("/api/docs?host=box&wt={}", root.display()));
    assert_eq!(status, 502);
    assert!(body.starts_with("box: "), "{body}");
    let (_, projects) = get(address, "/api/projects");
    let projects: serde_json::Value = serde_json::from_str(&projects).unwrap();
    assert_eq!(projects["unreachable"][0]["host"], "box");
    assert!(!projects["unreachable"][0]["error"].as_str().unwrap().is_empty());
    assert_eq!(get(address, &format!("/api/docs?host=elsewhere&wt={}", root.display())).0, 404);
}

#[test]
fn status_tones_follow_the_worktree_config() {
    let (_directory, root) = repository();
    std::fs::write(root.join(".spec-viewer.toml"), "[[status]]\nname = 'done'\ncolor = 'magenta'\n").unwrap();
    let address = start(None).address;
    let wt = root.display();
    let (_, theme) = get(address, &format!("/theme.css?wt={wt}"));
    assert!(theme.contains("--tone-0: var(--ansi-magenta);"), "{theme}");
    let (_, docs) = get(address, &format!("/api/docs?wt={wt}"));
    let docs: serde_json::Value = serde_json::from_str(&docs).unwrap();
    assert_eq!(
        docs["tabs"][0]["nodes"][0]["status"]["tone"],
        "var(--tone-0, var(--status-unknown))"
    );
    let (_, theme) = get(address, "/theme.css");
    assert!(
        theme.contains("--tone-0: var(--ansi-yellow);"),
        "the daemon's own statuses: {theme}"
    );
}
