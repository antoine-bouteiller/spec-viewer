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
    /// Nested under the document sharing its stem, per the tab's `companions`.
    pub companion: bool,
    pub children: Option<Vec<DocNode>>,
}

/// A configured tab and its documents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabDocs {
    pub name: String,
    pub nodes: Vec<DocNode>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Docs {
    /// Tab documents changed from the remote `main`; `None` when unknown.
    pub changed: Option<Vec<String>>,
    pub project: String,
    pub tabs: Vec<TabDocs>,
}
