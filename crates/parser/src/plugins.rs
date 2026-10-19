use crate::{Block, Component, Inline, TocEntry};
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct State {
    pub used: HashSet<String>,
    pub toc: Vec<TocEntry>,
}
fn strip_tags(value: &str) -> String {
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
pub(crate) fn inline_text(nodes: &[Inline]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            Inline::Text(value) | Inline::Code(value) => value.clone(),
            Inline::Strong(content) | Inline::Emphasis(content) | Inline::Delete(content) | Inline::Link { content, .. } => {
                inline_text(content)
            }
            Inline::Xref { id, .. } => id.clone(),
            Inline::Raw(value) => strip_tags(value),
            Inline::Break => "\n".into(),
            Inline::Image { .. } => String::new(),
            Inline::Component(component) => component_text(component),
        })
        .collect()
}
fn block_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(|block| match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => inline_text(content),
            Block::List { items, .. } => items.iter().map(|item| block_text(&item.blocks)).collect(),
            Block::Quote(blocks) => block_text(blocks),
            Block::Table { head, rows, .. } => head.iter().chain(rows.iter().flatten()).map(|cell| inline_text(cell)).collect(),
            Block::Code { source, .. } | Block::Mermaid { source } => source.clone(),
            Block::Raw(value) => strip_tags(value),
            Block::Component(component) => component_text(component),
            Block::Rule | Block::Degraded => String::new(),
        })
        .collect()
}
fn component_text(component: &Component) -> String {
    match component {
        Component::CodeTabs { tabs, blocks } => {
            tabs.iter()
                .map(|tab| format!("{}{}", tab.label, block_text(&tab.body.blocks)))
                .collect::<String>()
                + &block_text(blocks)
        }
        Component::Comparison { options, blocks } => {
            options
                .iter()
                .map(|option| format!("{}{}", option.name, block_text(&option.body.blocks)))
                .collect::<String>()
                + &block_text(blocks)
        }
        Component::DecisionMatrix { blocks, .. } | Component::Unknown { blocks, .. } => block_text(blocks),
        Component::AnnotatedDiff { blocks, notes } => {
            block_text(blocks) + &notes.iter().map(|note| block_text(&note.body.blocks)).collect::<String>()
        }
        Component::Flowchart { source, details } => {
            source.clone()
                + &details
                    .iter()
                    .map(|detail| format!("{}{}", detail.name, block_text(&detail.body.blocks)))
                    .collect::<String>()
        }
        Component::StepRail { steps, blocks } => {
            steps
                .iter()
                .map(|step| format!("{}{}", step.title, block_text(&step.body.blocks)))
                .collect::<String>()
                + &block_text(blocks)
        }
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
pub(crate) fn resolve(blocks: &mut [Block], state: &mut State) {
    // Component bodies were resolved independently while converting them.
    for block in blocks {
        match block {
            Block::Heading { depth, id, content } => {
                let text = inline_text(content);
                let base = slug(&text);
                *id = base.clone();
                let mut number = 2;
                while state.used.contains(id) {
                    *id = format!("{base}-{number}");
                    number += 1;
                }
                state.used.insert(id.clone());
                if *depth <= 3 {
                    state.toc.push(TocEntry {
                        id: id.clone(),
                        text,
                        depth: *depth,
                    });
                }
                xrefs(content);
            }
            Block::Paragraph(content) => xrefs(content),
            Block::List { items, .. } => {
                for item in items {
                    resolve(&mut item.blocks, state);
                }
            }
            Block::Quote(blocks) => resolve(blocks, state),
            Block::Table { head, rows, .. } => {
                for cell in head.iter_mut().chain(rows.iter_mut().flatten()) {
                    xrefs(cell);
                }
            }
            _ => {}
        }
    }
}
pub(crate) fn doc_link(href: &str, path: &str) -> Option<String> {
    if href.starts_with('/') {
        return None;
    }
    let target = href.split('#').next().unwrap_or("");
    if target.contains(['?', ':']) || !(target.ends_with(".md") || target.ends_with(".mdx")) {
        return None;
    }
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
    if escapes || resolved.is_empty() { None } else { Some(resolved) }
}
const KINDS: &[&str] = &["G", "NG", "PI", "KD", "SO", "CT", "VC", "C", "OQ", "EX"];
fn chip(id: &str, prefix: &str) -> Inline {
    Inline::Xref {
        id: id.into(),
        family: if KINDS.contains(&prefix) {
            prefix.to_ascii_lowercase()
        } else {
            "other".into()
        },
    }
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
fn chipify(value: &str) -> Vec<Inline> {
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
                let wrapped = index > 0 && bytes[index - 1] == b'[' && bytes.get(end) == Some(&b']');
                let (start, next) = if wrapped { (index - 1, end + 1) } else { (index, end) };
                if offset < start {
                    out.push(Inline::Text(value[offset..start].into()));
                }
                out.push(chip(&value[index..end], prefix));
                offset = next;
                index = next;
                continue;
            }
        }
        index += 1;
    }
    if offset < value.len() {
        out.push(Inline::Text(value[offset..].into()));
    }
    if out.is_empty() {
        out.push(Inline::Text(value.into()));
    }
    out
}
fn xrefs(nodes: &mut Vec<Inline>) {
    let mut out = Vec::new();
    for mut node in std::mem::take(nodes) {
        match &mut node {
            Inline::Text(value) => {
                out.extend(chipify(value));
                continue;
            }
            Inline::Code(value) => {
                if let Some((id, prefix)) = bracketed(value) {
                    out.push(chip(id, prefix));
                    continue;
                }
            }
            Inline::Strong(content) | Inline::Emphasis(content) | Inline::Delete(content) | Inline::Link { content, .. } => xrefs(content),
            _ => {}
        }
        out.push(node);
    }
    *nodes = out;
}
pub(crate) fn ordered_toc(entries: &[TocEntry], blocks: &[Block]) -> Vec<TocEntry> {
    fn inlines(nodes: &[Inline], entries: &[TocEntry], seen: &mut HashSet<String>, toc: &mut Vec<TocEntry>) {
        for node in nodes {
            match node {
                Inline::Strong(content) | Inline::Emphasis(content) | Inline::Delete(content) | Inline::Link { content, .. } => {
                    inlines(content, entries, seen, toc)
                }
                Inline::Component(value) => component(value, entries, seen, toc),
                _ => {}
            }
        }
    }
    fn component(value: &Component, entries: &[TocEntry], seen: &mut HashSet<String>, toc: &mut Vec<TocEntry>) {
        match value {
            Component::CodeTabs { tabs, blocks } => {
                for tab in tabs {
                    visit(&tab.body.blocks, entries, seen, toc);
                }
                visit(blocks, entries, seen, toc);
            }
            Component::Comparison { options, blocks } => {
                for option in options {
                    visit(&option.body.blocks, entries, seen, toc);
                }
                visit(blocks, entries, seen, toc);
            }
            Component::DecisionMatrix { blocks, .. } | Component::Unknown { blocks, .. } => visit(blocks, entries, seen, toc),
            Component::AnnotatedDiff { blocks, notes } => {
                visit(blocks, entries, seen, toc);
                for note in notes {
                    visit(&note.body.blocks, entries, seen, toc);
                }
            }
            Component::Flowchart { details, .. } => {
                for detail in details {
                    visit(&detail.body.blocks, entries, seen, toc);
                }
            }
            Component::StepRail { steps, blocks } => {
                for step in steps {
                    visit(&step.body.blocks, entries, seen, toc);
                }
                visit(blocks, entries, seen, toc);
            }
        }
    }
    fn visit(blocks: &[Block], entries: &[TocEntry], seen: &mut HashSet<String>, toc: &mut Vec<TocEntry>) {
        for block in blocks {
            match block {
                Block::Heading { id, content, .. } => {
                    if let Some(entry) = entries.iter().find(|entry| entry.id == *id)
                        && seen.insert(id.clone())
                    {
                        toc.push(entry.clone());
                    }
                    inlines(content, entries, seen, toc);
                }
                Block::Paragraph(content) => inlines(content, entries, seen, toc),
                Block::List { items, .. } => {
                    for item in items {
                        visit(&item.blocks, entries, seen, toc);
                    }
                }
                Block::Quote(blocks) => visit(blocks, entries, seen, toc),
                Block::Table { head, rows, .. } => {
                    for cell in head.iter().chain(rows.iter().flatten()) {
                        inlines(cell, entries, seen, toc);
                    }
                }
                Block::Component(value) => component(value, entries, seen, toc),
                _ => {}
            }
        }
    }
    let mut toc = Vec::new();
    visit(blocks, entries, &mut HashSet::new(), &mut toc);
    toc
}
