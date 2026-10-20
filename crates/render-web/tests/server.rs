use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use spec_viewer_config::Config;
use spec_viewer_core::Library;

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

fn start(root: &Path, preferred: Option<u16>) -> SocketAddr {
    spec_viewer_render_web::start(Library::new(root, Default::default()).unwrap(), &Config::default(), preferred).unwrap()
}

fn get(address: SocketAddr, target: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(stream, "GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").map(|(_, body)| body.to_owned()).unwrap_or_default();
    (status, body)
}

#[test]
fn serves_client_api_and_confines_reads() {
    let (_directory, root) = repository();
    let address = start(&root, None);
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

    let (_, projects) = get(address, "/api/projects");
    let projects: serde_json::Value = serde_json::from_str(&projects).unwrap();
    assert_eq!(projects["projects"][0]["worktrees"][0]["path"], wt.to_string());

    let (_, docs) = get(address, &format!("/api/docs?wt={wt}"));
    let docs: serde_json::Value = serde_json::from_str(&docs).unwrap();
    assert_eq!(docs["folders"][0]["name"], "plans");
    assert_eq!(docs["folders"][0]["nodes"][0]["name"], "Alpha");
    assert_eq!(docs["folders"][0]["nodes"][0]["status"]["name"], "done");
    assert_eq!(docs["folders"][0]["nodes"][0]["status"]["known"], true);

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
fn the_preferred_port_is_used_unless_taken() {
    let (_directory, root) = repository();
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let first = start(&root, Some(free));
    assert_eq!(first.port(), free);
    assert_eq!(start(&root, Some(first.port())).ip(), first.ip());
    assert_ne!(start(&root, Some(first.port())).port(), first.port());
}

#[test]
fn events_stream_a_reload_when_a_plan_changes() {
    let (_directory, root) = repository();
    let address = start(&root, None);
    let mut stream = TcpStream::connect(address).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(stream, "GET /api/events?wt={} HTTP/1.1\r\nHost: localhost\r\n\r\n", root.display()).unwrap();
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
}

#[test]
fn serves_https_on_all_interfaces_when_configured() {
    let (directory, root) = repository();
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let (cert_file, key_file) = (directory.path().join("cert.pem"), directory.path().join("key.pem"));
    std::fs::write(&cert_file, cert.pem()).unwrap();
    std::fs::write(&key_file, signing_key.serialize_pem()).unwrap();
    let mut config = Config::default();
    config.web.bind = "0.0.0.0".into();
    config.web.cert = Some(cert_file);
    config.web.key = Some(key_file);
    let address = spec_viewer_render_web::start(Library::new(&root, Default::default()).unwrap(), &config, None).unwrap();
    assert!(address.ip().is_unspecified());

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert.der().clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connection = rustls::ClientConnection::new(Arc::new(client), "localhost".try_into().unwrap()).unwrap();
    let tcp = TcpStream::connect(("127.0.0.1", address.port())).unwrap();
    let mut stream = rustls::StreamOwned::new(connection, tcp);
    write!(stream, "GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("src=\"/app.js\""));
}
