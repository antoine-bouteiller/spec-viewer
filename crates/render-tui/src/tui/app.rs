use std::{
    collections::HashSet,
    io::{self, Write},
    path::Path,
};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use super::navigation::{self, Row};
use crate::terminal::{Target, TargetSpan, TermDoc, Theme, to_terminal, wrap};
use spec_viewer_core::{self as core, ChipValue, Docs, Hit, Library, Opened, Project};

const PAN_STEP: isize = 8;

/// An index into `Docs::tabs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tab(pub usize);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Rail,
    Document,
}

/// The search dialog's state.
#[derive(Clone, Debug, Default)]
pub struct Search {
    pub query: String,
    pub selection: usize,
}

#[derive(Clone, Debug)]
pub struct Hint {
    pub label: String,
    pub span: TargetSpan,
}

/// A clickable region of the left pane, recorded by the last draw.
#[derive(Clone, Copy, Debug)]
enum Click {
    Worktree,
    Tab(Tab),
    Diff,
    Search,
    Row(usize),
    Entry(usize),
}

#[derive(Clone, Debug)]
pub enum RailEntry {
    File { name: String, path: String, badge: String },
    Section { text: String, depth: u8, line: usize },
}

pub struct App {
    library: Library,
    output: Box<dyn Write>,
    pub project: Project,
    pub worktree: usize,
    pub tab: Tab,
    pub docs: Docs,
    pub doc: Option<Opened>,
    /// The open search dialog.
    pub search: Option<Search>,
    pub diff: bool,
    pub expanded: HashSet<String>,
    pub focus: Focus,
    pub scroll: usize,
    /// Columns every wide block (code, diagram, table) is panned by.
    pub pan: usize,
    pub quit: bool,
    /// The file navigator and the rail share one pane; an open document shows the rail.
    pub navigator: bool,
    /// Whether the left pane (sidebar or rail) is shown at all.
    pub panel: bool,
    pub rendered: TermDoc,
    pub hints: Vec<Hint>,
    pub message: Option<String>,
    pub web_url: Option<String>,
    theme: Theme,
    sidebar_selection: usize,
    rail_selection: usize,
    picker: Option<usize>,
    hint_input: Option<String>,
    viewport: Rect,
    panel_area: Rect,
    document_area: Rect,
    clicks: Vec<(Rect, Click)>,
}

impl App {
    pub fn new(library: Library, theme: Theme, output: impl Write + 'static) -> io::Result<Self> {
        let project = library.project();
        let worktree = project
            .worktrees
            .iter()
            .position(|worktree| worktree.path == library.containing_worktree().path)
            .unwrap_or(0);
        let selected = project
            .worktrees
            .get(worktree)
            .ok_or_else(|| io::Error::other("project has no accessible worktree"))?;
        let docs = library.corpus(&selected.path);
        let mut app = Self {
            library,
            output: Box::new(output),
            project,
            worktree,
            tab: Tab(0),
            docs,
            doc: None,
            search: None,
            diff: false,
            expanded: HashSet::new(),
            focus: Focus::Sidebar,
            scroll: 0,
            pan: 0,
            quit: false,
            navigator: true,
            panel: true,
            rendered: TermDoc::default(),
            hints: Vec::new(),
            message: None,
            web_url: None,
            theme,
            sidebar_selection: 0,
            rail_selection: 0,
            picker: None,
            hint_input: None,
            viewport: Rect::default(),
            panel_area: Rect::default(),
            document_area: Rect::default(),
            clicks: Vec::new(),
        };
        app.settle_tab();
        Ok(app)
    }

    /// Tabs in config order.
    fn all_tabs(&self) -> impl Iterator<Item = Tab> {
        (0..self.docs.tabs.len()).map(Tab)
    }

    /// The tabs listing documents; all when none does.
    pub fn tabs(&self) -> Vec<Tab> {
        let tabs: Vec<_> = self.all_tabs().filter(|tab| !self.tab_docs(*tab).is_empty()).collect();
        if tabs.is_empty() { self.all_tabs().collect() } else { tabs }
    }

    pub fn tab_name(&self, tab: Tab) -> &str {
        &self.docs.tabs[tab.0].name
    }

    fn tab_docs(&self, tab: Tab) -> &[core::DocNode] {
        &self.docs.tabs[tab.0].nodes
    }

    fn all_docs(&self) -> Vec<core::DocNode> {
        self.docs.tabs.iter().flat_map(|tab| &tab.nodes).cloned().collect()
    }

    fn settle_tab(&mut self) {
        let tabs = self.tabs();
        if !tabs.contains(&self.tab) {
            self.tab = tabs[0];
        }
    }

    pub fn worktree_path(&self) -> &Path {
        &self.project.worktrees[self.worktree].path
    }

    pub(super) fn watch(&self) -> io::Result<core::Watch> {
        self.library.watch(self.worktree_path()).map_err(io::Error::other)
    }

    fn listed(&self) -> &[core::DocNode] {
        self.tab_docs(self.tab)
    }

    pub fn filtering(&self) -> bool {
        self.diff && self.docs.changed.is_some()
    }

    pub fn rows(&self) -> Vec<Row> {
        let changed = if self.diff { self.docs.changed.as_deref() } else { None };
        navigation::rows(
            self.listed(),
            &self.expanded,
            self.doc.as_ref().map(|doc| doc.root_path.as_str()),
            changed,
        )
    }

    /// The search dialog's matches across every tab, best first.
    pub fn hits(&self) -> Vec<Hit> {
        self.search
            .as_ref()
            .map(|search| core::search(&self.docs.tabs, &search.query))
            .unwrap_or_default()
    }

    pub fn rail_entries(&self) -> Vec<RailEntry> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let all = self.all_docs();
        let sections = || {
            self.rendered
                .toc
                .iter()
                .map(|(entry, line)| RailEntry::Section {
                    text: entry.text.clone(),
                    depth: entry.depth,
                    line: *line,
                })
                .collect::<Vec<_>>()
        };
        if let Some(files) = navigation::family(&all, &doc.root_path) {
            let mut entries = Vec::new();
            for (index, file) in files.into_iter().enumerate() {
                if let Some(path) = file.path {
                    let current = path == doc.root_path;
                    entries.push(RailEntry::File {
                        name: file.name,
                        path,
                        badge: if index == 0 {
                            "index".into()
                        } else if file.kind == "examples" {
                            "examples".into()
                        } else {
                            String::new()
                        },
                    });
                    if current {
                        entries.extend(sections());
                    }
                }
            }
            entries
        } else {
            sections()
        }
    }

    pub fn open(&mut self, path: &str) {
        match self.library.open(self.worktree_path(), path) {
            Ok(doc) => {
                let all = self.all_docs();
                self.expanded.extend(navigation::ancestors(&all, &doc.root_path, ""));
                self.rendered = to_terminal(&doc.document, self.viewport.width.max(1), &self.theme);
                self.doc = Some(doc);
                self.scroll = 0;
                self.pan = 0;
                self.navigator = false;
                if self.focus == Focus::Sidebar {
                    self.focus = Focus::Document;
                }
                self.rail_selection = self.rail_entries().iter().position(|entry| matches!(entry, RailEntry::File { path, .. } if self.doc.as_ref().is_some_and(|doc| doc.root_path == *path))).unwrap_or(0);
                self.message = None;
                if let Some(index) = self.rows().iter().position(|row| row.current) {
                    self.sidebar_selection = index;
                }
            }
            Err(error) => {
                self.doc = None;
                self.rendered = TermDoc::default();
                self.message = Some(format!("Could not load document: {error:?}"));
            }
        }
        self.cancel_hints();
    }

    pub fn reload(&mut self) {
        let path = self.worktree_path().to_path_buf();
        let project = self.library.refresh_project();
        if let Some(index) = project.worktrees.iter().position(|worktree| worktree.path == path) {
            self.worktree = index;
            self.project = project;
        }
        self.docs = self.library.corpus(self.worktree_path());
        self.settle_tab();
        self.expanded.clear();
        if let Some(path) = self.doc.as_ref().map(|doc| doc.root_path.clone()) {
            let (scroll, pan, navigator, focus) = (self.scroll, self.pan, self.navigator, self.focus);
            self.open(&path);
            (self.scroll, self.pan, self.navigator, self.focus) = (scroll, pan, navigator, focus);
        }
    }

    fn clear_document(&mut self) {
        self.doc = None;
        self.rendered = TermDoc::default();
        self.scroll = 0;
        self.pan = 0;
        self.rail_selection = 0;
        self.sidebar_selection = 0;
        self.expanded.clear();
        self.message = None;
        self.navigator = true;
        self.panel = true;
        if self.focus == Focus::Rail {
            self.focus = Focus::Sidebar;
        }
        self.cancel_hints();
    }

    fn cancel_hints(&mut self) {
        self.hints.clear();
        self.hint_input = None;
    }

    fn copy(&mut self, value: &str) {
        let sequence = format!("\x1b]52;c;{}\x07", base64(value.as_bytes()));
        if let Err(error) = self.output.write_all(sequence.as_bytes()).and_then(|()| self.output.flush()) {
            self.message = Some(format!("Clipboard failed: {error}"));
        } else {
            self.message = Some(format!("Copied {value} (OSC 52)"));
        }
    }

    /// The web app's URL for the current worktree, tab, filter mode, and document.
    pub fn web_link(&self) -> Option<String> {
        let base = self.web_url.as_ref()?;
        let mut query = vec![
            ("wt", self.worktree_path().to_string_lossy().into_owned()),
            ("tab", self.tab_name(self.tab).to_owned()),
        ];
        if let Some(doc) = &self.doc {
            query.push(("doc", doc.root_path.clone()));
        }
        if self.diff {
            query.push(("diff", "true".to_owned()));
        }
        let query = query
            .iter()
            .map(|(key, value)| format!("{key}={}", percent_encode(value)))
            .collect::<Vec<_>>()
            .join("&");
        Some(format!("{base}/?{query}"))
    }

    fn open_browser(&mut self) {
        let Some(link) = self.web_link() else {
            self.message = Some("The web app is not running.".into());
            return;
        };
        let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        let launched = std::process::Command::new(program)
            .arg(&link)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        self.message = Some(match launched {
            Ok(mut child) => {
                std::thread::spawn(move || child.wait());
                format!("Opened {link}")
            }
            // Over SSH there is usually no opener; the link can still be copied from the footer.
            Err(error) => format!("Could not run {program} ({error}): {link}"),
        });
    }

    fn start_hints(&mut self) {
        let visible: Vec<_> = self
            .rendered
            .targets_at(self.pan)
            .into_iter()
            .filter(|span| {
                span.line >= self.scroll
                    && span.line < self.scroll + usize::from(self.viewport.height)
                    && span.column < usize::from(self.viewport.width)
            })
            .collect();
        if visible.is_empty() {
            self.message = Some("No visible doc links or xrefs.".into());
            return;
        }
        let mut digits = 1;
        let mut capacity = 26;
        while capacity < visible.len() {
            digits += 1;
            capacity = capacity.saturating_mul(26);
        }
        self.hints = visible
            .into_iter()
            .enumerate()
            .map(|(index, span)| {
                let mut remainder = index;
                let mut label = vec!['a'; digits];
                for character in label.iter_mut().rev() {
                    *character = (b'a' + (remainder % 26) as u8) as char;
                    remainder /= 26;
                }
                let label = label.into_iter().collect();
                Hint { label, span }
            })
            .collect();
        self.hint_input = Some(String::new());
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        if self.hint_input.is_some() {
            match key.code {
                KeyCode::Esc => self.cancel_hints(),
                KeyCode::Backspace => {
                    if let Some(input) = &mut self.hint_input {
                        input.pop();
                    }
                }
                KeyCode::Char(character) => {
                    let input = self.hint_input.as_mut().unwrap();
                    input.push(character);
                    let target = self
                        .hints
                        .iter()
                        .find(|hint| hint.label == *input)
                        .map(|hint| hint.span.target.clone());
                    if let Some(target) = target {
                        self.cancel_hints();
                        match target {
                            Target::Doc(path) => self.open(&path),
                            Target::Xref(reference) => self.copy(&reference),
                        }
                    } else if !self.hints.iter().any(|hint| hint.label.starts_with(input.as_str())) {
                        self.cancel_hints();
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(selection) = self.picker {
            match key.code {
                KeyCode::Esc => self.picker = None,
                KeyCode::Up | KeyCode::Char('k') => self.picker = Some(selection.saturating_sub(1)),
                KeyCode::Down | KeyCode::Char('j') => {
                    self.picker = Some((selection + 1).min(self.project.worktrees.len().saturating_sub(1)))
                }
                KeyCode::Enter => {
                    self.worktree = selection;
                    self.picker = None;
                    self.clear_document();
                    self.docs = self.library.corpus(self.worktree_path());
                    self.settle_tab();
                }
                _ => {}
            }
            return;
        }
        if self.search.is_some() {
            self.search_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char(digit @ '1'..='9') => {
                let Some(tab) = self.all_tabs().nth(usize::from(digit as u8 - b'1')) else {
                    return;
                };
                if !self.tabs().contains(&tab) {
                    return;
                }
                self.tab = tab;
                self.navigator = true;
                self.panel = true;
                self.focus = Focus::Sidebar;
                self.sidebar_selection = self.rows().iter().position(|row| row.current).unwrap_or(0);
            }
            KeyCode::Char('w') => {
                let path = self.worktree_path().to_path_buf();
                self.project = self.library.refresh_project();
                if let Some(index) = self.project.worktrees.iter().position(|worktree| worktree.path == path) {
                    self.worktree = index;
                }
                if !self.project.worktrees.is_empty() {
                    self.picker = Some(self.worktree);
                }
            }
            KeyCode::Char('/') => self.search = Some(Search::default()),
            KeyCode::Esc => self.message = None,
            KeyCode::Char('d') if self.docs.changed.is_some() => {
                self.diff = !self.diff;
                self.sidebar_selection = 0;
            }
            KeyCode::Char('o') => self.open_browser(),
            KeyCode::Char('e') if self.doc.is_some() => {
                self.navigator = !self.navigator;
                self.panel = true;
                self.focus = if self.navigator { Focus::Sidebar } else { Focus::Document };
            }
            KeyCode::Char('b') if self.doc.is_some() => {
                self.panel = !self.panel;
                self.focus = match (self.panel, self.navigator) {
                    (false, _) => Focus::Document,
                    (true, true) => Focus::Sidebar,
                    (true, false) => Focus::Rail,
                };
            }
            KeyCode::Tab | KeyCode::BackTab if !self.panel => {}
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Document if self.navigator => Focus::Sidebar,
                    Focus::Document => Focus::Rail,
                    _ => Focus::Document,
                };
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(doc) = &self.doc {
                    let value = if key.code == KeyCode::Char('y') {
                        doc.path.to_string_lossy().into_owned()
                    } else {
                        doc.path.file_name().unwrap_or_default().to_string_lossy().into_owned()
                    };
                    self.copy(&value);
                }
            }
            KeyCode::Char('f') => self.start_hints(),
            KeyCode::PageUp => self.scroll_by(-isize::try_from(self.viewport.height.saturating_sub(1).max(1)).unwrap()),
            KeyCode::PageDown => self.scroll_by(isize::try_from(self.viewport.height.saturating_sub(1).max(1)).unwrap()),
            KeyCode::Char('g') => self.scroll = 0,
            KeyCode::Char('G') => self.scroll = self.max_scroll(),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(false),
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(true),
            KeyCode::Right | KeyCode::Char('l') if self.focus == Focus::Document => self.pan_by(PAN_STEP),
            KeyCode::Left | KeyCode::Char('h') if self.focus == Focus::Document => self.pan_by(-PAN_STEP),
            KeyCode::Right | KeyCode::Char('l') => self.expand(true),
            KeyCode::Left | KeyCode::Char('h') => self.expand(false),
            KeyCode::Enter => self.activate(),
            _ => {}
        }
    }

    fn search_key(&mut self, key: KeyEvent) {
        let hits = self.hits();
        let count = hits.len();
        let Some(search) = &mut self.search else {
            return;
        };
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.search = None,
            KeyCode::Enter => {
                if let Some(hit) = hits.into_iter().nth(search.selection) {
                    self.search = None;
                    self.tab = Tab(hit.tab);
                    self.open(&hit.path);
                }
            }
            KeyCode::Up => search.selection = search.selection.saturating_sub(1),
            KeyCode::Char('p' | 'k') if control => search.selection = search.selection.saturating_sub(1),
            KeyCode::Down => search.selection = (search.selection + 1).min(count.saturating_sub(1)),
            KeyCode::Char('n' | 'j') if control => search.selection = (search.selection + 1).min(count.saturating_sub(1)),
            KeyCode::Char('u') if control => {
                search.query.clear();
                search.selection = 0;
            }
            KeyCode::Char('w') if control => {
                let kept = search.query.trim_end().rfind(char::is_whitespace).map_or(0, |index| index + 1);
                search.query.truncate(kept);
                search.selection = 0;
            }
            KeyCode::Backspace => {
                search.query.pop();
                search.selection = 0;
            }
            KeyCode::Char(character) if !control => {
                search.query.push(character);
                search.selection = 0;
            }
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollLeft => self.pan_by(-PAN_STEP),
            MouseEventKind::ScrollRight => self.pan_by(PAN_STEP),
            MouseEventKind::ScrollUp if mouse.modifiers.contains(KeyModifiers::SHIFT) => self.pan_by(-PAN_STEP),
            MouseEventKind::ScrollDown if mouse.modifiers.contains(KeyModifiers::SHIFT) => self.pan_by(PAN_STEP),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::Down(MouseButton::Left) if self.picker.is_none() && self.search.is_none() => {
                self.click(Position::new(mouse.column, mouse.row));
            }
            _ => {}
        }
    }

    fn click(&mut self, position: Position) {
        self.cancel_hints();
        let press = |app: &mut Self, character| app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        if self.document_area.contains(position) {
            self.focus = Focus::Document;
            return;
        }
        if !self.panel || !self.panel_area.contains(position) {
            return;
        }
        self.focus = if self.navigator { Focus::Sidebar } else { Focus::Rail };
        let Some((_, click)) = self.clicks.iter().find(|(area, _)| area.contains(position)).copied() else {
            return;
        };
        match click {
            Click::Worktree => press(self, 'w'),
            Click::Tab(tab) => {
                let index = self.all_tabs().position(|other| other == tab).unwrap_or(0);
                press(self, char::from(b'1' + index as u8));
            }
            Click::Diff => press(self, 'd'),
            Click::Search => press(self, '/'),
            Click::Row(index) => {
                self.sidebar_selection = index;
                self.activate();
            }
            Click::Entry(index) => {
                self.rail_selection = index;
                self.activate();
            }
        }
    }

    fn max_scroll(&self) -> usize {
        self.rendered.lines.len().saturating_sub(usize::from(self.viewport.height))
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta).min(self.max_scroll());
        self.cancel_hints();
    }

    pub fn pan_by(&mut self, delta: isize) {
        self.pan = self.pan.saturating_add_signed(delta).min(self.rendered.max_pan());
        self.cancel_hints();
    }

    fn move_selection(&mut self, down: bool) {
        match self.focus {
            Focus::Document => self.scroll_by(if down { 1 } else { -1 }),
            Focus::Sidebar => {
                self.sidebar_selection = if down {
                    (self.sidebar_selection + 1).min(self.rows().len().saturating_sub(1))
                } else {
                    self.sidebar_selection.saturating_sub(1)
                }
            }
            Focus::Rail => {
                self.rail_selection = if down {
                    (self.rail_selection + 1).min(self.rail_entries().len().saturating_sub(1))
                } else {
                    self.rail_selection.saturating_sub(1)
                }
            }
        }
    }

    fn expand(&mut self, right: bool) {
        if self.focus != Focus::Sidebar || self.filtering() {
            return;
        }
        let rows = self.rows();
        let Some(row) = rows.get(self.sidebar_selection) else {
            return;
        };
        let has_children = !navigation::family_root(&row.node) && !navigation::children(&row.node).is_empty();
        if right {
            if has_children && !self.expanded.contains(&row.key) {
                self.expanded.insert(row.key.clone());
            } else if let Some(index) = rows.iter().position(|child| child.parent.as_deref() == Some(&row.key)) {
                self.sidebar_selection = index;
            }
        } else if has_children && self.expanded.contains(&row.key) {
            self.expanded.remove(&row.key);
        } else if let Some(index) = rows.iter().position(|parent| Some(&parent.key) == row.parent.as_ref()) {
            self.sidebar_selection = index;
        }
    }

    fn activate(&mut self) {
        match self.focus {
            Focus::Sidebar => {
                if let Some(row) = self.rows().get(self.sidebar_selection) {
                    if let Some(path) = &row.node.path {
                        self.open(path);
                    } else if self.expanded.contains(&row.key) {
                        self.expanded.remove(&row.key);
                    } else {
                        self.expanded.insert(row.key.clone());
                    }
                }
            }
            Focus::Rail => {
                if let Some(entry) = self.rail_entries().get(self.rail_selection) {
                    match entry {
                        RailEntry::File { path, .. } => self.open(path),
                        RailEntry::Section { line, .. } => self.scroll = *line,
                    }
                }
            }
            Focus::Document => {}
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let footer_height = u16::from(area.height > 1);
        let vertical = Layout::vertical([Constraint::Min(0), Constraint::Length(footer_height)]).split(area);
        let side_width = if self.panel {
            (area.width / 4).clamp(30, 42).min(area.width)
        } else {
            0
        };
        let panes = Layout::horizontal([Constraint::Length(side_width), Constraint::Min(0)]).split(vertical[0]);
        self.clicks.clear();
        (self.panel_area, self.document_area) = (panes[0], panes[1]);
        match (self.panel, self.navigator) {
            (false, _) => {}
            (true, true) => self.draw_sidebar(frame, panes[0]),
            (true, false) => self.draw_rail(frame, panes[0]),
        }
        self.draw_document(frame, panes[1]);
        let footer = if let Some(input) = &self.hint_input {
            format!("Hint: {input}  Esc cancel")
        } else if self.search.is_some() {
            "Search all tabs · ↑↓ C-n/C-p move · Enter open · C-u clear · Esc close".to_owned()
        } else {
            self.message.clone().unwrap_or_else(|| {
                let mut keys = vec![
                    "Tab focus".to_owned(),
                    "1/2 tabs".into(),
                    "w worktree".into(),
                    "/ search".into(),
                    "d Diff".into(),
                    "f links".into(),
                    "y/Y copy".into(),
                    "e files".into(),
                    "b panel".into(),
                    "q quit".into(),
                ];
                if self.doc.is_some() && self.rendered.max_pan() > 0 {
                    keys.insert(1, "←→ pan".into());
                }
                keys.extend(self.web_url.as_ref().map(|url| format!("o {url}")));
                fit_keys(&keys, usize::from(area.width))
            })
        };
        frame.render_widget(
            Paragraph::new(truncate(&footer, usize::from(area.width))).style(Style::default().fg(self.theme.roles.muted)),
            vertical[1],
        );
        if let Some(selection) = self.picker {
            let width = area.width.min(60);
            let height = area.height.min(self.project.worktrees.len() as u16 + 2);
            let popup = Rect::new(
                area.x + (area.width - width) / 2,
                area.y + (area.height - height) / 2,
                width,
                height,
            );
            frame.render_widget(Clear, popup);
            let items: Vec<_> = self
                .project
                .worktrees
                .iter()
                .map(|worktree| ListItem::new(format!("{}{}", worktree.name, if worktree.main { " (main)" } else { "" })))
                .collect();
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::bordered().title("Worktrees · Enter select · Esc cancel"))
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
                popup,
                &mut ListState::default().with_selected(Some(selection)),
            );
        }
        self.draw_search(frame, vertical[0]);
    }

    /// A centered prompt over a results list, after Telescope's layout without a preview.
    fn draw_search(&mut self, frame: &mut Frame, area: Rect) {
        let hits = self.hits();
        let total = core::search(&self.docs.tabs, "").len();
        let Some(search) = &mut self.search else {
            return;
        };
        search.selection = search.selection.min(hits.len().saturating_sub(1));
        let width = (area.width * 4 / 5).max(area.width.min(50)).min(100);
        let height = (area.height * 4 / 5).max(area.height.min(10));
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, popup);
        let [prompt, results] = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(popup);
        let accent = Style::default().fg(self.theme.roles.accent);
        let muted = Style::default().fg(self.theme.roles.muted);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("> ", accent),
                Span::raw(format!("{}▏", search.query)),
            ]))
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(accent)
                    .title(" Search ")
                    .title(Line::from(format!(" {}/{total} ", hits.len())).right_aligned()),
            ),
            prompt,
        );
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(muted)
            .title(" Results ");
        let inner = block.inner(results);
        frame.render_widget(block, results);
        if hits.is_empty() {
            frame.render_widget(Paragraph::new("no matching documents").style(muted), inner);
            return;
        }
        let matched = accent.add_modifier(Modifier::BOLD);
        let highlight = |text: &str, matches: &[usize], style: Style| -> Vec<Span<'static>> {
            text.chars()
                .enumerate()
                .map(|(index, character)| {
                    Span::styled(
                        character.to_string(),
                        if matches.binary_search(&index).is_ok() { matched } else { style },
                    )
                })
                .collect()
        };
        let width = usize::from(inner.width.saturating_sub(2));
        let items: Vec<_> = hits
            .iter()
            .map(|hit| {
                let mut spans = highlight(&hit.node.name, &hit.title_matches, Style::default());
                if hit.node.archived == Some(true) {
                    spans.push(Span::styled(" [archived]", Style::default().fg(self.theme.roles.status_unknown)));
                } else if let Some(status) = &hit.node.status {
                    spans.push(Span::styled(
                        format!(" [{}]", status.name),
                        Style::default().fg(self.theme.status_color(Some(status))),
                    ));
                }
                spans.push(Span::raw("  "));
                spans.extend(highlight(&hit.path, &hit.path_matches, muted));
                ListItem::new(truncate_line(Line::from(spans), width))
            })
            .collect();
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol(Line::styled("> ", accent))
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
            inner,
            &mut ListState::default().with_selected(Some(search.selection)),
        );
    }

    fn pane(&self, title: &str, focused: bool) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .title(title.to_owned())
            .border_style(Style::default().fg(if focused { self.theme.roles.accent } else { self.theme.roles.muted }))
    }

    fn draw_sidebar(&mut self, frame: &mut Frame, area: Rect) {
        let block = self.pane(&format!("{} · Plans", self.project.name), self.focus == Focus::Sidebar);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let regions = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(inner);
        let worktree = &self.project.worktrees[self.worktree];
        let header = regions[0];
        let row = |line: u16, x: u16, width: usize| Rect::new(header.x + x, header.y + line, width as u16, 1).intersection(header);
        let mut clicks = vec![
            (row(0, 0, usize::from(header.width)), Click::Worktree),
            (row(2, 0, usize::from(header.width)), Click::Search),
        ];
        let mut tabs = String::new();
        for tab in self.tabs() {
            if !tabs.is_empty() {
                tabs.push(' ');
            }
            let name = self.tab_name(tab);
            let label = if tab == self.tab { format!("[{name}]") } else { name.to_owned() };
            let width = unicode_width::UnicodeWidthStr::width;
            clicks.push((row(1, width(tabs.as_str()) as u16, width(label.as_str())), Click::Tab(tab)));
            tabs.push_str(&label);
        }
        let diff = if self.docs.changed.is_none() {
            " Diff unavailable"
        } else if self.diff {
            " [Diff]"
        } else {
            " Diff"
        };
        if self.docs.changed.is_some() {
            clicks.push((
                row(1, unicode_width::UnicodeWidthStr::width(tabs.as_str()) as u16 + 1, diff.len() - 1),
                Click::Diff,
            ));
        }
        let heading = vec![
            Line::raw(format!("{}{} ▾", worktree.name, if worktree.main { " (main)" } else { "" })),
            Line::raw(format!("{tabs}{diff}")),
            Line::styled("/ search all tabs", Style::default().fg(self.theme.roles.muted)),
        ];
        frame.render_widget(Paragraph::new(heading), regions[0]);
        let items: Vec<_> = self
            .rows()
            .iter()
            .map(|row| {
                let prefix = if row.node.path.is_some() {
                    "  "
                } else if self.expanded.contains(&row.key) {
                    "▾ "
                } else {
                    "▸ "
                };
                let suffix = if row.node.archived == Some(true) {
                    Some(Span::styled(" [archived]", Style::default().fg(self.theme.roles.status_unknown)))
                } else {
                    row.node.status.as_ref().map(|status| {
                        Span::styled(
                            format!(" [{}]", status.name),
                            Style::default().fg(self.theme.status_color(Some(status))),
                        )
                    })
                };
                let width = usize::from(regions[1].width);
                let label = truncate(
                    &format!("{}{prefix}{}", "  ".repeat(row.depth), row.node.name),
                    width.saturating_sub(suffix.as_ref().map_or(0, Span::width)),
                );
                let mut spans = vec![Span::styled(
                    label,
                    if row.current {
                        Style::default().fg(self.theme.roles.accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                )];
                spans.extend(suffix);
                let mut lines = vec![Line::from(spans)];
                if self.filtering() {
                    lines.push(Line::styled(
                        truncate(row.node.path.as_deref().unwrap_or_default(), width),
                        Style::default().fg(self.theme.roles.muted),
                    ));
                }
                ListItem::new(lines)
            })
            .collect();
        let heights: Vec<_> = items.iter().map(ListItem::height).collect();
        if items.is_empty() {
            frame.render_widget(
                Paragraph::new(if self.filtering() {
                    "no matching documents"
                } else {
                    "no documents"
                }),
                regions[1],
            );
        } else {
            let mut state = ListState::default().with_selected(Some(self.sidebar_selection));
            frame.render_stateful_widget(
                List::new(items).highlight_style(if self.focus == Focus::Sidebar {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                }),
                regions[1],
                &mut state,
            );
            clicks.extend(list_clicks(regions[1], state.offset(), &heights, Click::Row));
        }
        self.clicks = clicks;
    }

    fn draw_rail(&mut self, frame: &mut Frame, area: Rect) {
        let entries = self.rail_entries();
        let family = entries.iter().any(|entry| matches!(entry, RailEntry::File { .. }));
        let block = self.pane(if family { "Files" } else { "Sections" }, self.focus == Focus::Rail);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let active = self.doc.as_ref().map(|doc| doc.root_path.as_str());
        let current_section = entries
            .iter()
            .filter_map(|entry| {
                if let RailEntry::Section { line, .. } = entry {
                    Some(*line)
                } else {
                    None
                }
            })
            .filter(|line| *line <= self.scroll)
            .max();
        let items: Vec<_> = entries
            .iter()
            .map(|entry| {
                let (label, current) = match entry {
                    RailEntry::File { name, path, badge } => (
                        format!("{name}{}", if badge.is_empty() { String::new() } else { format!(" [{badge}]") }),
                        active == Some(path.as_str()),
                    ),
                    RailEntry::Section { text, depth, line } => (
                        format!(
                            "{}§ {text}",
                            " ".repeat(usize::from(depth.saturating_sub(2)) * 2 + usize::from(family) * 2)
                        ),
                        current_section == Some(*line),
                    ),
                };
                ListItem::new(
                    wrap(&label, usize::from(inner.width.max(1)))
                        .into_iter()
                        .map(Line::raw)
                        .collect::<Vec<_>>(),
                )
                .style(if current {
                    Style::default().fg(self.theme.roles.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                })
            })
            .collect();
        let heights: Vec<_> = items.iter().map(ListItem::height).collect();
        let mut state = ListState::default().with_selected(Some(self.rail_selection));
        frame.render_stateful_widget(
            List::new(items).highlight_style(if self.focus == Focus::Rail {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            }),
            inner,
            &mut state,
        );
        self.clicks = list_clicks(inner, state.offset(), &heights, Click::Entry);
    }

    fn draw_document(&mut self, frame: &mut Frame, area: Rect) {
        let block = self.pane("Document", self.focus == Focus::Document);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let Some(doc) = &self.doc else {
            self.viewport = inner;
            frame.render_widget(Paragraph::new(self.message.as_deref().unwrap_or("Select a document.")), inner);
            return;
        };
        let width = usize::from(inner.width);
        let status = Span::styled(
            doc.document
                .meta
                .status
                .as_ref()
                .map(|status| format!(" [{}]", status.name))
                .unwrap_or_default(),
            Style::default().fg(self.theme.status_color(doc.document.meta.status.as_ref())),
        );
        let mut header = vec![Line::from(vec![
            Span::styled(
                truncate(&doc.document.meta.title, width.saturating_sub(status.width())),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            status,
        ])];
        let mut metrics = Vec::new();
        for chip in &doc.document.meta.chips {
            if let ChipValue::Checklist { done, total } = chip.value
                && total > 0
            {
                metrics.push(Span::styled(
                    meter(&chip.label, done, total),
                    Style::default().fg(if done == total {
                        self.theme.roles.meter_complete
                    } else {
                        self.theme.roles.meter_progress
                    }),
                ));
            }
        }
        metrics.push(Span::raw(format!("{} lines", doc.document.meta.lines)));
        header.push(Line::from(metrics));
        let all = self.all_docs();
        let kind = navigation::find(&all, &doc.root_path)
            .map(|node| node.kind.as_str())
            .or(doc.document.meta.kind.as_deref())
            .unwrap_or("document");
        let mut details = vec![kind.to_string()];
        if let Some(owner) = navigation::family(&all, &doc.root_path).and_then(|family| family.into_iter().next())
            && owner.path.as_deref() != Some(doc.root_path.as_str())
        {
            details.push(owner.name);
        }
        for chip in &doc.document.meta.chips {
            if let ChipValue::Count(count) = chip.value
                && count > 0
            {
                details.push(format!("{count} {}", chip.label));
            }
        }
        if doc.document.meta.legacy {
            details.push("legacy format".into());
        }
        if let Some(author) = &doc.document.meta.author {
            details.push(author.clone());
        }
        if let Some(date) = &doc.document.meta.date {
            details.push(date.clone());
        }
        let mut metadata = vec![details.join(" · ")];
        if !doc.document.meta.related.is_empty() {
            metadata.push(format!("related: {}", doc.document.meta.related.join(", ")));
        }
        let keys = "  y path · Y name";
        metadata.push(format!(
            "{}{keys}",
            truncate(&doc.root_path, width.saturating_sub(unicode_width::UnicodeWidthStr::width(keys)))
        ));
        for value in metadata {
            header.push(Line::styled(value, Style::default().fg(self.theme.roles.muted)));
        }
        header.push(Line::raw("─".repeat(width)));
        let header: Vec<_> = header.into_iter().map(|line| truncate_line(line, width)).collect();
        let regions = Layout::vertical([Constraint::Length(header.len() as u16), Constraint::Min(0)]).split(inner);
        frame.render_widget(Paragraph::new(header), regions[0]);
        if self.viewport.width != regions[1].width {
            self.rendered = to_terminal(&doc.document, regions[1].width.max(1), &self.theme);
            self.pan = 0;
            self.cancel_hints();
        }
        self.viewport = regions[1];
        self.scroll = self.scroll.min(self.max_scroll());
        self.pan = self.pan.min(self.rendered.max_pan());
        let end = (self.scroll + usize::from(self.viewport.height)).min(self.rendered.lines.len());
        frame.render_widget(
            Paragraph::new(Text::from(
                (self.scroll..end)
                    .map(|index| self.rendered.line(index, self.pan))
                    .collect::<Vec<_>>(),
            )),
            self.viewport,
        );
        for hint in &self.hints {
            if hint.span.line >= self.scroll && hint.span.line < self.scroll + usize::from(self.viewport.height) {
                let column = hint
                    .span
                    .column
                    .min(usize::from(self.viewport.width.saturating_sub(hint.label.len() as u16)));
                let width = (hint.label.len() as u16).min(self.viewport.width.saturating_sub(column as u16));
                let area = Rect::new(
                    self.viewport.x + column as u16,
                    self.viewport.y + (hint.span.line - self.scroll) as u16,
                    width,
                    1,
                );
                frame.render_widget(
                    Paragraph::new(hint.label.clone()).style(
                        Style::default()
                            .fg(self.theme.roles.hint_fg)
                            .bg(self.theme.roles.hint_bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    area,
                );
            }
        }
    }
}

/// The rows each visible list item occupies, from the list's scroll offset.
fn list_clicks(area: Rect, offset: usize, heights: &[usize], click: fn(usize) -> Click) -> Vec<(Rect, Click)> {
    let mut y = 0;
    let mut clicks = Vec::new();
    for (index, height) in heights.iter().enumerate().skip(offset) {
        if y >= area.height {
            break;
        }
        let height = (*height as u16).min(area.height - y);
        clicks.push((Rect::new(area.x, area.y + y, area.width, height), click(index)));
        y += height;
    }
    clicks
}

fn meter(label: &str, done: usize, total: usize) -> String {
    let filled = done.saturating_mul(5).checked_div(total).unwrap_or(0).min(5);
    format!("{label} {}/{} {}{}  ", done, total, "▮".repeat(filled), "▯".repeat(5 - filled))
}

/// Standard padded base64.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk
            .iter()
            .enumerate()
            .fold(0_u32, |group, (index, byte)| group | u32::from(*byte) << (16 - 8 * index));
        for index in 0..4 {
            encoded.push(if index <= chunk.len() {
                char::from(ALPHABET[(group >> (18 - 6 * index) & 63) as usize])
            } else {
                '='
            });
        }
    }
    encoded
}

fn truncate(text: &str, width: usize) -> String {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if text.width() <= width {
        return text.to_owned();
    }
    let mut truncated = String::new();
    let mut used = 0;
    for character in text.chars() {
        let length = character.width().unwrap_or(0);
        if used + length + 1 > width {
            break;
        }
        truncated.push(character);
        used += length;
    }
    if width > 0 {
        truncated.push('…');
    }
    truncated
}

/// Joins whole key hints that fit in `width`, ending with `…` when some are dropped.
fn fit_keys(keys: &[String], width: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    let mut footer = String::new();
    for (index, key) in keys.iter().enumerate() {
        let separator = if footer.is_empty() { "" } else { " · " };
        let reserve = if index + 1 < keys.len() { " …".width() } else { 0 };
        if footer.width() + separator.width() + key.width() + reserve > width {
            if !footer.is_empty() {
                footer.push_str(" …");
            }
            break;
        }
        footer.push_str(separator);
        footer.push_str(key);
    }
    footer
}

/// Cuts a styled line to `width` columns, ending with `…` when it overflows.
fn truncate_line(line: Line<'static>, width: usize) -> Line<'static> {
    use unicode_width::UnicodeWidthChar;
    if line.width() <= width {
        return line;
    }
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    'spans: for span in line.spans {
        let mut content = String::new();
        for character in span.content.chars() {
            let length = character.width().unwrap_or(0);
            if used + length + 1 > width {
                if width > 0 {
                    content.push('…');
                }
                spans.push(Span::styled(content, span.style));
                break 'spans;
            }
            content.push(character);
            used += length;
        }
        spans.push(Span::styled(content, span.style));
    }
    Line::from(spans)
}

fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (input, encoded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(input.as_bytes()), encoded);
        }
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=");
    }

    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".plan")).unwrap();
        std::fs::write(root.path().join(".plan/first.md"), "## First\n\nG-1 [next](second.md)\n").unwrap();
        std::fs::write(root.path().join(".plan/second.md"), "## Second\n").unwrap();
        let app = app_at(&root);
        (root, app)
    }

    fn add_spec(root: &tempfile::TempDir) {
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(root.path())
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::create_dir(root.path().join("spec")).unwrap();
        std::fs::write(root.path().join("spec/a.spec.md"), "## A\n").unwrap();
    }

    fn app_at(root: &tempfile::TempDir) -> App {
        App::new(
            Library::new(root.path(), core::ParseConfig::default()).unwrap(),
            Theme::default(),
            io::sink(),
        )
        .unwrap()
    }

    #[test]
    fn modal_search_input_and_key_release_do_not_trigger_global_shortcuts() {
        let (_root, mut app) = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for character in "qdw12".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        assert_eq!(app.search.as_ref().unwrap().query, "qdw12");
        assert!(!app.quit);
        assert_eq!(app.tab, Tab(0));
        assert!(!app.diff);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.search.is_none());
        let mut released = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE);
        released.kind = KeyEventKind::Release;
        app.handle_key(released);
        assert!(!app.quit);
        app.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.quit);
    }

    #[test]
    fn web_link_carries_worktree_tab_and_open_document() {
        let (_root, mut app) = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
        assert_eq!(app.message.as_deref(), Some("The web app is not running."));
        app.web_url = Some("http://127.0.0.1:4100".into());
        app.open(".plan/first.md");
        let worktree = percent_encode(&app.worktree_path().to_string_lossy());
        assert_eq!(
            app.web_link().unwrap(),
            format!("http://127.0.0.1:4100/?wt={worktree}&tab=plans&doc=.plan/first.md")
        );
        assert_eq!(percent_encode("a b&c=é"), "a%20b%26c%3D%C3%A9");
    }

    #[test]
    fn section_activation_and_small_terminal_resizing_are_safe() {
        let (_root, mut app) = app();
        app.open(".plan/first.md");
        for (width, height) in [(80, 24), (30, 10), (8, 4), (1, 1)] {
            let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
        }
        app.focus = Focus::Rail;
        app.scroll = 12;
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.scroll, 0);
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        assert!(app.navigator);
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[test]
    fn footer_keeps_whole_key_hints_that_fit() {
        let keys = ["Tab focus".to_owned(), "1/2 tabs".into(), "q quit".into()];
        assert_eq!(fit_keys(&keys, 80), "Tab focus · 1/2 tabs · q quit");
        assert_eq!(fit_keys(&keys, 24), "Tab focus · 1/2 tabs …");
        assert_eq!(fit_keys(&keys, 5), "");
    }

    #[test]
    fn tab_switch_keeps_the_open_document() {
        let (root, _) = app();
        add_spec(&root);
        let mut app = app_at(&root);
        app.open(".plan/second.md");
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab(1));
        assert_eq!(app.doc.as_ref().unwrap().root_path, ".plan/second.md");
        assert!(app.navigator);
        assert_eq!(app.focus, Focus::Sidebar);
        app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE));
        assert_eq!(app.doc.as_ref().unwrap().root_path, ".plan/second.md");
        assert!(app.rows()[app.sidebar_selection].current);
    }

    #[test]
    fn configured_tabs_take_number_keys_in_order() {
        let (root, _) = app();
        add_spec(&root);
        std::fs::create_dir_all(root.path().join("docs/adr")).unwrap();
        std::fs::write(root.path().join("docs/adr/one.md"), "## One\n").unwrap();
        let mut config = core::ParseConfig::default();
        config.tabs.insert(
            1,
            spec_viewer_config::TabConfig {
                name: "adr".into(),
                folder: Some("docs/adr".into()),
                kinds: None,
                companions: vec![],
            },
        );
        let mut app = App::new(Library::new(root.path(), config).unwrap(), Theme::default(), io::sink()).unwrap();
        assert_eq!(app.tabs(), [Tab(0), Tab(1), Tab(2)]);
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab(1));
        assert_eq!(app.rows()[0].node.path.as_deref(), Some("docs/adr/one.md"));
        app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab(2));
        app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab(2));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("plans adr [specs]"), "{screen}");

        let mut config = core::ParseConfig::default();
        config.tabs.remove(0);
        let app = App::new(Library::new(root.path(), config).unwrap(), Theme::default(), io::sink()).unwrap();
        assert_eq!(app.tabs(), [Tab(0)]);
        assert_eq!(app.tab_name(Tab(0)), "specs");
        assert!(app.rows().iter().all(|row| row.node.path.as_deref() != Some(".plan/first.md")));
    }

    #[test]
    fn a_tab_without_documents_is_hidden() {
        let (root, mut app) = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert_eq!(app.tab, Tab(0));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen: String = terminal.backend().buffer().content.iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("[plans]") && !screen.contains("specs"), "{screen}");
        std::fs::remove_dir_all(root.path().join(".plan")).unwrap();
        add_spec(&root);
        assert_eq!(app_at(&root).tab, Tab(1));
    }

    #[test]
    fn header_lines_truncate_and_keep_the_copy_keys() {
        let (root, _) = app();
        let name = format!(".plan/{}.md", "long-name-".repeat(10));
        std::fs::write(
            root.path().join(&name),
            "---\ntitle: A very long title that cannot fit\nstatus: done\n---\n## Body\n",
        )
        .unwrap();
        let mut app = app_at(&root);
        app.open(&name);
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 12)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..12).map(|y| (0..40).map(|x| buffer[(x, y)].symbol()).collect()).collect();
        assert!(
            rows[1].contains("A very long title") && rows[1].contains("…") && rows[1].contains("[done]"),
            "{rows:#?}"
        );
        assert!(
            rows[4].contains(".plan/long-name-") && rows[4].contains("…  y path · Y name"),
            "{rows:#?}"
        );
        assert!(rows[5].contains("────"), "{rows:#?}");
    }

    #[test]
    fn clicks_focus_panes_and_activate_left_pane_entries() {
        let (_root, mut app) = app();
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        let click = |app: &mut App, column, row| {
            app.handle_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            })
        };
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let second = app.rows()[1].node.path.clone().unwrap();
        click(&mut app, 3, 5);
        assert_eq!(app.doc.as_ref().unwrap().root_path, second);
        assert_eq!(app.focus, Focus::Document);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        click(&mut app, 3, 20);
        assert_eq!(app.focus, Focus::Rail);
        click(&mut app, 50, 10);
        assert_eq!(app.focus, Focus::Document);
        app.scroll = 3;
        click(&mut app, 3, 1);
        assert_eq!(app.focus, Focus::Rail);
        assert_eq!(app.scroll, 0);
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        click(&mut app, 3, 3);
        assert!(app.search.is_some());
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        click(&mut app, 3, 1);
        assert_eq!(app.picker, Some(app.worktree));
    }

    #[test]
    fn clicking_a_tab_label_switches_tab() {
        let (root, _) = app();
        add_spec(&root);
        let mut app = app_at(&root);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let down = |column| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row: 2,
            modifiers: KeyModifiers::NONE,
        };
        app.handle_mouse(down(10));
        assert_eq!(app.tab, Tab(1));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_mouse(down(2));
        assert_eq!(app.tab, Tab(0));
    }

    #[test]
    fn hidden_panel_gives_the_document_the_full_width() {
        let (_root, mut app) = app();
        app.open(".plan/first.md");
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert!(!app.panel);
        assert_eq!(app.focus, Focus::Document);
        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Document);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert_eq!(app.viewport.width, 78);
        app.handle_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
        assert!(app.panel);
        assert_eq!(app.focus, Focus::Rail);
    }

    #[test]
    fn hint_labels_remain_unique_alphabetic_on_crowded_pages() {
        let (root, mut app) = app();
        std::fs::write(root.path().join(".plan/first.md"), format!("## Many\n\n{}", "G-1 ".repeat(900))).unwrap();
        app.open(".plan/first.md");
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(240, 80)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
        assert_eq!(app.hints.len(), 900);
        let labels: HashSet<_> = app.hints.iter().map(|hint| &hint.label).collect();
        assert_eq!(labels.len(), 900);
        assert!(
            labels
                .iter()
                .all(|label| label.len() == 3 && label.bytes().all(|byte| byte.is_ascii_lowercase()))
        );
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.hints.is_empty());
        assert!(app.message.is_none());
    }
}
