use std::{
    collections::HashSet,
    io::{self, Write},
    path::Path,
};

use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};

use super::navigation::{self, Row};
use crate::terminal::{Target, TargetSpan, TermDoc, Theme, to_terminal};
use spec_viewer_core::{self as core, ChipValue, Docs, Library, Opened, Project};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Plans,
    Specs,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Rail,
    Document,
}

#[derive(Clone, Debug)]
pub struct Hint {
    pub label: String,
    pub span: TargetSpan,
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
    pub filter: String,
    pub diff: bool,
    pub expanded: HashSet<String>,
    pub focus: Focus,
    pub scroll: usize,
    pub quit: bool,
    pub rail_visible: bool,
    pub rendered: TermDoc,
    pub hints: Vec<Hint>,
    pub message: Option<String>,
    pub web_url: Option<String>,
    theme: Theme,
    sidebar_selection: usize,
    rail_selection: usize,
    filter_editing: bool,
    picker: Option<usize>,
    hint_input: Option<String>,
    viewport: Rect,
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
        Ok(Self {
            library,
            output: Box::new(output),
            project,
            worktree,
            tab: Tab::Plans,
            docs,
            doc: None,
            filter: String::new(),
            diff: false,
            expanded: HashSet::new(),
            focus: Focus::Sidebar,
            scroll: 0,
            quit: false,
            rail_visible: true,
            rendered: TermDoc::default(),
            hints: Vec::new(),
            message: None,
            web_url: None,
            theme,
            sidebar_selection: 0,
            rail_selection: 0,
            filter_editing: false,
            picker: None,
            hint_input: None,
            viewport: Rect::default(),
        })
    }

    pub fn worktree_path(&self) -> &Path {
        &self.project.worktrees[self.worktree].path
    }

    pub(super) fn watch(&self) -> io::Result<core::Watch> {
        self.library.watch(self.worktree_path()).map_err(io::Error::other)
    }

    fn listed(&self) -> &[core::DocNode] {
        if self.tab == Tab::Plans {
            &self.docs.plans
        } else {
            &self.docs.specs
        }
    }

    pub fn filtering(&self) -> bool {
        !self.filter.trim().is_empty() || (self.tab == Tab::Specs && self.diff && self.docs.changed_specs.is_some())
    }

    pub fn rows(&self) -> Vec<Row> {
        let needle = self.filter.trim().to_lowercase();
        let changed = if self.tab == Tab::Specs && self.diff {
            self.docs.changed_specs.as_deref()
        } else {
            None
        };
        navigation::rows(
            self.listed(),
            &self.expanded,
            self.doc.as_ref().map(|doc| doc.root_path.as_str()),
            &needle,
            changed,
        )
    }

    pub fn rail_entries(&self) -> Vec<RailEntry> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let all: Vec<_> = self.docs.plans.iter().chain(&self.docs.specs).cloned().collect();
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
                let all: Vec<_> = self.docs.plans.iter().chain(&self.docs.specs).cloned().collect();
                self.expanded.extend(navigation::ancestors(&all, &doc.root_path, ""));
                self.rendered = to_terminal(&doc.document, self.viewport.width.max(1), &self.theme);
                self.doc = Some(doc);
                self.scroll = 0;
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
        self.expanded.clear();
        if let Some(path) = self.doc.as_ref().map(|doc| doc.root_path.clone()) {
            let scroll = self.scroll;
            self.open(&path);
            self.scroll = scroll;
        }
    }

    fn clear_document(&mut self) {
        self.doc = None;
        self.rendered = TermDoc::default();
        self.scroll = 0;
        self.rail_selection = 0;
        self.sidebar_selection = 0;
        self.expanded.clear();
        self.message = None;
        self.cancel_hints();
    }

    fn cancel_hints(&mut self) {
        self.hints.clear();
        self.hint_input = None;
    }

    fn copy(&mut self, value: &str) {
        let sequence = format!("\x1b]52;c;{}\x07", STANDARD.encode(value));
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
            ("tab", if self.tab == Tab::Specs { "specs" } else { "plans" }.to_owned()),
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
            .targets
            .iter()
            .filter(|span| {
                span.line >= self.scroll
                    && span.line < self.scroll + usize::from(self.viewport.height)
                    && span.column < usize::from(self.viewport.width)
            })
            .cloned()
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
                }
                _ => {}
            }
            return;
        }
        if self.filter_editing {
            match key.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filter_editing = false;
                }
                KeyCode::Enter => self.filter_editing = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => self.filter.push(character),
                _ => {}
            }
            self.sidebar_selection = 0;
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('1') | KeyCode::Char('2') => {
                self.tab = if key.code == KeyCode::Char('1') { Tab::Plans } else { Tab::Specs };
                self.clear_document();
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
            KeyCode::Char('/') => {
                self.filter_editing = true;
                self.focus = Focus::Sidebar;
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.sidebar_selection = 0;
                self.message = None;
            }
            KeyCode::Char('d') if self.tab == Tab::Specs && self.docs.changed_specs.is_some() => {
                self.diff = !self.diff;
                self.sidebar_selection = 0;
            }
            KeyCode::Char('o') => self.open_browser(),
            KeyCode::Char('r') => {
                self.rail_visible = !self.rail_visible;
                if self.focus == Focus::Rail && !self.rail_visible {
                    self.focus = Focus::Document;
                }
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let reverse = key.code == KeyCode::BackTab || key.modifiers.contains(KeyModifiers::SHIFT);
                self.focus = match (self.focus, reverse, self.rail_visible) {
                    (Focus::Sidebar, false, true) | (Focus::Document, true, true) => Focus::Rail,
                    (Focus::Sidebar, false, false) | (Focus::Sidebar, true, _) | (Focus::Rail, false, _) => Focus::Document,
                    _ => Focus::Sidebar,
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
            KeyCode::Right | KeyCode::Char('l') => self.expand(true),
            KeyCode::Left | KeyCode::Char('h') => self.expand(false),
            KeyCode::Enter => self.activate(),
            _ => {}
        }
    }

    fn max_scroll(&self) -> usize {
        self.rendered.lines.len().saturating_sub(usize::from(self.viewport.height))
    }

    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta).min(self.max_scroll());
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
        let sidebar_width = (area.width / 4).clamp(18, 42).min(area.width);
        let rail_width = if self.rail_visible { (area.width / 6).clamp(12, 30) } else { 0 };
        let panes = Layout::horizontal([
            Constraint::Length(sidebar_width),
            Constraint::Length(rail_width),
            Constraint::Min(0),
        ])
        .split(vertical[0]);
        self.draw_sidebar(frame, panes[0]);
        self.draw_document(frame, panes[2]);
        if self.rail_visible {
            self.draw_rail(frame, panes[1]);
        }
        let footer = if let Some(input) = &self.hint_input {
            format!("Hint: {input}  Esc cancel")
        } else {
            self.message.clone().unwrap_or_else(|| {
                let keys = "Tab focus · 1/2 tabs · w worktree · / filter · d Diff · f links · y/Y copy · r rail · q quit";
                match &self.web_url {
                    Some(url) => format!("{keys} · o {url}"),
                    None => keys.into(),
                }
            })
        };
        frame.render_widget(
            Paragraph::new(footer).style(Style::default().fg(self.theme.roles.muted)),
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
    }

    fn pane(&self, title: &str, focused: bool) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .title(title.to_owned())
            .border_style(Style::default().fg(if focused { self.theme.roles.accent } else { self.theme.roles.muted }))
    }

    fn draw_sidebar(&self, frame: &mut Frame, area: Rect) {
        let block = self.pane(&format!("{} · Plans", self.project.name), self.focus == Focus::Sidebar);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let regions = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(inner);
        let worktree = &self.project.worktrees[self.worktree];
        let tabs = if self.tab == Tab::Plans { "[plans] specs" } else { "plans [specs]" };
        let diff = if self.tab != Tab::Specs {
            ""
        } else if self.docs.changed_specs.is_none() {
            " Diff unavailable"
        } else if self.diff {
            " [Diff]"
        } else {
            " Diff"
        };
        let heading = vec![
            Line::raw(format!("{}{} ▾", worktree.name, if worktree.main { " (main)" } else { "" })),
            Line::raw(format!("{tabs}{diff}")),
            Line::styled(
                format!("/ {}{}", self.filter, if self.filter_editing { "▏" } else { "" }),
                Style::default().fg(self.theme.roles.muted),
            ),
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
            frame.render_stateful_widget(
                List::new(items).highlight_style(if self.focus == Focus::Sidebar {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                }),
                regions[1],
                &mut ListState::default().with_selected(Some(self.sidebar_selection)),
            );
        }
    }

    fn draw_rail(&self, frame: &mut Frame, area: Rect) {
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
                    textwrap::wrap(&label, usize::from(inner.width.max(1)))
                        .into_iter()
                        .map(|line| Line::raw(line.into_owned()))
                        .collect::<Vec<_>>(),
                )
                .style(if current {
                    Style::default().fg(self.theme.roles.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                })
            })
            .collect();
        frame.render_stateful_widget(
            List::new(items).highlight_style(if self.focus == Focus::Rail {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            }),
            inner,
            &mut ListState::default().with_selected(Some(self.rail_selection)),
        );
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
        let mut header = vec![Line::from(vec![
            Span::styled(doc.document.meta.title.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                doc.document
                    .meta
                    .status
                    .as_ref()
                    .map(|status| format!(" [{}]", status.name))
                    .unwrap_or_default(),
                Style::default().fg(self.theme.status_color(doc.document.meta.status.as_ref())),
            ),
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
        let kind = navigation::find(&self.docs.plans, &doc.root_path)
            .or_else(|| navigation::find(&self.docs.specs, &doc.root_path))
            .map(|node| node.kind.as_str())
            .or(doc.document.meta.kind.as_deref())
            .unwrap_or(if doc.root_path.starts_with(".plan/") { "plan" } else { "spec" });
        let mut details = vec![kind.to_string()];
        if doc.root_path.starts_with(".plan/")
            && let Some(owner) = self.docs.plans.iter().find(|node| navigation::contains(node, &doc.root_path))
            && owner.path.as_deref() != Some(doc.root_path.as_str())
        {
            details.push(owner.name.clone());
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
        metadata.push(format!("{}  y path · Y name", doc.root_path));
        for value in metadata {
            for wrapped in textwrap::wrap(&value, usize::from(inner.width.max(1))) {
                header.push(Line::styled(wrapped.into_owned(), Style::default().fg(self.theme.roles.muted)));
            }
        }
        header.push(Line::raw("─".repeat(usize::from(inner.width))));
        let header = wrap_lines(header, inner.width);
        let regions = Layout::vertical([Constraint::Length(header.len() as u16), Constraint::Min(0)]).split(inner);
        frame.render_widget(Paragraph::new(header), regions[0]);
        if self.viewport.width != regions[1].width {
            self.rendered = to_terminal(&doc.document, regions[1].width.max(1), &self.theme);
            self.cancel_hints();
        }
        self.viewport = regions[1];
        self.scroll = self.scroll.min(self.max_scroll());
        frame.render_widget(
            Paragraph::new(Text::from(
                self.rendered
                    .lines
                    .iter()
                    .skip(self.scroll)
                    .take(usize::from(self.viewport.height))
                    .cloned()
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

fn meter(label: &str, done: usize, total: usize) -> String {
    let filled = done.saturating_mul(5).checked_div(total).unwrap_or(0).min(5);
    format!("{label} {}/{} {}{}  ", done, total, "▮".repeat(filled), "▯".repeat(5 - filled))
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

fn wrap_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthChar;
    let capacity = usize::from(width.max(1));
    let mut wrapped = Vec::new();
    for line in lines {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut used = 0;
        for span in line.spans {
            for character in span.content.chars() {
                let length = character.width().unwrap_or(0);
                if used + length > capacity && !spans.is_empty() {
                    wrapped.push(Line::from(std::mem::take(&mut spans)));
                    used = 0;
                }
                let character = if length > capacity { '�' } else { character };
                if let Some(previous) = spans.last_mut()
                    && previous.style == span.style
                {
                    previous.content.to_mut().push(character);
                } else {
                    spans.push(Span::styled(character.to_string(), span.style));
                }
                used += character.width().unwrap_or(0);
            }
        }
        wrapped.push(Line::from(spans));
    }
    wrapped
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

    fn app() -> (tempfile::TempDir, App) {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".plan")).unwrap();
        std::fs::write(root.path().join(".plan/first.md"), "## First\n\nG-1 [next](second.md)\n").unwrap();
        std::fs::write(root.path().join(".plan/second.md"), "## Second\n").unwrap();
        let app = App::new(
            Library::new(root.path(), core::ParseConfig::default()).unwrap(),
            Theme::default(),
            io::sink(),
        )
        .unwrap();
        (root, app)
    }

    #[test]
    fn modal_filter_input_and_key_release_do_not_trigger_global_shortcuts() {
        let (_root, mut app) = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        for character in "qdw12".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        assert_eq!(app.filter, "qdw12");
        assert!(!app.quit);
        assert_eq!(app.tab, Tab::Plans);
        assert!(!app.diff);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.filter.is_empty());
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
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
        assert_eq!(app.focus, Focus::Document);
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
