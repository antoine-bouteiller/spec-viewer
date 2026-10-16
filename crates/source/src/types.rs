use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub name: String,
    pub main: bool,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub key: PathBuf,
    pub name: String,
    pub path: PathBuf,
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
