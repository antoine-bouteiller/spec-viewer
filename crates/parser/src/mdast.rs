//! Convert markdown's syntax tree to renderer-independent blocks and inlines.
use super::{
    components,
    plugins::{self, State},
};
use crate::{Align, Block, Inline, ListItem};
use markdown::mdast::{self as markdown_ast, Node as Md};
use std::collections::HashMap;

pub(crate) fn convert_children(children: &[Md], state: &mut State, path: &str) -> Vec<Block> {
    let mut converter = Converter {
        state,
        path,
        definitions: HashMap::new(),
        footnotes: HashMap::new(),
        foot_order: Vec::new(),
        foot_counts: HashMap::new(),
    };
    converter.collect(children);
    let mut blocks = converter.blocks(children);
    blocks.extend(converter.footer());
    plugins::resolve(&mut blocks, converter.state);
    blocks
}
pub(crate) fn is_block(node: &Md) -> bool {
    matches!(
        node,
        Md::Paragraph(_)
            | Md::Heading(_)
            | Md::Blockquote(_)
            | Md::Code(_)
            | Md::List(_)
            | Md::Table(_)
            | Md::ThematicBreak(_)
            | Md::MdxJsxFlowElement(_)
            | Md::MdxFlowExpression(_)
            | Md::MdxjsEsm(_)
    )
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
            // Each component has its own reference-definition scope.
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
    fn blocks(&mut self, nodes: &[Md]) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut pending = Vec::new();
        for node in nodes {
            if matches!(node, Md::Definition(_) | Md::FootnoteDefinition(_)) {
                continue;
            }
            if is_block(node) || matches!(node, Md::Html(_)) {
                if !pending.is_empty() {
                    blocks.push(Block::Paragraph(std::mem::take(&mut pending)));
                }
                blocks.extend(self.block(node));
            } else {
                pending.extend(self.inlines(std::slice::from_ref(node)));
            }
        }
        if !pending.is_empty() {
            blocks.push(Block::Paragraph(pending));
        }
        blocks
    }
    fn block(&mut self, node: &Md) -> Vec<Block> {
        let block = match node {
            Md::Root(node) => return self.blocks(&node.children),
            Md::Paragraph(node) => Block::Paragraph(self.inlines(&node.children)),
            Md::Heading(node) => Block::Heading {
                depth: node.depth,
                id: String::new(),
                content: self.inlines(&node.children),
            },
            Md::Blockquote(node) => Block::Quote(self.blocks(&node.children)),
            Md::Html(node) => Block::Raw(node.value.clone()),
            Md::Code(node) => {
                let source = if node.value.is_empty() {
                    String::new()
                } else {
                    format!("{}\n", node.value)
                };
                let lang = node
                    .lang
                    .as_deref()
                    .and_then(|value| value.split_whitespace().next())
                    .map(str::to_owned);
                if matches!(lang.as_deref(), Some("mermaid" | "mmd")) {
                    Block::Mermaid { source }
                } else {
                    Block::Code { lang, source }
                }
            }
            Md::ThematicBreak(_) => Block::Rule,
            Md::List(node) => {
                let loose = node.spread || node.children.iter().any(|node| matches!(node, Md::ListItem(item) if item.spread));
                let items = node
                    .children
                    .iter()
                    .filter_map(|node| {
                        if let Md::ListItem(item) = node {
                            Some(ListItem {
                                checked: item.checked,
                                blocks: self.blocks(&item.children),
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                Block::List {
                    ordered: node.ordered,
                    start: node.start,
                    loose,
                    items,
                }
            }
            Md::Table(node) => {
                let mut rows = node
                    .children
                    .iter()
                    .map(|row| {
                        let cells = row.children().map(Vec::as_slice).unwrap_or(&[]);
                        (0..node.align.len())
                            .map(|index| {
                                cells
                                    .get(index)
                                    .and_then(Md::children)
                                    .map(|children| self.inlines(children))
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>();
                let head = if rows.is_empty() { vec![] } else { rows.remove(0) };
                let align = node
                    .align
                    .iter()
                    .map(|align| match align {
                        markdown_ast::AlignKind::None => Align::None,
                        markdown_ast::AlignKind::Left => Align::Left,
                        markdown_ast::AlignKind::Center => Align::Center,
                        markdown_ast::AlignKind::Right => Align::Right,
                    })
                    .collect();
                Block::Table { align, head, rows }
            }
            Md::MdxJsxFlowElement(node) => {
                let name = node.name.as_deref().unwrap_or("unknown");
                if name.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
                    Block::Component(components::component(name, &node.attributes, &node.children, self.state, self.path))
                } else {
                    return self.literal_blocks(name, &node.attributes, &node.children);
                }
            }
            Md::MdxFlowExpression(node) => Block::Component(components::placeholder("expression", &node.value)),
            Md::MdxjsEsm(node) => Block::Component(components::placeholder("module", &node.value)),
            _ => Block::Paragraph(self.inlines(std::slice::from_ref(node))),
        };
        vec![block]
    }
    fn inlines(&mut self, nodes: &[Md]) -> Vec<Inline> {
        let mut out = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            let mut converted = self.inline(node);
            if index > 0 && matches!(nodes[index - 1], Md::Break(_)) && converted.len() == 1 {
                let first = match &mut converted[0] {
                    Inline::Text(value) | Inline::Code(value) => Some(value),
                    Inline::Strong(children)
                    | Inline::Emphasis(children)
                    | Inline::Delete(children)
                    | Inline::Link { content: children, .. } => match children.first_mut() {
                        Some(Inline::Text(value)) => Some(value),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(first) = first {
                    *first = first.trim_start_matches([' ', '\t']).into();
                }
            }
            out.extend(converted);
        }
        out
    }
    fn inline(&mut self, node: &Md) -> Vec<Inline> {
        let node = match node {
            Md::Text(node) => Inline::Text(trim_lines(&node.value)),
            Md::Html(node) => Inline::Raw(node.value.clone()),
            Md::Emphasis(node) => Inline::Emphasis(self.inlines(&node.children)),
            Md::Strong(node) => Inline::Strong(self.inlines(&node.children)),
            Md::Delete(node) => Inline::Delete(self.inlines(&node.children)),
            Md::InlineCode(node) => Inline::Code(node.value.replace("\r\n", " ").replace(['\r', '\n'], " ")),
            Md::Break(_) => Inline::Break,
            Md::Link(node) => Inline::Link {
                href: uri(&node.url),
                doc: plugins::doc_link(&uri(&node.url), self.path),
                content: self.inlines(&node.children),
            },
            Md::Image(node) => Inline::Image {
                src: uri(&node.url),
                alt: node.alt.clone(),
            },
            Md::LinkReference(node) => {
                if let Some(definition) = self.definitions.get(&node.identifier.to_uppercase()).cloned() {
                    Inline::Link {
                        href: uri(&definition.url),
                        doc: plugins::doc_link(&uri(&definition.url), self.path),
                        content: self.inlines(&node.children),
                    }
                } else {
                    return self.revert(&node.children, &node.reference_kind, node.label.as_deref(), None);
                }
            }
            Md::ImageReference(node) => {
                if let Some(definition) = self.definitions.get(&node.identifier.to_uppercase()) {
                    Inline::Image {
                        src: uri(&definition.url),
                        alt: node.alt.clone(),
                    }
                } else {
                    return self.revert(&[], &node.reference_kind, node.label.as_deref(), Some(&node.alt));
                }
            }
            Md::FootnoteReference(node) => {
                let id = node.identifier.to_uppercase();
                if !self.foot_counts.contains_key(&id) {
                    self.foot_order.push(id.clone());
                }
                let index = self.foot_order.iter().position(|value| value == &id).unwrap() + 1;
                *self.foot_counts.entry(id).or_insert(0) += 1;
                Inline::Link {
                    href: String::new(),
                    doc: None,
                    content: vec![Inline::Text(index.to_string())],
                }
            }
            Md::MdxJsxTextElement(node) => {
                let name = node.name.as_deref().unwrap_or("unknown");
                if name.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
                    Inline::Component(Box::new(components::component(
                        name,
                        &node.attributes,
                        &node.children,
                        self.state,
                        self.path,
                    )))
                } else {
                    let mut blocks = if matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote") {
                        self.literal_blocks(name, &node.attributes, &node.children)
                    } else {
                        vec![Block::Paragraph(self.literal_inlines(name, &node.attributes, &node.children))]
                    };
                    plugins::resolve(&mut blocks, self.state);
                    Inline::Component(Box::new(crate::Component::Unknown { name: name.into(), blocks }))
                }
            }
            Md::MdxTextExpression(node) => Inline::Component(Box::new(components::placeholder("expression", &node.value))),
            Md::Definition(_) | Md::FootnoteDefinition(_) => return vec![],
            _ => Inline::Text(node.to_string()),
        };
        vec![node]
    }
    fn literal_inlines(&mut self, name: &str, attrs: &[markdown_ast::AttributeContent], children: &[Md]) -> Vec<Inline> {
        let checked = attrs
            .iter()
            .any(|attr| matches!(attr, markdown_ast::AttributeContent::Property(property) if property.name == "checked"));
        let attrs = components::literal_attributes(attrs);
        let content = self.inlines(children);
        let node = match name {
            "strong" | "b" => Inline::Strong(content),
            "em" | "i" => Inline::Emphasis(content),
            "del" => Inline::Delete(content),
            "code" => return code_inlines(content),
            // Literal JSX links aren't document links; only markdown links are resolved.
            "a" => Inline::Link {
                href: components::value(&attrs, "href").unwrap_or("").into(),
                doc: None,
                content,
            },
            "br" => Inline::Break,
            "img" => Inline::Image {
                alt: components::value(&attrs, "alt").unwrap_or("").into(),
                src: components::value(&attrs, "src").unwrap_or("").into(),
            },
            "input" => Inline::Text(if checked { "[x] " } else { "[ ] " }.into()),
            _ => return content,
        };
        vec![node]
    }
    fn literal_blocks(&mut self, name: &str, attrs: &[markdown_ast::AttributeContent], children: &[Md]) -> Vec<Block> {
        match name {
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => vec![Block::Heading {
                depth: name.as_bytes()[1] - b'0',
                id: String::new(),
                content: self.inlines(children),
            }],
            "blockquote" => vec![Block::Quote(convert_children(children, self.state, self.path))],
            "hr" => vec![Block::Rule],
            "input" | "code" | "strong" | "b" | "em" | "i" | "del" | "a" | "br" | "img" => {
                vec![Block::Paragraph(self.literal_inlines(name, attrs, children))]
            }
            _ => convert_children(children, self.state, self.path),
        }
    }
    fn revert(&mut self, children: &[Md], kind: &markdown_ast::ReferenceKind, label: Option<&str>, image: Option<&str>) -> Vec<Inline> {
        let mut result = vec![Inline::Text(if image.is_some() { "![" } else { "[" }.into())];
        if let Some(alt) = image {
            result.push(Inline::Text(alt.into()));
        } else {
            result.extend(self.inlines(children));
        }
        let suffix = match kind {
            markdown_ast::ReferenceKind::Full => format!("][{}]", label.unwrap_or("")),
            markdown_ast::ReferenceKind::Collapsed => "][]".into(),
            markdown_ast::ReferenceKind::Shortcut => "]".into(),
        };
        result.push(Inline::Text(suffix));
        result
    }
    fn footer(&mut self) -> Vec<Block> {
        let mut items = Vec::new();
        let mut index = 0;
        while index < self.foot_order.len() {
            let id = self.foot_order[index].clone();
            if let Some(definition) = self.footnotes.get(&id).cloned() {
                let mut blocks = self.blocks(&definition.children);
                let mut back = Vec::new();
                for count in 1..=*self.foot_counts.get(&id).unwrap_or(&0) {
                    if !back.is_empty() {
                        back.push(Inline::Text(" ".into()));
                    }
                    back.push(Inline::Link {
                        href: String::new(),
                        doc: None,
                        content: vec![Inline::Text(if count > 1 { format!("↩{count}") } else { "↩".into() })],
                    });
                }
                if let Some(Block::Paragraph(content)) = blocks.last_mut() {
                    content.push(Inline::Text(" ".into()));
                    content.extend(back);
                } else {
                    blocks.push(Block::Paragraph(back));
                }
                items.push(ListItem { checked: None, blocks });
            }
            index += 1;
        }
        if items.is_empty() {
            vec![]
        } else {
            vec![
                Block::Heading {
                    depth: 2,
                    id: String::new(),
                    content: vec![Inline::Text("Footnotes".into())],
                },
                Block::List {
                    ordered: true,
                    start: None,
                    loose: true,
                    items,
                },
            ]
        }
    }
}
// Preserve nested formatting while applying inline-code colour to literal JSX prose.
fn code_inlines(nodes: Vec<Inline>) -> Vec<Inline> {
    nodes
        .into_iter()
        .flat_map(|node| match node {
            Inline::Text(value) => vec![Inline::Code(value)],
            Inline::Strong(content) => vec![Inline::Strong(code_inlines(content))],
            Inline::Emphasis(content) => vec![Inline::Emphasis(code_inlines(content))],
            Inline::Delete(content) => vec![Inline::Delete(code_inlines(content))],
            Inline::Link { href, doc, content } => vec![Inline::Link {
                href,
                doc,
                content: code_inlines(content),
            }],
            Inline::Component(component) => match *component {
                crate::Component::Unknown { name, mut blocks } if name.as_bytes().first().is_some_and(u8::is_ascii_lowercase) => {
                    for block in &mut blocks {
                        if let Block::Paragraph(content) | Block::Heading { content, .. } = block {
                            *content = code_inlines(std::mem::take(content));
                        }
                    }
                    vec![Inline::Component(Box::new(crate::Component::Unknown { name, blocks }))]
                }
                component => vec![Inline::Component(Box::new(component))],
            },
            node => vec![node],
        })
        .collect()
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
