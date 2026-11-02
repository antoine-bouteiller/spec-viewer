use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    /// The configured host serving it; `None` is the answering daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub name: String,
    pub main: bool,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub key: PathBuf,
    pub name: String,
    pub path: PathBuf,
    /// The normalized `remote.origin.url`, which identifies one repository across hosts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub worktrees: Vec<Worktree>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocError {
    Forbidden,
    NotFound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDoc {
    pub file: PathBuf,
    pub real: PathBuf,
    pub root_path: String,
}
