//! Corpus trees, cached documents, and the renderer-facing library facade.

pub mod cache;
mod corpus;
pub mod document;
mod js;
mod library;
mod sort;
mod tree;
mod types;

pub use corpus::docs;
pub use document::Opened;
pub use library::Library;
pub use spec_viewer_config::ParseConfig;
pub use spec_viewer_parser::{
    Align, Block, Chip, ChipValue, CodeTab, ComparisonOption, Component, ComponentBody, Degraded, Document, Inline, ListItem, MarginNote,
    Meta, NodeDetail, Status, Step, TocEntry, body_below_header, parse, parse_meta,
};
pub use spec_viewer_source::{DocError, Project, ResolvedDoc, Stamp, Watch, WatchResult, Worktree, project_of, watch};
pub use tree::build_tree;
pub use types::Docs as Corpus;
pub use types::*;
