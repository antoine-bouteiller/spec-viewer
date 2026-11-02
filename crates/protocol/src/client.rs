//! The blocking protocol client.
use std::{
    collections::HashMap,
    io::{BufReader, Read, Write},
    net::Shutdown,
    ops::Deref,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex, Weak,
        mpsc::{self, Receiver, Sender},
    },
};

use spec_viewer_config::Layer;
use spec_viewer_core::{Corpus, Opened, Project};

use crate::{ClientMessage, DaemonMessage, Error, HostState, PROTOCOL, Request, Resolved, Response, Runtime, receive, send};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Changed,
    HostDown,
}

/// A connection to a daemon. Calls block; one reader thread routes responses by id and events by
/// watch. A lost connection fails pending and later calls with `HostUnreachable`.
pub struct Client {
    shared: Arc<Shared>,
    web: Option<String>,
}

struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    next: u64,
    pending: HashMap<u64, Sender<Result<Response, Error>>>,
    watches: HashMap<u64, (Option<String>, Sender<Event>)>,
    /// Why the connection ended, once it has.
    lost: Option<String>,
}

/// The events of one watch. Dropping it sends `Unwatch`.
pub struct Watch {
    events: Receiver<Event>,
    id: u64,
    shared: Arc<Shared>,
}

impl Deref for Watch {
    type Target = Receiver<Event>;
    fn deref(&self) -> &Receiver<Event> {
        &self.events
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let removed = self.shared.state.lock().expect("client state").watches.remove(&self.id);
        if removed.is_some() {
            let _ = self.shared.unwatch(self.id);
        }
    }
}

impl Client {
    /// Connects to this host's daemon, spawning `<current exe> daemon --idle-exit 300` when none
    /// answers, and completes the handshake.
    pub fn connect() -> Result<Self, Error> {
        let exe = std::env::current_exe().map_err(Error::unreachable)?;
        let mut spawn = Command::new(exe);
        spawn.args(["daemon", "--idle-exit", "300"]);
        Self::connect_with(&Runtime::from_env(), spawn)
    }

    /// `connect` with another runtime directory and daemon command, for tests.
    #[doc(hidden)]
    pub fn connect_with(runtime: &Runtime, spawn: Command) -> Result<Self, Error> {
        Self::over_socket(runtime.connect(spawn).map_err(Error::unreachable)?, None)
    }

    /// `over` a socket, shut down once the client and its watches are dropped.
    pub fn over_socket(stream: UnixStream, user: Option<Layer>) -> Result<Self, Error> {
        let reader = stream.try_clone().map_err(Error::unreachable)?;
        Self::over(reader, Closing(stream), user)
    }

    /// The handshake over any pair of streams. `user` is sent in `Hello`; only the hub passes `Some`.
    pub fn over(reader: impl Read + Send + 'static, mut writer: impl Write + Send + 'static, user: Option<Layer>) -> Result<Self, Error> {
        let lost = |error| Error::unreachable(format!("the daemon closed the connection: {error}"));
        send(&mut writer, &ClientMessage::Hello { protocol: PROTOCOL, user }).map_err(lost)?;
        let mut reader = BufReader::new(reader);
        let web = match receive(&mut reader).map_err(lost)? {
            Some(DaemonMessage::Welcome { protocol, web, .. }) => {
                if protocol != PROTOCOL {
                    return Err(Error::Protocol {
                        daemon: protocol,
                        client: PROTOCOL,
                    });
                }
                web
            }
            other => return Err(Error::unreachable(format!("expected Welcome, got {other:?}"))),
        };
        let shared = Arc::new(Shared {
            writer: Mutex::new(Box::new(writer)),
            state: Mutex::new(State {
                next: 1,
                ..State::default()
            }),
        });
        let routed = Arc::downgrade(&shared);
        std::thread::spawn(move || Shared::route(&routed, reader));
        Ok(Self { shared, web })
    }

    pub fn web_url(&self) -> Option<&str> {
        self.web.as_deref()
    }

    pub fn hosts(&self) -> Result<Vec<HostState>, Error> {
        match self.call(None, Request::Hosts)? {
            Response::Hosts(hosts) => Ok(hosts),
            other => Err(unexpected(&other)),
        }
    }

    pub fn projects(&self, host: Option<&str>) -> Result<Vec<Project>, Error> {
        match self.call(host, Request::Projects)? {
            Response::Projects(projects) => Ok(projects),
            other => Err(unexpected(&other)),
        }
    }

    pub fn resolve(&self, host: Option<&str>, root: &Path) -> Result<Resolved, Error> {
        match self.call(host, Request::Resolve { root: root.into() })? {
            Response::Resolved(resolved) => Ok(resolved),
            other => Err(unexpected(&other)),
        }
    }

    pub fn project(&self, host: Option<&str>, key: &Path) -> Result<Project, Error> {
        match self.call(host, Request::Project { key: key.into() })? {
            Response::Project(project) => Ok(project),
            other => Err(unexpected(&other)),
        }
    }

    pub fn corpus(&self, host: Option<&str>, worktree: &Path) -> Result<Corpus, Error> {
        match self.call(host, Request::Corpus { worktree: worktree.into() })? {
            Response::Corpus(corpus) => Ok(corpus),
            other => Err(unexpected(&other)),
        }
    }

    pub fn open(&self, host: Option<&str>, worktree: &Path, path: &str) -> Result<Opened, Error> {
        let request = Request::Open {
            worktree: worktree.into(),
            path: path.into(),
        };
        match self.call(host, request)? {
            Response::Opened(opened) => Ok(opened),
            other => Err(unexpected(&other)),
        }
    }

    /// Dropping the returned watch sends `Unwatch`.
    pub fn watch(&self, host: Option<&str>, worktree: &Path) -> Result<Watch, Error> {
        let (sender, events) = mpsc::channel();
        let request = Request::Watch { worktree: worktree.into() };
        let (id, response) = self.shared.request(host, request, |state, id| {
            state.watches.insert(id, (host.map(str::to_owned), sender));
        })?;
        let watch = Watch {
            events,
            id,
            shared: Arc::clone(&self.shared),
        };
        match response.recv() {
            Ok(Ok(Response::Watching)) => Ok(watch),
            Ok(Ok(other)) => Err(unexpected(&other)),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(self.shared.lost()),
        }
    }

    /// Replaces this connection's user layer; every watch on it receives `Changed`.
    pub fn send_user(&self, user: Layer) -> Result<(), Error> {
        self.shared.message(&ClientMessage::User { user })
    }

    /// Replaces `worktree`'s `.spec-viewer.toml` for this connection; `None` restores it.
    pub fn send_override(&self, worktree: &Path, layer: Option<Layer>) -> Result<(), Error> {
        self.shared.message(&ClientMessage::Override {
            worktree: PathBuf::from(worktree),
            layer,
        })
    }

    fn call(&self, host: Option<&str>, request: Request) -> Result<Response, Error> {
        let (_, response) = self.shared.request(host, request, |_, _| {})?;
        response.recv().map_err(|_| self.shared.lost())?
    }
}

/// Shuts the socket down when dropped, which ends the reader thread's clone too.
struct Closing(UnixStream);

impl Write for Closing {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl Drop for Closing {
    fn drop(&mut self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

fn unexpected(response: &Response) -> Error {
    Error::unreachable(format!("unexpected response {response:?}"))
}

impl Shared {
    /// Registers a pending request, runs `register` under the same lock, and sends it.
    fn request(
        &self,
        host: Option<&str>,
        request: Request,
        register: impl FnOnce(&mut State, u64),
    ) -> Result<(u64, Receiver<Result<Response, Error>>), Error> {
        let (sender, receiver) = mpsc::channel();
        let id = {
            let mut state = self.state.lock().expect("client state");
            if let Some(reason) = &state.lost {
                return Err(Error::unreachable(reason));
            }
            let id = state.next;
            state.next += 1;
            state.pending.insert(id, sender);
            register(&mut state, id);
            id
        };
        let host = host.map(str::to_owned);
        if let Err(error) = self.message(&ClientMessage::Request { id, host, request }) {
            let mut state = self.state.lock().expect("client state");
            state.pending.remove(&id);
            state.watches.remove(&id);
            return Err(error);
        }
        Ok((id, receiver))
    }

    /// The receiving daemon owns watch ids, so `Unwatch` names no host; nobody awaits its response.
    fn unwatch(&self, watch: u64) -> Result<(), Error> {
        let id = {
            let mut state = self.state.lock().expect("client state");
            state.next += 1;
            state.next - 1
        };
        let request = Request::Unwatch { id: watch };
        self.message(&ClientMessage::Request { id, host: None, request })
    }

    fn message(&self, message: &ClientMessage) -> Result<(), Error> {
        let mut writer = self.writer.lock().expect("client writer");
        send(&mut *writer, message).map_err(|error| Error::unreachable(format!("the daemon closed the connection: {error}")))
    }

    fn lost(&self) -> Error {
        let state = self.state.lock().expect("client state");
        Error::unreachable(state.lost.as_deref().unwrap_or("the daemon closed the connection"))
    }

    /// Holds only a weak reference, so dropping the client and its watches drops the writer.
    fn route(shared: &Weak<Self>, mut reader: impl std::io::BufRead) {
        let reason = loop {
            let message = match receive::<DaemonMessage>(&mut reader) {
                Ok(Some(message)) => message,
                Ok(None) => break "the daemon closed the connection".to_owned(),
                Err(error) => break format!("the daemon connection failed: {error}"),
            };
            let Some(shared) = shared.upgrade() else { return };
            let mut state = shared.state.lock().expect("client state");
            match message {
                DaemonMessage::Response { id, result } => {
                    if let Some(sender) = state.pending.remove(&id) {
                        let _ = sender.send(result);
                    }
                }
                DaemonMessage::Changed { id } => {
                    if let Some((_, sender)) = state.watches.get(&id) {
                        let _ = sender.send(Event::Changed);
                    }
                }
                DaemonMessage::HostDown { host } => {
                    for (watched, sender) in state.watches.values() {
                        if watched.as_deref() == Some(host.as_str()) {
                            let _ = sender.send(Event::HostDown);
                        }
                    }
                }
                DaemonMessage::Welcome { .. } => {}
            }
        };
        let Some(shared) = shared.upgrade() else { return };
        let mut state = shared.state.lock().expect("client state");
        state.lost = Some(reason);
        state.pending.clear();
        state.watches.clear();
    }
}
