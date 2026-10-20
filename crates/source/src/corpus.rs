use std::path::{Path, PathBuf};

use spec_viewer_config::ParseConfig;

use super::js::name_order;
use super::project::{MARKDOWN, git};
use super::{DocError, ResolvedDoc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub real: PathBuf,
}

/// Node's resolve(root, path): normalize lexically before following any symlinks.
fn lexical(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().expect("current directory").join(path)
    };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            component => result.push(component.as_os_str()),
        }
    }
    result
}

fn contained(file: &Path, root: &Path) -> bool {
    file.strip_prefix(root).is_ok_and(|rel| !rel.as_os_str().is_empty())
}

fn real_file_in_root(file: &Path, root: &Path) -> Option<PathBuf> {
    let file = std::fs::canonicalize(file).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    contained(&file, &root).then_some(file)
}

pub fn resolve(worktree: &Path, rel: &str, config: &ParseConfig) -> Result<ResolvedDoc, DocError> {
    if rel.contains('\0') || Path::new(rel).is_absolute() {
        return Err(DocError::Forbidden);
    }
    let root = lexical(worktree);
    let file = lexical(&root.join(rel));
    if !contained(&file, &root) {
        return Err(DocError::Forbidden);
    }
    let root_path = file.strip_prefix(&root).expect("contained path").to_string_lossy().into_owned();
    if config.tab_of(&root_path).is_none() {
        return Err(DocError::Forbidden);
    }
    let real = std::fs::canonicalize(&file).map_err(|_| DocError::NotFound)?;
    let real_root = std::fs::canonicalize(&root).map_err(|_| DocError::NotFound)?;
    if !contained(&real, &real_root) {
        return Err(DocError::Forbidden);
    }
    Ok(ResolvedDoc { file, real, root_path })
}

/// Every file below `dir`, depth-first in name order, skipping symlinks.
fn walk(dir: &Path, root: &Path, files: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|entry| entry.file_name()).collect();
    entries.sort_by(|left, right| name_order(&left.to_string_lossy(), &right.to_string_lossy()));
    for entry in entries {
        let full = dir.join(&entry);
        let Ok(stats) = std::fs::symlink_metadata(&full) else { continue };
        if stats.is_dir() {
            walk(&full, root, files);
        } else if stats.is_file() {
            files.push(full.strip_prefix(root).expect("file in root").to_string_lossy().into_owned());
        }
    }
}

/// Files of the whole-worktree tabs, from Git's cached and unignored untracked files.
pub fn git_paths(worktree: &Path, config: &ParseConfig) -> Vec<String> {
    if config.tabs.iter().all(|tab| tab.folder.is_some()) {
        return vec![];
    }
    let mut args = vec!["ls-files", "--cached", "--others", "--exclude-standard", "--"];
    args.extend(MARKDOWN);
    let Ok(output) = git(worktree, &args) else { return vec![] };
    let mut paths: Vec<_> = output
        .split('\n')
        .filter(|path| config.tab_of(path).is_some_and(|index| config.tabs[index].folder.is_none()))
        .map(str::to_owned)
        .collect();
    paths.sort_by(|left, right| name_order(left, right));
    paths.dedup();
    paths
}

/// One list per configured tab, in order.
pub fn list(worktree: &Path, config: &ParseConfig) -> Vec<Vec<Entry>> {
    let mut tabs = vec![vec![]; config.tabs.len()];
    let mut paths = git_paths(worktree, config);
    for tab in &config.tabs {
        if let Some(folder) = &tab.folder {
            walk(&worktree.join(folder), worktree, &mut paths);
        }
    }
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        if let Some(index) = config.tab_of(&path)
            && seen.insert(path.clone())
            && let Some(real) = real_file_in_root(&worktree.join(&path), worktree)
        {
            tabs[index].push(Entry { path, real });
        }
    }
    tabs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn plan() -> ParseConfig {
        ParseConfig::default()
    }
    fn tab(name: &str, folder: Option<&str>, kinds: Option<&[&str]>, companions: &[&str]) -> spec_viewer_config::TabConfig {
        let strings = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect::<Vec<_>>();
        spec_viewer_config::TabConfig {
            name: name.into(),
            folder: folder.map(Into::into),
            kinds: kinds.map(strings),
            companions: strings(companions),
        }
    }
    fn config(tabs: Vec<spec_viewer_config::TabConfig>) -> ParseConfig {
        ParseConfig {
            tabs,
            ..ParseConfig::default()
        }
    }

    #[test]
    fn the_deepest_tab_folder_owns_a_file_and_its_kinds_decide_whether_it_is_listed() {
        let root = tempfile::tempdir().unwrap();
        for rel in [
            ".plan/a.md",
            ".plan/b.spec.md",
            "docs/adr/c.md",
            "docs/adr/c.spec.md",
            "d.spec.md",
            "d.examples.mdx",
            "README.md",
        ] {
            fs::create_dir_all(root.path().join(rel).parent().unwrap()).unwrap();
            fs::write(root.path().join(rel), "# Test").unwrap();
        }
        for args in [&["init", "-q"][..], &["config", "core.excludesFile", "/dev/null"]] {
            git(root.path(), args).unwrap();
        }
        let adr = config(vec![
            tab("docs", Some("docs"), None, &[]),
            tab("adr", Some("docs/adr"), None, &[]),
            tab("specs", None, Some(&["spec"]), &["examples"]),
        ]);
        assert!(resolve(root.path(), "docs/adr/c.md", &adr).is_ok());
        assert_eq!(resolve(root.path(), "README.md", &adr), Err(DocError::Forbidden));
        let paths = |tabs: Vec<Vec<Entry>>| {
            tabs.into_iter()
                .map(|entries| entries.into_iter().map(|entry| entry.path).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paths(list(root.path(), &adr)),
            [
                vec![],
                vec!["docs/adr/c.md", "docs/adr/c.spec.md"],
                vec![".plan/b.spec.md", "d.examples.mdx", "d.spec.md"]
            ]
        );
        assert_eq!(
            paths(list(root.path(), &plan())),
            [
                vec![".plan/a.md", ".plan/b.spec.md"],
                vec!["d.examples.mdx", "d.spec.md", "docs/adr/c.spec.md"]
            ]
        );
    }
    #[test]
    fn resolves_allowed_plan_and_spec() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".plan")).unwrap();
        for rel in [".plan/a.md", "a.spec.md", "a.discovery.mdx", "nested.examples.mdx"] {
            fs::write(root.path().join(rel), "# Test").unwrap();
            let resolved = resolve(root.path(), rel, &plan()).unwrap();
            assert_eq!(resolved.file, root.path().join(rel));
            assert_eq!(resolved.real, fs::canonicalize(root.path().join(rel)).unwrap());
            assert_eq!(resolved.root_path, rel);
        }
    }

    #[test]
    fn confinement_errors_and_lexical_resolution() {
        let root = tempfile::tempdir().unwrap();
        for rel in [
            "..",
            "../outside.spec.md",
            "/tmp/absolute.spec.md",
            "bad\0.spec.md",
            "README.md",
            "a.spec.txt",
            "",
            ".plan",
        ] {
            assert_eq!(resolve(root.path(), rel, &plan()), Err(DocError::Forbidden), "{rel:?}");
        }
        assert_eq!(resolve(root.path(), "missing.spec.md", &plan()), Err(DocError::NotFound));
        fs::write(root.path().join("a.spec.md"), "# A").unwrap();
        assert_eq!(
            resolve(root.path(), "does-not-exist/../a.spec.md", &plan()).unwrap().root_path,
            "a.spec.md"
        );
    }
    #[cfg(unix)]
    #[test]
    fn escaping_symlink_is_forbidden_but_internal_keeps_lexical_name() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("out.spec.md"), "outside").unwrap();
        symlink(outside.path().join("out.spec.md"), root.path().join("escape.spec.md")).unwrap();
        assert_eq!(resolve(root.path(), "escape.spec.md", &plan()), Err(DocError::Forbidden));
        fs::write(root.path().join("real.md"), "body").unwrap();
        symlink(root.path().join("real.md"), root.path().join("alias.spec.md")).unwrap();
        let resolved = resolve(root.path(), "alias.spec.md", &plan()).unwrap();
        assert_eq!(resolved.file, root.path().join("alias.spec.md"));
        assert_eq!(resolved.real, fs::canonicalize(root.path().join("real.md")).unwrap());
        assert_eq!(resolved.root_path, "alias.spec.md");
    }
}
