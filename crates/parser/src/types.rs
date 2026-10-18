#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub done: usize,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub name: String,
    pub known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub label: String,
    pub value: ChipValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipValue {
    Checklist { done: usize, total: usize },
    Count(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub title: String,
    pub status: Option<Status>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub related: Vec<String>,
    pub chips: Vec<Chip>,
    pub lines: usize,
    pub legacy: bool,
    pub kind: Option<String>,
    pub parent_spec: Option<String>,
    pub archived: Option<bool>,
}
