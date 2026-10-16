#[path = "support/fixture.rs"]
mod fixture;
#[path = "parity/html.rs"]
mod html;

use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader},
    path::Path,
    process::{Child, ChildStdout, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use fixture::Fixture;
use serde_json::Value;

struct Server {
    child: Child,
    origin: String,
    _config: tempfile::TempDir,
    _output: Option<BufReader<ChildStdout>>,
}

impl Server {
    fn start(mut command: Command, root: &Path) -> Self {
        let config = tempfile::tempdir().unwrap();
        for (name, _value) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("PLAN_VIEWER_") {
                command.env_remove(name);
            }
        }
        let mut child = command
            .arg(root)
            .args(["--port", "0"])
            .env("PLAN_VIEWER_CONFIG", config.path().join("viewers.json"))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        let mut server = Self {
            child,
            origin: String::new(),
            _config: config,
            _output: None,
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = output.read_line(&mut line);
            let _ = sender.send((result, line, output));
        });
        let (result, line, output) = receiver.recv_timeout(Duration::from_secs(30)).expect("server announce timeout");
        result.unwrap();
        assert!(line.ends_with('\n'), "incomplete announce: {line:?}");
        server.origin = line
            .strip_prefix("plan-viewer listening on ")
            .unwrap_or_else(|| panic!("invalid announce: {line:?}"))
            .trim_end()
            .to_owned();
        server._output = Some(output);
        server
    }

    fn response(&self, route: &str, query: &[(&str, &str)]) -> (u16, String) {
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(30)))
                .build(),
        );
        let mut request = agent.get(format!("{}{route}", self.origin));
        for (name, value) in query {
            request = request.query(name, value);
        }
        let mut response = request
            .call()
            .unwrap_or_else(|error| panic!("{}{route} {query:?}: {error}", self.origin));
        (response.status().as_u16(), response.body_mut().read_to_string().unwrap())
    }

    fn json(&self, route: &str, query: &[(&str, &str)]) -> Value {
        let (status, body) = self.response(route, query);
        assert_eq!(status, 200, "{}{route} {query:?}: {body}", self.origin);
        serde_json::from_str(&body).unwrap()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn json_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(expected_fields), Value::Object(actual_fields)) => {
            let keys: BTreeSet<_> = expected_fields.keys().chain(actual_fields.keys()).collect();
            for key in keys {
                match (expected_fields.get(key), actual_fields.get(key)) {
                    (Some(expected_value), Some(actual_value)) => {
                        if let Some(diff) = json_difference(expected_value, actual_value, &format!("{path}.{key}")) {
                            return Some(diff);
                        }
                    }
                    (expected_value, actual_value) => {
                        return Some(format!("{path}.{key}\nBun: {expected_value:?}\nRust: {actual_value:?}"));
                    }
                }
            }
        }
        (Value::Array(expected_values), Value::Array(actual_values)) if expected_values.len() == actual_values.len() => {
            for (index, (expected_value, actual_value)) in expected_values.iter().zip(actual_values).enumerate() {
                if let Some(diff) = json_difference(expected_value, actual_value, &format!("{path}[{index}]")) {
                    return Some(diff);
                }
            }
        }
        _ if expected != actual => return Some(format!("{path}\nBun:  {expected:#}\nRust: {actual:#}")),
        _ => {}
    }
    None
}

fn assert_json(expected: &Value, actual: &Value, context: &str) {
    if let Some(diff) = json_difference(expected, actual, "json") {
        panic!("{context}: {diff}");
    }
}

fn collect_paths(value: &Value, paths: &mut BTreeSet<String>) {
    match value {
        Value::Array(nodes) => {
            for node in nodes {
                collect_paths(node, paths);
            }
        }
        Value::Object(node) => {
            if let Some(path) = node.get("path") {
                paths.insert(path.as_str().expect("document path must be a string").to_owned());
            }
            if let Some(children) = node.get("children") {
                collect_paths(children, paths);
            }
        }
        _ => panic!("unexpected document tree node: {value}"),
    }
}

#[test]
fn vc_1_bun_and_rust_http_documents_and_confinement_match() {
    let fixture = Fixture::new();
    let mut bun_command = Command::new("bun");
    bun_command
        .arg("src/server.ts")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
    let bun = Server::start(bun_command, &fixture.root);
    let mut rust_command = Command::new(env!("CARGO_BIN_EXE_plan-viewer"));
    rust_command.arg("serve");
    let rust = Server::start(rust_command, &fixture.root);
    assert_json(&bun.json("/api/projects", &[]), &rust.json("/api/projects", &[]), "/api/projects");

    for (name, worktree) in [("root", &fixture.root), ("sibling", &fixture.sibling)] {
        let worktree = worktree.to_str().unwrap();
        let expected = bun.json("/api/docs", &[("wt", worktree)]);
        let actual = rust.json("/api/docs", &[("wt", worktree)]);
        assert_json(&expected, &actual, &format!("{name} /api/docs"));
        let mut paths = BTreeSet::new();
        for docs in [&expected, &actual] {
            for tree in ["plans", "specs"] {
                collect_paths(&docs[tree], &mut paths);
            }
        }
        assert!(!paths.is_empty(), "{name}: empty corpus");
        let mut byte_equal = 0;
        let mut normalized = 0;
        for path in &paths {
            let query = [("wt", worktree), ("path", path.as_str())];
            let mut expected = bun.json("/api/doc", &query);
            let mut actual = rust.json("/api/doc", &query);
            let expected_html = expected.as_object_mut().unwrap().remove("html").unwrap();
            let actual_html = actual.as_object_mut().unwrap().remove("html").unwrap();
            assert_json(&expected, &actual, &format!("{name} {path}"));
            let expected_html = expected_html.as_str().unwrap();
            let actual_html = actual_html.as_str().unwrap();
            html::compare(expected_html, actual_html).unwrap_or_else(|diff| panic!("{name} {path}: {diff}"));
            if expected_html == actual_html {
                byte_equal += 1;
            } else {
                normalized += 1;
            }
        }
        eprintln!(
            "VC-1 {name}: {} documents, {byte_equal} byte-equal HTML, {normalized} normalized HTML",
            paths.len()
        );

        for (path, status, body) in [
            ("../x.spec.md", 403, "forbidden"),
            ("spec/escape.spec.md", 403, "forbidden"),
            ("README.md", 403, "forbidden"),
            (".plan/x.txt", 403, "forbidden"),
            ("spec/missing.spec.md", 404, "not found"),
        ] {
            let query = [("wt", worktree), ("path", path)];
            for (server_name, server) in [("Bun", &bun), ("Rust", &rust)] {
                assert_eq!(
                    server.response("/api/doc", &query),
                    (status, body.to_owned()),
                    "{server_name} {name} {path}"
                );
            }
        }
    }
    for route in ["/api/docs", "/api/doc"] {
        for (server_name, server) in [("Bun", &bun), ("Rust", &rust)] {
            assert_eq!(
                server.response(
                    route,
                    &[("wt", fixture.outside.to_str().unwrap()), ("path", "spec/standalone.spec.md")]
                ),
                (403, "forbidden".to_owned()),
                "{server_name} {route}: unknown worktree"
            );
        }
    }
}
