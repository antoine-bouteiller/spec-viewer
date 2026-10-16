use std::path::{Path, PathBuf};

use super::js::{label, locale_compare, spec_path};
use super::project::{SPEC_PATHS, git};
use super::{DocError, DocMeta, Docs, ResolvedDoc, build_tree, changed_specs, meta_of};

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

fn is_allowed(file: &Path, root: &Path) -> bool {
    if !contained(file, root) {
        return false;
    }
    let rel = file.strip_prefix(root).expect("contained path").to_string_lossy();
    (rel.starts_with(".plan/") && rel.ends_with(".md")) || spec_path(&rel)
}

fn real_file_in_root(file: &Path, root: &Path) -> Option<PathBuf> {
    let file = std::fs::canonicalize(file).ok()?;
    let root = std::fs::canonicalize(root).ok()?;
    contained(&file, &root).then_some(file)
}

pub fn resolve_doc(worktree: &Path, rel: &str) -> Result<ResolvedDoc, DocError> {
    if rel.contains('\0') || Path::new(rel).is_absolute() {
        return Err(DocError::Forbidden);
    }
    let root = lexical(worktree);
    let file = lexical(&root.join(rel));
    if !is_allowed(&file, &root) {
        return Err(DocError::Forbidden);
    }
    let real = std::fs::canonicalize(&file).map_err(|_| DocError::NotFound)?;
    let real_root = std::fs::canonicalize(&root).map_err(|_| DocError::NotFound)?;
    if !contained(&real, &real_root) {
        return Err(DocError::Forbidden);
    }
    let root_path = file.strip_prefix(&root).expect("allowed path").to_string_lossy().into_owned();
    Ok(ResolvedDoc { file, real, root_path })
}

fn to_doc_meta(full: &Path, path: String, kind: Option<&str>) -> DocMeta {
    let meta = std::fs::read(full)
        .ok()
        .map(|source| meta_of(&String::from_utf8_lossy(&source), full));
    DocMeta {
        acceptance: meta.as_ref().map(|meta| meta.acceptance),
        archived: meta.as_ref().and_then(|meta| meta.archived),
        kind: kind.map(str::to_owned).or_else(|| meta.as_ref().and_then(|meta| meta.kind.clone())),
        parent_spec: meta.as_ref().and_then(|meta| meta.parent_spec.clone()),
        status: meta.as_ref().and_then(|meta| meta.status.clone()),
        tasks: meta.as_ref().map(|meta| meta.tasks),
        title: meta.as_ref().map_or_else(|| label(Path::new(&path)), |meta| meta.title.clone()),
        path,
    }
}

fn list_plans(dir: &Path, root: &Path, kind: &str) -> Vec<DocMeta> {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|entry| entry.file_name()).collect();
    entries.sort_by(|left, right| locale_compare(&left.to_string_lossy(), &right.to_string_lossy()));
    let mut plans = vec![];
    for entry in entries {
        let full = dir.join(&entry);
        let Ok(stats) = std::fs::symlink_metadata(&full) else { continue };
        if stats.is_symlink() {
            continue;
        }
        if stats.is_dir() {
            plans.extend(list_plans(&full, root, "phase"));
        } else if entry.to_string_lossy().ends_with(".md")
            && let Some(real) = real_file_in_root(&full, root)
        {
            let entry = entry.to_string_lossy();
            let plan_kind = if kind == "phase" && entry == "index.md" { "plan" } else { kind };
            let kind = if entry.ends_with(".examples.md") { "examples" } else { plan_kind };
            let path = full.strip_prefix(root).expect("plan in root").to_string_lossy().into_owned();
            plans.push(to_doc_meta(&real, path, Some(kind)));
        }
    }
    plans
}

pub fn spec_paths(worktree: &Path) -> Vec<String> {
    let mut args = vec!["ls-files", "--"];
    args.extend(SPEC_PATHS);
    let Ok(output) = git(worktree, &args) else { return vec![] };
    let mut paths: Vec<_> = output.split('\n').filter(|path| !path.is_empty()).map(str::to_owned).collect();
    paths.sort_by(|left, right| locale_compare(left, right));
    paths
}

fn list_specs(root: &Path) -> Vec<DocMeta> {
    spec_paths(root)
        .into_iter()
        .filter_map(|path| real_file_in_root(&root.join(&path), root).map(|full| to_doc_meta(&full, path, None)))
        .collect()
}

pub fn docs(worktree: &Path, project_name: &str) -> Docs {
    Docs {
        changed_specs: changed_specs(worktree),
        plans: build_tree(&list_plans(&worktree.join(".plan"), worktree, "plan"), true),
        project: project_name.to_owned(),
        specs: build_tree(&list_specs(worktree), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn resolves_allowed_plan_and_spec() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".plan")).unwrap();
        for rel in [".plan/a.md", "a.spec.md", "a.discovery.mdx", "nested.examples.mdx"] {
            fs::write(root.path().join(rel), "# Test").unwrap();
            let resolved = resolve_doc(root.path(), rel).unwrap();
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
            assert_eq!(resolve_doc(root.path(), rel), Err(DocError::Forbidden), "{rel:?}");
        }
        assert_eq!(resolve_doc(root.path(), "missing.spec.md"), Err(DocError::NotFound));
        fs::write(root.path().join("a.spec.md"), "# A").unwrap();
        assert_eq!(
            resolve_doc(root.path(), "does-not-exist/../a.spec.md").unwrap().root_path,
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
        assert_eq!(resolve_doc(root.path(), "escape.spec.md"), Err(DocError::Forbidden));
        fs::write(root.path().join("real.md"), "body").unwrap();
        symlink(root.path().join("real.md"), root.path().join("alias.spec.md")).unwrap();
        let resolved = resolve_doc(root.path(), "alias.spec.md").unwrap();
        assert_eq!(resolved.file, root.path().join("alias.spec.md"));
        assert_eq!(resolved.real, fs::canonicalize(root.path().join("real.md")).unwrap());
        assert_eq!(resolved.root_path, "alias.spec.md");
    }
    #[test]
    fn corpus_scan_folder_plans_tracked_specs_and_wire_null() {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-qb", "main"]).unwrap();
        fs::create_dir_all(root.path().join(".plan/folder")).unwrap();
        for (rel, source) in [
            (".plan/folder/index.md", "---\nstatus: review\n---\n# Folder"),
            (".plan/folder/1.md", "# Z phase"),
            (".plan/folder/2.md", "# A phase"),
            (".plan/folder/index.examples.md", "# Examples"),
            ("tracked.spec.mdx", "---\nkind: unknown\n---\n# Spec"),
            ("untracked.spec.md", "# Invisible"),
        ] {
            fs::write(root.path().join(rel), source).unwrap();
        }
        git(root.path(), &["add", "tracked.spec.mdx"]).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.path().join(".plan/folder/index.md"), root.path().join(".plan/link.md")).unwrap();
            std::os::unix::fs::symlink(root.path().join(".plan/folder"), root.path().join(".plan/link-dir")).unwrap();
        }
        let result = docs(root.path(), "Project");
        assert_eq!(result.project, "Project");
        assert_eq!(result.plans.len(), 1);
        let plan = &result.plans[0];
        assert_eq!(
            (&*plan.name, &*plan.kind, plan.status.as_deref()),
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
        assert_eq!(result.specs.len(), 1);
        assert_eq!((&*result.specs[0].name, &*result.specs[0].kind), ("Spec", "unknown"));
        assert_eq!(serde_json::to_value(result).unwrap()["changedSpecs"], serde_json::Value::Null);
        assert_eq!(spec_paths(root.path()), ["tracked.spec.mdx"]);
    }
}
