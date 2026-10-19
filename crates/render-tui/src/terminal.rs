use super::highlight;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use spec_viewer_core::{Block, Component, ComponentBody, Document, Inline, ListItem, TocEntry};
use unicode_width::UnicodeWidthChar;

pub use crate::Theme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Doc(String),
    Xref(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSpan {
    pub target: Target,
    pub line: usize,
    pub column: usize,
    pub length: usize,
}

#[derive(Clone, Debug, Default)]
pub struct TermDoc {
    pub lines: Vec<Line<'static>>,
    pub toc: Vec<(TocEntry, usize)>,
    pub targets: Vec<TargetSpan>,
}

pub fn to_terminal(document: &Document, width: u16, theme: &Theme) -> TermDoc {
    let mut renderer = Renderer::new(usize::from(width.max(1)), theme);
    renderer.blocks(&document.blocks);
    let toc = document
        .toc
        .iter()
        .filter_map(|entry| {
            renderer
                .anchors
                .iter()
                .find(|(id, _)| id == &entry.id)
                .map(|(_, line)| (entry.clone(), *line))
        })
        .collect();
    renderer.document.toc = toc;
    renderer.document
}

#[derive(Clone)]
struct Glyph {
    character: char,
    style: Style,
    target: Option<Target>,
}

fn glyphs(value: &str, style: Style, target: Option<Target>) -> Vec<Glyph> {
    value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .flat_map(|character| {
            let characters = if character == '\t' { vec![' '; 4] } else { vec![character] };
            characters
                .into_iter()
                .map(|character| Glyph {
                    character,
                    style,
                    target: target.clone(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn inline(theme: &Theme, nodes: &[Inline], style: Style, target: Option<Target>, compact: bool) -> Vec<Glyph> {
    nodes
        .iter()
        .flat_map(|node| match node {
            Inline::Text(value) => glyphs(value, style, target.clone()),
            Inline::Raw(value) => glyphs(value, style.add_modifier(Modifier::DIM), None),
            Inline::Strong(content) => inline(theme, content, style.add_modifier(Modifier::BOLD), target.clone(), compact),
            Inline::Emphasis(content) => inline(theme, content, style.add_modifier(Modifier::ITALIC), target.clone(), compact),
            Inline::Delete(content) => inline(theme, content, style.add_modifier(Modifier::CROSSED_OUT), target.clone(), compact),
            Inline::Code(value) => glyphs(value, style.fg(theme.roles.inline_code), target.clone()),
            Inline::Link { doc, content, .. } => inline(
                theme,
                content,
                style.add_modifier(Modifier::UNDERLINED),
                doc.as_ref().map(|path| Target::Doc(path.clone())).or_else(|| target.clone()),
                compact,
            ),
            Inline::Xref { id, family } => glyphs(id, style.fg(theme.family_color(family)), Some(Target::Xref(id.clone()))),
            Inline::Break => glyphs(if compact { "\n" } else { "\n\n" }, style, None),
            Inline::Image { alt, .. } => glyphs(alt, style, None),
            Inline::Component(component) => inline_component(theme, component, style, target.clone(), compact),
        })
        .collect()
}
fn inline_blocks(theme: &Theme, blocks: &[Block], style: Style, target: Option<Target>, compact: bool) -> Vec<Glyph> {
    blocks
        .iter()
        .flat_map(|block| match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => inline(theme, content, style, target.clone(), compact),
            Block::Quote(blocks) => inline_blocks(theme, blocks, style, target.clone(), compact),
            Block::List { items, .. } => items
                .iter()
                .flat_map(|item| inline_blocks(theme, &item.blocks, style, target.clone(), compact))
                .collect(),
            Block::Table { head, rows, .. } => head
                .iter()
                .chain(rows.iter().flatten())
                .flat_map(|cell| inline(theme, cell, style, target.clone(), compact))
                .collect(),
            Block::Code { source, .. } => glyphs(source, style.fg(theme.roles.inline_code), target.clone()),
            Block::Mermaid { source } => glyphs(source, style, target.clone()),
            Block::Raw(value) => glyphs(value, style.add_modifier(Modifier::DIM), None),
            Block::Component(component) => inline_component(theme, component, style, target.clone(), compact),
            Block::Rule | Block::Degraded => vec![],
        })
        .collect()
}
fn placeholder(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase) || matches!(name, "expression" | "module")
}

/// Inline-able nodes stay in the paragraph; other components and raw HTML break it.
fn breaks_prose(node: &Inline) -> bool {
    match node {
        Inline::Raw(_) => true,
        Inline::Component(component) => !matches!(&**component, Component::Unknown { name, .. } if placeholder(name)),
        _ => false,
    }
}

fn inline_component(theme: &Theme, component: &Component, style: Style, target: Option<Target>, compact: bool) -> Vec<Glyph> {
    let mut out = Vec::new();
    match component {
        Component::Unknown { name, blocks } if placeholder(name) => {
            let style = style.fg(theme.roles.warning);
            let (open, close) = if name == "expression" {
                ("{".to_string(), "}".to_string())
            } else {
                (format!("<{name}>"), format!("</{name}>"))
            };
            out.extend(glyphs(&open, style, None));
            out.extend(inline_blocks(theme, blocks, style, target, true));
            out.extend(glyphs(&close, style, None));
        }
        Component::CodeTabs { tabs, blocks } => {
            for tab in tabs {
                out.extend(glyphs(&tab.label, style, target.clone()));
                out.extend(inline_blocks(theme, &tab.body.blocks, style, target.clone(), compact));
            }
            out.extend(inline_blocks(theme, blocks, style, target, compact));
        }
        Component::Comparison { options, blocks } => {
            for option in options {
                out.extend(glyphs(&option.name, style, target.clone()));
                out.extend(inline_blocks(theme, &option.body.blocks, style, target.clone(), compact));
            }
            out.extend(inline_blocks(theme, blocks, style, target, compact));
        }
        Component::DecisionMatrix { blocks, .. } | Component::Unknown { blocks, .. } => {
            out.extend(inline_blocks(theme, blocks, style, target, compact))
        }
        Component::AnnotatedDiff { blocks, notes } => {
            out.extend(inline_blocks(theme, blocks, style, target.clone(), compact));
            for note in notes {
                out.extend(inline_blocks(theme, &note.body.blocks, style, target.clone(), compact));
            }
        }
        Component::Flowchart { source, details } => {
            out.extend(glyphs(source, style, target.clone()));
            for detail in details {
                out.extend(glyphs(&detail.name, style, target.clone()));
                out.extend(inline_blocks(theme, &detail.body.blocks, style, target.clone(), compact));
            }
        }
        Component::StepRail { steps, blocks } => {
            for step in steps {
                out.extend(glyphs(&step.title, style, target.clone()));
                out.extend(inline_blocks(theme, &step.body.blocks, style, target.clone(), compact));
            }
            out.extend(inline_blocks(theme, blocks, style, target, compact));
        }
    }
    out
}

struct Cell {
    blocks: Vec<Block>,
    title: Option<String>,
    heading: bool,
    compact: bool,
}

struct Renderer<'a> {
    width: usize,
    theme: &'a Theme,
    document: TermDoc,
    anchors: Vec<(String, usize)>,
}

impl<'a> Renderer<'a> {
    fn new(width: usize, theme: &'a Theme) -> Self {
        Self {
            width: width.max(1),
            theme,
            document: TermDoc::default(),
            anchors: Vec::new(),
        }
    }

    fn line(&mut self, values: &[Glyph]) {
        let line_index = self.document.lines.len();
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut column = 0;
        for glyph in values {
            let length = glyph.character.width().unwrap_or(0);
            if let Some(target) = &glyph.target {
                if let Some(previous) = self.document.targets.last_mut()
                    && previous.target == *target
                    && previous.line == line_index
                    && previous.column + previous.length == column
                {
                    previous.length += length;
                } else {
                    self.document.targets.push(TargetSpan {
                        target: target.clone(),
                        line: line_index,
                        column,
                        length,
                    });
                }
            }
            if let Some(span) = spans.last_mut()
                && span.style == glyph.style
            {
                span.content.to_mut().push(glyph.character);
            } else {
                spans.push(Span::styled(glyph.character.to_string(), glyph.style));
            }
            column += length;
        }
        self.document.lines.push(Line::from(spans));
    }

    fn blank(&mut self) {
        if self.document.lines.last().is_some_and(|line| line.width() > 0) {
            self.document.lines.push(Line::default());
        }
    }

    fn paragraph(&mut self, values: Vec<Glyph>, first: &str, following: &str, preserve: bool) {
        let mut offset = 0;
        let mut prefix = first;
        while offset < values.len() {
            if !preserve {
                while offset < values.len() && values[offset].character.is_whitespace() && values[offset].character != '\n' {
                    offset += 1;
                }
            }
            let mut row = glyphs(prefix, Style::default(), None);
            row.truncate(self.width.saturating_sub(1));
            let capacity = self.width.saturating_sub(row.len()).max(1);
            let mut end = offset;
            let mut used = 0;
            let mut last_space = None;
            while end < values.len() && values[end].character != '\n' {
                let length = values[end].character.width().unwrap_or(0);
                if used + length > capacity {
                    break;
                }
                used += length;
                if values[end].character.is_whitespace() {
                    last_space = Some(end);
                }
                end += 1;
            }
            if end == offset && end < values.len() && values[end].character != '\n' {
                // A wide glyph cannot fit a one-column viewport; replace it rather than overflowing.
                row.extend(glyphs("�", values[end].style, values[end].target.clone()));
                offset += 1;
            } else {
                if !preserve
                    && end < values.len()
                    && values[end].character != '\n'
                    && let Some(space) = last_space
                    && space > offset
                {
                    end = space;
                }
                let mut content_end = end;
                if !preserve {
                    while content_end > offset && values[content_end - 1].character.is_whitespace() {
                        content_end -= 1;
                    }
                }
                row.extend_from_slice(&values[offset..content_end]);
                offset = end;
                if offset < values.len() && values[offset].character == '\n' {
                    offset += 1;
                }
            }
            self.line(&row);
            prefix = following;
        }
        if values.is_empty() {
            self.line(&glyphs(first, Style::default(), None));
        }
    }

    fn blocks(&mut self, blocks: &[Block]) {
        self.blocks_context(blocks, false);
    }

    fn blocks_context(&mut self, blocks: &[Block], compact: bool) {
        for block in blocks {
            if let Block::Paragraph(content) = block {
                self.prose(content, compact);
            } else {
                self.block(block);
            }
        }
    }

    fn prose(&mut self, content: &[Inline], compact: bool) {
        if compact || content.iter().any(breaks_prose) {
            let mut pending = Vec::new();
            for node in content {
                if breaks_prose(node) {
                    if !pending.is_empty() {
                        self.paragraph(inline(self.theme, &pending, Style::default(), None, true), "", "", false);
                        pending.clear();
                    }
                    match node {
                        Inline::Component(component) => self.component(component, true),
                        Inline::Raw(value) => self.block(&Block::Raw(value.clone())),
                        _ => unreachable!(),
                    }
                } else if !matches!(node, Inline::Text(value) if !value.is_empty() && value.trim().is_empty()) {
                    pending.push(node.clone());
                }
            }
            if !pending.is_empty() {
                self.paragraph(inline(self.theme, &pending, Style::default(), None, true), "", "", false);
            }
        } else {
            self.paragraph(inline(self.theme, content, Style::default(), None, false), "", "", false);
        }
        if !compact {
            self.blank();
        }
    }

    fn embed(&mut self, child: Renderer<'_>, prefix: &str, suffix: &str, style: Style) {
        let line_offset = self.document.lines.len();
        let column_offset = unicode_width::UnicodeWidthStr::width(prefix);
        for line in child.document.lines {
            let padding = child.width.saturating_sub(line.width());
            let mut spans = vec![Span::styled(prefix.to_string(), style)];
            spans.extend(line.spans);
            if !suffix.is_empty() {
                spans.push(Span::raw(" ".repeat(padding)));
                spans.push(Span::styled(suffix.to_string(), style));
            }
            self.document.lines.push(Line::from(spans));
        }
        self.document.targets.extend(child.document.targets.into_iter().map(|mut target| {
            target.line += line_offset;
            target.column += column_offset;
            target
        }));
        self.anchors
            .extend(child.anchors.into_iter().map(|(id, line)| (id, line + line_offset)));
    }

    fn bordered(&mut self, label: &str, children: &[Block], compact: bool, border: Style) {
        if self.width < 6 {
            self.paragraph(glyphs(label, border.add_modifier(Modifier::BOLD), None), "", "", false);
            self.blocks_context(children, compact);
            return;
        }
        let label = wrap(label, self.width - 5).into_iter().next().unwrap_or_default();
        let mut top = glyphs("┌─ ", border, None);
        top.extend(glyphs(&label, border.add_modifier(Modifier::BOLD), None));
        top.extend(glyphs(
            &format!(
                " {}┐",
                "─".repeat(self.width - 5 - unicode_width::UnicodeWidthStr::width(label.as_str()))
            ),
            border,
            None,
        ));
        self.line(&top);
        let mut child = Self::new(self.width - 4, self.theme);
        child.blocks_context(children, compact);
        while child.document.lines.last().is_some_and(|line| line.width() == 0) {
            child.document.lines.pop();
        }
        self.embed(child, "│ ", " │", border);
        self.line(&glyphs(&format!("└{}┘", "─".repeat(self.width - 2)), border, None));
        self.blank();
    }

    /// Titled cards side by side, stacked as bordered blocks when the columns cannot fit.
    fn cards(&mut self, cards: &[(&str, &ComponentBody)]) {
        if cards.is_empty() {
            return;
        }
        if self.width > cards.len() * 24 {
            let row = cards
                .iter()
                .map(|(title, body)| Cell {
                    blocks: body.blocks.clone(),
                    title: Some((*title).to_string()),
                    heading: false,
                    compact: body.inline,
                })
                .collect();
            self.table(&[row]);
        } else {
            for (title, body) in cards {
                self.bordered(title, &body.blocks, body.inline, Style::default().fg(self.theme.roles.muted));
            }
        }
    }

    fn heading(&mut self, depth: u8, id: &str, content: &[Inline]) {
        self.blank();
        if !id.is_empty() {
            self.anchors.push((id.into(), self.document.lines.len()));
        }
        let color = self.theme.heading_color(depth);
        self.paragraph(
            inline(
                self.theme,
                content,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
                None,
                false,
            ),
            "",
            "",
            false,
        );
        self.blank();
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Heading { depth, id, content } => self.heading(*depth, id, content),
            Block::Paragraph(content) => self.prose(content, false),
            Block::Code { lang, source } => self.code(lang.as_deref().unwrap_or("text"), source),
            Block::Mermaid { source } => self.mermaid(source),
            Block::List {
                ordered,
                start,
                loose,
                items,
            } => self.list(*ordered, *start, *loose, items),
            Block::Table { head, rows, .. } => {
                let table_rows = std::iter::once(head)
                    .chain(rows)
                    .map(|row| {
                        row.iter()
                            .map(|content| Cell {
                                blocks: vec![Block::Paragraph(content.clone())],
                                title: None,
                                heading: std::ptr::eq(row, head),
                                compact: true,
                            })
                            .collect()
                    })
                    .collect::<Vec<_>>();
                self.table(&table_rows);
            }
            Block::Quote(blocks) => {
                let prefix = if self.width > 2 { "│ " } else { "" };
                let mut child = Self::new(self.width.saturating_sub(unicode_width::UnicodeWidthStr::width(prefix)), self.theme);
                child.blocks(blocks);
                self.embed(child, prefix, "", Style::default());
                self.blank();
            }
            Block::Rule => {
                self.line(&glyphs(&"─".repeat(self.width), Style::default().add_modifier(Modifier::DIM), None));
                self.blank();
            }
            Block::Raw(value) => {
                self.paragraph(glyphs(value, Style::default().add_modifier(Modifier::DIM), None), "", "", true);
                self.blank();
            }
            Block::Component(component) => self.component(component, false),
            Block::Degraded => {
                self.paragraph(
                    glyphs(
                        "MDX parsing failed; this document is shown as plain markdown.",
                        Style::default().fg(self.theme.roles.warning).add_modifier(Modifier::BOLD),
                        None,
                    ),
                    "⚠ ",
                    "",
                    false,
                );
                self.blank();
            }
        }
    }

    fn marker(&self, number: Option<usize>) -> String {
        let marker = number.map(|number| format!("{number}. ")).unwrap_or_else(|| "• ".into());
        if unicode_width::UnicodeWidthStr::width(marker.as_str()) < self.width {
            marker
        } else {
            String::new()
        }
    }

    fn list_child(&mut self, mut child: Renderer<'_>, marker: String) {
        if child.document.lines.is_empty() {
            child.document.lines.push(Line::default());
        }
        let line_start = self.document.lines.len();
        let indentation = " ".repeat(unicode_width::UnicodeWidthStr::width(marker.as_str()));
        self.embed(child, &indentation, "", Style::default());
        if let Some(line) = self.document.lines.get_mut(line_start) {
            line.spans[0] = Span::raw(marker);
        }
    }

    fn list(&mut self, ordered: bool, start: Option<u32>, loose: bool, items: &[ListItem]) {
        let start = start.unwrap_or(1) as usize;
        for (index, item) in items.iter().enumerate() {
            let marker = self.marker(ordered.then_some(start + index));
            let mut child = Self::new(
                self.width.saturating_sub(unicode_width::UnicodeWidthStr::width(marker.as_str())),
                self.theme,
            );
            let mut blocks = item.blocks.clone();
            if let Some(checked) = item.checked {
                if !matches!(blocks.first(), Some(Block::Paragraph(_))) {
                    blocks.insert(0, Block::Paragraph(vec![]));
                }
                if let Block::Paragraph(content) = &mut blocks[0] {
                    let spacing = if loose && !content.is_empty() { "  " } else { " " };
                    content.insert(0, Inline::Text(format!("[{}]{spacing}", if checked { "x" } else { " " })));
                }
            }
            child.blocks_context(&blocks, !loose);
            self.list_child(child, marker);
        }
        self.blank();
    }

    fn component(&mut self, component: &Component, compact: bool) {
        match component {
            Component::CodeTabs { tabs, blocks } => {
                for tab in tabs {
                    self.heading(3, "", &[Inline::Text(tab.label.clone())]);
                    self.blocks_context(&tab.body.blocks, tab.body.inline);
                }
                self.blocks_context(blocks, compact);
            }
            Component::Comparison { options, blocks } => {
                let cards: Vec<_> = options.iter().map(|option| (option.name.as_str(), &option.body)).collect();
                self.cards(&cards);
                self.blocks_context(blocks, compact);
            }
            Component::DecisionMatrix { score, blocks } => self.table(&[vec![Cell {
                blocks: blocks.clone(),
                title: (!score.is_empty()).then(|| format!("Score: {score}")),
                heading: false,
                compact,
            }]]),
            Component::AnnotatedDiff { blocks, notes } => {
                self.blocks_context(blocks, compact);
                for (index, note) in notes.iter().enumerate() {
                    let marker = self.marker(Some(index + 1));
                    let mut child = Self::new(
                        self.width.saturating_sub(unicode_width::UnicodeWidthStr::width(marker.as_str())),
                        self.theme,
                    );
                    child.paragraph(
                        glyphs(
                            &format!("line {}", note.line),
                            Style::default().fg(self.theme.roles.annotation),
                            None,
                        ),
                        "",
                        "",
                        false,
                    );
                    child.blocks_context(&note.body.blocks, note.body.inline);
                    self.list_child(child, marker);
                }
                self.blank();
            }
            Component::Flowchart { source, details } => {
                self.mermaid(source);
                let cards: Vec<_> = details.iter().map(|detail| (detail.name.as_str(), &detail.body)).collect();
                self.cards(&cards);
            }
            Component::StepRail { steps, .. } => {
                let rail = Style::default().fg(self.theme.roles.muted);
                let title = Style::default().add_modifier(Modifier::BOLD);
                for (index, step) in steps.iter().enumerate() {
                    if self.width < 4 {
                        self.paragraph(glyphs(&step.title, title, None), "", "", false);
                        self.blocks_context(&step.body.blocks, step.body.inline);
                        continue;
                    }
                    if index > 0 {
                        self.line(&glyphs("│", rail, None));
                    }
                    let mut child = Self::new(self.width - 2, self.theme);
                    child.paragraph(glyphs(&step.title, title, None), "", "", false);
                    child.blocks_context(&step.body.blocks, step.body.inline);
                    while child.document.lines.last().is_some_and(|line| line.width() == 0) {
                        child.document.lines.pop();
                    }
                    let start = self.document.lines.len();
                    self.embed(child, "│ ", "", rail);
                    self.document.lines[start].spans[0] = Span::styled("○ ", Style::default().fg(self.theme.roles.accent));
                }
                self.blank();
            }
            Component::Unknown { name, blocks } => {
                if placeholder(name) {
                    self.bordered(
                        &format!("Unsupported {name}"),
                        blocks,
                        compact || matches!(name.as_str(), "expression" | "module"),
                        Style::default().fg(self.theme.roles.warning),
                    );
                } else {
                    self.blocks_context(blocks, compact);
                }
            }
        }
    }

    fn mermaid(&mut self, source: &str) {
        let inner = self.width.saturating_sub(4);
        match mermaid_text::render_with_width(source, Some(inner)) {
            Ok(diagram)
                if flowchart_ids_valid(source) && diagram.lines().all(|line| unicode_width::UnicodeWidthStr::width(line) <= inner) =>
            {
                let lines = diagram.lines().map(|line| glyphs(line, Style::default(), None)).collect();
                self.framed("mermaid", lines);
            }
            _ => self.code("mermaid", source),
        }
    }

    fn code(&mut self, language: &str, source: &str) {
        let lines = if let Some(lines) = highlight::tokens(source, language) {
            lines
                .into_iter()
                .map(|tokens| {
                    tokens
                        .into_iter()
                        .flat_map(|token| {
                            let mut style = Style::default();
                            if let Some(slot) = highlight::slot(token.style.foreground) {
                                style = style.fg(self.theme.ansi[slot]);
                            }
                            if token.style.font_style.contains(syntect::highlighting::FontStyle::BOLD) {
                                style = style.add_modifier(Modifier::BOLD);
                            }
                            if token.style.font_style.contains(syntect::highlighting::FontStyle::ITALIC) {
                                style = style.add_modifier(Modifier::ITALIC);
                            }
                            if token.style.font_style.contains(syntect::highlighting::FontStyle::UNDERLINE) {
                                style = style.add_modifier(Modifier::UNDERLINED);
                            }
                            glyphs(&token.text, style, None)
                        })
                        .collect()
                })
                .collect()
        } else {
            source.split('\n').map(|line| glyphs(line, Style::default(), None)).collect()
        };
        self.framed(language, lines);
    }

    /// Boxes preformatted lines under a label on the `code_bg` role.
    fn framed(&mut self, label: &str, lines: Vec<Vec<Glyph>>) {
        let base = Style::default().bg(self.theme.roles.code_bg);
        let border = base.fg(self.theme.roles.muted);
        if self.width < 8 {
            self.paragraph(glyphs(label, border.add_modifier(Modifier::BOLD), None), "", "", false);
            for line in lines {
                self.paragraph(line, "", "", true);
            }
            self.blank();
            return;
        }
        let inner = self.width - 4;
        let label = wrap(label, inner.saturating_sub(2)).into_iter().next().unwrap_or_default();
        let mut top = glyphs("┌─ ", border, None);
        top.extend(glyphs(&label, border.add_modifier(Modifier::BOLD), None));
        top.extend(glyphs(
            &format!(
                " {}┐",
                "─".repeat(inner - unicode_width::UnicodeWidthStr::width(label.as_str()) - 1)
            ),
            border,
            None,
        ));
        self.line(&top);
        let mut lines = lines;
        while lines.last().is_some_and(Vec::is_empty) {
            lines.pop();
        }
        let mut child = Self::new(inner, self.theme);
        for line in lines {
            child.paragraph(line, "", "", true);
        }
        for line in child.document.lines {
            let padding = inner.saturating_sub(line.width());
            let mut row = glyphs("│ ", border, None);
            for span in line.spans {
                row.extend(glyphs(&span.content, base.patch(span.style), None));
            }
            row.extend(glyphs(&" ".repeat(padding), base, None));
            row.extend(glyphs(" │", border, None));
            self.line(&row);
        }
        self.line(&glyphs(&format!("└{}┘", "─".repeat(self.width - 2)), border, None));
        self.blank();
    }

    fn table(&mut self, table_rows: &[Vec<Cell>]) {
        let count = table_rows.iter().map(Vec::len).max().unwrap_or(0);
        if count == 0 {
            return;
        }
        if self.width < count * 4 + 1 {
            for row in table_rows {
                for cell in row {
                    if let Some(title) = &cell.title {
                        self.paragraph(glyphs(title, Style::default().add_modifier(Modifier::BOLD), None), "", "", false);
                    }
                    self.bordered("cell", &cell.blocks, cell.compact, Style::default().fg(self.theme.roles.muted));
                }
            }
            return;
        }
        let available = self.width - (count * 3 + 1);
        let mut widths = vec![1; count];
        for row in table_rows {
            for (index, cell) in row.iter().enumerate() {
                let value = plain_blocks(self.theme, &cell.blocks);
                let title = cell.title.as_deref().unwrap_or_default();
                let width = unicode_width::UnicodeWidthStr::width(value.as_str()).max(unicode_width::UnicodeWidthStr::width(title));
                widths[index] = widths[index].max(width.min(available));
            }
        }
        while widths.iter().sum::<usize>() > available {
            let index = widths.iter().enumerate().max_by_key(|(_, width)| **width).unwrap().0;
            widths[index] -= 1;
        }
        let border = |left: char, middle: char, right: char| {
            format!(
                "{left}{}{right}",
                widths
                    .iter()
                    .map(|width| "─".repeat(width + 2))
                    .collect::<Vec<_>>()
                    .join(&middle.to_string())
            )
        };
        let rule = Style::default().fg(self.theme.roles.muted);
        self.line(&glyphs(&border('┌', '┬', '┐'), rule, None));
        for (row_index, row) in table_rows.iter().enumerate() {
            let cells: Vec<_> = widths
                .iter()
                .enumerate()
                .map(|(index, width)| {
                    let mut child = Self::new(*width, self.theme);
                    if let Some(cell) = row.get(index) {
                        if let Some(title) = &cell.title {
                            child.paragraph(glyphs(title, Style::default().add_modifier(Modifier::BOLD), None), "", "", false);
                        }
                        child.blocks_context(&cell.blocks, cell.compact);
                        if cell.heading {
                            for line in &mut child.document.lines {
                                for span in &mut line.spans {
                                    span.style = span.style.add_modifier(Modifier::BOLD);
                                }
                            }
                        }
                    }
                    while child.document.lines.last().is_some_and(|line| line.width() == 0) {
                        child.document.lines.pop();
                    }
                    child
                })
                .collect();
            let height = cells.iter().map(|cell| cell.document.lines.len()).max().unwrap_or(1);
            let line_offset = self.document.lines.len();
            for inner_index in 0..height {
                let mut spans = vec![Span::styled("│", rule), Span::raw(" ")];
                for (cell_index, cell) in cells.iter().enumerate() {
                    let content = cell.document.lines.get(inner_index).cloned().unwrap_or_default();
                    let padding = widths[cell_index].saturating_sub(content.width());
                    spans.extend(content.spans);
                    spans.push(Span::raw(" ".repeat(padding + 1)));
                    spans.push(Span::styled("│", rule));
                    if cell_index + 1 < count {
                        spans.push(Span::raw(" "));
                    }
                }
                self.document.lines.push(Line::from(spans));
            }
            let mut column_offset = 2;
            for (cell_index, cell) in cells.into_iter().enumerate() {
                self.document.targets.extend(cell.document.targets.into_iter().map(|mut target| {
                    target.line += line_offset;
                    target.column += column_offset;
                    target
                }));
                self.anchors
                    .extend(cell.anchors.into_iter().map(|(id, line)| (id, line + line_offset)));
                column_offset += widths[cell_index] + 3;
            }
            if row_index + 1 < table_rows.len() {
                self.line(&glyphs(&border('├', '┼', '┤'), rule, None));
            }
        }
        self.line(&glyphs(&border('└', '┴', '┘'), rule, None));
        self.blank();
    }
}

/// mermaid-text keeps going on flowchart syntax errors, folding the bad text into node IDs.
fn flowchart_ids_valid(source: &str) -> bool {
    if !matches!(
        mermaid_text::detect::detect(source),
        Ok(mermaid_text::detect::DiagramKind::Flowchart)
    ) {
        return true;
    }
    mermaid_text::parser::parse(source).is_ok_and(|graph| {
        graph.nodes.iter().all(|node| {
            !node.id.is_empty()
                && node
                    .id
                    .chars()
                    .all(|character| !character.is_whitespace() && !"[](){}<>|\"".contains(character))
        })
    })
}

fn plain_inline(theme: &Theme, nodes: &[Inline]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            Inline::Text(value) | Inline::Code(value) => value.clone(),
            Inline::Strong(content) | Inline::Emphasis(content) | Inline::Delete(content) | Inline::Link { content, .. } => {
                plain_inline(theme, content)
            }
            Inline::Xref { id, .. } => id.clone(),
            Inline::Break => "\n".into(),
            Inline::Raw(_) | Inline::Image { .. } => String::new(),
            Inline::Component(component) => inline_component(theme, component, Style::default(), None, false)
                .iter()
                .map(|glyph| glyph.character)
                .collect(),
        })
        .collect()
}
fn plain_blocks(theme: &Theme, blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(content) | Block::Heading { content, .. } => plain_inline(theme, content),
            Block::Quote(blocks) => plain_blocks(theme, blocks),
            Block::Code { source, .. } | Block::Mermaid { source } => source.clone(),
            _ => String::new(),
        })
        .collect()
}

/// Greedy word wrap by display width; words wider than a line are split.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthStr;
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        let mut fresh = true;
        let mut rest = paragraph;
        loop {
            let gap = &rest[..rest.len() - rest.trim_start_matches(' ').len()];
            rest = &rest[gap.len()..];
            let (word, tail) = rest.split_at(rest.find(' ').unwrap_or(rest.len()));
            rest = tail;
            if word.is_empty() {
                break;
            }
            // A line break swallows the gap; the first gap is the paragraph's indentation.
            if !fresh && used + gap.width() + word.width() > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            } else {
                line.push_str(gap);
                used += gap.width();
            }
            for character in word.chars() {
                let length = character.width().unwrap_or(0);
                if used + length > width && used > 0 {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(character);
                used += length;
            }
            fresh = false;
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn wrap_breaks_at_spaces_and_splits_long_words() {
        assert_eq!(wrap("alpha beta  gamma", 11), ["alpha beta", "gamma"]);
        assert_eq!(wrap("  a.md  y path", 20), ["  a.md  y path"]);
        assert_eq!(wrap("ab abcdefgh", 4), ["ab", "abcd", "efgh"]);
        assert_eq!(wrap("一二三 x", 4), ["一二", "三 x"]);
        assert_eq!(wrap("one\ntwo ", 10), ["one", "two"]);
        assert_eq!(wrap("", 10), [""]);
    }

    fn parse(source: &str, path: &str) -> Document {
        spec_viewer_core::parse(source, path, &spec_viewer_config::ParseConfig::default())
    }

    #[test]
    fn literal_jsx_inputs_preserve_checked_markers() {
        for (attribute, marker) in [("checked", "[x]"), ("checked=\"checked\"", "[x]"), ("", "[ ]")] {
            for source in [
                format!("<input type=\"checkbox\" {attribute} />"),
                format!("before <input type=\"checkbox\" {attribute} /> after"),
            ] {
                let document = to_terminal(&parse(&source, "fixture.mdx"), 80, &Theme::default());
                let body = document.lines.iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
                assert!(body.contains(marker), "{source}: {body}");
            }
        }
    }

    #[test]
    fn literal_jsx_headings_and_blockquotes_preserve_terminal_styles() {
        for (depth, color) in [
            (1, Color::Magenta),
            (2, Color::Blue),
            (3, Color::Cyan),
            (4, Color::Green),
            (5, Color::Yellow),
            (6, Color::Red),
        ] {
            for source in [
                format!("<h{depth}>Heading</h{depth}>"),
                format!("before <h{depth}>Heading</h{depth}> after"),
            ] {
                let document = to_terminal(&parse(&source, "fixture.mdx"), 80, &Theme::default());
                let span = document
                    .lines
                    .iter()
                    .flat_map(|line| &line.spans)
                    .find(|span| span.content == "Heading")
                    .unwrap();
                assert_eq!(span.style.fg, Some(color), "{source}");
                assert!(span.style.add_modifier.contains(Modifier::BOLD), "{source}");
            }
        }
        for source in ["<blockquote>Quoted</blockquote>", "before <blockquote>Quoted</blockquote> after"] {
            let document = to_terminal(&parse(source, "fixture.mdx"), 80, &Theme::default());
            assert!(
                document.lines.iter().any(|line| line.to_string() == "│ Quoted"),
                "{source}: {:?}",
                document.lines
            );
        }
    }

    #[test]
    fn literal_jsx_code_preserves_nested_inline_formatting() {
        for source in [
            "<code>plain <strong>bold</strong> <em>italic</em> <del>removed</del></code>",
            "before <code>plain <strong>bold</strong> <em>italic</em> <del>removed</del></code> after",
        ] {
            let document = to_terminal(&parse(source, "fixture.mdx"), 80, &Theme::default());
            for (label, modifier) in [
                ("plain", Modifier::empty()),
                ("bold", Modifier::BOLD),
                ("italic", Modifier::ITALIC),
                ("removed", Modifier::CROSSED_OUT),
            ] {
                let span = document
                    .lines
                    .iter()
                    .flat_map(|line| &line.spans)
                    .find(|span| span.content.contains(label))
                    .unwrap();
                assert_eq!(span.style.fg, Some(Color::Yellow), "{source}: {label}");
                assert!(span.style.add_modifier.contains(modifier), "{source}: {label}");
            }
        }
    }

    #[test]
    fn headings_wrapping_and_target_coordinates() {
        let parsed = parse(
            "## Heading\n\nSee [the document](next.spec.md) and `[KD-3.1]`.\n",
            "spec/source.spec.md",
        );
        let rendered = to_terminal(&parsed, 18, &Theme::default());
        assert_eq!(rendered.toc[0].0.text, "Heading");
        assert_eq!(rendered.lines[rendered.toc[0].1].to_string(), "Heading");
        assert!(rendered.lines.iter().all(|line| line.width() <= 18));
        assert!(
            rendered
                .targets
                .iter()
                .any(|span| span.target == Target::Doc("spec/next.spec.md".into()))
        );
        let reference = rendered
            .targets
            .iter()
            .find(|span| span.target == Target::Xref("KD-3.1".into()))
            .unwrap();
        let line = rendered.lines[reference.line].to_string();
        assert_eq!(&line[reference.column..reference.column + reference.length], "KD-3.1");
    }

    #[test]
    fn nested_lists_tables_and_unknowns_fit_narrow_viewports() {
        let source = "- [x] finished task with a long label\n- [ ] pending\n  - nested G-1\n\n> Quoted paragraph with unicode 界.\n\n| Name | Value |\n| --- | --- |\n| 界 | [a long link](target.spec.md) |\n\n<FeatureViewer>\n\nRetained child G-2.\n\n</FeatureViewer>\n\n```ts\nconst wide = '界界界界界界界界界界界界'\n```\n\n```mermaid\ngraph LR; A[Build] --> B[Test] --> C[Deploy]\n```";
        let parsed = parse(source, "spec/narrow.spec.mdx");
        for width in 1..=30 {
            let document = to_terminal(&parsed, width, &Theme::default());
            assert!(
                document.lines.iter().all(|line| line.width() <= usize::from(width)),
                "width {width}: {:?}",
                document.lines
            );
            assert!(
                document
                    .targets
                    .iter()
                    .any(|span| span.target == Target::Doc("spec/target.spec.md".into()))
            );
            for target in &document.targets {
                assert!(target.column + target.length <= usize::from(width));
                assert!(target.line < document.lines.len());
            }
        }
        let document = to_terminal(&parsed, 50, &Theme::default());
        let body = document.lines.iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
        assert!(body.contains("• [x] finished"), "{body}");
        assert!(body.contains("• [ ] pending"));
        assert!(body.contains("Unsupported FeatureViewer"));
        assert!(body.contains("Retained child G-2."));
    }

    #[test]
    fn code_preserves_source_and_highlights_only_supported_languages() {
        let parsed = parse(
            "```ts\n  const value = 42\n```\n\n```brainfuck\n++[>++<-]>.\n```\n\n```mermaid\nflowchart TD\n  Source --> Render\n```\n\n```mermaid\nnotADiagram\n  Unknown --> Kind\n```\n\n```mermaid\nflowchart TD\n  Broken --> --> Syntax\n```",
            "spec/code.spec.md",
        );
        let theme = Theme::from_config(&spec_viewer_config::Config::default(), Some(spec_viewer_config::Mode::Light)).unwrap();
        let document = to_terminal(&parsed, 60, &theme);
        let source = document.lines.iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
        assert!(source.contains("┌─ ts ─"));
        assert!(source.contains("│   const value = 42 "));
        assert!(source.contains("┌─ brainfuck ─"));
        assert!(source.contains("│ ++[>++<-]>."));
        assert!(source.contains("┌─ mermaid ─"));
        assert!(source.contains("│ Render │"), "{source}");
        assert!(!source.contains("Source --> Render"));
        assert!(source.contains("│ notADiagram "), "{source}");
        assert!(source.contains("│   Broken --> --> Syntax "), "{source}");
        assert!(document.lines.iter().all(|line| line.width() == 0 || line.width() == 60));
        let highlighted = document.lines.iter().find(|line| line.to_string().contains("const value")).unwrap();
        assert!(
            highlighted
                .spans
                .iter()
                .all(|span| span.style.bg == Some(Color::Rgb(0xea, 0xea, 0xee)))
        );
        assert!(
            highlighted
                .spans
                .iter()
                .any(|span| span.style.fg.is_some_and(|color| theme.ansi.contains(&color)))
        );
        assert!(document.targets.is_empty());
    }

    #[test]
    fn dialect_components_retain_labels_scores_notes_and_steps() {
        let parsed = parse(include_str!("../../../fixtures/dialect.spec.mdx"), "spec/dialect.spec.mdx");
        let document = to_terminal(&parsed, 78, &Theme::default());
        let body = document.lines.iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
        for required in [
            "expression {someValue} so",
            "TypeScript\n\n┌─ ts ─",
            "JSON\n\n┌─ json ─",
            "│ Keep source visible ",
            "Score: 4",
            "┌─ diff ─",
            "1. line 1",
            "2. line 2",
            "Draft",
            "Accepted",
            "○ Author\n│ Write realistic",
            "○ Render",
            "○ Review",
            "┌─ Unsupported FeatureViewer ─",
            "┌─ mermaid ─",
            "│ Accepted │",
            "┌─ brainfuck ─",
        ] {
            assert!(body.contains(required), "missing {required}: {body}");
        }
        assert!(document.lines.iter().all(|line| line.width() <= 78));
        assert!(
            body.lines()
                .any(|line| line.contains("Keep source visible") && line.contains("Enhance in browser")),
            "{body}"
        );
        assert!(
            body.lines().any(|line| line.contains("│ Draft ") && line.contains("│ Accepted ")),
            "{body}"
        );
        let muted = Theme::default().roles.muted;
        for card in ["Keep source visible", "An author starts"] {
            let line = document.lines.iter().find(|line| line.to_string().contains(card)).unwrap();
            assert_eq!(line.spans[0].content, "│");
            assert_eq!(line.spans[0].style.fg, Some(muted), "{card}");
        }
    }

    #[test]
    fn degraded_banner_raw_html_and_inline_styles() {
        let parsed = parse("## Fallback\n\n<StepRail>\n\nbody", "spec/broken.spec.mdx");
        let document = to_terminal(&parsed, 90, &Theme::default());
        assert!(document.lines[0].to_string().contains("MDX parsing failed"));
        let parsed = parse(
            "## Heading\n\n**bold** *italic* ~~removed~~ [link](target.spec.md) `[AC-001]`\n\n<div>raw HTML</div>",
            "spec/source.spec.md",
        );
        let document = to_terminal(&parsed, 90, &Theme::default());
        let styles: Vec<_> = document.lines.iter().flat_map(|line| &line.spans).collect();
        for (label, modifier) in [
            ("bold", Modifier::BOLD),
            ("italic", Modifier::ITALIC),
            ("removed", Modifier::CROSSED_OUT),
            ("link", Modifier::UNDERLINED),
            ("<div>raw HTML</div>", Modifier::DIM),
        ] {
            assert!(
                styles
                    .iter()
                    .any(|span| span.content.contains(label) && span.style.add_modifier.contains(modifier)),
                "missing style for {label}"
            );
        }
        assert!(document.targets.iter().any(|span| span.target == Target::Xref("AC-001".into())));
    }

    #[test]
    fn table_targets_use_terminal_columns_after_wide_unicode_cells() {
        let parsed = parse("| 名 | Ref |\n| --- | --- |\n| 界 | G-1 |", "spec/table.spec.md");
        let document = to_terminal(&parsed, 20, &Theme::default());
        let target = document
            .targets
            .iter()
            .find(|span| span.target == Target::Xref("G-1".into()))
            .unwrap();
        assert_eq!(target.column, 7);
        assert_eq!(target.length, 3);
        assert_eq!(document.lines[target.line].to_string(), "│ 界 │ G-1 │");
    }
}
