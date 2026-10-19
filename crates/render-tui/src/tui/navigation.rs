use std::collections::HashSet;

use spec_viewer_core::DocNode;

pub(super) fn children(node: &DocNode) -> &[DocNode] {
    node.children.as_deref().unwrap_or_default()
}

pub(super) fn family_root(node: &DocNode) -> bool {
    node.path.is_some() && !children(node).is_empty()
}

pub(super) fn find<'a>(nodes: &'a [DocNode], path: &str) -> Option<&'a DocNode> {
    for node in nodes {
        if node.path.as_deref() == Some(path) {
            return Some(node);
        }
        if let Some(found) = find(children(node), path) {
            return Some(found);
        }
    }
    None
}

pub(super) fn contains(node: &DocNode, path: &str) -> bool {
    node.path.as_deref() == Some(path) || children(node).iter().any(|child| contains(child, path))
}

pub(super) fn ancestors(nodes: &[DocNode], path: &str, parent: &str) -> Vec<String> {
    for node in nodes {
        if node.path.as_deref() == Some(path) {
            return Vec::new();
        }
        if contains(node, path) {
            let key = node.path.clone().unwrap_or_else(|| format!("{parent}/{}", node.name));
            let mut result = vec![key.clone()];
            result.extend(ancestors(children(node), path, &key));
            return result;
        }
    }
    Vec::new()
}

pub(super) fn family(nodes: &[DocNode], path: &str) -> Option<Vec<DocNode>> {
    fn members(nodes: &[DocNode], result: &mut Vec<DocNode>) {
        for node in nodes {
            if node.path.is_some() {
                result.push(node.clone());
            }
            if !family_root(node) {
                members(children(node), result);
            }
        }
    }
    for node in nodes {
        if let Some(nested) = family(children(node), path) {
            return Some(nested);
        }
        if family_root(node) && contains(node, path) {
            let mut result = vec![node.clone()];
            members(children(node), &mut result);
            return Some(result);
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct Row {
    pub node: DocNode,
    pub key: String,
    pub parent: Option<String>,
    pub depth: usize,
    pub current: bool,
}

pub(super) fn rows(
    nodes: &[DocNode],
    expanded: &HashSet<String>,
    active: Option<&str>,
    needle: &str,
    changed: Option<&[String]>,
) -> Vec<Row> {
    struct Context<'a> {
        expanded: &'a HashSet<String>,
        active: Option<&'a str>,
        needle: &'a str,
        changed: Option<&'a [String]>,
        flat: bool,
    }
    fn collect(nodes: &[DocNode], context: &Context<'_>, parent: &str, depth: usize, result: &mut Vec<Row>) {
        for node in nodes {
            let key = node.path.clone().unwrap_or_else(|| format!("{parent}/{}", node.name));
            let matches = node.path.as_ref().is_some_and(|path| {
                (context.needle.is_empty()
                    || node.name.to_lowercase().contains(context.needle)
                    || path.to_lowercase().contains(context.needle))
                    && context.changed.is_none_or(|changed| changed.contains(path))
            });
            if !context.flat || matches {
                result.push(Row {
                    node: node.clone(),
                    key: key.clone(),
                    parent: (!parent.is_empty() && !context.flat).then(|| parent.into()),
                    depth: if context.flat { 0 } else { depth },
                    current: context.active.is_some_and(|active| {
                        node.path.as_deref() == Some(active) || (!context.flat && family_root(node) && contains(node, active))
                    }),
                });
            }
            if context.flat {
                let descendants: Vec<_> = children(node)
                    .iter()
                    .filter(|child| !(node.path.is_some() && child.kind == "examples"))
                    .cloned()
                    .collect();
                collect(&descendants, context, &key, depth + 1, result);
            } else if !family_root(node) && context.expanded.contains(&key) {
                collect(children(node), context, &key, depth + 1, result);
            }
        }
    }
    let mut result = Vec::new();
    collect(
        nodes,
        &Context {
            expanded,
            active,
            needle,
            changed,
            flat: !needle.is_empty() || changed.is_some(),
        },
        "",
        0,
        &mut result,
    );
    result
}
