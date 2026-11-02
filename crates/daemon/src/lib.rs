//! The per-host daemon: it holds the corpus, serves the protocol on every transport, and starts the
//! web gateway.
mod hub;
mod known;
mod lifecycle;
mod watches;

pub use known::known_file;
pub use lifecycle::{relay, run};

use std::{
    collections::HashMap,
    io::{BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use spec_viewer_config::{Config, Layer, ParseConfig};
use spec_viewer_core::{DocError, Project, docs, document::Documents};
use spec_viewer_protocol::{Client, ClientMessage, DaemonMessage, Error, PROTOCOL, Request, Resolved, Response, Runtime, receive, send};
use spec_viewer_source::{Watcher, project_of, watch_files};

use hub::Hub;
use known::Known;
use watches::Watches;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct Options {
    pub runtime: Runtime,
    /// The host's user config file, for the daemon's own user layer, `[web]`, and `[[host]]`.
    pub user_config: Option<PathBuf>,
    /// Where resolved projects are recorded; `None` records nothing.
    pub known: Option<PathBuf>,
    /// Exit after this long with no socket client and no SSE stream.
    pub idle_exit: Option<Duration>,
}

impl Options {
    pub fn from_env(idle_exit: Option<Duration>) -> Self {
        Self {
            runtime: Runtime::from_env(),
            user_config: Config::user_file(),
            known: known_file(),
            idle_exit,
        }
    }
}

pub struct Daemon {
    config: Config,
    user: Mutex<Layer>,
    known: Known,
    libraries: Mutex<HashMap<ParseConfig, Arc<Documents>>>,
    watches: Watches,
    hub: Hub,
    connections: Mutex<HashMap<u64, Weak<Connection>>>,
    next_connection: AtomicU64,
    web: Mutex<Option<String>>,
    activity: Arc<Activity>,
    user_watch: Mutex<Option<Watcher>>,
}

/// One client connection and the layers it set.
pub(crate) struct Connection {
    id: u64,
    writer: Mutex<Box<dyn Write + Send>>,
    /// `None` takes the daemon's own user layer.
    user: Mutex<Option<Layer>>,
    overrides: Mutex<HashMap<PathBuf, Layer>>,
    /// Dropping a handle ends its watch.
    watches: Mutex<HashMap<u64, Box<dyn Send>>>,
}

impl Connection {
    fn send(&self, message: &DaemonMessage) {
        let _ = send(&mut *self.writer.lock().expect("connection writer"), message);
    }
    fn changed_all(&self) {
        let ids: Vec<u64> = self.watches.lock().expect("connection watches").keys().copied().collect();
        for id in ids {
            self.send(&DaemonMessage::Changed { id });
        }
    }
}

/// Counts socket clients and SSE streams for `--idle-exit`.
#[derive(Default)]
struct Activity {
    state: Mutex<(usize, Option<Instant>)>,
}

/// Keeps the daemon from idling out while held.
pub struct Hold(Arc<Activity>);

impl Drop for Hold {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().expect("activity");
        state.0 -= 1;
        if state.0 == 0 {
            state.1 = Some(Instant::now());
        }
    }
}

impl Activity {
    fn hold(self: &Arc<Self>) -> Hold {
        self.state.lock().expect("activity").0 += 1;
        Hold(Arc::clone(self))
    }
    /// How long nothing has been held, from `start` when nothing ever was.
    fn idle(&self, start: Instant) -> Option<Duration> {
        let state = self.state.lock().expect("activity");
        (state.0 == 0).then(|| state.1.unwrap_or(start).elapsed())
    }
}

impl Daemon {
    /// Loads the user config, starts watching it, and connects to its hosts.
    pub fn new(options: &Options) -> Result<Arc<Self>, spec_viewer_config::Error> {
        let config = Config::load_file(options.user_config.as_deref())?;
        let daemon = Arc::new(Self {
            user: Mutex::new(config.layer.clone()),
            known: Known::load(options.known.clone()),
            libraries: Mutex::default(),
            watches: Watches::default(),
            hub: Hub::new(&config.hosts),
            connections: Mutex::default(),
            next_connection: AtomicU64::new(1),
            web: Mutex::default(),
            activity: Arc::default(),
            user_watch: Mutex::default(),
            config,
        });
        if let Some(file) = options.user_config.clone() {
            let weak = Arc::downgrade(&daemon);
            let watcher = watch_files(vec![file.clone()], move || {
                let Some(daemon) = weak.upgrade() else { return false };
                daemon.reload_user(&file);
                true
            });
            match watcher {
                Ok(watcher) => *daemon.user_watch.lock().expect("user watch") = Some(watcher),
                Err(error) => eprintln!(
                    "spec-viewer: cannot watch {}: {error}",
                    options.user_config.as_ref().unwrap().display()
                ),
            }
        }
        Hub::start(&daemon);
        Ok(daemon)
    }

    /// The user config read at start.
    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn web_url(&self) -> Option<String> {
        self.web.lock().expect("web url").clone()
    }

    pub fn set_web_url(&self, url: Option<String>) {
        *self.web.lock().expect("web url") = url;
    }

    /// Keeps an `--idle-exit` daemon up while the returned hold lives.
    pub fn hold(&self) -> Hold {
        self.activity.hold()
    }

    /// How long the daemon has had no socket client and no SSE stream, if it has none now.
    pub fn idle(&self, start: Instant) -> Option<Duration> {
        self.activity.idle(start)
    }

    /// The number of worktrees with a live watch.
    #[doc(hidden)]
    pub fn watched_worktrees(&self) -> usize {
        self.watches.len()
    }

    /// An in-process connection, which does not keep the daemon from idling out.
    pub fn connect(self: &Arc<Self>) -> Result<Client, Error> {
        let (near, far) = UnixStream::pair().map_err(Error::unreachable)?;
        let writer = far.try_clone().map_err(Error::unreachable)?;
        let daemon = Arc::clone(self);
        std::thread::spawn(move || daemon.serve(far, writer, None));
        Client::over_socket(near, None)
    }

    /// Serves one connection until it closes; `hold` is released then.
    pub fn serve(self: &Arc<Self>, reader: impl Read, mut writer: impl Write + Send + 'static, hold: Option<Hold>) {
        let mut reader = BufReader::new(reader);
        let Ok(Some(ClientMessage::Hello { protocol, user })) = receive(&mut reader) else {
            return;
        };
        let welcome = DaemonMessage::Welcome {
            protocol: PROTOCOL,
            version: VERSION.into(),
            web: self.web_url(),
        };
        if send(&mut writer, &welcome).is_err() {
            return;
        }
        if protocol != PROTOCOL {
            let error = Error::Protocol {
                daemon: PROTOCOL,
                client: protocol,
            };
            let _ = send(&mut writer, &DaemonMessage::Response { id: 0, result: Err(error) });
            return;
        }
        let id = self.next_connection.fetch_add(1, Ordering::Relaxed);
        let connection = Arc::new(Connection {
            id,
            writer: Mutex::new(Box::new(writer)),
            user: Mutex::new(user),
            overrides: Mutex::default(),
            watches: Mutex::default(),
        });
        self.connections
            .lock()
            .expect("connections")
            .insert(id, Arc::downgrade(&connection));
        while let Ok(Some(message)) = receive::<ClientMessage>(&mut reader) {
            match message {
                ClientMessage::Request { id, host, request } => {
                    let daemon = Arc::clone(self);
                    let connection = Arc::clone(&connection);
                    std::thread::spawn(move || {
                        let result = daemon.handle(&connection, id, host, request);
                        connection.send(&DaemonMessage::Response { id, result });
                    });
                }
                ClientMessage::User { user } => {
                    *connection.user.lock().expect("connection user") = Some(user);
                    connection.changed_all();
                }
                ClientMessage::Override { worktree, layer } => {
                    let worktree = std::fs::canonicalize(&worktree).unwrap_or(worktree);
                    let mut overrides = connection.overrides.lock().expect("connection overrides");
                    match layer {
                        Some(layer) => overrides.insert(worktree, layer),
                        None => overrides.remove(&worktree),
                    };
                    drop(overrides);
                    connection.changed_all();
                }
                ClientMessage::Hello { .. } => {}
            }
        }
        self.connections.lock().expect("connections").remove(&id);
        connection.watches.lock().expect("connection watches").clear();
        drop(hold);
    }

    fn handle(self: &Arc<Self>, connection: &Arc<Connection>, id: u64, host: Option<String>, request: Request) -> Result<Response, Error> {
        // Watch ids belong to the connection, whichever host serves the watch.
        if let Request::Unwatch { id } = request {
            connection.watches.lock().expect("connection watches").remove(&id);
            return Ok(Response::Unwatched);
        }
        if let Some(host) = host {
            return self.hub.forward(self, connection, id, &host, request);
        }
        // A hub's connection sees only this host, so a remote daemon never reaches further hosts.
        let merging = connection.user.lock().expect("connection user").is_none();
        match request {
            Request::Hosts => Ok(Response::Hosts(self.hub.states())),
            Request::Projects if merging => Ok(Response::Projects(self.hub.merge_all(self.projects()))),
            Request::Projects => Ok(Response::Projects(self.projects())),
            Request::Resolve { root } => {
                let mut resolved = self.resolve(connection, &root)?;
                if merging {
                    resolved.project = self.hub.merged(self, resolved.project, None);
                }
                Ok(Response::Resolved(resolved))
            }
            Request::Project { key } => {
                let project = project_of(&key).map_err(|_| Error::NotAProject)?;
                Ok(Response::Project(if merging {
                    self.hub.merged(self, project, None)
                } else {
                    project
                }))
            }
            Request::Corpus { worktree } => {
                let (worktree, project) = listed(&worktree)?;
                let config = self.parse_config(connection, &worktree)?;
                Ok(Response::Corpus(docs(&worktree, &project.name, &config)))
            }
            Request::Open { worktree, path } => {
                let (worktree, _) = listed(&worktree)?;
                let config = self.parse_config(connection, &worktree)?;
                match self.library(config).open(&worktree, &path) {
                    Ok(opened) => Ok(Response::Opened(opened)),
                    Err(DocError::Forbidden) => Err(Error::Forbidden),
                    Err(DocError::NotFound) => Err(Error::NotFound),
                }
            }
            Request::Watch { worktree } => {
                let (worktree, _) = listed(&worktree)?;
                let subscription = self.watches.subscribe(self, connection, id, worktree)?;
                connection
                    .watches
                    .lock()
                    .expect("connection watches")
                    .insert(id, Box::new(subscription));
                Ok(Response::Watching)
            }
            Request::Unwatch { .. } => unreachable!("handled above"),
        }
    }

    /// The known projects that still resolve.
    fn projects(&self) -> Vec<Project> {
        let mut projects: Vec<Project> = Vec::new();
        for path in self.known.paths() {
            if let Ok(project) = project_of(&path)
                && !projects.iter().any(|known| known.key == project.key)
            {
                projects.push(project);
            }
        }
        projects
    }

    fn resolve(&self, connection: &Connection, root: &Path) -> Result<Resolved, Error> {
        let root = std::fs::canonicalize(root).map_err(|_| Error::NotAProject)?;
        let project = project_of(&root).map_err(|_| Error::NotAProject)?;
        let worktree = project
            .worktrees
            .iter()
            .enumerate()
            .filter(|(_, worktree)| root.starts_with(&worktree.path))
            .max_by_key(|(_, worktree)| worktree.path.components().count())
            .map_or(0, |(index, _)| index);
        let config = self.parse_config(connection, &project.worktrees[worktree].path)?;
        self.known.record(&project.path);
        Ok(Resolved {
            project,
            worktree,
            statuses: config.statuses,
        })
    }

    /// The default, then the connection's user layer, then the worktree's file or its override.
    fn parse_config(&self, connection: &Connection, worktree: &Path) -> Result<ParseConfig, Error> {
        let user = connection
            .user
            .lock()
            .expect("connection user")
            .clone()
            .unwrap_or_else(|| self.user.lock().expect("user layer").clone());
        let file = match connection.overrides.lock().expect("connection overrides").get(worktree) {
            Some(layer) => layer.clone(),
            None => Layer::read(&worktree.join(".spec-viewer.toml"))
                .map_err(|error| Error::Config {
                    message: error.to_string(),
                })?
                .unwrap_or_default(),
        };
        Ok(ParseConfig::with(&[&user, &file]))
    }

    fn library(&self, config: ParseConfig) -> Arc<Documents> {
        let mut libraries = self.libraries.lock().expect("libraries");
        Arc::clone(libraries.entry(config.clone()).or_insert_with(|| Arc::new(Documents::new(config))))
    }

    /// Keeps the last valid user layer when the file stops loading.
    fn reload_user(&self, file: &Path) {
        let layer = match Config::load_file(Some(file)) {
            Ok(config) => config.layer,
            Err(error) => {
                eprintln!("spec-viewer: keeping the previous user config: {error}");
                return;
            }
        };
        {
            let mut user = self.user.lock().expect("user layer");
            if *user == layer {
                return;
            }
            user.clone_from(&layer);
        }
        self.hub.send_user(&layer);
        for connection in self.live_connections() {
            if connection.user.lock().expect("connection user").is_none() {
                connection.changed_all();
            }
        }
    }

    fn live_connections(&self) -> Vec<Arc<Connection>> {
        self.connections
            .lock()
            .expect("connections")
            .values()
            .filter_map(Weak::upgrade)
            .collect()
    }
}

/// The canonical worktree and its project, when the project lists it.
fn listed(worktree: &Path) -> Result<(PathBuf, Project), Error> {
    let worktree = std::fs::canonicalize(worktree).map_err(|_| Error::NotAProject)?;
    let project = project_of(&worktree).map_err(|_| Error::NotAProject)?;
    if !project.worktrees.iter().any(|listed| listed.path == worktree) {
        return Err(Error::UnlistedWorktree);
    }
    Ok((worktree, project))
}
