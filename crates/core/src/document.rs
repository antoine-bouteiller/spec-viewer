//! A document as the daemon sends it.
use std::path::PathBuf;
use std::sync::Arc;

use crate::Document;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opened {
    pub path: PathBuf,
    pub root_path: String,
    pub document: Arc<Document>,
}
