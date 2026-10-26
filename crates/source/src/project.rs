use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::js::{starts, take_while, terminator, trim};
use super::{Project, Worktree};
use spec_viewer_config::ParseConfig;

// Pathspec `*` also matches `/`.
pub(super) const MARKDOWN: [&str; 2] = ["*.md", "*.mdx"];

pub(super) fn git(root: &Path, args: &[&str]) -> io::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other("git command failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn basename(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn porcelain_field<'a>(block: &'a str, prefix: &str) -> Option<&'a str> {
    starts(block).into_iter().find_map(|start| {
        let value = block[start..].strip_prefix(prefix)?;
        let length = take_while(value, |character| !terminator(character));
        (length > 0).then_some(&value[..length])
    })
}

pub fn project_of(root: &Path) -> io::Result<Project> {
    let root = std::fs::canonicalize(root)?;
    if !root.is_dir() {
        return Err(io::Error::other("not a directory"));
    }
    let Ok(common) = git(&root, &["rev-parse", "--git-common-dir"]) else {
        return Ok(Project {
            key: root.clone(),
            name: basename(&root),
            path: root.clone(),
            worktrees: vec![Worktree {
                name: basename(&root),
                main: true,
                path: root,
            }],
        });
    };
    let key = std::fs::canonicalize(root.join(trim(&common)))?;
    let output = git(&root, &["worktree", "list", "--porcelain", "-z"])?;
    let worktrees: Vec<_> = trim(&output)
        .split("\0\0")
        .enumerate()
        .filter_map(|(index, block)| {
            let block = block.replace('\0', "\n");
            if starts(&block).into_iter().any(|start| block[start..].starts_with("prunable")) {
                return None;
            }
            let path = std::fs::canonicalize(porcelain_field(&block, "worktree ")?).ok()?;
            let branch = porcelain_field(&block, "branch refs/heads/");
            Some(Worktree {
                main: index == 0,
                name: branch.map_or_else(|| basename(&path), str::to_owned),
                path,
            })
        })
        .collect();
    let main = worktrees
        .iter()
        .find(|wt| wt.main)
        .or_else(|| worktrees.first())
        .ok_or_else(|| io::Error::other("project has no worktrees"))?;
    Ok(Project {
        key,
        name: basename(&main.path),
        path: main.path.clone(),
        worktrees,
    })
}

/// Tab documents changed since the merge base with the remote `main`, committed, uncommitted, or untracked.
pub fn changed(worktree: &Path, config: &ParseConfig) -> Option<Vec<String>> {
    let refs = git(worktree, &["for-each-ref", "--format=%(refname)", "refs/remotes"]).ok()?;
    let refs: Vec<_> = refs.split('\n').filter(|reference| reference.ends_with("/main")).collect();
    let remote = refs
        .iter()
        .find(|&&reference| reference == "refs/remotes/origin/main")
        .or_else(|| refs.first())?;
    let base = git(worktree, &["merge-base", "HEAD", remote]).ok()?;
    let mut args = vec!["diff", "--name-only", "-z", trim(&base), "--"];
    args.extend(MARKDOWN);
    let mut output = git(worktree, &args).ok()?;
    let mut args = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    args.extend(MARKDOWN);
    output += &git(worktree, &args).ok()?;
    Some(
        output
            .split('\0')
            .filter(|path| config.tab_of(path).is_some())
            .map(str::to_owned)
            .collect(),
    )
}

/// A fixed project identity whose currently accessible worktrees are refreshed on demand.
#[derive(Clone, Debug)]
pub struct Viewer {
    pub root: PathBuf,
    pub project: Project,
}

impl Viewer {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = std::fs::canonicalize(root)?;
        let project = project_of(&root)?;
        Ok(Self { root, project })
    }

    pub fn current(&self) -> Project {
        for root in std::iter::once(&self.root).chain(self.project.worktrees.iter().map(|wt| &wt.path)) {
            if let Ok(project) = project_of(root)
                && project.key == self.project.key
            {
                return project;
            }
        }
        let mut project = self.project.clone();
        project.worktrees.clear();
        project
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn run(root: &Path, args: &[&str]) {
        git(root, args).unwrap();
    }
    fn repository() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        run(root.path(), &["init", "-qb", "main"]);
        run(root.path(), &["config", "user.email", "spec-viewer@example.test"]);
        run(root.path(), &["config", "user.name", "Spec Viewer Test"]);
        run(root.path(), &["config", "core.excludesFile", "/dev/null"]);
        root
    }
    #[test]
    fn non_git_project() {
        let root = tempfile::tempdir().unwrap();
        let project = project_of(root.path()).unwrap();
        let real = fs::canonicalize(root.path()).unwrap();
        assert_eq!(project.key, real);
        assert_eq!(project.path, real);
        assert_eq!(
            project.worktrees,
            [Worktree {
                name: real.file_name().unwrap().to_string_lossy().into_owned(),
                main: true,
                path: real
            }]
        );
    }

    #[test]
    fn repo_and_second_worktree() {
        let root = repository();
        run(root.path(), &["commit", "--allow-empty", "-qm", "base"]);
        let second = tempfile::tempdir().unwrap();
        run(root.path(), &["worktree", "add", "-qb", "feature", second.path().to_str().unwrap()]);
        let project = project_of(second.path()).unwrap();
        assert_eq!(project.path, fs::canonicalize(root.path()).unwrap());
        assert_eq!(project.key, fs::canonicalize(root.path().join(".git")).unwrap());
        assert_eq!(
            project.worktrees.iter().map(|wt| (&*wt.name, wt.main)).collect::<Vec<_>>(),
            [("main", true), ("feature", false)]
        );
        let viewer = Viewer::new(second.path()).unwrap();
        run(root.path(), &["worktree", "remove", "--force", second.path().to_str().unwrap()]);
        assert_eq!(viewer.current().worktrees.len(), 1);
    }

    #[test]
    fn a_recreated_worktree_directory_is_not_listed() {
        let root = repository();
        run(root.path(), &["commit", "--allow-empty", "-qm", "base"]);
        let second = tempfile::tempdir().unwrap();
        run(root.path(), &["worktree", "add", "-qb", "feature", second.path().to_str().unwrap()]);
        let viewer = Viewer::new(root.path()).unwrap();
        fs::remove_dir_all(second.path()).unwrap();
        fs::create_dir(second.path()).unwrap();
        assert_eq!(viewer.current().worktrees.len(), 1);
    }

    #[test]
    fn committed_working_tree_and_untracked_tab_changes_from_remote_main() {
        let root = repository();
        for (name, content) in [
            ("first.spec.md", "first\n"),
            ("second.spec.md", "second\n"),
            ("remote-only.spec.md", "remote only\n"),
            ("README.md", "readme\n"),
            (".plan/plan.examples.md", "plan\n"),
        ] {
            fs::create_dir_all(root.path().join(name).parent().unwrap()).unwrap();
            fs::write(root.path().join(name), content).unwrap();
        }
        run(root.path(), &["add", "."]);
        run(root.path(), &["commit", "-qm", "base"]);
        run(root.path(), &["switch", "-qc", "remote-update"]);
        fs::write(root.path().join("remote-only.spec.md"), "changed only on remote main\n").unwrap();
        run(root.path(), &["commit", "-qam", "advance remote main"]);
        run(root.path(), &["update-ref", "refs/remotes/upstream/main", "HEAD"]);
        run(root.path(), &["switch", "-q", "main"]);
        fs::write(root.path().join("first.spec.md"), "changed in working tree\n").unwrap();
        fs::write(root.path().join("second.spec.md"), "changed in commit\n").unwrap();
        fs::write(root.path().join("README.md"), "not a spec\n").unwrap();
        fs::write(root.path().join(".plan/plan.examples.md"), "changed plan\n").unwrap();
        run(root.path(), &["add", "second.spec.md"]);
        run(root.path(), &["commit", "-qm", "change second spec"]);
        fs::write(root.path().join("untracked.spec.md"), "new\n").unwrap();
        fs::write(root.path().join("NOTES.md"), "not a spec\n").unwrap();
        let mut changed = changed(root.path(), &ParseConfig::default()).unwrap();
        changed.sort();
        assert_eq!(
            changed,
            [".plan/plan.examples.md", "first.spec.md", "second.spec.md", "untracked.spec.md"]
        );
    }

    #[test]
    fn porcelain_paths_obey_js_dot_line_terminators() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repo\u{2028}tail");
        fs::create_dir(&root).unwrap();
        run(&root, &["init", "-qb", "main"]);
        // The field ends at the Unicode line separator, leaving `repo`, which does not exist.
        assert_eq!(project_of(&root).unwrap_err().to_string(), "project has no worktrees");
    }

    #[test]
    fn no_remote_main_ref() {
        assert_eq!(changed(repository().path(), &ParseConfig::default()), None);
    }
}
