//! The mdast-util-to-hast handlers, with a structured component bridge.
use super::{
    components,
    plugins::{self, State},
    tree::{Node, el, text, wrap},
};
use markdown::mdast::{self as markdown_ast, Node as Md};
use std::collections::HashMap;

pub(crate) fn render_children(children: &[Md], state: &mut State, path: &str) -> Vec<Node> {
    let mut converter = Converter {
        state,
        path,
        definitions: HashMap::new(),
        footnotes: HashMap::new(),
        foot_order: Vec::new(),
        foot_counts: HashMap::new(),
    };
    converter.collect(children);
    let all = converter.all(children);
    let mut nodes = wrap(all, false);
    if let Some(footer) = converter.footer() {
        nodes.push(text("\n"));
        nodes.push(footer);
    }
    plugins::resolve(&mut nodes, state, path);
    nodes
}
struct Converter<'a> {
    state: &'a mut State,
    path: &'a str,
    definitions: HashMap<String, markdown_ast::Definition>,
    footnotes: HashMap<String, markdown_ast::FootnoteDefinition>,
    foot_order: Vec<String>,
    foot_counts: HashMap<String, usize>,
}
impl Converter<'_> {
    fn collect(&mut self, nodes: &[Md]) {
        for node in nodes {
            // Bun has already replaced component subtrees with raw HTML before collecting definitions.
            if matches!(node, Md::MdxJsxFlowElement(_) | Md::MdxJsxTextElement(_)) {
                continue;
            }
            match node {
                Md::Definition(definition) => {
                    self.definitions
                        .entry(definition.identifier.to_uppercase())
                        .or_insert_with(|| definition.clone());
                }
                Md::FootnoteDefinition(definition) => {
                    self.footnotes
                        .entry(definition.identifier.to_uppercase())
                        .or_insert_with(|| definition.clone());
                }
                _ => {}
            }
            if let Some(children) = node.children() {
                self.collect(children);
            }
        }
    }
    fn all(&mut self, children: &[Md]) -> Vec<Node> {
        let mut out = Vec::new();
        for (index, node) in children.iter().enumerate() {
            let mut nodes = self.one(node);
            if index > 0 && matches!(children[index - 1], Md::Break(_)) && nodes.len() == 1 {
                let first = match &mut nodes[0] {
                    Node::Text(value) => Some(value),
                    Node::Element(element) => match element.children.first_mut() {
                        Some(Node::Text(value)) => Some(value),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(first) = first {
                    *first = first.trim_start_matches([' ', '\t']).into();
                }
            }
            out.extend(nodes);
        }
        out
    }
    fn one(&mut self, node: &Md) -> Vec<Node> {
        let node = match node {
            Md::Root(markdown_node) => return wrap(self.all(&markdown_node.children), false),
            Md::Paragraph(markdown_node) => el("p", &[], self.all(&markdown_node.children)),
            Md::Heading(markdown_node) => el(&format!("h{}", markdown_node.depth), &[], self.all(&markdown_node.children)),
            Md::Blockquote(markdown_node) => el("blockquote", &[], wrap(self.all(&markdown_node.children), true)),
            Md::Emphasis(markdown_node) => el("em", &[], self.all(&markdown_node.children)),
            Md::Strong(markdown_node) => el("strong", &[], self.all(&markdown_node.children)),
            Md::Delete(markdown_node) => el("del", &[], self.all(&markdown_node.children)),
            Md::Text(markdown_node) => text(trim_lines(&markdown_node.value)),
            Md::Html(markdown_node) => Node::Raw(markdown_node.value.clone()),
            Md::InlineCode(markdown_node) => el(
                "code",
                &[],
                vec![text(markdown_node.value.replace("\r\n", " ").replace(['\r', '\n'], " "))],
            ),
            Md::Code(markdown_node) => {
                let value = if markdown_node.value.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", markdown_node.value)
                };
                let lang = markdown_node.lang.as_deref().and_then(|value| value.split_whitespace().next());
                let attrs = lang.map(|value| format!("language-{value}"));
                el(
                    "pre",
                    &[],
                    vec![el(
                        "code",
                        &attrs.as_ref().map(|value| vec![("class", value.as_str())]).unwrap_or_default(),
                        vec![text(value)],
                    )],
                )
            }
            Md::Break(_) => return vec![el("br", &[], vec![]), text("\n")],
            Md::ThematicBreak(_) => el("hr", &[], vec![]),
            Md::Link(markdown_node) => {
                let mut attrs = vec![("href".into(), Some(uri(&markdown_node.url)))];
                if let Some(title) = &markdown_node.title {
                    attrs.push(("title".into(), Some(title.clone())));
                }
                let mut link = el("a", &[], self.all(&markdown_node.children));
                if let Node::Element(element) = &mut link {
                    element.attrs = attrs;
                }
                link
            }
            Md::Image(markdown_node) => image(&markdown_node.url, &markdown_node.alt, markdown_node.title.as_deref()),
            Md::LinkReference(markdown_node) => {
                if let Some(definition) = self.definitions.get(&markdown_node.identifier.to_uppercase()).cloned() {
                    let mut link = el("a", &[("href", &uri(&definition.url))], self.all(&markdown_node.children));
                    if let Some(title) = definition.title
                        && let Node::Element(element) = &mut link
                    {
                        element.set("title", title);
                    }
                    link
                } else {
                    return self.revert(
                        node,
                        &markdown_node.children,
                        &markdown_node.reference_kind,
                        markdown_node.label.as_deref(),
                    );
                }
            }
            Md::ImageReference(markdown_node) => {
                if let Some(definition) = self.definitions.get(&markdown_node.identifier.to_uppercase()).cloned() {
                    image(&definition.url, &markdown_node.alt, definition.title.as_deref())
                } else {
                    return self.revert(node, &[], &markdown_node.reference_kind, markdown_node.label.as_deref());
                }
            }
            Md::List(markdown_node) => {
                let loose = markdown_node.spread
                    || markdown_node
                        .children
                        .iter()
                        .any(|markdown_node| matches!(markdown_node,Md::ListItem(index) if index.spread));
                let items = markdown_node
                    .children
                    .iter()
                    .flat_map(|markdown_node| {
                        if let Md::ListItem(index) = markdown_node {
                            vec![self.list_item(index, loose)]
                        } else {
                            self.one(markdown_node)
                        }
                    })
                    .collect::<Vec<_>>();
                let task = items
                    .iter()
                    .any(|markdown_node| matches!(markdown_node,Node::Element(element) if element.attr("class")==Some("task-list-item")));
                let mut list = el(if markdown_node.ordered { "ol" } else { "ul" }, &[], wrap(items, true));
                if let Node::Element(element) = &mut list {
                    if let Some(start) = markdown_node.start
                        && start != 1
                    {
                        element.set("start", start.to_string());
                    }
                    if task {
                        element.set("class", "contains-task-list");
                    }
                }
                list
            }
            Md::ListItem(markdown_node) => self.list_item(markdown_node, markdown_node.spread),
            Md::Table(markdown_node) => {
                let rows = markdown_node
                    .children
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let cells = row.children().map(Vec::as_slice).unwrap_or(&[]);
                        let cells = markdown_node
                            .align
                            .iter()
                            .enumerate()
                            .map(|(cell_index, align)| {
                                let align = match align {
                                    markdown_ast::AlignKind::Left => Some("left"),
                                    markdown_ast::AlignKind::Right => Some("right"),
                                    markdown_ast::AlignKind::Center => Some("center"),
                                    markdown_ast::AlignKind::None => None,
                                };
                                el(
                                    if index == 0 { "th" } else { "td" },
                                    &align.map(|value| vec![("align", value)]).unwrap_or_default(),
                                    cells
                                        .get(cell_index)
                                        .and_then(Md::children)
                                        .map(|children| self.all(children))
                                        .unwrap_or_default(),
                                )
                            })
                            .collect();
                        el("tr", &[], wrap(cells, true))
                    })
                    .collect::<Vec<_>>();
                let mut groups = Vec::new();
                let mut iter = rows.into_iter();
                if let Some(head) = iter.next() {
                    groups.push(el("thead", &[], wrap(vec![head], true)));
                }
                let body = iter.collect::<Vec<_>>();
                if !body.is_empty() {
                    groups.push(el("tbody", &[], wrap(body, true)));
                }
                el("table", &[], wrap(groups, true))
            }
            Md::TableRow(markdown_node) => el("tr", &[], wrap(self.all(&markdown_node.children), true)),
            Md::TableCell(markdown_node) => el("td", &[], self.all(&markdown_node.children)),
            Md::FootnoteReference(markdown_node) => {
                let id = markdown_node.identifier.to_uppercase();
                let safe = uri(&id.to_lowercase());
                if !self.foot_counts.contains_key(&id) {
                    self.foot_order.push(id.clone());
                }
                let index = self.foot_order.iter().position(|value| value == &id).unwrap() + 1;
                let count = self.foot_counts.entry(id).or_insert(0);
                *count += 1;
                let suffix = if *count > 1 { format!("-{count}") } else { String::new() };
                let mut anchor = el(
                    "a",
                    &[
                        ("href", &format!("#user-content-fn-{safe}")),
                        ("id", &format!("user-content-fnref-{safe}{suffix}")),
                    ],
                    vec![text(index.to_string())],
                );
                if let Node::Element(element) = &mut anchor {
                    element.attrs.push(("data-footnote-ref".into(), None));
                    element.set("aria-describedby", "footnote-label");
                }
                el("sup", &[], vec![anchor])
            }
            Md::MdxJsxFlowElement(markdown_node) => Node::Bridge(vec![components::component(
                markdown_node.name.as_deref(),
                &markdown_node.attributes,
                &markdown_node.children,
                false,
                self.state,
                self.path,
            )]),
            Md::MdxJsxTextElement(markdown_node) => Node::Bridge(vec![components::component(
                markdown_node.name.as_deref(),
                &markdown_node.attributes,
                &markdown_node.children,
                true,
                self.state,
                self.path,
            )]),
            Md::MdxFlowExpression(markdown_node) => components::placeholder("expression", &markdown_node.value),
            Md::MdxTextExpression(markdown_node) => components::placeholder("expression", &markdown_node.value),
            Md::MdxjsEsm(markdown_node) => components::placeholder("module", &markdown_node.value),
            Md::Definition(_) | Md::FootnoteDefinition(_) => return vec![],
            _ => text(node.to_string()),
        };
        vec![node]
    }
    fn revert(&mut self, node: &Md, children: &[Md], kind: &markdown_ast::ReferenceKind, label: Option<&str>) -> Vec<Node> {
        let image = matches!(node, Md::ImageReference(_));
        let mut result = vec![text(if image { "![" } else { "[" })];
        if let Md::ImageReference(markdown_node) = node {
            result.push(text(&markdown_node.alt));
        } else {
            result.extend(self.all(children));
        }
        let suffix = match kind {
            markdown_ast::ReferenceKind::Full => format!("][{}]", label.unwrap_or("")),
            markdown_ast::ReferenceKind::Collapsed => "][]".into(),
            markdown_ast::ReferenceKind::Shortcut => "]".into(),
        };
        result.push(text(suffix));
        result
    }
    fn list_item(&mut self, markdown_node: &markdown_ast::ListItem, loose: bool) -> Node {
        let mut results = self.all(&markdown_node.children);
        let mut attrs = Vec::new();
        if let Some(checked) = markdown_node.checked {
            if !matches!(results.first(),Some(Node::Element(element)) if element.tag=="p") {
                results.insert(0, el("p", &[], vec![]));
            }
            if let Node::Element(paragraph) = &mut results[0] {
                if !paragraph.children.is_empty() {
                    paragraph.children.insert(0, text(" "));
                }
                let mut input = el("input", &[("type", "checkbox")], vec![]);
                if let Node::Element(element) = &mut input {
                    if checked {
                        element.attrs.push(("checked".into(), None));
                    }
                    element.attrs.push(("disabled".into(), None));
                }
                paragraph.children.insert(0, input);
            }
            attrs.push(("class", "task-list-item"));
        }
        let tail_p = matches!(results.last(),Some(Node::Element(element)) if element.tag=="p");
        let nonempty = !results.is_empty();
        let mut children = Vec::new();
        for (index, child) in results.into_iter().enumerate() {
            let paragraph = matches!(&child,Node::Element(element) if element.tag=="p");
            if loose || index != 0 || !paragraph {
                children.push(text("\n"));
            }
            if !loose
                && paragraph
                && let Node::Element(element) = child
            {
                children.extend(element.children);
            } else {
                children.push(child);
            }
        }
        if nonempty && (loose || !tail_p) {
            children.push(text("\n"));
        }
        el("li", &attrs, children)
    }
    fn footer(&mut self) -> Option<Node> {
        let mut items = Vec::new();
        let mut index = 0;
        while index < self.foot_order.len() {
            let id = self.foot_order[index].clone();
            if let Some(definition) = self.footnotes.get(&id).cloned() {
                let mut content = self.all(&definition.children);
                let safe = uri(&id.to_lowercase());
                let mut back = Vec::new();
                for count in 1..=*self.foot_counts.get(&id).unwrap_or(&0) {
                    if !back.is_empty() {
                        back.push(text(" "));
                    }
                    let suffix = if count > 1 { format!("-{count}") } else { String::new() };
                    let mut children = vec![text("↩")];
                    if count > 1 {
                        children.push(el("sup", &[], vec![text(count.to_string())]));
                    }
                    back.push(el(
                        "a",
                        &[
                            ("href", &format!("#user-content-fnref-{safe}{suffix}")),
                            ("data-footnote-backref", ""),
                            ("aria-label", &format!("Back to reference {}{suffix}", index + 1)),
                            ("class", "data-footnote-backref"),
                        ],
                        children,
                    ));
                }
                if let Some(Node::Element(paragraph)) = content.last_mut()
                    && paragraph.tag == "p"
                {
                    if let Some(Node::Text(value)) = paragraph.children.last_mut() {
                        value.push(' ');
                    } else {
                        paragraph.children.push(text(" "));
                    }
                    paragraph.children.extend(back);
                } else {
                    content.extend(back);
                }
                items.push(el("li", &[("id", &format!("user-content-fn-{safe}"))], wrap(content, true)));
            }
            index += 1;
        }
        if items.is_empty() {
            None
        } else {
            let mut section = el(
                "section",
                &[],
                vec![
                    el("h2", &[("class", "sr-only"), ("id", "footnote-label")], vec![text("Footnotes")]),
                    text("\n"),
                    el("ol", &[], wrap(items, true)),
                    text("\n"),
                ],
            );
            if let Node::Element(element) = &mut section {
                element.attrs = vec![("data-footnotes".into(), None), ("class".into(), Some("footnotes".into()))];
            }
            Some(section)
        }
    }
}
fn image(url: &str, alt: &str, title: Option<&str>) -> Node {
    let mut image = el("img", &[("src", &uri(url)), ("alt", alt)], vec![]);
    if let Some(title) = title
        && let Node::Element(element) = &mut image
    {
        element.set("title", title);
    }
    image
}
pub(crate) fn uri(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' && index + 2 < bytes.len() && bytes[index + 1].is_ascii_alphanumeric() && bytes[index + 2].is_ascii_alphanumeric() {
            out.push_str(&value[index..index + 3]);
            index += 3;
            continue;
        }
        if byte == b'!'
            || byte == b'#'
            || byte == b'$'
            || (b'&'..=b';').contains(&byte)
            || byte == b'='
            || (b'?'..=b'Z').contains(&byte)
            || byte == b'_'
            || byte.is_ascii_lowercase()
            || byte == b'~'
        {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
        index += 1;
    }
    out
}
fn trim_lines(value: &str) -> String {
    let mut result = String::new();
    let mut rest = value;
    let mut first = true;
    while let Some(index) = rest.find(['\r', '\n']) {
        let line = &rest[..index];
        let line = if first { line } else { line.trim_start_matches([' ', '\t']) };
        result.push_str(line.trim_end_matches([' ', '\t']));
        let length = if rest[index..].starts_with("\r\n") { 2 } else { 1 };
        result.push_str(&rest[index..index + length]);
        rest = &rest[index + length..];
        first = false;
    }
    result.push_str(if first { rest } else { rest.trim_start_matches([' ', '\t']) });
    result
}
