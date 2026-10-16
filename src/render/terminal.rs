use super::highlight;
use super::tree::{Element, Node, plain_text};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

pub use super::highlight::Theme;
use super::{Parsed, TocEntry};

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

pub fn to_terminal(parsed: &Parsed, width: u16, theme: Theme) -> TermDoc {
    let mut renderer = Renderer::new(usize::from(width.max(1)), theme);
    renderer.blocks(&parsed.children);
    let toc = parsed
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

pub fn status_color(status: &str) -> Color {
    match status {
        "accepted" | "converging" | "ready" => Color::Blue,
        "amended" | "draft" => Color::Yellow,
        "blocked" => Color::Red,
        "done" | "implemented" => Color::Green,
        "implementing" | "in-progress" | "review" => Color::Cyan,
        _ => Color::DarkGray,
    }
}

pub fn family_color(family: &str) -> Color {
    match family {
        "g" => Color::Cyan,
        "ng" => Color::Red,
        "pi" => Color::Magenta,
        "kd" => Color::Blue,
        "so" | "c" => Color::Yellow,
        "ct" => Color::LightBlue,
        "vc" => Color::Green,
        "oq" => Color::LightMagenta,
        "ex" => Color::LightCyan,
        _ => Color::DarkGray,
    }
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

fn inline(nodes: &[Node], style: Style, target: Option<Target>) -> Vec<Glyph> {
    nodes
        .iter()
        .flat_map(|node| match node {
            Node::Text(value) | Node::SourceText(value) | Node::ComponentText(value) => glyphs(value, style, target.clone()),
            Node::Raw(value) => glyphs(value, style.add_modifier(Modifier::DIM), None),
            Node::Bridge(children) => inline(children, style, target.clone()),
            Node::Element(element) => {
                let mut next_style = style;
                let mut next_target = target.clone();
                match element.tag.as_str() {
                    "strong" | "b" => next_style = next_style.add_modifier(Modifier::BOLD),
                    "em" | "i" => next_style = next_style.add_modifier(Modifier::ITALIC),
                    "del" => next_style = next_style.add_modifier(Modifier::CROSSED_OUT),
                    "code" => next_style = next_style.fg(Color::Yellow),
                    "a" => {
                        next_style = next_style.add_modifier(Modifier::UNDERLINED);
                        if let Some(path) = element.attr("data-doc") {
                            next_target = Some(Target::Doc(path.into()));
                        }
                    }
                    "button" if element.attr("class") == Some("xref-chip") => {
                        next_style = next_style.fg(family_color(element.attr("data-kind").unwrap_or("other")));
                        next_target = element.attr("data-ref").map(|reference| Target::Xref(reference.into()));
                    }
                    "br" => return glyphs("\n", style, None),
                    "input" => {
                        return glyphs(
                            if element.attrs.iter().any(|(name, _)| name == "checked") {
                                "[x] "
                            } else {
                                "[ ] "
                            },
                            style,
                            None,
                        );
                    }
                    "img" => return glyphs(element.attr("alt").unwrap_or(""), style, None),
                    _ => {}
                }
                inline(&element.children, next_style, next_target)
            }
        })
        .collect()
}

struct Renderer {
    width: usize,
    theme: Theme,
    document: TermDoc,
    anchors: Vec<(String, usize)>,
}

impl Renderer {
    fn new(width: usize, theme: Theme) -> Self {
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

    fn blocks(&mut self, nodes: &[Node]) {
        let mut pending = Vec::new();
        for node in nodes {
            if is_block(node) {
                if !pending.is_empty() {
                    self.paragraph(inline(&pending, Style::default(), None), "", "", false);
                    pending.clear();
                }
                self.block(node);
            } else if !matches!(node, Node::Text(value) if value.trim().is_empty()) {
                pending.push(node.clone());
            }
        }
        if !pending.is_empty() {
            self.paragraph(inline(&pending, Style::default(), None), "", "", false);
        }
    }

    fn embed(&mut self, child: Renderer, prefix: &str, suffix: &str) {
        let line_offset = self.document.lines.len();
        let column_offset = unicode_width::UnicodeWidthStr::width(prefix);
        for line in child.document.lines {
            let padding = child.width.saturating_sub(line.width());
            let mut spans = vec![Span::raw(prefix.to_string())];
            spans.extend(line.spans);
            if !suffix.is_empty() {
                spans.push(Span::raw(format!("{}{suffix}", " ".repeat(padding))));
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

    fn bordered(&mut self, label: &str, children: &[Node]) {
        if self.width < 5 {
            self.paragraph(glyphs(label, Style::default().add_modifier(Modifier::BOLD), None), "", "", false);
            self.blocks(children);
            return;
        }
        let label = textwrap::wrap(label, self.width - 4)
            .first()
            .map(|value| value.to_string())
            .unwrap_or_default();
        self.line(&glyphs(
            &format!(
                "┌─ {label}{}┐",
                "─".repeat(self.width - 4 - unicode_width::UnicodeWidthStr::width(label.as_str()))
            ),
            Style::default(),
            None,
        ));
        let mut child = Self::new(self.width - 4, self.theme);
        child.blocks(children);
        self.embed(child, "│ ", " │");
        self.line(&glyphs(&format!("└{}┘", "─".repeat(self.width - 2)), Style::default(), None));
        self.blank();
    }

    fn block(&mut self, node: &Node) {
        match node {
            Node::Bridge(children) => self.blocks(children),
            Node::Raw(value) => {
                self.paragraph(glyphs(value, Style::default().add_modifier(Modifier::DIM), None), "", "", true);
                self.blank();
            }
            Node::Element(element) => {
                let class = element.attr("class").unwrap_or("");
                if class == "md-unknown" {
                    self.bordered(
                        &format!("<{}>", element.attr("data-component").unwrap_or("unknown")),
                        &element.children,
                    );
                    return;
                }
                if class == "md-degraded" {
                    self.paragraph(
                        inline(
                            &element.children,
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                            None,
                        ),
                        "⚠ ",
                        "",
                        false,
                    );
                    self.blank();
                    return;
                }
                match element.tag.as_str() {
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        self.blank();
                        if let Some(id) = element.attr("id") {
                            self.anchors.push((id.into(), self.document.lines.len()));
                        }
                        let color = match element.tag.as_str() {
                            "h1" => Color::Magenta,
                            "h2" => Color::Blue,
                            "h3" => Color::Cyan,
                            "h4" => Color::Green,
                            "h5" => Color::Yellow,
                            _ => Color::Red,
                        };
                        self.paragraph(
                            inline(&element.children, Style::default().fg(color).add_modifier(Modifier::BOLD), None),
                            "",
                            "",
                            false,
                        );
                        self.blank();
                    }
                    "p" => {
                        if element.children.iter().any(is_block) {
                            self.blocks(&element.children);
                        } else {
                            self.paragraph(inline(&element.children, Style::default(), None), "", "", false);
                        }
                        self.blank();
                    }
                    "pre" => self.code(element),
                    "table" => self.table(element),
                    "ul" | "ol" => self.list(element),
                    "blockquote" => {
                        let prefix = if self.width > 2 { "│ " } else { "" };
                        let mut child = Self::new(self.width.saturating_sub(unicode_width::UnicodeWidthStr::width(prefix)), self.theme);
                        child.blocks(&element.children);
                        self.embed(child, prefix, "");
                        self.blank();
                    }
                    "hr" => {
                        self.line(&glyphs(&"─".repeat(self.width), Style::default().add_modifier(Modifier::DIM), None));
                        self.blank();
                    }
                    _ => self.blocks(&element.children),
                }
            }
            _ => self.paragraph(inline(std::slice::from_ref(node), Style::default(), None), "", "", false),
        }
    }

    fn list(&mut self, element: &Element) {
        let mut number = element.attr("start").and_then(|value| value.parse::<usize>().ok()).unwrap_or(1);
        for node in &element.children {
            if let Node::Element(item) = node
                && item.tag == "li"
            {
                let marker = if element.tag == "ol" {
                    format!("{number}. ")
                } else {
                    "• ".into()
                };
                let marker = if unicode_width::UnicodeWidthStr::width(marker.as_str()) < self.width {
                    marker
                } else {
                    String::new()
                };
                let marker_width = unicode_width::UnicodeWidthStr::width(marker.as_str());
                let mut child = Self::new(self.width.saturating_sub(marker_width), self.theme);
                if item.attr("class") == Some("md-margin-note") {
                    child.paragraph(
                        glyphs(
                            &format!("line {}", item.attr("data-line").unwrap_or("?")),
                            Style::default().fg(Color::Yellow),
                            None,
                        ),
                        "",
                        "",
                        false,
                    );
                }
                child.blocks(&item.children);
                if child.document.lines.is_empty() {
                    child.document.lines.push(Line::default());
                }
                let line_start = self.document.lines.len();
                let indentation = " ".repeat(marker_width);
                self.embed(child, &indentation, "");
                if let Some(line) = self.document.lines.get_mut(line_start) {
                    line.spans[0] = Span::raw(marker);
                }
                number += 1;
            }
        }
        self.blank();
    }

    fn code(&mut self, element: &Element) {
        let code = element.children.iter().find_map(|node| match node {
            Node::Element(code) if code.tag == "code" => Some(code),
            _ => None,
        });
        let language = if element.attr("class") == Some("mermaid") {
            "mermaid"
        } else {
            code.and_then(|code| code.attr("class"))
                .and_then(|class| class.strip_prefix("language-"))
                .unwrap_or("text")
        };
        self.paragraph(
            glyphs(
                &format!("── {language} ──"),
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
                None,
            ),
            "",
            "",
            false,
        );
        let source = element.children.iter().map(plain_text).collect::<String>();
        if let Some(lines) = highlight::tokens(&source, language, self.theme) {
            for tokens in lines {
                let values = tokens
                    .into_iter()
                    .flat_map(|token| {
                        let color = token.style.foreground;
                        let mut style = Style::default().fg(Color::Rgb(color.r, color.g, color.b));
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
                    .collect();
                self.paragraph(values, "", "", true);
            }
        } else {
            for line in source.split('\n') {
                self.paragraph(glyphs(line, Style::default(), None), "", "", true);
            }
        }
        self.blank();
    }

    fn table(&mut self, element: &Element) {
        fn rows<'a>(nodes: &'a [Node], found: &mut Vec<Vec<&'a Element>>) {
            for node in nodes {
                if let Node::Element(element) = node {
                    if element.tag == "tr" {
                        found.push(
                            element
                                .children
                                .iter()
                                .filter_map(|node| match node {
                                    Node::Element(cell) if matches!(cell.tag.as_str(), "th" | "td") => Some(cell),
                                    _ => None,
                                })
                                .collect(),
                        );
                    } else {
                        rows(&element.children, found);
                    }
                }
            }
        }
        let mut table_rows = Vec::new();
        rows(&element.children, &mut table_rows);
        let count = table_rows.iter().map(Vec::len).max().unwrap_or(0);
        if count == 0 {
            return;
        }
        if self.width < count * 4 + 1 {
            for row in table_rows {
                for cell in row {
                    if let Some(score) = cell.attr("data-score")
                        && !score.is_empty()
                    {
                        self.paragraph(
                            glyphs(&format!("Score: {score}"), Style::default().add_modifier(Modifier::BOLD), None),
                            "",
                            "",
                            false,
                        );
                    }
                    self.bordered("cell", &cell.children);
                }
            }
            return;
        }
        let available = self.width - (count * 3 + 1);
        let mut widths = vec![1; count];
        for row in &table_rows {
            for (index, cell) in row.iter().enumerate() {
                let value = cell.children.iter().map(plain_text).collect::<String>();
                widths[index] = widths[index].max(unicode_width::UnicodeWidthStr::width(value.as_str()).min(available));
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
        self.line(&glyphs(&border('┌', '┬', '┐'), Style::default(), None));
        for (row_index, row) in table_rows.iter().enumerate() {
            let cells: Vec<_> = widths
                .iter()
                .enumerate()
                .map(|(index, width)| {
                    let mut child = Self::new(*width, self.theme);
                    if let Some(cell) = row.get(index) {
                        if let Some(score) = cell.attr("data-score")
                            && !score.is_empty()
                        {
                            child.paragraph(
                                glyphs(&format!("Score: {score}"), Style::default().add_modifier(Modifier::BOLD), None),
                                "",
                                "",
                                false,
                            );
                        }
                        child.blocks(&cell.children);
                        if cell.tag == "th" {
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
                let mut spans = vec![Span::raw("│ ")];
                for (cell_index, cell) in cells.iter().enumerate() {
                    let content = cell.document.lines.get(inner_index).cloned().unwrap_or_default();
                    let padding = widths[cell_index].saturating_sub(content.width());
                    spans.extend(content.spans);
                    spans.push(Span::raw(format!(
                        "{} │{}",
                        " ".repeat(padding),
                        if cell_index + 1 < count { " " } else { "" }
                    )));
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
                self.line(&glyphs(&border('├', '┼', '┤'), Style::default(), None));
            }
        }
        self.line(&glyphs(&border('└', '┴', '┘'), Style::default(), None));
        self.blank();
    }
}

fn is_block(node: &Node) -> bool {
    match node {
        Node::Bridge(_) | Node::Raw(_) => true,
        Node::Element(element) => {
            element.attr("class") == Some("md-unknown")
                || matches!(
                    element.tag.as_str(),
                    "h1" | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "p"
                        | "pre"
                        | "ul"
                        | "ol"
                        | "table"
                        | "blockquote"
                        | "hr"
                        | "div"
                        | "section"
                        | "article"
                        | "figure"
                )
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::parse;

    #[test]
    fn headings_wrapping_and_target_coordinates() {
        let parsed = parse(
            "## Heading\n\nSee [the document](next.spec.md) and `[KD-3.1]`.\n",
            "spec/source.spec.md",
        );
        let rendered = to_terminal(&parsed, 18, Theme::Dark);
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
        let source = "- [x] finished task with a long label\n- [ ] pending\n  - nested G-1\n\n> Quoted paragraph with unicode 界.\n\n| Name | Value |\n| --- | --- |\n| 界 | [a long link](target.spec.md) |\n\n<FeatureViewer>\n\nRetained child G-2.\n\n</FeatureViewer>";
        let parsed = parse(source, "spec/narrow.spec.mdx");
        for width in 1..=30 {
            let document = to_terminal(&parsed, width, Theme::Dark);
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
        let document = to_terminal(&parsed, 50, Theme::Dark);
        let body = document.lines.iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
        assert!(body.contains("• [x] finished"), "{body}");
        assert!(body.contains("• [ ] pending"));
        assert!(body.contains("<FeatureViewer>"));
        assert!(body.contains("Retained child G-2."));
    }

    #[test]
    fn code_preserves_source_and_highlights_only_supported_languages() {
        let parsed = parse(
            "```ts\n  const value = 42\n```\n\n```brainfuck\n++[>++<-]>.\n```\n\n```mermaid\nflowchart TD\n  Source --> Render\n```",
            "spec/code.spec.md",
        );
        let document = to_terminal(&parsed, 60, Theme::Light);
        let source = document.lines.iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
        assert!(source.contains("── ts ──\n  const value = 42"));
        assert!(source.contains("── brainfuck ──\n++[>++<-]>."));
        assert!(source.contains("── mermaid ──\nflowchart TD\n  Source --> Render"));
        let highlighted = document.lines.iter().find(|line| line.to_string() == "  const value = 42").unwrap();
        assert!(
            highlighted
                .spans
                .iter()
                .any(|span| matches!(span.style.fg, Some(Color::Rgb(_, _, _))))
        );
        assert!(document.targets.is_empty());
    }

    #[test]
    fn dialect_components_retain_labels_scores_notes_and_step_numbers() {
        let parsed = parse(include_str!("../../../../fixtures/dialect.spec.mdx"), "spec/dialect.spec.mdx");
        let document = to_terminal(&parsed, 78, Theme::Dark);
        let body = document.lines.iter().map(Line::to_string).collect::<Vec<_>>().join("\n");
        for required in [
            "TypeScript",
            "JSON",
            "Keep source visible",
            "Enhance in browser",
            "Score: 4",
            "── diff ──",
            "1. line 1",
            "2. line 2",
            "Draft",
            "Accepted",
            "1. Author",
            "2. Render",
            "3. Review",
            "<FeatureViewer>",
            "── mermaid ──",
            "── brainfuck ──",
        ] {
            assert!(body.contains(required), "missing {required}: {body}");
        }
        assert!(document.lines.iter().all(|line| line.width() <= 78));
    }

    #[test]
    fn degraded_banner_raw_html_and_inline_styles() {
        let parsed = parse("## Fallback\n\n<StepRail>\n\nbody", "spec/broken.spec.mdx");
        let document = to_terminal(&parsed, 90, Theme::Dark);
        assert!(document.lines[0].to_string().contains("MDX parsing failed"));
        let parsed = parse(
            "## Heading\n\n**bold** *italic* ~~removed~~ [link](target.spec.md) `[AC-001]`\n\n<div>raw HTML</div>",
            "spec/source.spec.md",
        );
        let document = to_terminal(&parsed, 90, Theme::Dark);
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
        let document = to_terminal(&parsed, 20, Theme::Dark);
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
