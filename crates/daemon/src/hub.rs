//! Remote hosts: one protocol connection per configured host over its command's stdio, request
//! forwarding, project merging by origin, and the overrides that make a remote clone follow the
//! local `.spec-viewer.toml`.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use spec_viewer_config::{HostConfig, Layer};
use spec_viewer_core::Project;
use spec_viewer_protocol::{Client, DaemonMessage, Error, Event, HostState, Request, Resolved, Response, Watch};
use spec_viewer_source::{Watcher, project_of, watch_files};

use crate::{Connection, Daemon};

const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(120);
/// How long a request waits for a reconnection attempt it triggered.
const CONNECT_WAIT: Duration = Duration::from_secs(10);

pub(crate) struct Hub {
    hosts: Vec<Arc<Host>>,
    /// Forwarded watches by connection and watch id; re-made on every reconnection.
    forwards: Mutex<HashMap<(u64, u64), Forward>>,
    /// Each local config file an override follows, and the remote worktrees following it.
    followed: Mutex<HashMap<PathBuf, (Watcher, Followers)>>,
    /// The origin of each known local project path.
    origins: Mutex<HashMap<PathBuf, Option<String>>>,
}

/// `(host, worktree)` pairs.
type Followers = HashSet<(String, PathBuf)>;

pub(crate) struct Host {
    name: String,
    argv: Vec<String>,
    link: Mutex<Link>,
    changed: Condvar,
}

#[derive(Default)]
struct Link {
    client: Option<Arc<Client>>,
    error: String,
    /// Finished connection attempts.
    attempts: u64,
    /// A request asks for an attempt now rather than after the backoff.
    wake: bool,
    /// The origin of each remote worktree seen.
    origins: HashMap<PathBuf, Option<String>>,
    /// The overrides sent on the current connection.
    sent: HashMap<PathBuf, Option<Layer>>,
}

struct Forward {
    connection: Weak<Connection>,
    host: Arc<Host>,
    worktree: PathBuf,
    stop: Arc<AtomicBool>,
}

/// Ends a forwarded watch when dropped.
struct ForwardHandle {
    daemon: Weak<Daemon>,
    key: (u64, u64),
}

impl Drop for ForwardHandle {
    fn drop(&mut self) {
        if let Some(daemon) = self.daemon.upgrade()
            && let Some(forward) = daemon.hub.forwards.lock().expect("forwards").remove(&self.key)
        {
            forward.stop.store(true, Ordering::Relaxed);
        }
    }
}

impl Hub {
    pub(crate) fn new(hosts: &[HostConfig]) -> Self {
        Self {
            hosts: hosts
                .iter()
                .map(|host| {
                    Arc::new(Host {
                        name: host.name.clone(),
                        argv: host.argv(),
                        link: Mutex::default(),
                        changed: Condvar::new(),
                    })
                })
                .collect(),
            forwards: Mutex::default(),
            followed: Mutex::default(),
            origins: Mutex::default(),
        }
    }

    /// Connects to every host in the background, retrying with backoff.
    pub(crate) fn start(daemon: &Arc<Daemon>) {
        for host in &daemon.hub.hosts {
            let (daemon, host) = (Arc::downgrade(daemon), Arc::clone(host));
            std::thread::spawn(move || supervise(&daemon, &host));
        }
    }

    pub(crate) fn states(&self) -> Vec<HostState> {
        self.hosts
            .iter()
            .map(|host| {
                let link = host.link.lock().expect("link");
                HostState {
                    name: host.name.clone(),
                    connected: link.client.is_some(),
                    error: link.client.is_none().then(|| link.error.clone()),
                }
            })
            .collect()
    }

    pub(crate) fn send_user(&self, user: &Layer) {
        for host in &self.hosts {
            let client = host.link.lock().expect("link").client.clone();
            if let Some(client) = client {
                let _ = client.send_user(user.clone());
            }
        }
    }

    fn host(&self, name: &str) -> Option<&Arc<Host>> {
        self.hosts.iter().find(|host| host.name == name)
    }

    /// Every connected host's known projects, tagged with the host, in config order.
    fn remote_projects(&self, except: Option<&str>) -> Vec<Project> {
        let mut projects = Vec::new();
        for host in self.hosts.iter().filter(|host| Some(host.name.as_str()) != except) {
            let client = host.link.lock().expect("link").client.clone();
            if let Some(Ok(listed)) = client.map(|client| client.projects(None)) {
                projects.extend(listed.into_iter().map(|project| host.tag(project)));
            }
        }
        projects
    }

    /// Local projects lead, then every connected host's, merged by origin.
    pub(crate) fn merge_all(&self, local: Vec<Project>) -> Vec<Project> {
        merge(local.into_iter().chain(self.remote_projects(None)).collect())
    }

    /// `project`, from `host` or local, merged with the projects of the same origin elsewhere: the
    /// local one leads, then `project`, then other hosts'.
    pub(crate) fn merged(&self, daemon: &Daemon, project: Project, host: Option<&str>) -> Project {
        let Some(origin) = project.origin.clone() else { return project };
        let mut group = Vec::new();
        if host.is_some()
            && let Some(local) = self.local_with_origin(daemon, &origin)
        {
            group.push(local);
        }
        group.push(project);
        let others = self.remote_projects(host);
        group.extend(
            others
                .into_iter()
                .filter(|project| project.origin.as_deref() == Some(origin.as_str())),
        );
        merge(group).into_iter().next().expect("the merged project")
    }

    /// The known local project with `origin`, freshly listed.
    fn local_with_origin(&self, daemon: &Daemon, origin: &str) -> Option<Project> {
        for path in daemon.known.paths() {
            let cached = self.origins.lock().expect("origins").get(&path).cloned();
            let known = match cached {
                Some(known) => known,
                None => match project_of(&path) {
                    Ok(project) => {
                        self.origins.lock().expect("origins").insert(path.clone(), project.origin.clone());
                        if project.origin.as_deref() == Some(origin) {
                            return Some(project);
                        }
                        continue;
                    }
                    Err(_) => continue,
                },
            };
            if known.as_deref() == Some(origin)
                && let Ok(project) = project_of(&path)
            {
                return Some(project);
            }
        }
        None
    }

    /// Handles a request naming a host.
    pub(crate) fn forward(
        &self,
        daemon: &Arc<Daemon>,
        connection: &Arc<Connection>,
        id: u64,
        name: &str,
        request: Request,
    ) -> Result<Response, Error> {
        let host = self.host(name).ok_or(Error::UnknownHost)?;
        let client = host.client()?;
        match request {
            Request::Resolve { root } => {
                let mut resolved = client.resolve(None, &root)?;
                host.learn(&resolved.project);
                let worktree = resolved.project.worktrees[resolved.worktree].path.clone();
                if self.ensure_override(daemon, host, &client, &worktree)? {
                    resolved = client.resolve(None, &root)?;
                }
                let project = self.merged(daemon, host.tag(resolved.project), Some(name));
                let index = project
                    .worktrees
                    .iter()
                    .position(|listed| listed.host.as_deref() == Some(name) && listed.path == worktree)
                    .unwrap_or(0);
                Ok(Response::Resolved(Resolved {
                    project,
                    worktree: index,
                    statuses: resolved.statuses,
                }))
            }
            Request::Project { key } => {
                let project = client.project(None, &key)?;
                host.learn(&project);
                Ok(Response::Project(self.merged(daemon, host.tag(project), Some(name))))
            }
            Request::Projects => {
                let projects = client.projects(None)?;
                projects.iter().for_each(|project| host.learn(project));
                Ok(Response::Projects(projects.into_iter().map(|project| host.tag(project)).collect()))
            }
            Request::Corpus { ref worktree } | Request::Open { ref worktree, .. } => {
                self.ensure_override(daemon, host, &client, worktree)?;
                client.call(None, request)
            }
            Request::Watch { worktree } => {
                self.ensure_override(daemon, host, &client, &worktree)?;
                let remote = client.watch(None, &worktree)?;
                let stop = Arc::new(AtomicBool::new(false));
                pump(remote, Arc::downgrade(connection), id, Arc::clone(&stop));
                let forward = Forward {
                    connection: Arc::downgrade(connection),
                    host: Arc::clone(host),
                    worktree,
                    stop,
                };
                let key = (connection.id, id);
                self.forwards.lock().expect("forwards").insert(key, forward);
                let handle = ForwardHandle {
                    daemon: Arc::downgrade(daemon),
                    key,
                };
                connection.watches.lock().expect("connection watches").insert(id, Box::new(handle));
                Ok(Response::Watching)
            }
            Request::Hosts | Request::Unwatch { .. } => client.call(None, request),
        }
    }

    /// Makes `worktree` on `host` read the local main worktree's `.spec-viewer.toml` when its
    /// project has a local clone, and its own file otherwise. Returns whether that changed.
    fn ensure_override(&self, daemon: &Arc<Daemon>, host: &Arc<Host>, client: &Client, worktree: &Path) -> Result<bool, Error> {
        let known = host.link.lock().expect("link").origins.get(worktree).cloned();
        let origin = match known {
            Some(origin) => origin,
            None => match client.resolve(None, worktree) {
                Ok(resolved) => {
                    host.learn(&resolved.project);
                    resolved.project.origin
                }
                // The forwarded request reports the error.
                Err(_) => return Ok(false),
            },
        };
        let desired = match origin.and_then(|origin| self.local_with_origin(daemon, &origin)) {
            Some(local) => {
                let file = local.path.join(".spec-viewer.toml");
                self.follow(daemon, &file, &host.name, worktree);
                Some(
                    Layer::read(&file)
                        .map_err(|error| Error::Config {
                            message: error.to_string(),
                        })?
                        .unwrap_or_default(),
                )
            }
            None => None,
        };
        let mut link = host.link.lock().expect("link");
        if link.sent.get(worktree).cloned().flatten() == desired {
            return Ok(false);
        }
        client.send_override(worktree, desired.clone())?;
        link.sent.insert(worktree.to_path_buf(), desired);
        Ok(true)
    }

    /// Re-sends the overrides that follow `file` whenever it changes.
    fn follow(&self, daemon: &Arc<Daemon>, file: &Path, host: &str, worktree: &Path) {
        let mut followed = self.followed.lock().expect("followed");
        if let Some((_, followers)) = followed.get_mut(file) {
            followers.insert((host.to_owned(), worktree.to_path_buf()));
            return;
        }
        let (weak, changed) = (Arc::downgrade(daemon), file.to_path_buf());
        let watcher = watch_files(vec![file.to_path_buf()], move || {
            let Some(daemon) = weak.upgrade() else { return false };
            daemon.hub.refresh(&daemon, &changed);
            true
        });
        match watcher {
            Ok(watcher) => {
                followed.insert(
                    file.to_path_buf(),
                    (watcher, HashSet::from([(host.to_owned(), worktree.to_path_buf())])),
                );
            }
            Err(error) => eprintln!("spec-viewer: cannot watch {}: {error}", file.display()),
        }
    }

    fn refresh(&self, daemon: &Arc<Daemon>, file: &Path) {
        let followers = self
            .followed
            .lock()
            .expect("followed")
            .get(file)
            .map(|(_, followers)| followers.clone())
            .unwrap_or_default();
        for (name, worktree) in followers {
            let Some(host) = self.host(&name) else { continue };
            let client = host.link.lock().expect("link").client.clone();
            if let Some(client) = client
                && let Err(error) = self.ensure_override(daemon, host, &client, &worktree)
            {
                eprintln!("spec-viewer: {error}");
            }
        }
    }
}

impl Host {
    fn tag(&self, mut project: Project) -> Project {
        for worktree in &mut project.worktrees {
            worktree.host = Some(self.name.clone());
        }
        project
    }

    fn learn(&self, project: &Project) {
        let mut link = self.link.lock().expect("link");
        for worktree in &project.worktrees {
            link.origins.insert(worktree.path.clone(), project.origin.clone());
        }
    }

    /// The connection, after one reconnection attempt when there is none.
    fn client(&self) -> Result<Arc<Client>, Error> {
        let mut link = self.link.lock().expect("link");
        if let Some(client) = &link.client {
            return Ok(Arc::clone(client));
        }
        let target = link.attempts + 1;
        link.wake = true;
        self.changed.notify_all();
        let deadline = Instant::now() + CONNECT_WAIT;
        while link.attempts < target && link.client.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            link = self.changed.wait_timeout(link, left).expect("link").0;
        }
        link.client
            .clone()
            .ok_or_else(|| Error::unreachable(format!("{}: {}", self.name, link.error)))
    }

    fn finish(&self, result: Result<Arc<Client>, String>) {
        let mut link = self.link.lock().expect("link");
        link.attempts += 1;
        link.sent.clear();
        match result {
            Ok(client) => link.client = Some(client),
            Err(error) => link.error = error,
        }
        self.changed.notify_all();
    }

    fn lost(&self, error: String) {
        let mut link = self.link.lock().expect("link");
        link.client = None;
        link.error = error;
        self.changed.notify_all();
    }

    /// Waits `backoff`, or less when a request asks for an attempt.
    fn wait(&self, backoff: Duration) {
        let deadline = Instant::now() + backoff;
        let mut link = self.link.lock().expect("link");
        while !link.wake {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            link = self.changed.wait_timeout(link, left).expect("link").0;
        }
        link.wake = false;
    }
}

fn supervise(daemon: &Weak<Daemon>, host: &Arc<Host>) {
    let mut backoff = FIRST_RETRY;
    loop {
        let Some(user) = daemon.upgrade().map(|daemon| daemon.user.lock().expect("user layer").clone()) else {
            return;
        };
        match connect(&host.argv, user) {
            Ok((client, mut child)) => {
                host.finish(Ok(Arc::new(client)));
                backoff = FIRST_RETRY;
                if let Some(daemon) = daemon.upgrade() {
                    daemon.host_connected(host);
                }
                let status = child.wait().map_or_else(|error| error.to_string(), |status| status.to_string());
                host.lost(format!("the connection ended ({status})"));
                if let Some(daemon) = daemon.upgrade() {
                    daemon.host_down(host);
                }
            }
            Err(error) => host.finish(Err(error)),
        }
        host.wait(backoff);
        backoff = (backoff * 2).min(LAST_RETRY);
    }
}

fn connect(argv: &[String], user: Layer) -> Result<(Client, Child), String> {
    let (program, args) = argv.split_first().ok_or("an empty command")?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    let (stdout, stdin) = (child.stdout.take().expect("piped stdout"), child.stdin.take().expect("piped stdin"));
    match Client::over(stdout, stdin, Some(user)) {
        Ok(client) => Ok((client, child)),
        Err(error) => {
            let _ = child.kill();
            let status = child.wait().map_or_else(|error| error.to_string(), |status| status.to_string());
            Err(format!("{error} ({status})"))
        }
    }
}

/// Forwards a remote watch's changes as `Changed { id }` until stopped or disconnected.
fn pump(remote: Watch, connection: Weak<Connection>, id: u64, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        loop {
            match remote.recv_timeout(Duration::from_millis(500)) {
                Ok(Event::Changed) if !stop.load(Ordering::Relaxed) => match connection.upgrade() {
                    Some(connection) => connection.send(&DaemonMessage::Changed { id }),
                    None => return,
                },
                Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

/// Merges projects with the same origin into the first, which leads; one without origin never merges.
fn merge(projects: Vec<Project>) -> Vec<Project> {
    let mut merged: Vec<Project> = Vec::new();
    for project in projects {
        let lead = project
            .origin
            .as_ref()
            .and_then(|origin| merged.iter_mut().find(|lead| lead.origin.as_ref() == Some(origin)));
        match lead {
            Some(lead) => lead.worktrees.extend(project.worktrees),
            None => merged.push(project),
        }
    }
    merged
}

impl Daemon {
    /// Re-makes the host's forwarded watches; every watch receives `Changed`.
    fn host_connected(&self, host: &Arc<Host>) {
        let client = host.link.lock().expect("link").client.clone();
        if let Some(client) = client {
            let mut forwards = self.hub.forwards.lock().expect("forwards");
            for ((_, id), forward) in forwards.iter_mut().filter(|(_, forward)| Arc::ptr_eq(&forward.host, host)) {
                if let Ok(remote) = client.watch(None, &forward.worktree) {
                    forward.stop.store(true, Ordering::Relaxed);
                    forward.stop = Arc::new(AtomicBool::new(false));
                    pump(remote, forward.connection.clone(), *id, Arc::clone(&forward.stop));
                }
            }
        }
        for connection in self.live_connections() {
            connection.changed_all();
        }
    }

    /// Every connection receives `HostDown`, and every watch `Changed`.
    fn host_down(&self, host: &Host) {
        for connection in self.live_connections() {
            connection.send(&DaemonMessage::HostDown { host: host.name.clone() });
            connection.changed_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spec_viewer_core::Worktree;

    fn project(name: &str, origin: Option<&str>, host: Option<&str>) -> Project {
        Project {
            key: format!("/{name}/.git").into(),
            name: name.into(),
            path: format!("/{name}").into(),
            origin: origin.map(str::to_owned),
            worktrees: vec![Worktree {
                host: host.map(str::to_owned),
                name: "main".into(),
                main: true,
                path: format!("/{name}").into(),
            }],
        }
    }

    #[test]
    fn vc_10_projects_merge_by_origin_into_the_first_and_never_without_one() {
        let merged = merge(vec![
            project("a", Some("github.com/o/r"), None),
            project("c", None, Some("box")),
            project("b", Some("github.com/o/r"), Some("box")),
            project("d", None, Some("box")),
        ]);
        assert_eq!(
            merged.iter().map(|project| project.name.as_str()).collect::<Vec<_>>(),
            ["a", "c", "d"]
        );
        assert_eq!(
            merged[0]
                .worktrees
                .iter()
                .map(|worktree| (worktree.host.as_deref(), worktree.path.to_str().unwrap()))
                .collect::<Vec<_>>(),
            [(None, "/a"), (Some("box"), "/b")]
        );
    }
}
