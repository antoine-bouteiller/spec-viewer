//! One effective config's corpus listing and document cache; only the daemon holds one.
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::cache::Cache;
use crate::{Corpus, DocError, Document, Opened, ParseConfig, docs, parse};
use spec_viewer_source::{read, resolve};

#[derive(Debug)]
pub struct Library {
    config: ParseConfig,
    cache: Mutex<Cache<Arc<Document>>>,
}

impl Library {
    pub fn new(config: ParseConfig) -> Self {
        Self {
            config,
            cache: Mutex::default(),
        }
    }

    pub fn config(&self) -> &ParseConfig {
        &self.config
    }

    /// The tab trees of `worktree`, named after its project.
    pub fn corpus(&self, worktree: &Path, project: &str) -> Corpus {
        docs(worktree, project, &self.config)
    }

    /// A confined document, parsed once per file stamp.
    pub fn open(&self, worktree: &Path, rel: &str) -> Result<Opened, DocError> {
        let resolved = resolve(worktree, rel, &self.config)?;
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

    fn library() -> Library {
        Library::new(ParseConfig::default())
    }

    #[test]
    fn open_reads_meta_and_renders_body_below_header() {
        let directory = worktree();
        let document = library().open(directory.path(), ".plan/a.md").unwrap();
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
        let library = library();
        let first = library.open(directory.path(), ".plan/a.md").unwrap();
        let second = library.open(directory.path(), ".plan/a.md").unwrap();
        assert!(Arc::ptr_eq(&first.document, &second.document));
        std::fs::write(directory.path().join(".plan/a.md"), "# Alpha\n\n## Changed section\n").unwrap();
        let third = library.open(directory.path(), ".plan/a.md").unwrap();
        assert_eq!(third.document.toc[0].id, "changed-section");
        assert!(!Arc::ptr_eq(&first.document, &third.document));
    }

    #[test]
    fn open_propagates_confinement_errors() {
        let directory = worktree();
        assert_eq!(library().open(directory.path(), "../x.md").unwrap_err(), DocError::Forbidden);
        assert_eq!(
            library().open(directory.path(), ".plan/missing.md").unwrap_err(),
            DocError::NotFound
        );
    }
}
