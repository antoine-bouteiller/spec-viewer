//! The web app a daemon process serves, browsing a remote host.
mod support;

use std::{
    io::{Read, Write},
    net::TcpStream,
};

use support::{Host, repository, wait_connected, wait_until};

fn get(port: u16, target: &str, host: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(stream, "GET {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (response[9..12].parse().unwrap(), serde_json::from_str(body).unwrap_or_default())
}

#[test]
fn vc_6_the_gateway_serves_a_remote_document_and_lists_its_worktree() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let b = repository(&base.path().join("box/b"), None);
    local.write_user(&remote.host_entry("box"));
    let mut daemon = local.command(&["daemon"]).spawn().unwrap();
    wait_until("the socket", || local.runtime.socket().exists());
    let client = local.client();
    let port: u16 = client.web_url().unwrap().rsplit(':').next().unwrap().parse().unwrap();
    wait_connected(&client, "box");
    let host = format!("localhost:{port}");

    let (status, doc) = get(port, &format!("/api/doc?host=box&wt={}&path=.plan%2Fa.md", b.display()), &host);
    assert_eq!(status, 200, "{doc}");
    assert_eq!(doc["meta"]["title"], "Alpha");
    let (_, projects) = get(port, "/api/projects", &host);
    let worktrees = &projects["projects"][0]["worktrees"];
    assert_eq!(worktrees[0]["host"], "box", "{projects}");
    assert_eq!(worktrees[0]["path"], b.display().to_string());
    assert_eq!(get(port, "/api/projects", &format!("evil.example:{port}")).0, 403);
    daemon.kill().unwrap();
    daemon.wait().unwrap();
}
