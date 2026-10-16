use super::{
    mdast::render_children,
    plugins::State,
    tree::{Node, el},
};
use markdown::mdast::{AttributeContent, AttributeValue, Node as Md};

fn element(tag: &str, attrs: &[(&str, &str)], children: Vec<Node>) -> Node {
    let mut node = el(tag, attrs, children);
    if let Node::Element(element_node) = &mut node {
        element_node.component = true;
    }
    node
}
fn ct(value: &str) -> Node {
    Node::ComponentText(value.into())
}
pub(crate) fn placeholder(name: &str, value: &str) -> Node {
    Node::Bridge(vec![element(
        "span",
        &[("class", "md-unknown"), ("data-component", name)],
        vec![ct(value)],
    )])
}
fn literal_attributes(attrs: &[AttributeContent]) -> Vec<(&str, &str)> {
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
fn value<'a>(attrs: &'a [(&str, &str)], name: &str) -> Option<&'a str> {
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
                    || matches!(node,Md::Text(text_node) if text_node.value.chars().all(char::is_whitespace))
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
pub(crate) fn component(
    name: Option<&str>,
    attrs: &[AttributeContent],
    children: &[Md],
    inline: bool,
    state: &mut State,
    path: &str,
) -> Node {
    let name = name.unwrap_or("unknown");
    let attrs = literal_attributes(attrs);
    match name {
        "CodeTabs" | "Comparison" | "Flowchart" | "StepRail" => {
            let (child_name, attr_name, fallback, tag, class, data) = match name {
                "CodeTabs" => ("CodeTab", "label", "Tab", "section", "md-codetab-panel", "data-label"),
                "Comparison" => ("Option", "name", "Option", "article", "md-comparison-option", "data-name"),
                "Flowchart" => ("NodeDetail", "name", "Node", "section", "md-node-detail", "data-name"),
                _ => ("Step", "title", "Step", "li", "md-step", "data-title"),
            };
            let (accepted, remaining) = groups(children, child_name);
            let mut panels = Vec::new();
            for (index, node) in accepted.into_iter().enumerate() {
                let (_, attributes, children) = parts(node).unwrap();
                let attributes = literal_attributes(attributes);
                let default = format!("{fallback} {}", index + 1);
                let label = value(&attributes, attr_name).unwrap_or(&default);
                let mut body = vec![element("h3", &[], vec![ct(label)])];
                body.extend(render_children(children, state, path));
                panels.push(element(tag, &[("class", class), (data, label)], body));
            }
            match name {
                "CodeTabs" => {
                    panels.extend(render_children(&remaining, state, path));
                    element("div", &[("class", "md-codetabs-wrapper")], panels)
                }
                "Comparison" => {
                    panels.extend(render_children(&remaining, state, path));
                    element(
                        "section",
                        &[("class", "md-comparison")],
                        vec![element("div", &[("class", "md-comparison-grid")], panels)],
                    )
                }
                "StepRail" => {
                    panels.extend(render_children(&remaining, state, path));
                    element("ol", &[("class", "md-steprail")], panels)
                }
                _ => {
                    state.has_mermaid = true;
                    let diagram = remaining.iter().map(ToString::to_string).collect::<String>();
                    element(
                        "div",
                        &[("class", "md-flowchart")],
                        vec![
                            element("pre", &[("class", "mermaid")], vec![ct(&diagram)]),
                            element("div", &[("class", "md-node-details")], panels),
                        ],
                    )
                }
            }
        }
        "DecisionMatrix" => element(
            "table",
            &[("class", "md-matrix")],
            vec![element(
                "tbody",
                &[],
                vec![element(
                    "tr",
                    &[],
                    vec![element(
                        "td",
                        &[("data-score", value(&attrs, "score").unwrap_or(""))],
                        render_children(children, state, path),
                    )],
                )],
            )],
        ),
        "AnnotatedDiff" => {
            let (notes, remaining) = groups(children, "MarginNote");
            let mut markup = Vec::new();
            for (index, node) in notes.into_iter().enumerate() {
                let (_, attributes, children) = parts(node).unwrap();
                let attributes = literal_attributes(attributes);
                let default = (index + 1).to_string();
                let line = value(&attributes, "line").unwrap_or(&default);
                markup.push(element(
                    "li",
                    &[("class", "md-margin-note"), ("data-line", line)],
                    render_children(children, state, path),
                ));
            }
            element(
                "figure",
                &[("class", "md-annotated-diff")],
                vec![
                    element(
                        "div",
                        &[("class", "md-annotated-diff-content")],
                        render_children(&remaining, state, path),
                    ),
                    element("ol", &[("class", "md-margin-notes")], markup),
                ],
            )
        }
        _ if name.as_bytes().first().is_some_and(u8::is_ascii_uppercase) => element(
            if inline { "span" } else { "div" },
            &[("class", "md-unknown"), ("data-component", name)],
            render_children(children, state, path),
        ),
        _ => {
            let attrs = attrs
                .into_iter()
                .filter(|(name, _)| {
                    let mut chars = name.chars();
                    chars
                        .next()
                        .is_some_and(|children| children.is_ascii_alphabetic() || children == '_' || children == ':')
                        && chars.all(|children| children.is_ascii_alphanumeric() || matches!(children, ':' | '_' | '.' | '-'))
                })
                .collect::<Vec<_>>();
            element(name, &attrs, render_children(children, state, path))
        }
    }
}
