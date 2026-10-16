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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub title: String,
    pub status: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub related: Vec<String>,
    pub tasks: Counts,
    pub acceptance: Counts,
    pub phases: Vec<String>,
    pub lines: usize,
    pub legacy: bool,
    pub kind: Option<String>,
    pub parent_spec: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocMeta {
    pub title: String,
    pub path: String,
    pub kind: Option<String>,
    pub status: Option<String>,
    pub archived: Option<bool>,
    pub tasks: Option<Counts>,
    pub acceptance: Option<Counts>,
    pub parent_spec: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocNode {
    pub name: String,
    pub path: Option<String>,
    pub kind: String,
    pub status: Option<String>,
    pub archived: Option<bool>,
    pub tasks: Option<Counts>,
    pub acceptance: Option<Counts>,
    pub children: Option<Vec<DocNode>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Docs {
    pub changed_specs: Option<Vec<String>>,
    pub plans: Vec<DocNode>,
    pub project: String,
    pub specs: Vec<DocNode>,
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
