//! Confined, mtime-cached document reads for the terminal shell.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::cache::Cache;
use crate::{DocError, Document, ParseConfig, parse};
use plan_viewer_source::{read, resolve};

#[derive(Debug, Clone)]
pub struct Opened {
    pub path: PathBuf,
    pub root_path: String,
    pub document: Arc<Document>,
}

#[derive(Debug, Default)]
pub struct Documents {
    cache: Mutex<Cache<Arc<Document>>>,
    config: ParseConfig,
}

impl Documents {
    pub fn new(config: ParseConfig) -> Self {
        Self {
            cache: Mutex::default(),
            config,
        }
    }

    pub fn open(&self, worktree: &Path, rel: &str) -> Result<Opened, DocError> {
        let resolved = resolve(worktree, rel)?;
        let (source, stamp) = read(&resolved)?;
        let real_root = std::fs::canonicalize(worktree).map_err(|_| DocError::NotFound)?;
        let render_path = resolved
            .real
            .strip_prefix(&real_root)
            .unwrap_or(&resolved.real)
            .to_string_lossy()
            .into_owned();
        let document = self
            .cache
            .lock()
            .expect("document cache")
            .cached(&resolved.real, stamp, || Arc::new(parse(&source, &render_path, &self.config)));
        Ok(Opened {
            path: resolved.file,
            root_path: resolved.root_path,
            document,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join(".plan")).unwrap();
        std::fs::write(
            directory.path().join(".plan/a.md"),
            "---\ntitle: Alpha\n---\n# Alpha\n\n## Why\n\nBody VC-1\n",
        )
        .unwrap();
        directory
    }

    #[test]
    fn open_reads_meta_and_renders_body_below_header() {
        let directory = worktree();
        let documents = Documents::default();
        let document = documents.open(directory.path(), ".plan/a.md").unwrap();
        assert_eq!(document.document.meta.title, "Alpha");
        assert_eq!(document.root_path, ".plan/a.md");
        let parsed = &document.document;
        assert!(matches!(parsed.blocks.first(), Some(crate::Block::Heading { depth: 2, id, .. }) if id == "why"));
        assert!(matches!(parsed.blocks.last(), Some(crate::Block::Paragraph(content))
            if content.iter().any(|node| matches!(node, crate::Inline::Xref { id, .. } if id == "VC-1"))));
    }

    #[test]
    fn open_reuses_the_parse_until_the_file_changes() {
        let directory = worktree();
        let documents = Documents::default();
        let first = documents.open(directory.path(), ".plan/a.md").unwrap();
        let second = documents.open(directory.path(), ".plan/a.md").unwrap();
        assert!(Arc::ptr_eq(&first.document, &second.document));
        std::fs::write(directory.path().join(".plan/a.md"), "# Alpha\n\n## Changed section\n").unwrap();
        let third = documents.open(directory.path(), ".plan/a.md").unwrap();
        assert_eq!(third.document.toc[0].id, "changed-section");
        assert!(!Arc::ptr_eq(&first.document, &third.document));
    }

    #[test]
    fn open_propagates_confinement_errors() {
        let directory = worktree();
        let documents = Documents::default();
        assert_eq!(documents.open(directory.path(), "../x.md").unwrap_err(), DocError::Forbidden);
        assert_eq!(
            documents.open(directory.path(), ".plan/missing.md").unwrap_err(),
            DocError::NotFound
        );
    }
}
