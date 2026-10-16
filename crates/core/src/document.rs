//! Confined, mtime-cached document reads for the terminal shell.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::cache::Cache;
use crate::{DocError, Meta, Parsed, body_below_header, meta_of, parse};
use plan_viewer_source::{read, resolve};

/// One parse per document version.
#[derive(Debug)]
pub struct Body {
    pub parsed: Parsed,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub path: PathBuf,
    pub root_path: String,
    pub meta: Meta,
    pub body: Arc<Body>,
}

#[derive(Debug, Default)]
pub struct Documents {
    cache: Mutex<Cache<Arc<Body>>>,
}

impl Documents {
    pub fn open(&self, worktree: &Path, rel: &str) -> Result<Document, DocError> {
        let resolved = resolve(worktree, rel)?;
        let (source, stamp) = read(&resolved)?;
        let real_root = std::fs::canonicalize(worktree).map_err(|_| DocError::NotFound)?;
        let render_path = resolved
            .real
            .strip_prefix(&real_root)
            .unwrap_or(&resolved.real)
            .to_string_lossy()
            .into_owned();
        let body = self.cache.lock().expect("document cache").cached(&resolved.real, stamp, || {
            Arc::new(Body {
                parsed: parse(&body_below_header(&source), &render_path),
            })
        });
        Ok(Document {
            meta: meta_of(&source, &resolved.file),
            path: resolved.file,
            root_path: resolved.root_path,
            body,
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
        assert_eq!(document.meta.title, "Alpha");
        assert_eq!(document.root_path, ".plan/a.md");
        let parsed = &document.body.parsed;
        assert!(matches!(parsed.children.first(), Some(crate::Node::Element(element))
            if element.tag == "h2" && element.attr("id") == Some("why")));
        assert!(matches!(parsed.children.last(), Some(crate::Node::Element(element))
            if element.children.iter().any(|node| matches!(node, crate::Node::Element(chip)
                if chip.attr("data-ref") == Some("VC-1")))));
    }

    #[test]
    fn open_reuses_the_parse_until_the_file_changes() {
        let directory = worktree();
        let documents = Documents::default();
        let first = documents.open(directory.path(), ".plan/a.md").unwrap();
        let second = documents.open(directory.path(), ".plan/a.md").unwrap();
        assert!(Arc::ptr_eq(&first.body, &second.body));
        std::fs::write(directory.path().join(".plan/a.md"), "# Alpha\n\n## Changed section\n").unwrap();
        let third = documents.open(directory.path(), ".plan/a.md").unwrap();
        assert_eq!(third.body.parsed.toc[0].id, "changed-section");
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
