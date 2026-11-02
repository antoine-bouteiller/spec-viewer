//! The project paths a daemon has resolved, kept across restarts in `projects.toml`.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use toml::{Table, Value};

pub(crate) struct Known {
    file: Option<PathBuf>,
    paths: Mutex<Vec<PathBuf>>,
}

/// `$XDG_STATE_HOME/spec-viewer/projects.toml`, else `~/.local/state/spec-viewer/projects.toml`.
pub fn known_file() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|dir| dir.join("spec-viewer/projects.toml"))
}

impl Known {
    /// A missing or malformed file starts empty; `None` records nothing.
    pub(crate) fn load(file: Option<PathBuf>) -> Self {
        let paths = file
            .as_deref()
            .and_then(|file| fs::read_to_string(file).ok())
            .and_then(|source| source.parse::<Table>().ok())
            .and_then(|table| table.get("projects").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect();
        Self {
            file,
            paths: Mutex::new(paths),
        }
    }

    pub(crate) fn paths(&self) -> Vec<PathBuf> {
        self.paths.lock().expect("known projects").clone()
    }

    /// An unwritable state directory only costs the list across restarts.
    pub(crate) fn record(&self, path: &Path) {
        let mut paths = self.paths.lock().expect("known projects");
        if paths.iter().any(|known| known == path) {
            return;
        }
        paths.push(path.to_path_buf());
        if let Some(file) = &self.file {
            let projects = paths
                .iter()
                .map(|path| Value::String(path.to_string_lossy().into_owned()))
                .collect();
            let table = Table::from_iter([("projects".to_owned(), Value::Array(projects))]);
            if let Some(directory) = file.parent() {
                let _ = fs::create_dir_all(directory);
            }
            if let Err(error) = fs::write(file, table.to_string()) {
                eprintln!("spec-viewer: cannot record known projects in {}: {error}", file.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_paths_survive_a_reload_once_each() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("state/spec-viewer/projects.toml");
        let known = Known::load(Some(file.clone()));
        known.record(Path::new("/a"));
        known.record(Path::new("/b"));
        known.record(Path::new("/a"));
        assert_eq!(Known::load(Some(file.clone())).paths(), [PathBuf::from("/a"), PathBuf::from("/b")]);
        fs::write(&file, "projects = [").unwrap();
        assert!(Known::load(Some(file)).paths().is_empty());
    }
}
