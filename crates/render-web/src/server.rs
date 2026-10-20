//! HTTP(S) server: the embedded client, the JSON API, and server-sent reload events.
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::RecvTimeoutError},
    time::Duration,
};

use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use serde::Serialize;
use spec_viewer_config::{Color, Config, Mode, WebSettings};
use spec_viewer_core::{ChipValue, DocError, DocNode, Document, Library, Status, TocEntry};

use crate::html::{Html, to_html};

const INDEX: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const APP_CSS: &str = include_str!("../assets/app.css");
const HEARTBEAT: Duration = Duration::from_secs(15);
const HEAD_LIMIT: u64 = 16 * 1024;

/// Binds `<web.bind>:<preferred>`, else any free port, and serves on background threads until exit.
/// A `web.bind` that is not an IP binds all interfaces; `web.cert` and `web.key` enable TLS.
pub fn start(library: Library, config: &Config, preferred: Option<u16>) -> io::Result<SocketAddr> {
    let tls = tls(&config.web)?;
    // A hostname often resolves to loopback via /etc/hosts, which would defeat a remote bind.
    let ip = config.web.bind.parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let listener = preferred
        .and_then(|port| TcpListener::bind((ip, port)).ok())
        .map_or_else(|| TcpListener::bind((ip, 0)), Ok)?;
    let address = listener.local_addr()?;
    let state = Arc::new(State::new(library, config));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = Arc::clone(&state);
            let tls = tls.clone();
            std::thread::spawn(move || {
                if stream.set_read_timeout(Some(Duration::from_secs(5))).is_err() {
                    return;
                }
                let mut stream = match tls.map(ServerConnection::new) {
                    None => Conn::Plain(stream),
                    Some(Ok(connection)) => Conn::Tls(Box::new(StreamOwned::new(connection, stream))),
                    Some(Err(_)) => return,
                };
                if let Some(target) = request_target(&mut stream) {
                    state.handle(stream, &target);
                }
            });
        }
    });
    Ok(address)
}

fn tls(web: &WebSettings) -> io::Result<Option<Arc<ServerConfig>>> {
    let (Some(cert), Some(key)) = (&web.cert, &web.key) else {
        return Ok(None);
    };
    let invalid = |path: &Path, error: &dyn std::fmt::Display| io::Error::other(format!("{}: {error}", path.display()));
    let certs = CertificateDer::pem_file_iter(cert)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|error| invalid(cert, &error))?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|error| invalid(key, &error))?;
    let config = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .and_then(|builder| builder.with_no_client_auth().with_single_cert(certs, key))
        .map_err(|error| invalid(cert, &error))?;
    Ok(Some(Arc::new(config)))
}

enum Conn {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.read(buf),
            Self::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(stream) => stream.write(buf),
            Self::Tls(stream) => stream.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(stream) => stream.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        if let Self::Tls(stream) = self {
            stream.conn.send_close_notify();
            let _ = stream.flush();
        }
    }
}

/// Reads the request head and returns its target. Any method is served as GET and every response
/// closes the connection, so request bodies and keep-alive never need handling.
fn request_target(stream: &mut Conn) -> Option<String> {
    let mut reader = BufReader::new(stream.take(HEAD_LIMIT));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let target = line.split(' ').nth(1)?.to_owned();
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" || line == "\n" {
            return Some(target);
        }
    }
}

struct Reply {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: Vec<u8>,
}

impl Reply {
    fn send(self, stream: &mut Conn) {
        let reason = match self.status {
            200 => "OK",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Internal Server Error",
        };
        let mut head = format!(
            "HTTP/1.1 {} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.status,
            self.body.len()
        );
        for (name, value) in self.headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(&self.body))
            .and_then(|()| stream.flush());
    }
}

/// HTML per document file, reused while the core cache returns the same parse.
type Rendered = HashMap<PathBuf, (Arc<Document>, Arc<Html>)>;

struct State {
    library: Library,
    statuses: Vec<String>,
    theme_css: String,
    rendered: Mutex<Rendered>,
}

impl State {
    fn new(library: Library, config: &Config) -> Self {
        let statuses: Vec<String> = config.parse.statuses.iter().map(|status| status.name.clone()).collect();
        let mut theme_css = String::new();
        for (mode, dark) in [(Mode::Light, false), (Mode::Dark, true)] {
            let mut rules = String::new();
            // A mode whose palette lacks a status colour leaves those statuses at the unknown tone.
            let resolved = config.resolve(mode).ok();
            if let Some(resolved) = &resolved {
                for (index, name) in statuses.iter().enumerate() {
                    if let Some(color) = resolved.statuses.get(name) {
                        rules.push_str(&format!("--tone-{index}: {};\n", css_color(*color)));
                    }
                }
                // The design tokens behind ANSI slots are foreground colours; only a hex surface applies.
                if let Color::Rgb(..) = resolved.roles.code_bg {
                    rules.push_str(&format!("--code-bg: {};\n", css_color(resolved.roles.code_bg)));
                }
            }
            let ansi = resolved.map_or(spec_viewer_config::ANSI_SLOTS.map(|(_, slot)| Color::Ansi(slot)), |resolved| {
                resolved.ansi
            });
            rules.push_str(&crate::highlight::rules(&ansi.map(css_color)));
            let name = if dark { "dark" } else { "light" };
            theme_css.push_str(&format!(
                "@media (prefers-color-scheme: {name}) {{ :root:not([data-theme]) {{\n{rules}}} }}\n:root[data-theme='{name}'] {{\n{rules}}}\n"
            ));
        }
        Self {
            library,
            statuses,
            theme_css,
            rendered: Mutex::default(),
        }
    }

    fn handle(&self, mut stream: Conn, target: &str) {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let query = Query(query);
        if path == "/api/events" {
            return match self.worktree(&query) {
                Some(worktree) => self.events(&mut stream, &worktree),
                None => text(403, "forbidden").send(&mut stream),
            };
        }
        let reply = match path {
            "/" => asset(INDEX, "text/html; charset=utf-8"),
            "/app.js" => asset(APP_JS, "text/javascript; charset=utf-8"),
            "/app.css" => asset(APP_CSS, "text/css; charset=utf-8"),
            "/theme.css" => asset(&self.theme_css, "text/css; charset=utf-8"),
            "/api/projects" => self.projects(),
            "/api/docs" => self
                .worktree(&query)
                .map_or_else(|| text(403, "forbidden"), |worktree| self.docs(&worktree)),
            "/api/doc" => self.worktree(&query).map_or_else(
                || text(403, "forbidden"),
                |worktree| self.doc(&worktree, query.get("path").as_deref().unwrap_or("")),
            ),
            _ => text(404, "not found"),
        };
        reply.send(&mut stream);
    }

    /// Only a worktree the project currently lists may be read.
    fn worktree(&self, query: &Query) -> Option<PathBuf> {
        let wanted = std::fs::canonicalize(query.get("wt")?).ok()?;
        self.library
            .refresh_project()
            .worktrees
            .into_iter()
            .find(|worktree| worktree.path == wanted)
            .map(|worktree| worktree.path)
    }

    fn projects(&self) -> Reply {
        let project = self.library.refresh_project();
        json(&serde_json::json!({
            "projects": [{
                "name": project.name,
                "path": project.path,
                "worktrees": project.worktrees.iter().map(|worktree| serde_json::json!({
                    "name": worktree.name,
                    "main": worktree.main,
                    "path": worktree.path,
                })).collect::<Vec<_>>(),
            }],
        }))
    }

    fn docs(&self, worktree: &Path) -> Reply {
        let corpus = self.library.corpus(worktree);
        json(&DocsJson {
            changed_specs: corpus.changed_specs,
            folders: corpus
                .folders
                .iter()
                .map(|folder| FolderJson {
                    name: &folder.name,
                    nodes: folder.nodes.iter().map(|node| self.node(node)).collect(),
                })
                .collect(),
            project: corpus.project,
            specs: corpus.specs.iter().map(|node| self.node(node)).collect(),
        })
    }

    fn doc(&self, worktree: &Path, rel: &str) -> Reply {
        let opened = match self.library.open(worktree, rel) {
            Ok(opened) => opened,
            Err(DocError::Forbidden) => return text(403, "forbidden"),
            Err(DocError::NotFound) => return text(404, "not found"),
        };
        let html = {
            let mut rendered = self.rendered.lock().expect("render cache");
            match rendered.get(&opened.path) {
                Some((document, html)) if Arc::ptr_eq(document, &opened.document) => Arc::clone(html),
                _ => {
                    let html = Arc::new(to_html(&opened.document));
                    rendered.insert(opened.path.clone(), (Arc::clone(&opened.document), Arc::clone(&html)));
                    html
                }
            }
        };
        let meta = &opened.document.meta;
        json(&DocJson {
            path: &opened.path,
            root_path: &opened.root_path,
            meta: MetaJson {
                title: &meta.title,
                status: self.status(meta.status.as_ref()),
                author: meta.author.as_deref(),
                date: meta.date.as_deref(),
                related: &meta.related,
                chips: meta
                    .chips
                    .iter()
                    .map(|chip| match chip.value {
                        ChipValue::Checklist { done, total } => ChipJson::Checklist {
                            label: &chip.label,
                            done,
                            total,
                        },
                        ChipValue::Count(count) => ChipJson::Count { label: &chip.label, count },
                    })
                    .collect(),
                lines: meta.lines,
                legacy: meta.legacy,
                kind: meta.kind.as_deref(),
            },
            html: &html.html,
            toc: opened.document.toc.iter().map(TocJson::from).collect(),
            has_mermaid: html.has_mermaid,
            degraded: opened.document.degraded.as_ref().map(|degraded| DegradedJson {
                reason: &degraded.reason,
                message: &degraded.message,
                line: degraded.line,
            }),
        })
    }

    fn events(&self, stream: &mut Conn, worktree: &Path) {
        let watch = match self.library.watch(worktree) {
            Ok(watch) => watch,
            Err(error) => return text(500, &error.to_string()).send(stream),
        };
        let mut send = |chunk: &str| stream.write_all(chunk.as_bytes()).and_then(|()| stream.flush());
        if send("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n").is_err() {
            return;
        }
        // The heartbeat surfaces a closed connection as a write error, which drops the watch.
        loop {
            let chunk = match watch.reloads.recv_timeout(HEARTBEAT) {
                Ok(()) => "data: reload\n\n",
                Err(RecvTimeoutError::Timeout) => ": ping\n\n",
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if send(chunk).is_err() {
                return;
            }
        }
    }

    fn node(&self, node: &DocNode) -> NodeJson {
        NodeJson {
            name: node.name.clone(),
            path: node.path.clone(),
            kind: node.kind.clone(),
            status: self.status(node.status.as_ref()),
            archived: node.archived.unwrap_or(false),
            children: node
                .children
                .as_ref()
                .map(|children| children.iter().map(|child| self.node(child)).collect()),
        }
    }

    fn status(&self, status: Option<&Status>) -> Option<StatusJson> {
        let status = status?;
        let index = status
            .known
            .then(|| self.statuses.iter().position(|name| *name == status.name))
            .flatten();
        Some(StatusJson {
            name: status.name.clone(),
            known: index.is_some(),
            tone: index.map_or_else(
                || "var(--status-unknown)".into(),
                |index| format!("var(--tone-{index}, var(--status-unknown))"),
            ),
        })
    }
}

fn css_color(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Ansi(slot) => {
            let name = spec_viewer_config::ANSI_SLOTS
                .iter()
                .find(|(_, candidate)| *candidate == slot)
                .map_or("white", |(name, _)| name);
            format!("var(--ansi-{name})")
        }
        Color::Reset => "var(--fg)".into(),
    }
}

struct Query<'a>(&'a str);

impl Query<'_> {
    fn get(&self, key: &str) -> Option<String> {
        self.0.split('&').find_map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name) == key).then(|| decode(value))
        })
    }
}

/// `application/x-www-form-urlencoded` decoding; malformed escapes stay literal.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if let Some(byte) = value.get(index + 1..index + 3).and_then(|hex| u8::from_str_radix(hex, 16).ok()) => {
                out.push(byte);
                index += 2;
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn asset(body: &str, content_type: &'static str) -> Reply {
    Reply {
        status: 200,
        headers: vec![("Content-Type", content_type), ("Cache-Control", "no-cache")],
        body: body.as_bytes().to_vec(),
    }
}

fn text(status: u16, body: &str) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type", "text/plain; charset=utf-8")],
        body: body.as_bytes().to_vec(),
    }
}

fn json(value: &impl Serialize) -> Reply {
    Reply {
        status: 200,
        headers: vec![("Content-Type", "application/json"), ("Cache-Control", "no-store")],
        body: serde_json::to_vec(value).expect("serializable response"),
    }
}

#[derive(Serialize)]
struct StatusJson {
    name: String,
    known: bool,
    tone: String,
}

#[derive(Serialize)]
struct NodeJson {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<StatusJson>,
    archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<NodeJson>>,
}

#[derive(Serialize)]
struct FolderJson<'a> {
    name: &'a str,
    nodes: Vec<NodeJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocsJson<'a> {
    changed_specs: Option<Vec<String>>,
    folders: Vec<FolderJson<'a>>,
    project: String,
    specs: Vec<NodeJson>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocJson<'a> {
    path: &'a Path,
    root_path: &'a str,
    meta: MetaJson<'a>,
    html: &'a str,
    toc: Vec<TocJson<'a>>,
    has_mermaid: bool,
    degraded: Option<DegradedJson<'a>>,
}

#[derive(Serialize)]
struct MetaJson<'a> {
    title: &'a str,
    status: Option<StatusJson>,
    author: Option<&'a str>,
    date: Option<&'a str>,
    related: &'a [String],
    chips: Vec<ChipJson<'a>>,
    lines: usize,
    legacy: bool,
    kind: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ChipJson<'a> {
    Checklist { label: &'a str, done: usize, total: usize },
    Count { label: &'a str, count: usize },
}

#[derive(Serialize)]
struct TocJson<'a> {
    id: &'a str,
    text: &'a str,
    depth: u8,
}

impl<'a> From<&'a TocEntry> for TocJson<'a> {
    fn from(entry: &'a TocEntry) -> Self {
        Self {
            id: &entry.id,
            text: &entry.text,
            depth: entry.depth,
        }
    }
}

#[derive(Serialize)]
struct DegradedJson<'a> {
    reason: &'a str,
    message: &'a str,
    line: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_are_form_decoded() {
        let query = Query("wt=%2Ftmp%2Fa+b&path=.plan%2Fx.md&bad=%zz");
        assert_eq!(query.get("wt").as_deref(), Some("/tmp/a b"));
        assert_eq!(query.get("path").as_deref(), Some(".plan/x.md"));
        assert_eq!(query.get("bad").as_deref(), Some("%zz"));
        assert_eq!(query.get("missing"), None);
    }
}
