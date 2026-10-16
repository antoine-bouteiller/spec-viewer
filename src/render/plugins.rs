use super::{
    TocEntry, html,
    tree::{Node, el, plain_text, text},
};
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct State {
    pub used: HashSet<String>,
    pub toc: Vec<TocEntry>,
    pub has_mermaid: bool,
}
pub(crate) fn strip_tags(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find('>') {
            rest = &rest[start + end + 1..];
        } else {
            out.push_str(&rest[start..]);
            return out;
        }
    }
    out.push_str(rest);
    out
}
fn heading_text(node: &Node) -> String {
    match node {
        Node::Text(value) | Node::ComponentText(value) | Node::SourceText(value) => value.clone(),
        Node::Raw(value) => strip_tags(value),
        Node::Element(element) => element.children.iter().map(heading_text).collect(),
        Node::Bridge(nodes) => strip_tags(&html::serialize(nodes, false)),
    }
}
fn slug(value: &str) -> String {
    let value: String = strip_tags(value)
        .to_lowercase()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_' || *character == '-' || character.is_whitespace())
        .collect();
    let slug = value.split_whitespace().collect::<Vec<_>>().join("-");
    if slug.is_empty() { "section".into() } else { slug }
}
pub(crate) fn resolve(nodes: &mut Vec<Node>, state: &mut State, path: &str) {
    headings(nodes, state);
    doc_links(nodes, path);
    mermaid(nodes, state);
    xrefs(nodes, false);
}
fn headings(nodes: &mut [Node], state: &mut State) {
    for node in nodes {
        if let Node::Element(element) = node {
            if element.tag.len() == 2
                && element.tag.starts_with('h')
                && let Ok(depth @ 1..=6) = element.tag[1..].parse::<u8>()
            {
                let text = element.children.iter().map(heading_text).collect::<String>();
                let base = slug(&text);
                let mut id = base.clone();
                let mut number = 2;
                while state.used.contains(&id) {
                    id = format!("{base}-{number}");
                    number += 1;
                }
                state.used.insert(id.clone());
                element.set("id", id.clone());
                if depth <= 3 {
                    state.toc.push(TocEntry { id, text, depth });
                }
            }
            headings(&mut element.children, state);
        }
    }
}
fn doc_links(nodes: &mut [Node], path: &str) {
    for node in nodes {
        if let Node::Element(element) = node {
            if element.tag == "a"
                && let Some(href) = element.attr("href")
                && !href.starts_with('/')
            {
                let target = href.split('#').next().unwrap_or("");
                if !target.contains(['?', ':']) && (target.ends_with(".md") || target.ends_with(".mdx")) {
                    let mut segments = path.split('/').collect::<Vec<_>>();
                    segments.pop();
                    let mut escapes = false;
                    for part in target.split('/').filter(|value| !value.is_empty() && *value != ".") {
                        if part == ".." {
                            if segments.pop().is_none() {
                                escapes = true;
                            }
                        } else {
                            segments.push(part);
                        }
                    }
                    let resolved = segments.join("/");
                    if !escapes && !resolved.is_empty() {
                        element.set("data-doc", resolved);
                    }
                }
            }
            doc_links(&mut element.children, path);
        }
    }
}
fn mermaid(nodes: &mut [Node], state: &mut State) {
    for node in nodes {
        if let Node::Element(element) = node {
            if element.tag == "pre"
                && let Some(Node::Element(code)) = element
                    .children
                    .iter()
                    .find(|character| matches!(character,Node::Element(element) if element.tag=="code"))
                && code.attr("class").is_some_and(|character| {
                    character
                        .split_whitespace()
                        .any(|character| character == "language-mermaid" || character == "language-mmd")
                })
            {
                let value = code.children.iter().map(plain_text).collect::<String>();
                element.attrs = vec![("class".into(), Some("mermaid".into()))];
                element.children = vec![text(value)];
                state.has_mermaid = true;
            }
            mermaid(&mut element.children, state);
        }
    }
}
const KINDS: &[&str] = &["G", "NG", "PI", "KD", "SO", "CT", "VC", "C", "OQ", "EX"];
fn chip(id: &str, prefix: &str) -> Node {
    let kind = if KINDS.contains(&prefix) {
        prefix.to_ascii_lowercase()
    } else {
        "other".into()
    };
    el(
        "button",
        &[
            ("class", "xref-chip"),
            ("data-kind", &kind),
            ("data-ref", id),
            ("title", &format!("Copy {id}")),
            ("type", "button"),
        ],
        vec![text(id)],
    )
}
fn bracketed(value: &str) -> Option<(&str, &str)> {
    let id = value.strip_prefix('[')?.strip_suffix(']')?;
    let (prefix, number) = id.split_once('-')?;
    if prefix.is_empty() || !prefix.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return None;
    }
    let mut parts = number.split('.');
    let first = parts.next()?;
    if first.is_empty() || !first.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if let Some(second) = parts.next()
        && (second.is_empty() || !second.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    Some((id, prefix))
}
fn word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
fn chipify(value: &str) -> Vec<Node> {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut offset = 0;
    let mut index = 0;
    while index < bytes.len() {
        if index == 0 || !word(bytes[index - 1]) {
            let found = KINDS.iter().find_map(|prefix| {
                let rest = value.get(index..)?;
                let number = rest.strip_prefix(prefix)?.strip_prefix('-')?;
                let count = number.bytes().take_while(u8::is_ascii_digit).count();
                let end = index + prefix.len() + 1 + count;
                if (1..=4).contains(&count) && (end == bytes.len() || !word(bytes[end])) {
                    Some((end, *prefix))
                } else {
                    None
                }
            });
            if let Some((end, prefix)) = found {
                if offset < index {
                    out.push(text(&value[offset..index]));
                }
                out.push(chip(&value[index..end], prefix));
                offset = end;
                index = end;
                continue;
            }
        }
        index += 1;
    }
    if offset < value.len() {
        out.push(text(&value[offset..]));
    }
    if out.is_empty() {
        out.push(text(value));
    }
    out
}
fn xrefs(nodes: &mut Vec<Node>, protected: bool) {
    let mut out = Vec::new();
    for mut node in std::mem::take(nodes) {
        match &mut node {
            Node::Text(value) if !protected => {
                out.extend(chipify(value));
                continue;
            }
            Node::Element(element) => {
                if !protected
                    && element.tag == "code"
                    && let Some((id, prefix)) = bracketed(&element.children.iter().map(plain_text).collect::<String>())
                {
                    out.push(chip(id, prefix));
                    continue;
                }
                xrefs(&mut element.children, protected || element.tag == "pre" || element.tag == "code");
            }
            _ => {}
        }
        out.push(node);
    }
    *nodes = out;
}
