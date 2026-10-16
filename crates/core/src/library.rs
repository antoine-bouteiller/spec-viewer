//! The renderer-facing project, corpus, document cache, and watch facade.
use std::{io, path::Path};

use crate::document::Documents;
use crate::{Corpus, DocError, Opened, Project, Watch, WatchResult, docs};
use plan_viewer_source::Viewer;

#[derive(Debug)]
pub struct Library {
    viewer: Viewer,
    documents: Documents,
}

impl Library {
    pub fn new(root: &Path) -> io::Result<Self> {
        Ok(Self {
            viewer: Viewer::new(root)?,
            documents: Documents::default(),
        })
    }

    pub fn project(&self) -> Project {
        self.viewer.current()
    }

    pub fn corpus(&self, worktree: &Path) -> Corpus {
        docs(worktree, &self.project().name)
    }

    pub fn open(&self, worktree: &Path, rel: &str) -> Result<Opened, DocError> {
        self.documents.open(worktree, rel)
    }

    pub fn watch(&self, worktree: &Path) -> WatchResult<Watch> {
        plan_viewer_source::watch(worktree)
    }
}
