//! The renderer-facing project, corpus, document cache, and watch facade.
use std::{io, path::Path};

use crate::document::Documents;
use crate::{Corpus, DocError, Opened, ParseConfig, Project, Watch, WatchResult, Worktree, docs};
use spec_viewer_source::Viewer;

#[derive(Debug)]
pub struct Library {
    viewer: Viewer,
    config: ParseConfig,
    documents: Documents,
    initial_worktree: usize,
}

impl Library {
    pub fn new(root: &Path, config: ParseConfig) -> io::Result<Self> {
        let viewer = Viewer::new(root)?;
        let initial_worktree = viewer
            .project
            .worktrees
            .iter()
            .enumerate()
            .filter(|(_, worktree)| viewer.root.starts_with(&worktree.path))
            .max_by_key(|(_, worktree)| worktree.path.components().count())
            .map_or(0, |(index, _)| index);
        Ok(Self {
            viewer,
            documents: Documents::new(config.clone()),
            config,
            initial_worktree,
        })
    }

    /// Configure parsing after loading the startup worktree's config, without rediscovery.
    pub fn with_parse_config(mut self, config: ParseConfig) -> Self {
        self.documents = Documents::new(config.clone());
        self.config = config;
        self
    }

    pub fn containing_worktree(&self) -> &Worktree {
        &self.viewer.project.worktrees[self.initial_worktree]
    }

    pub fn project(&self) -> Project {
        self.viewer.project.clone()
    }

    pub fn refresh_project(&self) -> Project {
        self.viewer.current()
    }

    pub fn corpus(&self, worktree: &Path) -> Corpus {
        docs(worktree, &self.viewer.project.name, &self.config)
    }

    pub fn open(&self, worktree: &Path, rel: &str) -> Result<Opened, DocError> {
        self.documents.open(worktree, rel)
    }

    pub fn watch(&self, worktree: &Path) -> WatchResult<Watch> {
        spec_viewer_source::watch(worktree, &self.config)
    }
}
