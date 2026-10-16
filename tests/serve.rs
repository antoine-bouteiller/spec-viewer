use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Server {
    child: Child,
    origin: String,
    output: BufReader<std::process::ChildStdout>,
}

impl Server {
    fn start(root: &std::path::Path, config: &std::path::Path, arguments: &[&str], environment: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_plan-viewer"));
        command.arg("serve").arg(root).args(arguments).env("PLAN_VIEWER_CONFIG", config);
        for name in ["PLAN_VIEWER_BIND", "PLAN_VIEWER_CERT", "PLAN_VIEWER_KEY", "PLAN_VIEWER_MANAGED"] {
            command.env_remove(name);
        }
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = output.read_line(&mut line);
            let _ = sender.send((result, line, output));
        });
        let (result, line, output) = match receiver.recv_timeout(Duration::from_secs(10)) {
            Ok(announced) => announced,
            Err(error) => {
                child.kill().unwrap();
                panic!("announce timeout: {error}");
            }
        };
        result.unwrap();
        if !line.starts_with("plan-viewer listening on ") {
            let mut diagnostic = String::new();
            child.stderr.take().unwrap().read_to_string(&mut diagnostic).unwrap();
            panic!("missing announce: {line:?}; {diagnostic}");
        }
        assert!(line.ends_with('\n'));
        assert_eq!(line.lines().count(), 1);
        let origin = line.strip_prefix("plan-viewer listening on ").unwrap().trim_end().to_owned();
        Self { child, origin, output }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn vc_2_missing_root_exits_with_one_stderr_line() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing");
    for (root, message) in [
        (None, "root directory is required".to_owned()),
        (Some(&missing), format!("cannot resolve root: {}", missing.display())),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_plan-viewer"));
        command
            .arg("serve")
            .env("PLAN_VIEWER_CONFIG", directory.path().join("viewers.json"));
        if let Some(root) = root {
            command.arg(root);
        }
        let output = command.output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, b"");
        assert_eq!(String::from_utf8(output.stderr).unwrap(), format!("plan-viewer: {message}\n"));
    }
}

#[test]
fn vc_2_stdout_is_only_the_announce_line() {
    let directory = tempfile::tempdir().unwrap();
    let mut server = Server::start(directory.path(), &directory.path().join("viewers.json"), &[], &[]);
    assert!(server.origin.starts_with("http://127.0.0.1:"));
    assert_eq!(ureq::get(format!("{}/api/projects", server.origin)).call().unwrap().status(), 200);
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    let mut remainder = String::new();
    server.output.read_to_string(&mut remainder).unwrap();
    assert_eq!(remainder, "");
}

fn wait_for_exit(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(std::time::Instant::now() < deadline, "server did not exit within {timeout:?}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn vc_2_managed_dead_holders_exit_and_preserve_the_port() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let config = root.join("viewers.json");
    let mut server = Server::start(&root, &config, &[], &[("PLAN_VIEWER_MANAGED", "1")]);
    let mut dead_child = Command::new("true").spawn().unwrap();
    let dead_pid = dead_child.id();
    dead_child.wait().unwrap();
    let port: u16 = server.origin.rsplit_once(':').unwrap().1.parse().unwrap();
    let entry = serde_json::json!({"viewers": {root.to_str().unwrap(): {
        "holders": [dead_pid], "port": port, "pid": server.child.id(), "url": server.origin
    }}});
    std::fs::write(&config, serde_json::to_vec(&entry).unwrap()).unwrap();
    assert_eq!(wait_for_exit(&mut server.child, Duration::from_secs(10)).code(), Some(0));
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(
        saved["viewers"][root.to_str().unwrap()],
        serde_json::json!({"holders": [], "port": port})
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        format!(
            "{{\"viewers\":{{{}:{{\"holders\":[],\"port\":{port}}}}}}}",
            serde_json::to_string(root.to_str().unwrap()).unwrap()
        )
    );
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(3)))
            .build(),
    )
}

#[test]
fn vc_2_taken_port_falls_back_to_an_os_assigned_port() {
    let directory = tempfile::tempdir().unwrap();
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port().to_string();
    let server = Server::start(directory.path(), &directory.path().join("viewers.json"), &["--port", &port], &[]);
    assert_ne!(server.origin.rsplit_once(':').unwrap().1, port);
    assert_eq!(agent().get(format!("{}/api/projects", server.origin)).call().unwrap().status(), 200);
}

#[test]
fn bind_literals_and_hostnames_announce_buns_origin() {
    let directory = tempfile::tempdir().unwrap();
    for (bind, host) in [
        ("127.0.0.1", "127.0.0.1"),
        ("localhost", "0.0.0.0"),
        ("::1", "[::1]"),
        ("::ffff:127.0.0.1", "[::ffff:7f00:1]"),
        ("", "127.0.0.1"),
    ] {
        let server = Server::start(
            directory.path(),
            &directory.path().join("viewers.json"),
            &[],
            &[("PLAN_VIEWER_BIND", bind)],
        );
        assert!(server.origin.starts_with(&format!("http://{host}:")), "{bind}: {}", server.origin);
    }
}

#[test]
fn vc_3_bundled_app_and_every_referenced_asset_are_available() {
    let directory = tempfile::tempdir().unwrap();
    let server = Server::start(directory.path(), &directory.path().join("viewers.json"), &[], &[]);
    let mut page = agent().get(format!("{}/", server.origin)).call().unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(page.headers()["content-type"], "text/html;charset=utf-8");
    assert_eq!(page.headers()["cache-control"], "no-cache");
    let etag = page.headers()["etag"].to_str().unwrap().to_owned();
    assert_eq!(
        agent()
            .get(format!("{}/", server.origin))
            .header("if-none-match", &etag)
            .call()
            .unwrap()
            .status(),
        304
    );
    let html = page.body_mut().read_to_string().unwrap();
    assert!(html.contains("id=\"root\""));
    let mut referenced = 0;
    for attribute in ["src=\"", "href=\""] {
        for fragment in html.split(attribute).skip(1) {
            let reference = fragment.split('"').next().unwrap();
            if reference.starts_with("data:") {
                continue;
            }
            let mut asset = agent().get(format!("{}{}", server.origin, reference)).call().unwrap();
            assert_eq!(asset.status(), 200, "{reference}");
            let expected = if reference.ends_with(".js") {
                "text/javascript;charset=utf-8"
            } else {
                "text/css;charset=utf-8"
            };
            assert_eq!(asset.headers()["content-type"], expected);
            assert_eq!(asset.headers()["cache-control"], "public, max-age=31536000, immutable");
            let etag = asset.headers()["etag"].to_str().unwrap().to_owned();
            assert_eq!(
                agent()
                    .get(format!("{}{}", server.origin, reference))
                    .header("if-none-match", &etag)
                    .call()
                    .unwrap()
                    .status(),
                304
            );
            assert!(!asset.body_mut().read_to_string().unwrap().is_empty());
            assert_eq!(
                agent()
                    .post(format!("{}{}", server.origin, reference))
                    .send_empty()
                    .unwrap()
                    .status(),
                200
            );
            referenced += 1;
        }
    }
    assert!(referenced >= 2);
    let mut mermaid = agent()
        .get(format!("{}/assets/mermaid/dist/mermaid.esm.min.mjs", server.origin))
        .call()
        .unwrap();
    assert_eq!(mermaid.status(), 200);
    assert_eq!(mermaid.headers()["content-type"], "text/javascript; charset=utf-8");
    assert!(mermaid.body_mut().read_to_string().unwrap().contains("export"));
    let chunk_directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../node_modules/mermaid/dist/chunks/mermaid.esm.min");
    let chunk = std::fs::read_dir(chunk_directory)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|extension| extension == "mjs"))
        .unwrap();
    assert_eq!(
        agent()
            .get(format!(
                "{}/assets/mermaid/dist/chunks/mermaid.esm.min/{}",
                server.origin,
                chunk.file_name().to_string_lossy()
            ))
            .call()
            .unwrap()
            .status(),
        200
    );
}

#[test]
fn routes_enforce_worktree_and_document_confinement_and_error_bodies() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    std::fs::create_dir(root.join(".plan")).unwrap();
    std::fs::write(root.join(".plan/alpha.md"), "# Alpha\n\n## Why\n\nA simple paragraph.\n").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.md"), "secret").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret.md"), root.join(".plan/escape.md")).unwrap();
    let server = Server::start(&root, &root.join("viewers.json"), &[], &[]);
    for route in ["docs", "doc", "events"] {
        for worktree in ["", "/unknown/worktree"] {
            let mut response = agent()
                .get(format!("{}/api/{route}", server.origin))
                .query("wt", worktree)
                .call()
                .unwrap();
            assert_eq!(response.status(), 403, "{route} {worktree}");
            assert_eq!(response.headers()["content-type"], "text/plain;charset=utf-8");
            assert_eq!(response.body_mut().read_to_string().unwrap(), "forbidden");
        }
    }
    for (path, status, body) in [
        ("..", 403, "forbidden"),
        ("README.md", 403, "forbidden"),
        ("/absolute.spec.md", 403, "forbidden"),
        (".plan/escape.md", 403, "forbidden"),
        (".plan/missing.md", 404, "not found"),
        ("\0.spec.md", 403, "forbidden"),
    ] {
        let mut response = agent()
            .get(format!("{}/api/doc", server.origin))
            .query("wt", root.to_str().unwrap())
            .query("path", path)
            .call()
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(response.body_mut().read_to_string().unwrap(), body);
    }
    let mut response = agent()
        .get(format!("{}/api/doc", server.origin))
        .query("wt", root.to_str().unwrap())
        .query("path", ".plan/alpha.md")
        .call()
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/json;charset=utf-8");
    let document: serde_json::Value = serde_json::from_str(&response.body_mut().read_to_string().unwrap()).unwrap();
    assert_eq!(document["meta"]["title"], "Alpha");
    assert_eq!(document["path"], root.join(".plan/alpha.md").to_str().unwrap());
    assert_eq!(document["rootPath"], ".plan/alpha.md");
    assert_eq!(document["hasMermaid"], false);
    assert_eq!(document["degraded"], serde_json::Value::Null);
    assert_eq!(document["toc"], serde_json::json!([{"id":"why","text":"Why","depth":2}]));
    assert!(document["html"].as_str().unwrap().contains("<p>A simple paragraph.</p>"));
    for (path, status, body) in [
        ("/unknown", 404, "not found"),
        ("/assets/mermaid/package.json", 404, "not found"),
        ("/assets/mermaid/", 403, "forbidden"),
        ("/assets/mermaid/missing.mjs", 404, "not found"),
    ] {
        let mut response = agent().get(format!("{}{path}", server.origin)).call().unwrap();
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(response.body_mut().read_to_string().unwrap(), body);
    }
    let head = agent().head(format!("{}/unknown", server.origin)).call().unwrap();
    assert_eq!(head.status(), 404);
    assert!(!head.headers().contains_key("content-type"));
    assert_eq!(head.headers()["content-length"], "9");
    assert_eq!(
        agent()
            .post(format!("{}/assets/mermaid/dist/mermaid.esm.min.mjs", server.origin))
            .send_empty()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        agent()
            .post(format!("{}/api/projects", server.origin))
            .send_empty()
            .unwrap()
            .status(),
        200
    );
}

#[test]
fn vc_6_events_stream_emits_a_reload_after_a_plan_write() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    std::fs::create_dir(root.join(".plan")).unwrap();
    let server = Server::start(&root, &root.join("viewers.json"), &[], &[]);
    let mut connection = std::net::TcpStream::connect(server.origin.strip_prefix("http://").unwrap()).unwrap();
    connection.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    use std::io::Write;
    write!(
        connection,
        "GET /api/events?wt={} HTTP/1.1\r\nHost: localhost\r\n\r\n",
        root.to_str().unwrap()
    )
    .unwrap();
    let mut stream = BufReader::new(connection);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        stream.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        headers.push_str(&line);
    }
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert!(headers.contains("content-type: text/event-stream\r\n"));
    assert!(headers.contains("cache-control: no-cache\r\n"));
    std::fs::write(root.join(".plan/new.md"), "# New\n").unwrap();
    let mut received = String::new();
    while !received.contains("data: reload\n\n") {
        let mut line = String::new();
        assert!(stream.read_line(&mut line).unwrap() > 0);
        received.push_str(&line);
    }
}

#[test]
fn certificate_and_key_must_be_set_together() {
    let directory = tempfile::tempdir().unwrap();
    for (name, other) in [("PLAN_VIEWER_CERT", "PLAN_VIEWER_KEY"), ("PLAN_VIEWER_KEY", "PLAN_VIEWER_CERT")] {
        let output = Command::new(env!("CARGO_BIN_EXE_plan-viewer"))
            .arg("serve")
            .arg(directory.path())
            .env("PLAN_VIEWER_CONFIG", directory.path().join("viewers.json"))
            .env(name, "missing.pem")
            .env_remove(other)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(output.stdout, b"");
        assert_eq!(
            output.stderr,
            b"plan-viewer: PLAN_VIEWER_CERT and PLAN_VIEWER_KEY must be set together\n"
        );
    }
}

#[test]
fn managed_signals_clear_only_the_owned_run() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let config = root.join("viewers.json");
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let mut server = Server::start(&root, &config, &[], &[("PLAN_VIEWER_MANAGED", "1")]);
        let port: u16 = server.origin.rsplit_once(':').unwrap().1.parse().unwrap();
        let entry = serde_json::json!({"viewers": {root.to_str().unwrap(): {
            "holders": [std::process::id()], "port": port, "pid": server.child.id(), "url": server.origin
        }}});
        std::fs::write(&config, serde_json::to_vec(&entry).unwrap()).unwrap();
        assert_eq!(unsafe { libc::kill(server.child.id() as i32, signal) }, 0);
        assert_eq!(wait_for_exit(&mut server.child, Duration::from_secs(3)).code(), Some(0));
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert_eq!(
            saved["viewers"][root.to_str().unwrap()],
            serde_json::json!({"holders": [], "port": port})
        );
    }
}

#[test]
fn tls_serves_https_with_pem_certificate_and_key() {
    let directory = tempfile::tempdir().unwrap();
    let cert = directory.path().join("cert.pem");
    let key = directory.path().join("key.pem");
    let config = directory.path().join("openssl.cnf");
    std::fs::write(&config, "[req]\ndistinguished_name=dn\nx509_extensions=v3\n[dn]\n[v3]\nbasicConstraints=CA:FALSE\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n").unwrap();
    let certificate = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-keyout",
        ])
        .arg(&key)
        .arg("-out")
        .arg(&cert)
        .arg("-config")
        .arg(&config)
        .output()
        .unwrap();
    assert!(certificate.status.success(), "{}", String::from_utf8_lossy(&certificate.stderr));
    let server = Server::start(
        directory.path(),
        &directory.path().join("viewers.json"),
        &[],
        &[
            ("PLAN_VIEWER_CERT", cert.to_str().unwrap()),
            ("PLAN_VIEWER_KEY", key.to_str().unwrap()),
        ],
    );
    assert!(server.origin.starts_with("https://127.0.0.1:"));
    let response = Command::new("curl")
        .args(["--noproxy", "*", "--max-time", "3", "--http1.1", "-ksS", "-D", "-"])
        .arg(format!("{}/api/projects", server.origin))
        .output()
        .unwrap();
    assert!(response.status.success(), "{}", String::from_utf8_lossy(&response.stderr));
    let response = String::from_utf8(response.stdout).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert!(headers.contains("content-type: application/json;charset=utf-8"));
    let project: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(project["projects"].as_array().unwrap().len(), 1);
}

#[test]
fn v1_certificate_fails_startup_with_one_stderr_line_without_panicking() {
    let directory = tempfile::tempdir().unwrap();
    let cert = directory.path().join("v1.pem");
    let key = directory.path().join("key.pem");
    let config = directory.path().join("openssl.cnf");
    std::fs::write(&config, "[req]\ndistinguished_name=dn\n[dn]\n").unwrap();
    assert!(
        Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-keyout"
            ])
            .arg(&key)
            .arg("-out")
            .arg(&cert)
            .arg("-config")
            .arg(config)
            .output()
            .unwrap()
            .status
            .success()
    );
    let certificate = Command::new("openssl")
        .args(["x509", "-noout", "-text", "-in"])
        .arg(&cert)
        .output()
        .unwrap();
    assert!(String::from_utf8(certificate.stdout).unwrap().contains("Version: 1"));
    let output = Command::new(env!("CARGO_BIN_EXE_plan-viewer"))
        .arg("serve")
        .arg(directory.path())
        .env("PLAN_VIEWER_CONFIG", directory.path().join("viewers.json"))
        .env("PLAN_VIEWER_CERT", cert)
        .env("PLAN_VIEWER_KEY", key)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stdout, b"");
    let diagnostic = String::from_utf8(output.stderr).unwrap();
    assert_eq!(diagnostic.lines().count(), 1);
    assert!(diagnostic.starts_with("plan-viewer: "));
    assert!(diagnostic.contains("UnsupportedCertVersion"));
}

#[test]
fn managed_live_holders_keep_running_and_replaced_ownership_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let config = root.join("viewers.json");
    let mut server = Server::start(&root, &config, &[], &[("PLAN_VIEWER_MANAGED", "1")]);
    let port: u16 = server.origin.rsplit_once(':').unwrap().1.parse().unwrap();
    let mut entry = serde_json::json!({"viewers": {root.to_str().unwrap(): {
        "holders": [std::process::id()], "port": port, "pid": server.child.id(), "url": server.origin
    }}});
    std::fs::write(&config, serde_json::to_vec(&entry).unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(5500));
    assert!(server.child.try_wait().unwrap().is_none());
    entry["viewers"][root.to_str().unwrap()]["pid"] = serde_json::json!(std::process::id());
    std::fs::write(&config, serde_json::to_vec(&entry).unwrap()).unwrap();
    assert_eq!(wait_for_exit(&mut server.child, Duration::from_secs(6)).code(), Some(0));
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(saved, entry);
}

#[test]
fn managed_missing_entry_exits_and_xdg_config_home_is_used() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let config_home = root.join("config");
    let mut server = Server::start(
        &root,
        &root.join("unused.json"),
        &[],
        &[
            ("PLAN_VIEWER_MANAGED", "1"),
            ("PLAN_VIEWER_CONFIG", ""),
            ("XDG_CONFIG_HOME", config_home.to_str().unwrap()),
        ],
    );
    assert_eq!(wait_for_exit(&mut server.child, Duration::from_secs(10)).code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(config_home.join("plan-viewer/viewers.json")).unwrap(),
        "{\"viewers\":{}}"
    );
    assert!(!root.join("unused.json").exists());
}
