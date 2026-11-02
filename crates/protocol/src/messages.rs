use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};
use spec_viewer_config::{Layer, StatusConfig};
use spec_viewer_core::{Corpus, Opened, Project};

/// Bumped by any change to the wire format, including the pinned snapshots.
pub const PROTOCOL: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// The first line, exactly once. `user` is the project sections of the sender's user config,
    /// `None` for the daemon's own.
    Hello { protocol: u32, user: Option<Layer> },
    /// `id` is unique per connection; `host: None` is the receiving daemon.
    Request { id: u64, host: Option<String>, request: Request },
    /// The sender's user config changed.
    User { user: Layer },
    /// Replaces the worktree's `.spec-viewer.toml` for this connection; `None` restores it.
    Override { worktree: PathBuf, layer: Option<Layer> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Hosts,
    /// The known projects of this daemon and its connected hosts, merged by origin.
    Projects,
    /// `root` is absolute on the target host.
    Resolve {
        root: PathBuf,
    },
    /// A fresh worktree list.
    Project {
        key: PathBuf,
    },
    Corpus {
        worktree: PathBuf,
    },
    Open {
        worktree: PathBuf,
        path: String,
    },
    /// Answered with `Watching`, then one `Changed` with this request's id per burst of changes.
    Watch {
        worktree: PathBuf,
    },
    Unwatch {
        id: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonMessage {
    /// `web` is the gateway URL.
    Welcome {
        protocol: u32,
        version: String,
        web: Option<String>,
    },
    Response {
        id: u64,
        result: Result<Response, Error>,
    },
    Changed {
        id: u64,
    },
    /// Once per lost connection; open watches on the host resume with a `Changed` on reconnect.
    HostDown {
        host: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Hosts(Vec<HostState>),
    Projects(Vec<Project>),
    Resolved(Resolved),
    Project(Project),
    Corpus(Corpus),
    Opened(Opened),
    Watching,
    Unwatched,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostState {
    pub name: String,
    pub connected: bool,
    /// Why the last connection attempt failed, while disconnected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolved {
    pub project: Project,
    /// The index of the worktree containing the resolved root, by longest path prefix.
    pub worktree: usize,
    /// The project's configured statuses, for the frontend to resolve against its own palette.
    pub statuses: Vec<StatusConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Error {
    Protocol {
        daemon: u32,
        client: u32,
    },
    UnknownHost,
    HostUnreachable {
        message: String,
    },
    /// The root is not inside a Git worktree.
    NotAProject,
    /// The worktree is not listed by its project.
    UnlistedWorktree,
    Forbidden,
    NotFound,
    /// The worktree's config does not load.
    Config {
        message: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol { daemon, client } => write!(f, "the daemon speaks protocol {daemon}, this client {client}"),
            Self::UnknownHost => f.write_str("unknown host"),
            Self::HostUnreachable { message } | Self::Config { message } => f.write_str(message),
            Self::NotAProject => f.write_str("not inside a Git worktree"),
            Self::UnlistedWorktree => f.write_str("worktree not listed by its project"),
            Self::Forbidden => f.write_str("forbidden"),
            Self::NotFound => f.write_str("not found"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub fn unreachable(message: impl fmt::Display) -> Self {
        Self::HostUnreachable {
            message: message.to_string(),
        }
    }
}
