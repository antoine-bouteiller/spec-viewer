//! Remote hosts: one protocol connection per configured host over its command's stdio, request
//! forwarding, project merging by origin, and the overrides that make a remote clone follow the
//! local `.spec-viewer.toml`.
use std::{
    collections::{HashMap, HashSet},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
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
/// How long a host command has to answer the handshake.
const HANDSHAKE: Duration = Duration::from_secs(30);

pub(crate) struct Hub {
    hosts: Vec<Arc<Host>>,
    /// Forwarded watches by connection and watch id; re-made on every reconnection.
    forwards: Mutex<HashMap<(u64, u64), Forward>>,
    /// A watcher per local config file that a forwarded watch's override follows.
    followed: Mutex<HashMap<PathBuf, Watcher>>,
    /// The origin of each known local project path.
    origins: Mutex<HashMap<PathBuf, Option<String>>>,
}

pub(crate) struct Host {
    name: String,
    argv: Vec<String>,
    link: Mutex<Link>,
    changed: Condvar,
}

#[derive(Default)]
struct Link {
    client: Option<Arc<Client>>,
    /// Counts connections, so work begun on a lost one is not credited to its successor.
    generation: u64,
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
    /// Stops the pump of the current remote watch.
    stop: Arc<AtomicBool>,
    /// The connection generation of the current remote watch; 0 before the first.
    generation: u64,
    /// The local config file the worktree's override follows.
    file: Option<PathBuf>,
}

/// Ends a forwarded watch when dropped.
struct ForwardHandle {
    daemon: Weak<Daemon>,
    key: (u64, u64),
}

impl Drop for ForwardHandle {
    fn drop(&mut self) {
        if let Some(daemon) = self.daemon.upgrade() {
            daemon.hub.unforward(self.key);
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

    /// The caller holds the daemon's user layer lock, so sends follow the order of edits.
    pub(crate) fn send_user(&self, user: &Layer) {
        for host in &self.hosts {
            let client = host.link.lock().expect("link").client.clone();
            if let Some(client) = client {
                let _ = client.send_user(user.clone());
            }
        }
    }

    pub(crate) fn followed_len(&self) -> usize {
        self.followed.lock().expect("followed").len()
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
        let (client, generation) = host.client()?;
        match request {
            Request::Resolve { root } => {
                // The override applies before Resolve loads the worktree's config.
                if let Ok(project) = client.project(None, &root) {
                    host.learn(&project);
                    // An uncanonical root matches no worktree; every one takes the same override.
                    let worktrees = match containing(&project, &root) {
                        Some(worktree) => vec![worktree],
                        None => project.worktrees.iter().map(|worktree| worktree.path.clone()).collect(),
                    };
                    for worktree in worktrees {
                        self.ensure_override(daemon, host, &client, generation, &worktree)?;
                    }
                }
                let mut resolved = client.resolve(None, &root)?;
                host.learn(&resolved.project);
                let worktree = resolved.project.worktrees[resolved.worktree].path.clone();
                if self.ensure_override(daemon, host, &client, generation, &worktree)?.0 {
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
                self.ensure_override(daemon, host, &client, generation, worktree)?;
                client.call(None, request)
            }
            Request::Watch { worktree } => self.watch(daemon, connection, id, host, worktree),
            Request::Hosts | Request::Unwatch { .. } => client.call(None, request),
        }
    }

    /// Registers the forwarded watch before making the remote one, so a reconnection meanwhile
    /// re-makes it; a remote watch made on a connection lost since is made again on the current one.
    fn watch(
        &self,
        daemon: &Arc<Daemon>,
        connection: &Arc<Connection>,
        id: u64,
        host: &Arc<Host>,
        worktree: PathBuf,
    ) -> Result<Response, Error> {
        let (mut client, mut generation) = host.client()?;
        let (_, file) = self.ensure_override(daemon, host, &client, generation, &worktree)?;
        let key = (connection.id, id);
        self.forwards.lock().expect("forwards").insert(
            key,
            Forward {
                connection: Arc::downgrade(connection),
                host: Arc::clone(host),
                worktree: worktree.clone(),
                stop: Arc::new(AtomicBool::new(false)),
                generation: 0,
                file: file.clone(),
            },
        );
        let handle = ForwardHandle {
            daemon: Arc::downgrade(daemon),
            key,
        };
        if let Some(file) = &file {
            self.follow(daemon, file);
        }
        loop {
            let made = client.watch(None, &worktree);
            let current = {
                let link = host.link.lock().expect("link");
                link.client.clone().map(|client| (client, link.generation))
            };
            let mut forwards = self.forwards.lock().expect("forwards");
            let Some(forward) = forwards.get_mut(&key) else {
                return Err(Error::unreachable("the watch ended while it started"));
            };
            if forward.generation != 0 {
                break;
            }
            match (made, current) {
                (Ok(remote), Some((_, now))) if now == generation => {
                    forward.generation = generation;
                    pump(remote, Arc::downgrade(connection), id, Arc::clone(&forward.stop));
                    break;
                }
                (Err(error), Some((_, now))) if now == generation => {
                    drop(forwards);
                    drop(handle);
                    return Err(error);
                }
                // Lost: the reconnection re-makes it.
                (_, None) => break,
                (_, Some((now_client, now))) => {
                    drop(forwards);
                    (client, generation) = (now_client, now);
                    self.ensure_override(daemon, host, &client, generation, &worktree)?;
                }
            }
        }
        connection.watches.lock().expect("connection watches").insert(id, Box::new(handle));
        Ok(Response::Watching)
    }

    fn unforward(&self, key: (u64, u64)) {
        let removed = self.forwards.lock().expect("forwards").remove(&key);
        if let Some(forward) = removed {
            forward.stop.store(true, Ordering::Relaxed);
            if let Some(file) = forward.file
                && !self
                    .forwards
                    .lock()
                    .expect("forwards")
                    .values()
                    .any(|other| other.file.as_ref() == Some(&file))
            {
                self.followed.lock().expect("followed").remove(&file);
            }
        }
    }

    /// Makes `worktree` on `host` read the local main worktree's `.spec-viewer.toml` when its
    /// project has a local clone, and its own file otherwise. Returns whether that changed, and the
    /// local file it reads.
    fn ensure_override(
        &self,
        daemon: &Arc<Daemon>,
        host: &Arc<Host>,
        client: &Client,
        generation: u64,
        worktree: &Path,
    ) -> Result<(bool, Option<PathBuf>), Error> {
        let known = host.link.lock().expect("link").origins.get(worktree).cloned();
        let origin = match known {
            Some(origin) => origin,
            // Project, unlike Resolve, loads no config, so a broken remote file can be replaced.
            None => match client.project(None, worktree) {
                Ok(project) => {
                    host.learn(&project);
                    project.origin
                }
                // The forwarded request reports the error.
                Err(_) => return Ok((false, None)),
            },
        };
        let file = origin
            .and_then(|origin| self.local_with_origin(daemon, &origin))
            .map(|local| local.path.join(".spec-viewer.toml"));
        self.retarget(daemon, host, worktree, file.as_deref());
        let desired = match &file {
            Some(file) => Some(
                Layer::read(file)
                    .map_err(|error| Error::Config {
                        message: error.to_string(),
                    })?
                    .unwrap_or_default(),
            ),
            None => None,
        };
        let mut link = host.link.lock().expect("link");
        // Overrides belong to a connection; one sent on a lost connection would be credited to the next.
        if link.generation != generation || link.sent.get(worktree).cloned().flatten() == desired {
            return Ok((false, file));
        }
        client.send_override(worktree, desired.clone())?;
        link.sent.insert(worktree.to_path_buf(), desired);
        Ok((true, file))
    }

    /// Points the forwarded watches of `worktree` on `host` at `file`, as a local clone appears,
    /// goes, or changes, and follows only the files some forward still uses.
    fn retarget(&self, daemon: &Arc<Daemon>, host: &Arc<Host>, worktree: &Path, file: Option<&Path>) {
        let used: HashSet<PathBuf> = {
            let mut forwards = self.forwards.lock().expect("forwards");
            let mut moved = false;
            for forward in forwards.values_mut() {
                if Arc::ptr_eq(&forward.host, host) && forward.worktree == worktree && forward.file.as_deref() != file {
                    forward.file = file.map(Path::to_path_buf);
                    moved = true;
                }
            }
            if !moved {
                return;
            }
            forwards.values().filter_map(|forward| forward.file.clone()).collect()
        };
        self.followed
            .lock()
            .expect("followed")
            .retain(|followed, _| used.contains(followed));
        if let Some(file) = file {
            self.follow(daemon, file);
        }
    }

    /// Watches `file` while a forwarded watch's override follows it.
    fn follow(&self, daemon: &Arc<Daemon>, file: &Path) {
        let mut followed = self.followed.lock().expect("followed");
        if followed.contains_key(file) {
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
                followed.insert(file.to_path_buf(), watcher);
            }
            Err(error) => eprintln!("spec-viewer: cannot watch {}: {error}", file.display()),
        }
    }

    /// Re-sends the overrides following `file`; their watches receive `Changed` either way, and a
    /// file that no longer loads fails their next request.
    fn refresh(&self, daemon: &Arc<Daemon>, file: &Path) {
        let followers: Vec<_> = self
            .forwards
            .lock()
            .expect("forwards")
            .iter()
            .filter(|(_, forward)| forward.file.as_deref() == Some(file))
            .map(|(_, forward)| (Arc::clone(&forward.host), forward.worktree.clone()))
            .collect();
        let mut done = HashSet::new();
        for (host, worktree) in followers {
            let current = {
                let link = host.link.lock().expect("link");
                link.client.clone().map(|client| (client, link.generation))
            };
            let Some((client, generation)) = current else { continue };
            if !done.insert((host.name.clone(), worktree.clone())) {
                continue;
            }
            match self.ensure_override(daemon, &host, &client, generation, &worktree) {
                // The remote daemon sends `Changed` for the new override.
                Ok((true, _)) => {}
                // An unchanged layer may follow a broken edit, which the watches saw fail.
                Ok((false, _)) => self.notify(&host, &worktree),
                Err(error) => {
                    eprintln!("spec-viewer: {error}");
                    self.notify(&host, &worktree);
                }
            }
        }
    }

    /// Sends `Changed` to every forwarded watch of `worktree` on `host`.
    fn notify(&self, host: &Arc<Host>, worktree: &Path) {
        let targets: Vec<_> = self
            .forwards
            .lock()
            .expect("forwards")
            .iter()
            .filter(|(_, forward)| Arc::ptr_eq(&forward.host, host) && forward.worktree == worktree)
            .map(|((_, id), forward)| (forward.connection.clone(), *id))
            .collect();
        for (connection, id) in targets {
            if let Some(connection) = connection.upgrade() {
                connection.send(&DaemonMessage::Changed { id });
            }
        }
    }
}

/// The worktree of `project` containing `root`, by longest path prefix.
fn containing(project: &Project, root: &Path) -> Option<PathBuf> {
    project
        .worktrees
        .iter()
        .filter(|worktree| root.starts_with(&worktree.path))
        .max_by_key(|worktree| worktree.path.components().count())
        .map(|worktree| worktree.path.clone())
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

    /// The connection and its generation, after one reconnection attempt when there is none.
    fn client(&self) -> Result<(Arc<Client>, u64), Error> {
        let mut link = self.link.lock().expect("link");
        if let Some(client) = &link.client {
            return Ok((Arc::clone(client), link.generation));
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
        match &link.client {
            Some(client) => Ok((Arc::clone(client), link.generation)),
            None => Err(Error::unreachable(format!("{}: {}", self.name, link.error))),
        }
    }

    fn finish(&self, result: Result<Arc<Client>, String>) {
        let mut link = self.link.lock().expect("link");
        link.attempts += 1;
        link.sent.clear();
        match result {
            Ok(client) => {
                link.client = Some(client);
                link.generation += 1;
            }
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
        match connect(&host.argv, user.clone()) {
            Ok((client, mut child)) => {
                let client = Arc::new(client);
                host.finish(Ok(Arc::clone(&client)));
                backoff = FIRST_RETRY;
                if let Some(daemon) = daemon.upgrade() {
                    {
                        // An edit during the handshake skipped this host.
                        let current = daemon.user.lock().expect("user layer");
                        if *current != user {
                            let _ = client.send_user(current.clone());
                        }
                    }
                    daemon.host_connected(host);
                }
                // A command may outlive the protocol on its stdio, or end it.
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(status)) => break status.to_string(),
                        Ok(None) if client.is_lost() => break kill(&mut child),
                        Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                        Err(error) => break error.to_string(),
                    }
                };
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

/// Runs the host command and completes the handshake within `HANDSHAKE`.
fn connect(argv: &[String], user: Layer) -> Result<(Client, Child), String> {
    let (program, args) = argv.split_first().ok_or("an empty command")?;
    // Its own process group, so killing it also ends descendants holding its stdout.
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .process_group(0)
        .spawn()
        .map_err(|error| format!("cannot run {program}: {error}"))?;
    let (stdout, stdin) = (child.stdout.take().expect("piped stdout"), child.stdin.take().expect("piped stdin"));
    let (done, handshake) = mpsc::channel();
    std::thread::spawn(move || done.send(Client::over(stdout, stdin, Some(user))));
    let result = handshake
        .recv_timeout(HANDSHAKE)
        .unwrap_or_else(|_| Err(Error::unreachable(format!("no answer within {} s", HANDSHAKE.as_secs()))));
    match result {
        Ok(client) => Ok((client, child)),
        Err(error) => {
            let status = kill(&mut child);
            Err(format!("{error} ({status})"))
        }
    }
}

/// Kills the command's process group and reaps it.
fn kill(child: &mut Child) -> String {
    if let Ok(group) = i32::try_from(child.id()) {
        // SAFETY: signals the group `connect` created for this child.
        unsafe { libc::kill(-group, libc::SIGKILL) };
    }
    child.wait().map_or_else(|error| error.to_string(), |status| status.to_string())
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
                Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
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
    /// Restores the overrides, re-makes the host's forwarded watches, and sends `Changed` to every
    /// watch. Blocking requests run without the forwards lock.
    fn host_connected(self: &Arc<Self>, host: &Arc<Host>) {
        let current = {
            let link = host.link.lock().expect("link");
            link.client.clone().map(|client| (client, link.generation))
        };
        if let Some((client, generation)) = current {
            let pending: Vec<_> = self
                .hub
                .forwards
                .lock()
                .expect("forwards")
                .iter()
                .filter(|(_, forward)| Arc::ptr_eq(&forward.host, host) && forward.generation < generation)
                .map(|(key, forward)| (*key, forward.worktree.clone()))
                .collect();
            for (key, worktree) in pending {
                if let Err(error) = self.hub.ensure_override(self, host, &client, generation, &worktree) {
                    eprintln!("spec-viewer: {error}");
                }
                let Ok(remote) = client.watch(None, &worktree) else { continue };
                let mut forwards = self.hub.forwards.lock().expect("forwards");
                if let Some(forward) = forwards.get_mut(&key)
                    && forward.generation < generation
                {
                    forward.stop.store(true, Ordering::Relaxed);
                    forward.stop = Arc::new(AtomicBool::new(false));
                    forward.generation = generation;
                    pump(remote, forward.connection.clone(), key.1, Arc::clone(&forward.stop));
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
