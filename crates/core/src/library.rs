//! The renderer-facing project, corpus, document cache, and watch facade.
use std::{io, path::Path};

use crate::document::Documents;
use crate::{Corpus, DocError, Opened, ParseConfig, Project, Watch, WatchResult, docs};
use plan_viewer_source::Viewer;

#[derive(Debug)]
pub struct Library {
    viewer: Viewer,
    config: ParseConfig,
    documents: Documents,
}

impl Library {
    pub fn new(root: &Path, config: ParseConfig) -> io::Result<Self> {
        Ok(Self {
            viewer: Viewer::new(root)?,
            documents: Documents::new(config.clone()),
            config,
        })
    }

    pub fn root(&self) -> &Path {
        &self.viewer.root
    }

    pub fn project(&self) -> Project {
        self.viewer.current()
    }

    pub fn corpus(&self, worktree: &Path) -> Corpus {
        docs(worktree, &self.project().name, &self.config)
    }

    pub fn open(&self, worktree: &Path, rel: &str) -> Result<Opened, DocError> {
        self.documents.open(worktree, rel)
    }

    pub fn watch(&self, worktree: &Path) -> WatchResult<Watch> {
        plan_viewer_source::watch(worktree)
    }
}
