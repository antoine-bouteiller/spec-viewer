use std::{
    path::Path,
    process::{Command, Output},
};

fn run(root: &Path, user: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_spec-viewer"))
        .arg(root)
        .args(args)
        .env("XDG_CONFIG_HOME", user)
        .output()
        .unwrap()
}
fn assert_error(output: Output, file: &Path, key: &str) {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.starts_with("spec-viewer: "), "{stderr}");
    assert!(stderr.contains(&file.display().to_string()), "{stderr}");
    assert!(stderr.contains(key), "{stderr}");
}

#[test]
fn vc_7_binary_config_errors_are_single_lines_with_file_and_key() {
    let root = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    std::fs::create_dir(user.path().join("spec-viewer")).unwrap();
    let user_file = user.path().join("spec-viewer/config.toml");
    std::fs::write(&user_file, "[theme\naccent='red'").unwrap();
    assert_error(run(root.path(), user.path(), &[]), &user_file, "line 1");
    std::fs::write(&user_file, "[theme]\nunknown='red'").unwrap();
    assert_error(run(root.path(), user.path(), &[]), &user_file, "theme.unknown");
    std::fs::remove_file(&user_file).unwrap();
    let worktree_file = root.path().join(".spec-viewer.toml");
    std::fs::write(&worktree_file, "[[status]]\nname=''\ncolor='red'").unwrap();
    assert_error(run(root.path(), user.path(), &[]), &worktree_file, "status[0].name");
}

#[test]
fn vc_8_binary_rejects_frontend_sections_in_a_worktree_file_and_config_option() {
    let root = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let file = root.path().join(".spec-viewer.toml");
    for section in [
        "[[host]]\nname='box'\nssh='box'",
        "[web]\nport=1",
        "[keys]\nquit='x'",
        "[theme]\nmode='dark'",
        "[tui]\ndiff=true",
    ] {
        std::fs::write(&file, section).unwrap();
        let key = section.trim_start_matches('[').split([']', '\n']).next().unwrap();
        assert_error(run(root.path(), user.path(), &[]), &file, key);
    }
    let output = run(root.path(), user.path(), &["--config", "x.toml"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "spec-viewer: unknown option: --config\n");
}

#[test]
fn vc_6_binary_rejects_missing_palette_role_and_unknown_theme() {
    let root = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    std::fs::create_dir(user.path().join("spec-viewer")).unwrap();
    let file = user.path().join("spec-viewer/config.toml");
    std::fs::write(&file, "[theme.roles]\naccent='missing'").unwrap();
    assert_error(run(root.path(), user.path(), &[]), &file, "theme.roles.accent");
    std::fs::write(&file, "[theme]\nname='catppuccin-mocha'").unwrap();
    assert_error(run(root.path(), user.path(), &[]), &file, "theme.name");
}

#[test]
fn vc_7_binary_loads_the_worktree_containing_nested_root() {
    let root = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git").arg("-C").arg(root.path()).args(args).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    };
    git(&["init", "-qb", "main"]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.test",
        "commit",
        "--allow-empty",
        "-qm",
        "initial",
    ]);
    let sibling = root.path().join("second");
    git(&["worktree", "add", "-qb", "second", sibling.to_str().unwrap()]);
    let nested = sibling.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let file = sibling.join(".spec-viewer.toml");
    // A different main-worktree error would expose choosing project.path instead.
    std::fs::write(root.path().join(".spec-viewer.toml"), "unknown=true").unwrap();
    std::fs::write(&file, "tab=[]").unwrap();
    assert_error(run(&nested, user.path(), &[]), &file, "tab");
}

#[test]
fn vc_7_binary_cli_boundary_errors_are_single_lines() {
    let root = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    for (args, expected) in [
        (vec!["--theme", "blue"], "invalid theme: blue"),
        (vec!["--theme"], "invalid theme:"),
        (vec!["--config"], "unknown option: --config"),
    ] {
        let output = run(root.path(), user.path(), &args);
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr.lines().count(), 1);
        assert!(stderr.contains(expected), "{stderr}");
    }
    for (path, expected) in [
        (root.path().join("missing"), "cannot resolve root"),
        (root.path().join("file"), "root is not a directory"),
    ] {
        if expected.starts_with("root") {
            std::fs::write(&path, "").unwrap();
        }
        let output = run(&path, user.path(), &[]);
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert_eq!(stderr.lines().count(), 1);
        assert!(stderr.contains(expected));
    }
}
