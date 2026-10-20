use std::collections::HashSet;
use std::path::Path;

use spec_viewer_source::{Entry, changed, list};

use crate::js::label;
use crate::{DocMeta, Docs, ParseConfig, TabConfig, TabDocs, build_tree, parse_meta};

/// A companion's kind, else `plan` for a subfolder's `index.md` and `phase` for the files it owns.
fn derived_kind(path: &str, tab: &TabConfig, listed: &HashSet<&str>) -> Option<String> {
    if let Some(kind) = tab.companion(path) {
        return Some(kind.to_owned());
    }
    let relative = path.strip_prefix(tab.root())?.trim_start_matches('/');
    let (directory, name) = path.rsplit_once('/').filter(|_| relative.contains('/'))?;
    if matches!(name, "index.md" | "index.mdx") {
        Some("plan".into())
    } else {
        ["index.md", "index.mdx"]
            .iter()
            .any(|index| listed.contains(format!("{directory}/{index}").as_str()))
            .then(|| "phase".into())
    }
}

fn to_doc_meta(entry: Entry, kind: Option<String>, config: &ParseConfig) -> DocMeta {
    let Entry { real, path } = entry;
    let meta = std::fs::read(&real)
        .ok()
        .map(|source| parse_meta(&String::from_utf8_lossy(&source), &real, config));
    DocMeta {
        archived: meta.as_ref().and_then(|meta| meta.archived),
        kind: kind.or_else(|| meta.as_ref().and_then(|meta| meta.kind.clone())),
        parent_spec: meta.as_ref().and_then(|meta| meta.parent_spec.clone()),
        status: meta.as_ref().and_then(|meta| meta.status.clone()),
        title: meta.as_ref().map_or_else(|| label(Path::new(&path)), |meta| meta.title.clone()),
        path,
    }
}

pub fn docs(worktree: &Path, project_name: &str, config: &ParseConfig) -> Docs {
    Docs {
        changed: changed(worktree, config),
        project: project_name.to_owned(),
        tabs: config
            .tabs
            .iter()
            .zip(list(worktree, config))
            .map(|(tab, entries)| {
                let listed: HashSet<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
                let kinds: Vec<_> = entries.iter().map(|entry| derived_kind(&entry.path, tab, &listed)).collect();
                let docs: Vec<_> = entries
                    .into_iter()
                    .zip(kinds)
                    .map(|(entry, kind)| to_doc_meta(entry, kind, config))
                    .collect();
                TabDocs {
                    name: tab.name.clone(),
                    nodes: build_tree(&docs, tab),
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spec_viewer_source::git_paths;
    use std::fs;

    fn git(root: &Path, args: &[&str]) -> std::io::Result<()> {
        let output = std::process::Command::new("git").arg("-C").arg(root).args(args).output()?;
        if !output.status.success() {
            return Err(std::io::Error::other("git command failed"));
        }
        Ok(())
    }
    #[test]
    fn corpus_scan_tab_trees_unignored_specs_and_wire_null() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-qb", "main"]).unwrap();
        git(root.path(), &["config", "core.excludesFile", "/dev/null"]).unwrap();
        fs::create_dir_all(root.path().join(".plan/folder")).unwrap();
        for (rel, source) in [
            (".plan/folder/index.md", "---\nstatus: review\n---\n# Folder"),
            (".plan/folder/1.md", "# Z phase"),
            (".plan/folder/2.md", "# A phase"),
            (".plan/folder/index.examples.md", "# Examples"),
            ("tracked.spec.mdx", "---\nkind: unknown\n---\n# Spec"),
            ("untracked.spec.md", "# Untracked"),
            ("ignored.spec.md", "# Ignored"),
            (".gitignore", "ignored.spec.md\n"),
        ] {
            fs::write(root.path().join(rel), source).unwrap();
        }
        git(root.path(), &["add", "tracked.spec.mdx"]).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join(".plan/folder/index.md"), root.path().join(".plan/link.md")).unwrap();
            std::os::unix::fs::symlink(root.path().join(".plan/folder"), root.path().join(".plan/link-dir")).unwrap();
        }
        let result = docs(root.path(), "Project", &ParseConfig::default());
        assert_eq!(result.project, "Project");
        assert_eq!(result.tabs.iter().map(|tab| &*tab.name).collect::<Vec<_>>(), ["plans", "specs"]);
        assert_eq!(result.tabs[0].nodes.len(), 1);
        let plan = &result.tabs[0].nodes[0];
        assert_eq!(
            (
                &*plan.name,
                &*plan.kind,
                plan.path.as_deref(),
                plan.status.as_ref().map(|status| status.name.as_str())
            ),
            ("folder", "plan", Some(".plan/folder/index.md"), Some("ready"))
        );
        assert_eq!(
            plan.children
                .as_ref()
                .unwrap()
                .iter()
                .map(|node| (&*node.name, &*node.kind))
                .collect::<Vec<_>>(),
            [("Z phase", "phase"), ("A phase", "phase"), ("Examples", "examples")]
        );
        let specs = &result.tabs[1].nodes;
        assert_eq!(specs.len(), 2);
        assert_eq!((&*specs[0].name, &*specs[0].kind), ("Spec", "unknown"));
        assert_eq!(result.changed, None);
        assert_eq!(
            git_paths(root.path(), &ParseConfig::default()),
            ["tracked.spec.mdx", "untracked.spec.md"]
        );
    }
}
