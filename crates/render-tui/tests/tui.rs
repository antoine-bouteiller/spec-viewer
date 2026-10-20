mod support;

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use spec_viewer_core::{Library, ParseConfig, watch};
use spec_viewer_render_tui::{
    terminal::{Target, Theme},
    tui::{App, Focus, RailEntry, Tab},
};
use support::fixture::{Fixture, write};

#[derive(Clone, Default)]
struct Clipboard(Arc<Mutex<Vec<u8>>>);

impl Write for Clipboard {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn key(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}
fn type_text(app: &mut App, value: &str) {
    for character in value.chars() {
        key(app, KeyCode::Char(character));
    }
}
fn draw(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(160, 48)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..48)
        .map(|row| {
            (0..160)
                .map(|column| buffer[(column, row)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn vc_1_plan_header_tree_family_and_legacy_snapshots() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    app.open(".plan/rollout/index.md");
    insta::assert_snapshot!("vc_1_plan_index", draw(&mut app));
    key(&mut app, KeyCode::Char('e'));
    insta::assert_snapshot!("vc_1_plan_tree", draw(&mut app));
    app.open(".plan/rollout/01-design.md");
    insta::assert_snapshot!("vc_1_plan_phase", draw(&mut app));
    assert!(
        app.rows()
            .iter()
            .any(|row| row.current && row.node.path.as_deref() == Some(".plan/rollout/index.md"))
    );
    app.open(".plan/legacy.md");
    insta::assert_snapshot!("vc_1_legacy", draw(&mut app));
}

#[test]
fn vc_1_spec_outline_umbrella_and_fallback_snapshots() {
    let fixture = Fixture::new();
    assert!(fixture.outside.exists());
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    key(&mut app, KeyCode::Char('2'));
    app.open("spec/standalone.spec.md");
    insta::assert_snapshot!("vc_1_spec_outline", draw(&mut app));
    assert!(app.rail_entries().iter().all(|entry| matches!(entry, RailEntry::Section { .. })));
    app.open("doc/architecture/tree.spec.md");
    insta::assert_snapshot!("vc_1_spec_family", draw(&mut app));
    app.open("spec/broken.spec.mdx");
    insta::assert_snapshot!("vc_1_degraded", draw(&mut app));
    app.open("spec/raw.spec.md");
    insta::assert_snapshot!("vc_1_raw_html", draw(&mut app));
}

#[test]
fn vc_1_every_dialect_construct_across_document_pages() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    key(&mut app, KeyCode::Char('2'));
    app.open("spec/dialect.spec.mdx");
    let first = draw(&mut app);
    insta::assert_snapshot!("vc_1_dialect_0", first);
    let mut page = 1;
    loop {
        let previous = app.scroll;
        key(&mut app, KeyCode::PageDown);
        if app.scroll == previous {
            break;
        }
        insta::assert_snapshot!(format!("vc_1_dialect_{page}"), draw(&mut app));
        page += 1;
    }
    assert!(page >= 4);
}

#[test]
fn vc_2_filter_is_flat_with_paths_and_without_attached_examples() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "rollout");
    key(&mut app, KeyCode::Enter);
    let screen = draw(&mut app);
    assert!(app.filtering());
    let rows = app.rows();
    assert!(rows.iter().all(|row| row.depth == 0 && row.node.path.is_some()));
    assert!(
        rows.iter()
            .any(|row| row.node.path.as_deref() == Some(".plan/rollout/01-design.md"))
    );
    assert!(rows.iter().all(|row| row.node.kind != "examples"));
    assert!(screen.contains(".plan/rollout/index.md"));
    assert!(!screen.contains("Rollout examples"));
    key(&mut app, KeyCode::Esc);
    assert!(!app.filtering());
}

#[test]
fn vc_2_diff_is_specs_only_and_restricts_to_changed_paths() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    key(&mut app, KeyCode::Char('d'));
    assert!(!app.diff);
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Char('d'));
    let paths: Vec<_> = app.rows().into_iter().map(|row| row.node.path.unwrap()).collect();
    assert_eq!(paths, ["spec/standalone.spec.md"]);
    let screen = draw(&mut app);
    assert!(screen.contains("[Diff]"));
    assert!(screen.contains("spec/standalone.spec.md"));
    app.docs.changed_specs = None;
    key(&mut app, KeyCode::Char('d'));
    assert!(app.diff);
    assert!(!app.filtering());
}

#[test]
fn vc_2_worktree_switch_clears_document_and_tabs_keep_it_with_filter_and_diff() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root.join(".plan/rollout"), ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    assert_eq!(app.worktree_path(), fixture.root);
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Char('d'));
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "standalone");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    assert!(app.doc.is_some());
    key(&mut app, KeyCode::Char('w'));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.worktree_path(), fixture.sibling);
    assert!(app.doc.is_none());
    assert_eq!(app.filter, "standalone");
    assert!(app.diff);
    app.open("spec/standalone.spec.md");
    key(&mut app, KeyCode::Char('1'));
    assert_eq!(app.tab, Tab::Plans);
    assert_eq!(app.doc.as_ref().unwrap().root_path, "spec/standalone.spec.md");
    assert_eq!(app.filter, "standalone");
    assert!(app.diff);
    let sibling = App::new(
        Library::new(&fixture.sibling.join("spec"), ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    assert_eq!(sibling.worktree_path(), fixture.sibling);
}

#[test]
fn vc_2_phase_opened_from_rail_keeps_owner_current_and_documents_never_expand() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    key(&mut app, KeyCode::Char('/'));
    type_text(&mut app, "API rollout");
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    draw(&mut app);
    let count = app.rows().len();
    key(&mut app, KeyCode::Right);
    assert_eq!(app.rows().len(), count);
    key(&mut app, KeyCode::Tab);
    let phase_position = app
        .rail_entries()
        .iter()
        .position(|entry| matches!(entry, RailEntry::File { path, .. } if path == ".plan/rollout/01-design.md"))
        .unwrap();
    for _index in 0..phase_position {
        key(&mut app, KeyCode::Down);
    }
    key(&mut app, KeyCode::Enter);
    assert_eq!(app.doc.as_ref().unwrap().root_path, ".plan/rollout/01-design.md");
    assert!(
        app.rows()
            .iter()
            .any(|row| row.current && row.node.path.as_deref() == Some(".plan/rollout/index.md"))
    );
}

#[test]
fn vc_2_visible_hints_copy_xrefs_open_docs_and_copy_paths_with_osc_52() {
    let fixture = Fixture::new();
    let clipboard = Clipboard::default();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        clipboard.clone(),
    )
    .unwrap();
    app.open(".plan/actions.md");
    draw(&mut app);
    key(&mut app, KeyCode::Char('f'));
    let reference = app
        .hints
        .iter()
        .find(|hint| hint.span.target == Target::Xref("G-1".into()))
        .unwrap()
        .label
        .clone();
    type_text(&mut app, &reference);
    assert_eq!(&*clipboard.0.lock().unwrap(), b"\x1b]52;c;Ry0x\x07");
    key(&mut app, KeyCode::Char('f'));
    let document = app
        .hints
        .iter()
        .find(|hint| hint.span.target == Target::Doc("spec/standalone.spec.md".into()))
        .unwrap()
        .label
        .clone();
    type_text(&mut app, &document);
    assert_eq!(app.doc.as_ref().unwrap().root_path, "spec/standalone.spec.md");
    clipboard.0.lock().unwrap().clear();
    key(&mut app, KeyCode::Char('Y'));
    assert_eq!(&*clipboard.0.lock().unwrap(), b"\x1b]52;c;c3RhbmRhbG9uZS5zcGVjLm1k\x07");
    clipboard.0.lock().unwrap().clear();
    key(&mut app, KeyCode::Char('y'));
    use base64::{Engine, engine::general_purpose::STANDARD};
    let sequence = format!(
        "\x1b]52;c;{}\x07",
        STANDARD.encode(fixture.root.join("spec/standalone.spec.md").to_string_lossy().as_bytes())
    );
    assert_eq!(&*clipboard.0.lock().unwrap(), sequence.as_bytes());
}

#[test]
fn vc_3_live_reload_adds_plan_refreshes_body_and_preserves_scroll() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    app.open(".plan/actions.md");
    key(&mut app, KeyCode::Char('e'));
    draw(&mut app);
    key(&mut app, KeyCode::PageDown);
    let scroll = app.scroll;
    assert!(scroll > 0);
    let watcher = watch(&fixture.root).unwrap();
    write(
        &fixture.root,
        ".plan/new.md",
        "---\ntitle: New plan\nstatus: ready\n---\n## New\n\nNewly written body.\n",
    );
    let path = fixture.root.join(".plan/actions.md");
    let body = std::fs::read_to_string(&path)
        .unwrap()
        .replace("reading and navigating", "EDITED live document");
    std::fs::write(&path, body).unwrap();
    watcher.reloads.recv_timeout(Duration::from_secs(5)).unwrap();
    app.reload();
    let screen = draw(&mut app);
    assert_eq!(app.scroll, scroll);
    assert!(app.rows().iter().any(|row| row.node.path.as_deref() == Some(".plan/new.md")));
    assert!(screen.contains("EDITED live document"));
    assert!(screen.contains("New plan"));
    assert!(app.navigator);
}

#[test]
fn vc_2_document_focus_pans_wide_blocks_with_h_and_l() {
    let fixture = Fixture::new();
    let line = (0..40).map(|index| format!("col{index:02}")).collect::<Vec<_>>().join(" ");
    write(
        &fixture.root,
        ".plan/wide.md",
        &format!("---\ntitle: Wide\n---\n```text\n{line}\n```\n"),
    );
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    app.open(".plan/wide.md");
    let screen = draw(&mut app);
    assert!(screen.contains("←→ pan"), "{screen}");
    assert!(!screen.contains("col39"));
    key(&mut app, KeyCode::Char('l'));
    assert_eq!(app.pan, 8);
    assert!(draw(&mut app).contains("│‹l01 col02 col03"));
    for _ in 0..40 {
        key(&mut app, KeyCode::Right);
    }
    assert_eq!(app.pan, app.rendered.max_pan());
    assert!(draw(&mut app).contains("col39"));
    key(&mut app, KeyCode::Char('h'));
    assert_eq!(app.pan, app.rendered.max_pan() - 8);
}

#[test]
fn vc_2_focus_scrolling_expansion_and_quit_state_transitions() {
    let fixture = Fixture::new();
    let mut app = App::new(
        Library::new(&fixture.root, ParseConfig::default()).unwrap(),
        Theme::default(),
        io::sink(),
    )
    .unwrap();
    app.open(".plan/actions.md");
    assert!(!app.navigator);
    assert_eq!(app.focus, Focus::Document);
    draw(&mut app);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Rail);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Document);
    key(&mut app, KeyCode::Char('j'));
    assert_eq!(app.scroll, 1);
    key(&mut app, KeyCode::PageDown);
    assert!(app.scroll > 1);
    key(&mut app, KeyCode::Char('G'));
    let bottom = app.scroll;
    key(&mut app, KeyCode::Char('k'));
    assert_eq!(app.scroll, bottom - 1);
    key(&mut app, KeyCode::Char('g'));
    assert_eq!(app.scroll, 0);
    key(&mut app, KeyCode::BackTab);
    assert_eq!(app.focus, Focus::Rail);
    key(&mut app, KeyCode::Char('e'));
    assert!(app.navigator);
    assert_eq!(app.focus, Focus::Sidebar);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Document);
    key(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Sidebar);
    key(&mut app, KeyCode::Char('e'));
    assert!(!app.navigator);
    assert_eq!(app.focus, Focus::Document);
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Char('2'));
    key(&mut app, KeyCode::Right);
    assert!(app.rows().len() > 2);
    key(&mut app, KeyCode::Right);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Left);
    assert_eq!(app.rows().len(), 2);
    app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.quit);
}
