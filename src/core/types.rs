use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worktree {
    pub name: String,
    pub main: bool,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Project {
    #[serde(skip)]
    pub key: PathBuf,
    pub name: String,
    pub path: PathBuf,
    pub worktrees: Vec<Worktree>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    pub related: Vec<String>,
    pub tasks: Counts,
    pub acceptance: Counts,
    pub phases: Vec<String>,
    pub lines: usize,
    pub legacy: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(rename = "parent-spec", skip_serializing_if = "Option::is_none")]
    pub parent_spec: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocMeta {
    pub title: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Counts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Counts>,
    #[serde(rename = "parent-spec", skip_serializing_if = "Option::is_none")]
    pub parent_spec: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocNode {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Counts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<Counts>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<DocNode>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Docs {
    #[serde(rename = "changedSpecs")]
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
