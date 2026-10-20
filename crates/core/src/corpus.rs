use std::path::Path;

use spec_viewer_source::{Entry, changed_specs, list};

use crate::js::label;
use crate::{DocMeta, Docs, FolderDocs, ParseConfig, build_tree, parse_meta};

fn to_doc_meta(entry: Entry, config: &ParseConfig) -> DocMeta {
    let Entry { real, path, kind } = entry;
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
    let folders = config.folders();
    let changed_specs = changed_specs(worktree, &folders);
    let listing = list(worktree, &folders);
    Docs {
        changed_specs,
        folders: config
            .tabs
            .iter()
            .zip(listing.folders)
            .map(|(tab, entries)| FolderDocs {
                name: tab.name.clone(),
                folder: tab.folder.clone(),
                nodes: build_tree(
                    &entries.into_iter().map(|entry| to_doc_meta(entry, config)).collect::<Vec<_>>(),
                    true,
                ),
            })
            .collect(),
        project: project_name.to_owned(),
        specs: build_tree(
            &listing
                .specs
                .into_iter()
                .map(|entry| to_doc_meta(entry, config))
                .collect::<Vec<_>>(),
            false,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spec_viewer_source::spec_paths;
    use std::fs;

    fn git(root: &Path, args: &[&str]) -> std::io::Result<()> {
        let output = std::process::Command::new("git").arg("-C").arg(root).args(args).output()?;
        if !output.status.success() {
            return Err(std::io::Error::other("git command failed"));
        }
        Ok(())
    }
    #[test]
    fn corpus_scan_folder_plans_unignored_specs_and_wire_null() {
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
        assert_eq!(result.folders.len(), 1);
        assert_eq!(result.folders[0].nodes.len(), 1);
        let plan = &result.folders[0].nodes[0];
        assert_eq!(
            (&*plan.name, &*plan.kind, plan.status.as_ref().map(|status| status.name.as_str())),
            ("Folder", "plan", Some("ready"))
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
        assert_eq!(result.specs.len(), 2);
        assert_eq!((&*result.specs[0].name, &*result.specs[0].kind), ("Spec", "unknown"));
        assert_eq!(result.changed_specs, None);
        assert_eq!(
            spec_paths(root.path(), &[".plan".into()]),
            ["tracked.spec.mdx", "untracked.spec.md"]
        );
    }
}
