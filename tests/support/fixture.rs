use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// The shared VC-1 Git corpus: `root` is the main worktree on `feature`, `sibling` is a second
/// worktree, and `origin/main` contains the committed baseline. Only `spec/standalone.spec.md`
/// differs from that baseline. Plans exercise phases, examples and legacy metadata; specs exercise
/// an umbrella family, companions, an unlinked sibling, the checked-in dialect, malformed MDX and
/// an escaping symlink. All paths are canonical, contents/dates are fixed, and the owning TempDir
/// keeps both worktrees, the bare origin and the symlink's external target alive for HTTP/TUI tests.
pub struct Fixture {
    _directory: tempfile::TempDir,
    pub root: PathBuf,
    pub sibling: PathBuf,
    pub outside: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(directory.path()).unwrap();
        let root = base.join("corpus");
        let sibling = base.join("sibling");
        let remote = base.join("origin.git");
        let outside = base.join("outside.spec.md");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-qb", "main"]);
        git(&root, &["config", "user.name", "Fixture Author"]);
        git(&root, &["config", "user.email", "fixture@example.invalid"]);
        write(
            &root,
            ".plan/rollout/index.md",
            "---\ntitle: API rollout\nstatus: in-progress\nauthor: Fixture Author\ndate: 2026-01-02\nrelated: [spec/standalone.spec.md]\n---\n# API rollout\n\n## Why\n\nDeploy safely.\n\n## Acceptance criteria\n- [x] First check\n- [ ] Second check\n\n## Implementation\n### Design\n- [x] Design task\n### Deploy\n- [ ] Deploy task\n",
        );
        write(
            &root,
            ".plan/rollout/01-design.md",
            "---\ntitle: Design phase\nstatus: ready\nauthor: Fixture Author\ndate: 2026-01-02\nrelated: [spec/standalone.spec.md]\n---\n# Design phase\n\n## Why\n\nDesign the API.\n\n## Acceptance criteria\n- [x] Review\n- [ ] Sign off\n\n## Implementation\n### Design\n- [x] Contract\n- [ ] Rollout\n",
        );
        write(
            &root,
            ".plan/rollout/02-deploy.md",
            "---\ntitle: Deploy phase\nstatus: blocked\n---\n## Deploy\n\nWait for review.\n",
        );
        write(
            &root,
            ".plan/rollout/index.examples.md",
            "---\ntitle: Rollout examples\n---\n## Example\n\nAn attached example.\n",
        );
        write(
            &root,
            ".plan/legacy.md",
            "# Legacy rollout\n\n**Status:** done\n\n## Why\n\nOld format remains readable.\n",
        );
        write(
            &root,
            ".plan/archived.md",
            "---\ntitle: Old design\nstatus: accepted\narchived: true\n---\n## History\n\nArchived body.\n",
        );
        let paragraphs = (1..=65)
            .map(|index| format!("Paragraph {index}: reading and navigating source content.\n\n"))
            .collect::<String>();
        write(
            &root,
            ".plan/actions.md",
            &format!(
                "---\ntitle: Actions\nstatus: draft\n---\n## Navigation\n\nG-1 [Source spec](../spec/standalone.spec.md).\n\n{paragraphs}"
            ),
        );
        write(
            &root,
            "doc/architecture/viewer.spec.md",
            "---\ntitle: Viewer umbrella\nstatus: accepted\nkind: umbrella\n---\n## Goals\n\nG-1 owns navigation.\n",
        );
        write(
            &root,
            "doc/architecture/tree.spec.md",
            "---\ntitle: Tree leaf\nstatus: amended\nparent-spec: doc/architecture/viewer.spec.md\n---\n## Tree\n\nFlat document rows.\n",
        );
        write(
            &root,
            "doc/architecture/viewer.examples.mdx",
            "---\ntitle: Viewer examples\n---\n## Examples\n\nAn example.\n",
        );
        write(
            &root,
            "doc/architecture/viewer.discovery.md",
            "---\ntitle: Viewer discovery\n---\n## Discovery\n\nDiscovery notes.\n",
        );
        write(
            &root,
            "doc/architecture/unlinked.spec.md",
            "---\ntitle: Unlinked sibling\nstatus: draft\n---\n## Independent\n\nNot in the umbrella family.\n",
        );
        write(
            &root,
            "spec/standalone.spec.md",
            "---\ntitle: Standalone spec\nstatus: implemented\nkind: leaf\nauthor: Fixture Author\ndate: 2026-01-02\nrelated: [doc/architecture/viewer.spec.md]\n---\n## Overview\n\nBaseline body.\n\n### Details\n\nA section outline.\n",
        );
        write(
            &root,
            "spec/broken.spec.mdx",
            "---\ntitle: Broken MDX\nstatus: draft\n---\n## Fallback\n\n<StepRail>\n\nUnclosed source remains visible.\n",
        );
        write(
            &root,
            "spec/raw.spec.md",
            "---\ntitle: Raw HTML\n---\n## Raw\n\n<div>Raw HTML source</div>\n",
        );
        write(
            &root,
            "spec/dialect.spec.mdx",
            include_str!("../../../../fixtures/dialect.spec.mdx"),
        );
        fs::write(&outside, "# Outside the worktree\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("spec/escape.spec.md")).unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-qm", "Fixture baseline"]);
        git(&root, &["init", "--bare", "-q", remote.to_str().unwrap()]);
        git(&root, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&root, &["push", "-q", "origin", "main"]);
        git(&root, &["fetch", "-q", "origin"]);
        git(&root, &["checkout", "-qb", "feature"]);
        let standalone = fs::read_to_string(root.join("spec/standalone.spec.md"))
            .unwrap()
            .replace("Baseline body.", "Changed body on the current branch.");
        write(&root, "spec/standalone.spec.md", &standalone);
        git(&root, &["add", "spec/standalone.spec.md"]);
        git(&root, &["commit", "-qm", "Change standalone spec"]);
        git(&root, &["worktree", "add", "-qb", "sibling", sibling.to_str().unwrap(), "main"]);
        Self {
            _directory: directory,
            root,
            sibling,
            outside,
        }
    }
}

pub fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn git(root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .env("GIT_AUTHOR_DATE", "2026-01-02T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-02T00:00:00Z")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
