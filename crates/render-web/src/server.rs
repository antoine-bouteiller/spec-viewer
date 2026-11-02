//! HTTP server: the embedded client, the JSON API, and server-sent reload events, as a client of
//! the daemon.
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc::RecvTimeoutError},
    time::{Duration, Instant},
};

use serde::Serialize;
use spec_viewer_config::{Binding, Code, Color, Config, Key, Keys, Mode, StatusConfig};
use spec_viewer_protocol::{ChipValue, Client, DocNode, Document, Error, Status, TocEntry};

use crate::html::{Html, to_html};

const INDEX: &str = include_str!("../assets/index.html");
const APP_JS: &str = include_str!("../assets/app.js");
const APP_CSS: &str = include_str!("../assets/app.css");
const HEARTBEAT: Duration = Duration::from_secs(15);
const HEAD_LIMIT: u64 = 16 * 1024;

/// The gateway's view of its daemon.
pub struct Backend {
    /// An in-process connection.
    pub client: Client,
    /// Keeps the daemon from idling out while the returned value lives; held per events stream.
    pub hold: Box<dyn Fn() -> Box<dyn Send> + Send + Sync>,
}

/// Serves the web app on `listener` on background threads until exit. `config` is the daemon
/// host's user config, for the keys and the palette.
pub fn start(listener: TcpListener, config: &Config, backend: Backend) -> io::Result<()> {
    let port = listener.local_addr()?.port();
    let state = Arc::new(State::new(config, backend, port));
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let state = Arc::clone(&state);
            std::thread::spawn(move || {
                if stream.set_read_timeout(Some(Duration::from_secs(5))).is_err() {
                    return;
                }
                if let Some((target, host)) = request_head(&mut stream) {
                    if allowed(host.as_deref(), state.port) {
                        state.handle(stream, &target);
                    } else {
                        text(403, "forbidden").send(&mut stream);
                    }
                }
            });
        }
    });
    Ok(())
}

/// A `Host` naming loopback by name or address, with the gateway's port. Any other name could be a
/// DNS-rebound page reaching the API.
fn allowed(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else { return false };
    let host = host.to_ascii_lowercase();
    ["localhost", "127.0.0.1", "[::1]"]
        .iter()
        .any(|name| host == format!("{name}:{port}") || (port == 80 && host == *name))
}

/// Reads the request head and returns its target and `Host` header. Any method is served as GET
/// and every response closes the connection, so request bodies and keep-alive never need handling.
fn request_head(stream: &mut TcpStream) -> Option<(String, Option<String>)> {
    let mut reader = BufReader::new(stream.take(HEAD_LIMIT));
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let target = line.split(' ').nth(1)?.to_owned();
    let mut host = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        if line == "\r\n" || line == "\n" {
            return Some((target, host));
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("host")
        {
            host = Some(value.trim().to_owned());
        }
    }
}

struct Reply {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body: Vec<u8>,
}

impl Reply {
    fn send(self, stream: &mut TcpStream) {
        let reason = match self.status {
            200 => "OK",
            403 => "Forbidden",
            404 => "Not Found",
            502 => "Bad Gateway",
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

/// A worktree named by `host` and `wt`, as the daemon resolved it.
#[derive(Clone)]
struct Target {
    host: Option<String>,
    path: PathBuf,
    statuses: Vec<StatusConfig>,
}

/// HTML per host and document file, reused while the document is unchanged.
type Rendered = HashMap<(Option<String>, PathBuf), (Arc<Document>, Arc<Html>)>;

struct State {
    backend: Backend,
    config: Config,
    port: u16,
    /// For `/theme.css` without a worktree: the daemon's own statuses.
    theme_css: String,
    keys: serde_json::Value,
    targets: Mutex<HashMap<(Option<String>, String), Target>>,
    rendered: Mutex<Rendered>,
}

fn theme_css(config: &Config, statuses: &[StatusConfig]) -> String {
    let mut theme_css = String::new();
    for (mode, dark) in [(Mode::Light, false), (Mode::Dark, true)] {
        let mut rules = String::new();
        // A status naming no palette colour of the mode keeps the unknown tone.
        let colors = config.status_colors(mode, statuses);
        for (index, status) in statuses.iter().enumerate() {
            if let Some(color) = colors.get(&status.name) {
                rules.push_str(&format!("--tone-{index}: {};\n", css_color(*color)));
            }
        }
        let resolved = config.resolve(mode).ok();
        // The design tokens behind ANSI slots are foreground colours; only a hex surface applies.
        if let Some(resolved) = &resolved
            && let Color::Rgb(..) = resolved.roles.code_bg
        {
            rules.push_str(&format!("--code-bg: {};\n", css_color(resolved.roles.code_bg)));
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
    theme_css
}

/// The status and body for a failed daemon request.
fn failure(error: &Error) -> Reply {
    match error {
        Error::NotAProject | Error::UnlistedWorktree | Error::Forbidden => text(403, "forbidden"),
        Error::NotFound => text(404, "not found"),
        Error::UnknownHost => text(404, "unknown host"),
        Error::HostUnreachable { message } => text(502, message),
        Error::Config { .. } | Error::Protocol { .. } => text(500, &error.to_string()),
    }
}

impl State {
    fn new(config: &Config, backend: Backend, port: u16) -> Self {
        Self {
            theme_css: theme_css(config, &config.parse.statuses),
            keys: keys_json(&config.keys),
            config: config.clone(),
            backend,
            port,
            targets: Mutex::default(),
            rendered: Mutex::default(),
        }
    }

    fn client(&self) -> &Client {
        &self.backend.client
    }

    fn handle(&self, mut stream: TcpStream, path_and_query: &str) {
        let (path, query) = path_and_query.split_once('?').unwrap_or((path_and_query, ""));
        let query = Query(query);
        if path == "/api/events" {
            return match self.target(&query, false) {
                Ok(target) => self.events(&mut stream, &target),
                Err(reply) => reply.send(&mut stream),
            };
        }
        let reply = match path {
            "/" => asset(INDEX, "text/html; charset=utf-8"),
            "/app.js" => asset(APP_JS, "text/javascript; charset=utf-8"),
            "/app.css" => asset(APP_CSS, "text/css; charset=utf-8"),
            "/theme.css" if query.get("wt").is_some() => self.target(&query, false).map_or_else(
                |reply| reply,
                |target| asset(&theme_css(&self.config, &target.statuses), "text/css; charset=utf-8"),
            ),
            "/theme.css" => asset(&self.theme_css, "text/css; charset=utf-8"),
            "/api/projects" => self.projects(),
            "/api/keys" => json(&self.keys),
            "/api/docs" => self.target(&query, true).map_or_else(|reply| reply, |target| self.docs(&target)),
            "/api/search" => self.target(&query, false).map_or_else(
                |reply| reply,
                |target| self.search(&target, query.get("q").as_deref().unwrap_or("")),
            ),
            "/api/doc" => self.target(&query, false).map_or_else(
                |reply| reply,
                |target| self.doc(&target, query.get("path").as_deref().unwrap_or("")),
            ),
            _ => text(404, "not found"),
        };
        reply.send(&mut stream);
    }

    /// The worktree that `Resolve` finds for `wt` on `host`, resolved again on `refresh`. Resolving
    /// adds its project to that host's known projects.
    fn target(&self, query: &Query, refresh: bool) -> Result<Target, Reply> {
        let wt = query.get("wt").ok_or_else(|| text(403, "forbidden"))?;
        let host = query.get("host").filter(|host| !host.is_empty());
        let key = (host.clone(), wt.clone());
        if !refresh && let Some(target) = self.targets.lock().expect("targets").get(&key) {
            return Ok(target.clone());
        }
        let resolved = self
            .client()
            .resolve(host.as_deref(), PathBuf::from(&wt).as_path())
            .map_err(|error| failure(&error))?;
        let worktree = &resolved.project.worktrees[resolved.worktree];
        let target = Target {
            host: worktree.host.clone(),
            path: worktree.path.clone(),
            statuses: resolved.statuses,
        };
        self.targets.lock().expect("targets").insert(key, target.clone());
        Ok(target)
    }

    fn projects(&self) -> Reply {
        let projects = match self.client().projects(None) {
            Ok(projects) => projects,
            Err(error) => return failure(&error),
        };
        let unreachable: Vec<_> = self
            .client()
            .hosts()
            .unwrap_or_default()
            .into_iter()
            .filter(|host| !host.connected)
            .map(|host| serde_json::json!({ "host": host.name, "error": host.error.unwrap_or_default() }))
            .collect();
        json(&serde_json::json!({
            "projects": projects.iter().map(|project| {
                let mut value = serde_json::json!({
                    "name": project.name,
                    "path": project.path,
                    "worktrees": project.worktrees,
                });
                if let Some(origin) = &project.origin {
                    value["origin"] = origin.as_str().into();
                }
                value
            }).collect::<Vec<_>>(),
            "unreachable": unreachable,
        }))
    }

    fn docs(&self, target: &Target) -> Reply {
        let corpus = match self.client().corpus(target.host.as_deref(), &target.path) {
            Ok(corpus) => corpus,
            Err(error) => return failure(&error),
        };
        json(&DocsJson {
            changed: corpus.changed,
            project: corpus.project,
            tabs: corpus
                .tabs
                .iter()
                .map(|tab| TabJson {
                    name: &tab.name,
                    nodes: tab.nodes.iter().map(|node| node_json(&target.statuses, node)).collect(),
                })
                .collect(),
        })
    }

    fn search(&self, target: &Target, query: &str) -> Reply {
        let corpus = match self.client().corpus(target.host.as_deref(), &target.path) {
            Ok(corpus) => corpus,
            Err(error) => return failure(&error),
        };
        let hits = spec_viewer_protocol::search(&corpus.tabs, query);
        json(&SearchJson {
            total: spec_viewer_protocol::search(&corpus.tabs, "").len(),
            hits: hits
                .iter()
                .map(|hit| HitJson {
                    tab: &corpus.tabs[hit.tab].name,
                    name: &hit.node.name,
                    path: &hit.path,
                    status: status_json(&target.statuses, hit.node.status.as_ref()),
                    archived: hit.node.archived.unwrap_or(false),
                    title_matches: &hit.title_matches,
                    path_matches: &hit.path_matches,
                })
                .collect(),
        })
    }

    fn doc(&self, target: &Target, rel: &str) -> Reply {
        let opened = match self.client().open(target.host.as_deref(), &target.path, rel) {
            Ok(opened) => opened,
            Err(error) => return failure(&error),
        };
        let html = {
            let mut rendered = self.rendered.lock().expect("render cache");
            let key = (target.host.clone(), opened.path.clone());
            match rendered.get(&key) {
                Some((document, html)) if **document == *opened.document => Arc::clone(html),
                _ => {
                    let html = Arc::new(to_html(&opened.document));
                    rendered.insert(key, (Arc::clone(&opened.document), Arc::clone(&html)));
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
                status: status_json(&target.statuses, meta.status.as_ref()),
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

    /// Holds the daemon up while open, and checks for a closed peer twice a second, so a closed
    /// page releases it within an `--idle-exit` of one second.
    fn events(&self, stream: &mut TcpStream, target: &Target) {
        let watch = match self.client().watch(target.host.as_deref(), &target.path) {
            Ok(watch) => watch,
            Err(error) => return failure(&error).send(stream),
        };
        let _hold = (self.backend.hold)();
        let send = |stream: &mut TcpStream, chunk: &str| stream.write_all(chunk.as_bytes()).and_then(|()| stream.flush());
        if send(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .is_err()
        {
            return;
        }
        let mut pinged = Instant::now();
        loop {
            let chunk = match watch.recv_timeout(Duration::from_millis(500)) {
                Ok(_) => "data: reload\n\n",
                Err(RecvTimeoutError::Timeout) if closed(stream) => return,
                Err(RecvTimeoutError::Timeout) if pinged.elapsed() >= HEARTBEAT => ": ping\n\n",
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            pinged = Instant::now();
            if send(stream, chunk).is_err() {
                return;
            }
        }
    }
}

/// Whether the peer closed `stream`. An events client sends nothing after its request.
fn closed(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let peeked = stream.peek(&mut [0; 1]);
    let _ = stream.set_nonblocking(false);
    match peeked {
        Ok(read) => read == 0,
        Err(error) => error.kind() != io::ErrorKind::WouldBlock,
    }
}

fn node_json(statuses: &[StatusConfig], node: &DocNode) -> NodeJson {
    NodeJson {
        name: node.name.clone(),
        path: node.path.clone(),
        kind: node.kind.clone(),
        status: status_json(statuses, node.status.as_ref()),
        archived: node.archived.unwrap_or(false),
        companion: node.companion,
        children: node
            .children
            .as_ref()
            .map(|children| children.iter().map(|child| node_json(statuses, child)).collect()),
    }
}

/// `tone` indexes the worktree's statuses, as `/theme.css?host=&wt=` numbers them.
fn status_json(statuses: &[StatusConfig], status: Option<&Status>) -> Option<StatusJson> {
    let status = status?;
    let index = status
        .known
        .then(|| statuses.iter().position(|configured| configured.name == status.name))
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

/// The `[keys]` tables the client uses, each action mapping to `{ key, ctrl, alt, shift, meta, label }`
/// with `key` in `KeyboardEvent.key` terms. Ranges (`switch_tab`) have no web action and are left out.
fn keys_json(keys: &Keys) -> serde_json::Value {
    let table = |entries: Vec<(&str, &Binding)>| {
        entries
            .into_iter()
            .map(|(name, binding)| {
                let chords = binding.0.iter().filter(|chord| !chord.range).map(|chord| {
                    let Key { code, modifiers } = chord.key;
                    let key = match code {
                        Code::Char(character) => character.to_string(),
                        Code::Enter => "Enter".into(),
                        Code::Tab => "Tab".into(),
                        Code::Esc => "Escape".into(),
                        Code::Backspace => "Backspace".into(),
                        Code::Delete => "Delete".into(),
                        Code::Insert => "Insert".into(),
                        Code::Up => "ArrowUp".into(),
                        Code::Down => "ArrowDown".into(),
                        Code::Left => "ArrowLeft".into(),
                        Code::Right => "ArrowRight".into(),
                        Code::Home => "Home".into(),
                        Code::End => "End".into(),
                        Code::PageUp => "PageUp".into(),
                        Code::PageDown => "PageDown".into(),
                        Code::F(number) => format!("F{number}"),
                    };
                    serde_json::json!({
                        "key": key,
                        "ctrl": modifiers.ctrl,
                        "alt": modifiers.alt,
                        "shift": modifiers.shift,
                        "meta": modifiers.cmd,
                        "label": chord.key.label(),
                    })
                });
                (name.to_owned(), serde_json::Value::Array(chords.collect()))
            })
            .collect::<serde_json::Map<_, _>>()
    };
    serde_json::json!({ "main": table(keys.entries()), "search_dialog": table(keys.search_dialog.entries()) })
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
    companion: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<NodeJson>>,
}

#[derive(Serialize)]
struct TabJson<'a> {
    name: &'a str,
    nodes: Vec<NodeJson>,
}

#[derive(Serialize)]
struct DocsJson<'a> {
    changed: Option<Vec<String>>,
    project: String,
    tabs: Vec<TabJson<'a>>,
}

#[derive(Serialize)]
struct SearchJson<'a> {
    total: usize,
    hits: Vec<HitJson<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HitJson<'a> {
    tab: &'a str,
    name: &'a str,
    path: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<StatusJson>,
    archived: bool,
    /// Character (code point) indices into `name` and `path`.
    title_matches: &'a [usize],
    path_matches: &'a [usize],
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
