//! File discovery, project identity, confined reads, and change notifications.

mod corpus;
mod js;
mod project;
mod stamp;
mod types;
mod watch;

pub use corpus::{Entry, Listing, list, resolve, spec_paths};
pub use project::{Viewer, changed_specs, project_of};
pub use stamp::Stamp;
pub use types::{DocError, Project, ResolvedDoc, Worktree};
pub use watch::{Watch, watch};

pub type WatchResult<T> = notify::Result<T>;

pub fn read(resolved: &ResolvedDoc) -> Result<(String, Stamp), DocError> {
    let stamp = std::fs::metadata(&resolved.real)
        .map(|metadata| Stamp::from(&metadata))
        .map_err(|_| DocError::NotFound)?;
    let bytes = std::fs::read(&resolved.real).map_err(|_| DocError::NotFound)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), stamp))
}
