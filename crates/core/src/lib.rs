//! Corpus trees, cached documents, and fuzzy search.

pub mod cache;
mod corpus;
mod document;
mod js;
mod library;
mod search;
mod sort;
mod tree;
mod types;

use corpus::docs;
pub use document::Opened;
pub use library::Library;
pub use search::{Hit, search};
pub use spec_viewer_config::{ParseConfig, TabConfig};
pub use spec_viewer_parser::{
    Align, Block, Chip, ChipValue, CodeTab, ComparisonOption, Component, ComponentBody, Degraded, Document, Inline, ListItem, MarginNote,
    Meta, NodeDetail, Status, Step, TocEntry, body_below_header, parse, parse_meta,
};
pub use spec_viewer_source::{DocError, Project, ResolvedDoc, Stamp, Worktree};
pub use tree::build_tree;
pub use types::Docs as Corpus;
pub use types::*;
