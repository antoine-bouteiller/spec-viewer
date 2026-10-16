//! Project model, corpus scan, metadata, tree, and confinement shared by both shells.

mod corpus;
mod js;
mod metadata;
mod project;
mod sort;
mod tree;
mod types;

pub use corpus::{docs, resolve_doc, spec_paths};
pub use metadata::{body_below_header, meta_of};
pub use project::{Viewer, changed_specs, project_of};
pub use tree::build_tree;
pub use types::*;
