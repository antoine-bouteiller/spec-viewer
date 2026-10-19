//! Semantic document to HTML, with the element structure and class names the web client styles.
use spec_viewer_core::{Align, Block, Component, ComponentBody, Document, Inline, ListItem};

use crate::highlight;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Html {
    pub html: String,
    pub has_mermaid: bool,
}

pub fn to_html(document: &Document) -> Html {
    let mut writer = Writer::default();
    writer.blocks(&document.blocks, false);
    Html {
        html: writer.out,
        has_mermaid: writer.has_mermaid,
    }
}

pub(crate) fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(character),
        }
    }
    out
}

fn placeholder(name: &str) -> bool {
    name.as_bytes().first().is_some_and(u8::is_ascii_uppercase) || matches!(name, "expression" | "module")
}

#[derive(Default)]
struct Writer {
    out: String,
    has_mermaid: bool,
}

impl Writer {
    /// `tight` renders paragraphs as bare inline content, as in tight list items and inline component bodies.
    fn blocks(&mut self, blocks: &[Block], tight: bool) {
        for block in blocks {
            match block {
                Block::Paragraph(content) if tight => self.inlines(content),
                block => self.block(block),
            }
        }
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Heading { depth, id, content } => {
                self.out.push_str(&format!("<h{depth} id=\"{}\">", escape(id)));
                self.inlines(content);
                self.out.push_str(&format!("</h{depth}>"));
            }
            Block::Paragraph(content) => {
                self.out.push_str("<p>");
                self.inlines(content);
                self.out.push_str("</p>");
            }
            Block::List {
                ordered,
                start,
                loose,
                items,
            } => self.list(*ordered, *start, *loose, items),
            Block::Quote(blocks) => {
                self.out.push_str("<blockquote>");
                self.blocks(blocks, false);
                self.out.push_str("</blockquote>");
            }
            Block::Table { align, head, rows } => {
                self.out.push_str("<table><thead><tr>");
                for (index, cell) in head.iter().enumerate() {
                    self.cell("th", align.get(index), cell);
                }
                self.out.push_str("</tr></thead>");
                if !rows.is_empty() {
                    self.out.push_str("<tbody>");
                    for row in rows {
                        self.out.push_str("<tr>");
                        for (index, cell) in row.iter().enumerate() {
                            self.cell("td", align.get(index), cell);
                        }
                        self.out.push_str("</tr>");
                    }
                    self.out.push_str("</tbody>");
                }
                self.out.push_str("</table>");
            }
            Block::Code { lang, source } => self.out.push_str(&highlight::code(lang.as_deref(), source)),
            Block::Mermaid { source } => self.mermaid(source),
            Block::Rule => self.out.push_str("<hr>"),
            Block::Raw(value) => self.out.push_str(value),
            Block::Component(component) => self.component(component, false),
            Block::Degraded => self
                .out
                .push_str("<div class=\"md-degraded\">MDX parsing failed; this document is shown as plain markdown.</div>"),
        }
    }

    fn list(&mut self, ordered: bool, start: Option<u32>, loose: bool, items: &[ListItem]) {
        let tag = if ordered { "ol" } else { "ul" };
        let task = items.iter().any(|item| item.checked.is_some());
        self.out.push('<');
        self.out.push_str(tag);
        if task {
            self.out.push_str(" class=\"contains-task-list\"");
        }
        if let Some(start) = start.filter(|start| ordered && *start != 1) {
            self.out.push_str(&format!(" start=\"{start}\""));
        }
        self.out.push('>');
        for item in items {
            match item.checked {
                Some(checked) => {
                    self.out.push_str("<li class=\"task-list-item\"><input type=\"checkbox\" disabled");
                    if checked {
                        self.out.push_str(" checked");
                    }
                    self.out.push_str("> ");
                }
                None => self.out.push_str("<li>"),
            }
            self.blocks(&item.blocks, !loose);
            self.out.push_str("</li>");
        }
        self.out.push_str(&format!("</{tag}>"));
    }

    fn cell(&mut self, tag: &str, align: Option<&Align>, content: &[Inline]) {
        let style = match align {
            Some(Align::Left) => " style=\"text-align:left\"",
            Some(Align::Center) => " style=\"text-align:center\"",
            Some(Align::Right) => " style=\"text-align:right\"",
            _ => "",
        };
        self.out.push_str(&format!("<{tag}{style}>"));
        self.inlines(content);
        self.out.push_str(&format!("</{tag}>"));
    }

    fn mermaid(&mut self, source: &str) {
        self.has_mermaid = true;
        self.out.push_str(&format!("<pre class=\"mermaid\">{}</pre>", escape(source)));
    }

    fn body(&mut self, body: &ComponentBody) {
        self.blocks(&body.blocks, body.inline);
    }

    fn titled(&mut self, tag: &str, class: &str, data: &str, title: &str, body: &ComponentBody) {
        let title = escape(title);
        self.out
            .push_str(&format!("<{tag} class=\"{class}\" data-{data}=\"{title}\"><h3>{title}</h3>"));
        self.body(body);
        self.out.push_str(&format!("</{tag}>"));
    }

    fn component(&mut self, component: &Component, inline: bool) {
        match component {
            Component::CodeTabs { tabs, blocks } => {
                self.out.push_str("<div class=\"md-codetabs-wrapper\">");
                for tab in tabs {
                    self.titled("section", "md-codetab-panel", "label", &tab.label, &tab.body);
                }
                self.blocks(blocks, false);
                self.out.push_str("</div>");
            }
            Component::Comparison { options, blocks } => {
                self.out
                    .push_str("<section class=\"md-comparison\"><div class=\"md-comparison-grid\">");
                for option in options {
                    self.titled("article", "md-comparison-option", "name", &option.name, &option.body);
                }
                self.blocks(blocks, false);
                self.out.push_str("</div></section>");
            }
            Component::DecisionMatrix { score, blocks } => {
                self.out.push_str(&format!(
                    "<table class=\"md-matrix\"><tbody><tr><td data-score=\"{}\">",
                    escape(score)
                ));
                self.blocks(blocks, false);
                self.out.push_str("</td></tr></tbody></table>");
            }
            Component::AnnotatedDiff { blocks, notes } => {
                self.out
                    .push_str("<figure class=\"md-annotated-diff\"><div class=\"md-annotated-diff-content\">");
                self.blocks(blocks, false);
                self.out.push_str("</div><ol class=\"md-margin-notes\">");
                for note in notes {
                    self.out
                        .push_str(&format!("<li class=\"md-margin-note\" data-line=\"{}\">", escape(&note.line)));
                    self.body(&note.body);
                    self.out.push_str("</li>");
                }
                self.out.push_str("</ol></figure>");
            }
            Component::Flowchart { source, details } => {
                self.out.push_str("<div class=\"md-flowchart\">");
                self.mermaid(source);
                self.out.push_str("<div class=\"md-node-details\">");
                for detail in details {
                    self.titled("section", "md-node-detail", "name", &detail.name, &detail.body);
                }
                self.out.push_str("</div></div>");
            }
            Component::StepRail { steps, blocks } => {
                self.out.push_str("<ol class=\"md-steprail\">");
                for step in steps {
                    self.titled("li", "md-step", "title", &step.title, &step.body);
                }
                self.blocks(blocks, false);
                self.out.push_str("</ol>");
            }
            Component::Unknown { name, blocks } if placeholder(name) => {
                let tag = if inline { "span" } else { "div" };
                self.out
                    .push_str(&format!("<{tag} class=\"md-unknown\" data-component=\"{}\">", escape(name)));
                self.blocks(blocks, inline);
                self.out.push_str(&format!("</{tag}>"));
            }
            // A literal lowercase JSX element is transparent, as in the terminal.
            Component::Unknown { blocks, .. } => self.blocks(blocks, inline),
        }
    }

    fn inlines(&mut self, content: &[Inline]) {
        for node in content {
            self.inline(node);
        }
    }

    fn inline(&mut self, node: &Inline) {
        match node {
            Inline::Text(value) => self.out.push_str(&escape(value)),
            Inline::Strong(content) => self.wrap("strong", content),
            Inline::Emphasis(content) => self.wrap("em", content),
            Inline::Delete(content) => self.wrap("del", content),
            Inline::Code(value) => self.out.push_str(&format!("<code>{}</code>", escape(value))),
            Inline::Link { href, doc, content } => {
                self.out.push_str(&format!("<a href=\"{}\"", escape(href)));
                if let Some(doc) = doc {
                    self.out.push_str(&format!(" data-doc=\"{}\"", escape(doc)));
                }
                self.out.push('>');
                self.inlines(content);
                self.out.push_str("</a>");
            }
            Inline::Xref { id, family } => {
                let id = escape(id);
                self.out.push_str(&format!(
                    "<button class=\"xref-chip\" data-kind=\"{}\" data-ref=\"{id}\" title=\"Copy {id}\" type=\"button\">{id}</button>",
                    escape(family)
                ));
            }
            Inline::Image { alt, src } => self.out.push_str(&format!("<img src=\"{}\" alt=\"{}\">", escape(src), escape(alt))),
            Inline::Break => self.out.push_str("<br>"),
            Inline::Raw(value) => self.out.push_str(value),
            Inline::Component(component) => self.component(component, true),
        }
    }

    fn wrap(&mut self, tag: &str, content: &[Inline]) {
        self.out.push_str(&format!("<{tag}>"));
        self.inlines(content);
        self.out.push_str(&format!("</{tag}>"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spec_viewer_core::{ParseConfig, parse};

    fn render(source: &str, path: &str) -> Html {
        to_html(&parse(source, path, &ParseConfig::default()))
    }

    #[test]
    fn markdown_constructs_keep_ids_doc_links_xrefs_and_escaping() {
        let html = render(
            "# Title\n\n## Why `List<T>`\n\nSee [other](../b.spec.md) and KD-2 `[VC-1]`.\n\n- [x] done\n- [ ] todo\n\n| a | b |\n|:-|-:|\n| 1 | 2 |\n",
            "spec/a.spec.md",
        )
        .html;
        assert!(html.contains(">Why <code>List&lt;T&gt;</code></h2>"), "{html}");
        assert!(html.contains("<a href=\"../b.spec.md\" data-doc=\"b.spec.md\">other</a>"), "{html}");
        assert!(html.contains("data-kind=\"kd\" data-ref=\"KD-2\""), "{html}");
        assert!(html.contains("data-kind=\"vc\" data-ref=\"VC-1\""), "{html}");
        assert!(
            html.contains("<li class=\"task-list-item\"><input type=\"checkbox\" disabled checked> done</li>"),
            "{html}"
        );
        assert!(
            html.contains("<th style=\"text-align:left\">a</th><th style=\"text-align:right\">b</th>"),
            "{html}"
        );
    }

    #[test]
    fn dialect_fixture_renders_components_mermaid_and_highlighting() {
        let source = include_str!("../../../fixtures/dialect.spec.mdx");
        let Html { html, has_mermaid } = render(source, "fixtures/dialect.spec.mdx");
        assert!(has_mermaid);
        for expected in [
            "<section class=\"md-codetab-panel\" data-label=\"TypeScript\"><h3>TypeScript</h3>",
            "<article class=\"md-comparison-option\" data-name=\"Keep source visible\">",
            "<td data-score=\"4\">",
            "<li class=\"md-margin-note\" data-line=\"1\">The removed value was only provisional.</li>",
            "<div class=\"md-flowchart\"><pre class=\"mermaid\">",
            "<li class=\"md-step\" data-title=\"Author\"><h3>Author</h3>",
            "<div class=\"md-unknown\" data-component=\"FeatureViewer\">",
            "<span class=\"md-unknown\" data-component=\"expression\">someValue</span>",
            "<pre class=\"mermaid\">flowchart TD\n  Broken --&gt; --&gt; Syntax",
            "<code class=\"language-brainfuck\">++[&gt;++&lt;-]&gt;.",
            "<div class=\"md-code\" data-lang=\"bash\"><pre class=\"hl\"><code class=\"language-bash\"><span class=\"hl-",
        ] {
            assert!(html.contains(expected), "missing {expected:?} in {html}");
        }
    }

    #[test]
    fn degraded_document_starts_with_the_banner() {
        let html = render("# T\n\n<Open>\n\nbody\n", "a.spec.mdx").html;
        assert!(html.starts_with("<div class=\"md-degraded\">"), "{html}");
    }
}
