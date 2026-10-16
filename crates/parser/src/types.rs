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
