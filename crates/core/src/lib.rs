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
pub use document::{Body, Document, Document as Opened};
pub use library::Library;
pub use plan_viewer_parser::tree::{Element, Node, plain_text};
pub use plan_viewer_parser::{Counts, Degraded, Meta, Parsed, TocEntry, body_below_header, meta_of, parse};
pub use plan_viewer_source::{DocError, Project, ResolvedDoc, Stamp, Watch, WatchResult, Worktree, watch};
pub use tree::build_tree;
pub use types::Docs as Corpus;
pub use types::*;
