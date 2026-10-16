use super::{mdast::convert_children, plugins::State};
use crate::{CodeTab, ComparisonOption, Component, ComponentBody, MarginNote, NodeDetail, Step};
use markdown::mdast::{AttributeContent, AttributeValue, Node as Md};

pub(crate) fn placeholder(name: &str, value: &str) -> Component {
    Component::Unknown {
        name: name.into(),
        blocks: vec![crate::Block::Paragraph(vec![crate::Inline::Text(value.into())])],
    }
}
pub(crate) fn literal_attributes(attrs: &[AttributeContent]) -> Vec<(&str, &str)> {
    attrs
        .iter()
        .filter_map(|attributes| {
            if let AttributeContent::Property(property) = attributes
                && let Some(AttributeValue::Literal(value)) = &property.value
            {
                Some((property.name.as_str(), value.as_str()))
            } else {
                None
            }
        })
        .collect()
}
pub(crate) fn value<'a>(attrs: &'a [(&str, &str)], name: &str) -> Option<&'a str> {
    attrs.iter().rev().find(|(key, _)| *key == name).map(|(_, value)| *value)
}
fn parts(node: &Md) -> Option<(&str, &[AttributeContent], &[Md])> {
    match node {
        Md::MdxJsxFlowElement(node) => Some((node.name.as_deref().unwrap_or("unknown"), &node.attributes, &node.children)),
        Md::MdxJsxTextElement(node) => Some((node.name.as_deref().unwrap_or("unknown"), &node.attributes, &node.children)),
        _ => None,
    }
}
fn groups<'a>(children: &'a [Md], name: &str) -> (Vec<&'a Md>, Vec<Md>) {
    let mut accepted = Vec::new();
    let mut remaining = Vec::new();
    for child in children {
        if parts(child).is_some_and(|(node, _, _)| node == name) {
            accepted.push(child);
        } else if let Md::Paragraph(property) = child
            && property
                .children
                .iter()
                .any(|node| parts(node).is_some_and(|(value, _, _)| value == name))
            && property.children.iter().all(|node| {
                parts(node).is_some_and(|(value, _, _)| value == name)
                    || matches!(node, Md::Text(text) if text.value.chars().all(char::is_whitespace))
            })
        {
            accepted.extend(
                property
                    .children
                    .iter()
                    .filter(|node| parts(node).is_some_and(|(value, _, _)| value == name)),
            );
        } else {
            remaining.push(child.clone());
        }
    }
    (accepted, remaining)
}
fn body(children: &[Md], state: &mut State, path: &str) -> ComponentBody {
    ComponentBody {
        inline: children.iter().all(|node| !super::mdast::is_block(node)),
        blocks: convert_children(children, state, path),
    }
}
pub(crate) fn component(name: &str, attrs: &[AttributeContent], children: &[Md], state: &mut State, path: &str) -> Component {
    let attrs = literal_attributes(attrs);
    match name {
        "CodeTabs" => {
            let (accepted, remaining) = groups(children, "CodeTab");
            let tabs = accepted
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let (_, attrs, children) = parts(node).unwrap();
                    let attrs = literal_attributes(attrs);
                    CodeTab {
                        label: value(&attrs, "label")
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("Tab {}", index + 1)),
                        body: body(children, state, path),
                    }
                })
                .collect();
            Component::CodeTabs {
                tabs,
                blocks: convert_children(&remaining, state, path),
            }
        }
        "Comparison" => {
            let (accepted, remaining) = groups(children, "Option");
            let options = accepted
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let (_, attrs, children) = parts(node).unwrap();
                    let attrs = literal_attributes(attrs);
                    ComparisonOption {
                        name: value(&attrs, "name")
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("Option {}", index + 1)),
                        body: body(children, state, path),
                    }
                })
                .collect();
            Component::Comparison {
                options,
                blocks: convert_children(&remaining, state, path),
            }
        }
        "DecisionMatrix" => Component::DecisionMatrix {
            score: value(&attrs, "score").unwrap_or("").into(),
            blocks: convert_children(children, state, path),
        },
        "AnnotatedDiff" => {
            let (accepted, remaining) = groups(children, "MarginNote");
            let notes = accepted
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let (_, attrs, children) = parts(node).unwrap();
                    let attrs = literal_attributes(attrs);
                    MarginNote {
                        line: value(&attrs, "line").map(str::to_owned).unwrap_or_else(|| (index + 1).to_string()),
                        body: body(children, state, path),
                    }
                })
                .collect();
            Component::AnnotatedDiff {
                blocks: convert_children(&remaining, state, path),
                notes,
            }
        }
        "Flowchart" => {
            let (accepted, remaining) = groups(children, "NodeDetail");
            let details = accepted
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let (_, attrs, children) = parts(node).unwrap();
                    let attrs = literal_attributes(attrs);
                    NodeDetail {
                        name: value(&attrs, "name")
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("Node {}", index + 1)),
                        body: body(children, state, path),
                    }
                })
                .collect();
            Component::Flowchart {
                source: remaining.iter().map(ToString::to_string).collect(),
                details,
            }
        }
        "StepRail" => {
            let (accepted, remaining) = groups(children, "Step");
            let steps = accepted
                .iter()
                .enumerate()
                .map(|(index, node)| {
                    let (_, attrs, children) = parts(node).unwrap();
                    let attrs = literal_attributes(attrs);
                    Step {
                        title: value(&attrs, "title")
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("Step {}", index + 1)),
                        body: body(children, state, path),
                    }
                })
                .collect();
            Component::StepRail {
                steps,
                blocks: convert_children(&remaining, state, path),
            }
        }
        _ => Component::Unknown {
            name: name.into(),
            blocks: convert_children(children, state, path),
        },
    }
}
