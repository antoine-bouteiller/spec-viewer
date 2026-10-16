//! Renderer-independent document structure. Source code stays unhighlighted.
use crate::{Degraded, Meta, TocEntry};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document {
    pub meta: Meta,
    pub blocks: Vec<Block>,
    pub toc: Vec<TocEntry>,
    pub degraded: Option<Degraded>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Heading {
        depth: u8,
        id: String,
        content: Vec<Inline>,
    },
    Paragraph(Vec<Inline>),
    List {
        ordered: bool,
        start: Option<u32>,
        loose: bool,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Table {
        align: Vec<Align>,
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Code {
        lang: Option<String>,
        source: String,
    },
    Mermaid {
        source: String,
    },
    Rule,
    Raw(String),
    Component(Component),
    Degraded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Delete(Vec<Inline>),
    Code(String),
    Link {
        href: String,
        doc: Option<String>,
        content: Vec<Inline>,
    },
    Xref {
        id: String,
        family: String,
    },
    Image {
        alt: String,
        src: String,
    },
    Break,
    Raw(String),
    Component(Box<Component>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Component {
    CodeTabs {
        tabs: Vec<CodeTab>,
        blocks: Vec<Block>,
    },
    Comparison {
        options: Vec<ComparisonOption>,
        blocks: Vec<Block>,
    },
    DecisionMatrix {
        score: String,
        blocks: Vec<Block>,
    },
    AnnotatedDiff {
        blocks: Vec<Block>,
        notes: Vec<MarginNote>,
    },
    Flowchart {
        source: String,
        details: Vec<NodeDetail>,
    },
    StepRail {
        steps: Vec<Step>,
        blocks: Vec<Block>,
    },
    Unknown {
        name: String,
        blocks: Vec<Block>,
    },
}

/// Component children may be inline prose or markdown blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentBody {
    pub blocks: Vec<Block>,
    pub inline: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeTab {
    pub label: String,
    pub body: ComponentBody,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComparisonOption {
    pub name: String,
    pub body: ComponentBody,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarginNote {
    pub line: String,
    pub body: ComponentBody,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeDetail {
    pub name: String,
    pub body: ComponentBody,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub title: String,
    pub body: ComponentBody,
}
