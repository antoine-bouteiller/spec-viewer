//! A local daemon browsing a `box` host, reached through `spec-viewer daemon --stdio`.
mod support;

use spec_viewer_protocol::{Error, Event};
use support::{Host, WAIT, repository, settle, wait_connected, wait_until, write};

fn tabs(corpus: &spec_viewer_protocol::Corpus) -> Vec<&str> {
    corpus.tabs.iter().map(|tab| tab.name.as_str()).collect()
}

fn recv_until(watch: &spec_viewer_protocol::Watch, wanted: Event) {
    loop {
        match watch.recv_timeout(WAIT) {
            Ok(event) if event == wanted => return,
            Ok(_) => {}
            Err(error) => panic!("no {wanted:?}: {error}"),
        }
    }
}

#[test]
fn vc_4_a_remote_worktree_is_served_by_its_own_daemon_with_live_reload() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let b = repository(&base.path().join("box/b"), None);
    local.write_user(&remote.host_entry("box"));
    let client = local.daemon().connect().unwrap();
    wait_connected(&client, "box");

    let resolved = client.resolve(Some("box"), &b).unwrap();
    let worktree = &resolved.project.worktrees[resolved.worktree];
    assert_eq!((worktree.host.as_deref(), worktree.path.as_path()), (Some("box"), b.as_path()));
    let recorded = |host: &Host| std::fs::read_to_string(host.known_file()).unwrap_or_default();
    assert!(recorded(&remote).contains(&b.display().to_string()), "{}", recorded(&remote));
    assert!(!recorded(&local).contains(&b.display().to_string()));

    let watch = client.watch(Some("box"), &b).unwrap();
    settle(&watch);
    write(&b, ".plan/a.md", "# Alpha edited\n");
    recv_until(&watch, Event::Changed);
    assert_eq!(remote.client().corpus(Some("box"), &b).unwrap_err(), Error::UnknownHost);
}

#[test]
fn vc_5_a_lost_host_reports_host_down_then_reconnects_on_the_next_request() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let b = repository(&base.path().join("box/b"), None);
    local.write_user(&remote.host_entry("box"));
    let client = local.daemon().connect().unwrap();
    wait_connected(&client, "box");
    let watch = client.watch(Some("box"), &b).unwrap();
    settle(&watch);

    // A runtime directory of the wrong mode keeps the relay from starting another daemon.
    remote.set_runtime_mode(0o755);
    remote.kill();
    recv_until(&watch, Event::HostDown);
    let error = client.corpus(Some("box"), &b).unwrap_err();
    assert!(matches!(error, Error::HostUnreachable { .. }), "{error:?}");

    while watch.try_recv().is_ok() {}
    remote.set_runtime_mode(0o700);
    assert_eq!(tabs(&client.corpus(Some("box"), &b).unwrap()), ["plans", "specs"]);
    recv_until(&watch, Event::Changed);
    settle(&watch);
    write(&b, ".plan/a.md", "# Alpha after the reconnection\n");
    recv_until(&watch, Event::Changed);
}

#[test]
fn vc_9_a_remote_document_takes_the_statuses_of_the_local_user_config() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let b = repository(&base.path().join("box/b"), None);
    let shipped = "[[status]]\nname = 'shipped'\ncolor = 'green'\n";
    local.write_user(&format!("{shipped}{}", remote.host_entry("box")));
    let client = local.daemon().connect().unwrap();
    wait_connected(&client, "box");
    let known = |opened: spec_viewer_protocol::Opened| opened.document.meta.status.as_ref().map(|status| status.known);

    assert_eq!(known(client.open(Some("box"), &b, ".plan/a.md").unwrap()), Some(true));
    assert_eq!(known(remote.client().open(None, &b, ".plan/a.md").unwrap()), Some(false));
    let watch = client.watch(Some("box"), &b).unwrap();
    settle(&watch);
    local.write_user(&remote.host_entry("box"));
    recv_until(&watch, Event::Changed);
    wait_until("the status to become unknown", || {
        known(client.open(Some("box"), &b, ".plan/a.md").unwrap()) == Some(false)
    });
}

#[test]
fn vc_10_a_remote_clone_joins_the_local_project_by_origin() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let a = repository(&base.path().join("local/r"), Some("git@GitHub.com:o/r.git"));
    let b = repository(&base.path().join("box/b"), Some("https://github.com/o/r"));
    let c = repository(&base.path().join("box/r"), None);
    write(
        &remote.dir,
        "state/spec-viewer/projects.toml",
        &format!("projects = [{:?}, {:?}]\n", b.display().to_string(), c.display().to_string()),
    );
    local.write_user(&remote.host_entry("box"));
    let client = local.daemon().connect().unwrap();
    wait_connected(&client, "box");

    let resolved = client.resolve(None, &a).unwrap();
    let worktrees: Vec<_> = resolved
        .project
        .worktrees
        .iter()
        .map(|worktree| (worktree.host.as_deref(), worktree.path.as_path()))
        .collect();
    assert_eq!(worktrees, [(None, a.as_path()), (Some("box"), b.as_path())]);
    assert_eq!(resolved.worktree, 0);
    let projects = client.projects(None).unwrap();
    assert_eq!(projects.len(), 2, "{projects:#?}");
    assert_eq!(projects[0].worktrees.len(), 2);
    assert_eq!(projects[1].worktrees[0].path, c);

    let resolved = client.resolve(Some("box"), &b).unwrap();
    assert_eq!(resolved.project.path, a, "the local project leads");
    assert_eq!(resolved.worktree, 1);
}

#[test]
fn vc_11_a_remote_clone_follows_the_local_worktree_config() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let a = repository(&base.path().join("local/r"), Some("git@github.com:o/r.git"));
    let b = repository(&base.path().join("box/b"), Some("https://github.com/o/r"));
    let tab = |name: &str| format!("[[tab]]\nname = '{name}'\nfolder = '.plan'\n");
    write(&a, ".spec-viewer.toml", &tab("notes"));
    write(&b, ".spec-viewer.toml", &tab("other"));
    local.write_user(&remote.host_entry("box"));
    let client = local.daemon().connect().unwrap();
    wait_connected(&client, "box");
    client.resolve(None, &a).unwrap();

    assert_eq!(tabs(&client.corpus(Some("box"), &b).unwrap()), ["notes"]);
    let watch = client.watch(Some("box"), &b).unwrap();
    settle(&watch);
    write(&a, ".spec-viewer.toml", &tab("renamed"));
    recv_until(&watch, Event::Changed);
    wait_until("the renamed tab", || tabs(&client.corpus(Some("box"), &b).unwrap()) == ["renamed"]);

    std::fs::remove_dir_all(&a).unwrap();
    assert_eq!(tabs(&client.corpus(Some("box"), &b).unwrap()), ["other"], "without a local clone");
}

#[test]
fn a_followed_override_survives_broken_files_and_reconnection() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let a = repository(&base.path().join("local/r"), Some("git@github.com:o/r.git"));
    let b = repository(&base.path().join("box/b"), Some("https://github.com/o/r"));
    let tab = |name: &str, folder: &str| format!("[[tab]]\nname = '{name}'\nfolder = '{folder}'\n");
    write(&a, ".spec-viewer.toml", &tab("notes", "notes"));
    // A section only the user config may set: B's own file does not load.
    write(
        &b,
        ".spec-viewer.toml",
        &format!("{}[theme]\nmode = 'dark'\n", tab("other", "other")),
    );
    write(&b, "notes/n.md", "# Note\n");
    local.write_user(&remote.host_entry("box"));
    let daemon = local.daemon();
    let client = daemon.connect().unwrap();
    wait_connected(&client, "box");
    client.resolve(None, &a).unwrap();
    assert_eq!(
        tabs(&client.corpus(Some("box"), &b).unwrap()),
        ["notes"],
        "the local file replaces a broken remote one"
    );

    let watch = client.watch(Some("box"), &b).unwrap();
    assert_eq!(daemon.followed_configs(), 1);
    settle(&watch);
    write(&a, ".spec-viewer.toml", "[[tab]\n");
    recv_until(&watch, Event::Changed);
    let Error::Config { message } = client.corpus(Some("box"), &b).unwrap_err() else {
        panic!("a broken local file is a config error");
    };
    assert!(message.contains(&a.join(".spec-viewer.toml").display().to_string()), "{message}");
    write(&a, ".spec-viewer.toml", &tab("notes", "notes"));
    recv_until(&watch, Event::Changed);

    remote.kill();
    recv_until(&watch, Event::HostDown);
    recv_until(&watch, Event::Changed);
    wait_connected(&client, "box");
    settle(&watch);
    write(&b, "notes/n.md", "# Note after the reconnection\n");
    recv_until(&watch, Event::Changed);
    drop(watch);
    wait_until("the followed config to be released", || daemon.followed_configs() == 0);
}

#[test]
fn a_local_clone_found_later_is_followed_and_a_symlinked_root_takes_its_override() {
    let base = support::base();
    let (local, remote) = (Host::new(base.path(), "local"), Host::new(base.path(), "box"));
    let a = repository(&base.path().join("local/r"), Some("git@github.com:o/r.git"));
    let b = repository(&base.path().join("box/b"), Some("https://github.com/o/r"));
    let tab = |name: &str| format!("[[tab]]\nname = '{name}'\nfolder = '.plan'\n");
    write(&a, ".spec-viewer.toml", &tab("notes"));
    write(&b, ".spec-viewer.toml", "[theme]\nmode = 'dark'\n");
    local.write_user(&remote.host_entry("box"));
    let daemon = local.daemon();
    let client = daemon.connect().unwrap();
    wait_connected(&client, "box");

    let link = base.path().join("box/link");
    std::os::unix::fs::symlink(&b, &link).unwrap();
    assert!(
        matches!(client.resolve(Some("box"), &link), Err(Error::Config { .. })),
        "no local clone yet"
    );
    write(&b, ".spec-viewer.toml", &tab("other"));
    let watch = client.watch(Some("box"), &b).unwrap();
    assert_eq!(daemon.followed_configs(), 0);
    write(&b, ".spec-viewer.toml", "[theme]\nmode = 'dark'\n");
    client.resolve(None, &a).unwrap();
    let resolved = client.resolve(Some("box"), &link).unwrap();
    assert_eq!(
        resolved.project.path, a,
        "the broken remote file is replaced through a symlinked root"
    );
    assert_eq!(tabs(&client.corpus(Some("box"), &b).unwrap()), ["notes"]);
    assert_eq!(daemon.followed_configs(), 1, "the watch follows the clone found later");
    settle(&watch);
    write(&a, ".spec-viewer.toml", &tab("renamed"));
    recv_until(&watch, Event::Changed);
    wait_until("the renamed tab", || tabs(&client.corpus(Some("box"), &b).unwrap()) == ["renamed"]);
}
