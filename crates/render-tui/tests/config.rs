#[allow(dead_code)] // Shared fixture also contains fields used only by the snapshot suite.
mod support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Color};
use spec_viewer_config::{Config, Mode};
use spec_viewer_core::Library;
use spec_viewer_render_tui::{
    Theme,
    tui::{App, Focus},
};
use std::{io, path::Path};
use support::fixture::Fixture;

fn configured(root: &Path, source: &str, override_mode: Option<Mode>) -> App {
    std::fs::write(root.join(".spec-viewer.toml"), source).unwrap();
    let config = Config::load_layers(None, root, None).unwrap();
    let theme = Theme::from_config(&config, override_mode).unwrap();
    App::new(Library::new(root, config.parse).unwrap(), theme, io::sink()).unwrap()
}
fn draw(app: &mut App) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(160, 48)).unwrap();
    terminal.draw(|frame| app.draw(frame)).unwrap();
    terminal.backend().buffer().clone()
}
fn text(buffer: &Buffer) -> String {
    buffer.content.iter().map(|cell| cell.symbol()).collect()
}
fn assert_text_fg(buffer: &Buffer, text: &str, color: Color) -> usize {
    assert_text_fg_from_column(buffer, text, color, 0)
}
fn assert_text_fg_from_column(buffer: &Buffer, text: &str, color: Color, start: u16) -> usize {
    let chars: Vec<_> = text.chars().map(|c| c.to_string()).collect();
    let mut matches = 0;
    for y in 0..buffer.area.height {
        for x in start..=buffer.area.width.saturating_sub(chars.len() as u16) {
            if chars.iter().enumerate().all(|(i, c)| buffer[(x + i as u16, y)].symbol() == c) {
                matches += 1;
                for i in 0..chars.len() {
                    assert_eq!(buffer[(x + i as u16, y)].fg, color, "{text} at {x},{y}");
                }
            }
        }
    }
    assert!(matches > 0, "missing {text}");
    matches
}

#[test]
fn vc_4_worktree_statuses_replace_defaults_in_tree_and_header() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".plan")).unwrap();
    std::fs::write(
        root.path().join(".plan/shipped.md"),
        "---\ntitle: Released plan\nstatus: RELEASED — note\n---\n## Body\n",
    )
    .unwrap();
    std::fs::write(
        root.path().join(".plan/done.md"),
        "---\ntitle: Done plan\nstatus: done\n---\n## Body\n",
    )
    .unwrap();
    let mut app = configured(
        root.path(),
        "[[status]]\nname='shipped'\naliases=['released']\ncolor='magenta'",
        Some(Mode::Dark),
    );
    app.open(".plan/shipped.md");
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(assert_text_fg(&draw(&mut app), "[shipped]", Color::Magenta), 2);
    assert!(app.doc.as_ref().unwrap().document.meta.status.as_ref().unwrap().known);
    app.open(".plan/done.md");
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(assert_text_fg(&draw(&mut app), "[done]", Color::DarkGray), 2);
    assert!(!app.doc.as_ref().unwrap().document.meta.status.as_ref().unwrap().known);
}

#[test]
fn vc_5_custom_checklist_replaces_all_header_chips() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".plan")).unwrap();
    std::fs::write(root.path().join(".plan/risks.md"), "# Risk plan\n## Risks\n- [x] identified\n- [ ] remaining\n## Implementation\n### Phase\n- [x] task\n## Acceptance criteria\n- [x] done\n").unwrap();
    let mut app = configured(
        root.path(),
        "[[chip]]\nlabel='R'\nkind='checklist'\nsections=['risks']",
        Some(Mode::Dark),
    );
    app.open(".plan/risks.md");
    let buffer = draw(&mut app);
    assert_text_fg(&buffer, "R 1/2 ▮▮▯▯▯", Color::Cyan);
    let screen = text(&buffer);
    assert!(!screen.contains("AC 1/1"));
    assert!(!screen.contains("T 1/1"));
    assert!(!screen.contains("1 phases"));
    assert_eq!(app.doc.as_ref().unwrap().document.meta.chips.len(), 1);
}

#[test]
fn vc_6_palette_overrides_recolour_border_xrefs_and_status() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".plan")).unwrap();
    std::fs::write(
        root.path().join(".plan/a.md"),
        "---\ntitle: Plan\nstatus: accepted\n---\n## Why\nKD-1\n",
    )
    .unwrap();
    let source = "[theme]\nmode='dark'\n[theme.palette]\nblue='#123456'\n[theme.palette.light]\nblue='#654321'";
    for (mode, expected) in [
        (None, Color::Rgb(0x12, 0x34, 0x56)),
        (Some(Mode::Dark), Color::Rgb(0x12, 0x34, 0x56)),
        (Some(Mode::Light), Color::Rgb(0x65, 0x43, 0x21)),
    ] {
        let mut app = configured(root.path(), source, mode);
        app.open(".plan/a.md");
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        let buffer = draw(&mut app);
        assert_eq!(buffer[(0, 0)].fg, expected);
        assert_text_fg(&buffer, "KD-1", expected);
        assert_eq!(assert_text_fg(&buffer, "[accepted]", expected), 2);
    }
    let mut app = configured(
        root.path(),
        "[theme.palette]\nbrand='rgb(18,52,86)'\n[theme.roles]\naccent='brand'",
        Some(Mode::Dark),
    );
    assert_eq!(draw(&mut app)[(0, 0)].fg, Color::Rgb(0x12, 0x34, 0x56));
    let mut app = configured(
        root.path(),
        "[theme]\nmode='dark'\n[theme.palette.dark]\nbrand='#123456'\n[theme.roles]\naccent='brand'",
        Some(Mode::Dark),
    );
    assert_eq!(draw(&mut app)[(0, 0)].fg, Color::Rgb(0x12, 0x34, 0x56));
    let config = Config::load_layers(None, root.path(), None).unwrap();
    let error = Theme::from_config(&config, Some(Mode::Light)).unwrap_err();
    assert_eq!(error.key, "theme.roles.accent");
}

#[test]
fn vc_6_hex_palette_dialect_buffer_uses_only_palette_colours() {
    let fixture = Fixture::new();
    let slots = [
        "black",
        "red",
        "green",
        "yellow",
        "blue",
        "magenta",
        "cyan",
        "white",
        "bright_black",
        "bright_red",
        "bright_green",
        "bright_yellow",
        "bright_blue",
        "bright_magenta",
        "bright_cyan",
        "bright_white",
    ];
    let hex = |index: usize| Color::Rgb(0x10, 0x20, index as u8);
    let mut source = String::from("[theme.palette]\n");
    for (index, name) in slots.iter().enumerate() {
        source.push_str(&format!("{name}='#1020{index:02x}'\n"));
    }
    source.push_str("[theme.palette.dark]\nsurface='#102099'\n");
    let mut app = configured(&fixture.root, &source, Some(Mode::Dark));
    app.open("spec/dialect.spec.mdx");
    let surface = Color::Rgb(0x10, 0x20, 0x99);
    let mut allowed: Vec<_> = (0..16).map(hex).collect();
    allowed.extend([surface, Color::Reset]);
    let (mut pages, mut code_cells) = (0, 0);
    loop {
        let buffer = draw(&mut app);
        for cell in &buffer.content {
            assert!(allowed.contains(&cell.fg), "unexpected fg {:?} for {}", cell.fg, cell.symbol());
            assert!(allowed.contains(&cell.bg), "unexpected bg {:?} for {}", cell.bg, cell.symbol());
            code_cells += usize::from(cell.bg == surface);
        }
        pages += 1;
        let before = app.scroll;
        app.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        if before == app.scroll {
            break;
        }
    }
    assert!(pages >= 4);
    assert!(code_cells > 0);

    // Exercise specific role assignments too: palette membership alone misses swapped slots.
    std::fs::write(
        fixture.root.join(".plan/roles.md"),
        "---\ntitle: Palette roles\nstatus: accepted\n---\n# Palette roles\n\n## Body\n# Palette heading\n\nKD-1\n",
    )
    .unwrap();
    app.reload();
    app.open(".plan/roles.md");
    app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
    assert_eq!(app.focus, Focus::Sidebar);
    let buffer = draw(&mut app);
    assert_eq!(buffer[(0, 0)].fg, hex(4));
    assert_eq!(buffer[(40, 0)].fg, hex(8));
    assert_text_fg_from_column(&buffer, "Palette heading", hex(5), 40);
    assert_eq!(assert_text_fg(&buffer, "[accepted]", hex(4)), 2);
    assert_text_fg(&buffer, "KD-1", hex(4));
}

#[test]
fn vc_7_session_keeps_startup_config_on_reload_and_worktree_switch() {
    let fixture = Fixture::new();
    let mut app = configured(
        &fixture.root,
        "[theme.roles]\naccent='magenta'\n[[status]]\nname='shipped'\naliases=['in-progress']\ncolor='magenta'",
        Some(Mode::Dark),
    );
    std::fs::write(fixture.root.join(".spec-viewer.toml"), "[theme.roles]\naccent='red'").unwrap();
    std::fs::write(fixture.sibling.join(".spec-viewer.toml"), "[theme.roles]\naccent='green'").unwrap();
    app.open(".plan/rollout/index.md");
    app.reload();
    assert_text_fg(&draw(&mut app), "[shipped]", Color::Magenta);
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.worktree_path(), fixture.sibling);
    app.focus = Focus::Sidebar;
    assert_eq!(draw(&mut app)[(0, 0)].fg, Color::Magenta);
    app.open(".plan/rollout/index.md");
    assert_text_fg(&draw(&mut app), "[shipped]", Color::Magenta);
}

#[test]
fn vc_5_headings_chip_label_is_configured_and_zero_chips_are_omitted() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join(".plan")).unwrap();
    std::fs::write(root.path().join(".plan/a.md"), "# Plan\n## Risks\n### First\n### Second\n").unwrap();
    let mut app = configured(
        root.path(),
        "[[chip]]\nlabel='milestones'\nkind='headings'\nsections=['missing','risks']\n[[chip]]\nlabel='Empty meter'\nkind='checklist'\nsections=['risks']\n[[chip]]\nlabel='empty count'\nkind='headings'\nsections=['missing']",
        Some(Mode::Dark),
    );
    app.open(".plan/a.md");
    let buffer = draw(&mut app);
    let screen = text(&buffer);
    assert!(screen.contains("leaf · 2 milestones"));
    assert!(!screen.contains("Empty meter"));
    assert!(!screen.contains("empty count"));
    assert!(!screen.contains("phases"));
}
