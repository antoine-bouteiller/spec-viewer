use crate::Status;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocMeta {
    pub title: String,
    pub path: String,
    pub kind: Option<String>,
    pub status: Option<Status>,
    pub archived: Option<bool>,
    pub parent_spec: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocNode {
    pub name: String,
    pub path: Option<String>,
    pub kind: String,
    pub status: Option<Status>,
    pub archived: Option<bool>,
    pub children: Option<Vec<DocNode>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Docs {
    pub changed_specs: Option<Vec<String>>,
    pub plans: Vec<DocNode>,
    pub project: String,
    pub specs: Vec<DocNode>,
}
