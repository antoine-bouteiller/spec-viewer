use super::{
    highlight,
    tree::{Element, Node},
};

pub(crate) fn escape(value: &str, attr: bool) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        out.push_str(match ch {
            '&' => "&#x26;",
            '"' if attr => "&#x22;",
            '\'' if attr => "&#x27;",
            '`' if attr => "&#x60;",
            '<' if !attr => "&#x3C;",
            '\0' if attr => "&#x0;",
            _ => {
                out.push(ch);
                continue;
            }
        });
    }
    out
}
pub(crate) fn component_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
pub(crate) fn index_escape(value: &str) -> String {
    component_escape(value).replace("&#x27;", "&#39;")
}

pub(crate) fn serialize(nodes: &[Node], highlighting: bool) -> String {
    nodes.iter().map(|node| node_html(node, highlighting)).collect()
}
fn node_html(node: &Node, highlighting: bool) -> String {
    match node {
        Node::Text(value) => escape(value, false),
        Node::ComponentText(value) => component_escape(value),
        Node::SourceText(value) => index_escape(value),
        Node::Raw(value) => value.clone(),
        Node::Bridge(nodes) => serialize(nodes, highlighting),
        Node::Element(element) => element_html(element, highlighting),
    }
}
fn element_html(element: &Element, highlighting: bool) -> String {
    if highlighting
        && element.tag == "pre"
        && let Some(Node::Element(code)) = element
            .children
            .iter()
            .find(|node| matches!(node,Node::Element(code_element) if code_element.tag == "code"))
        && let Some(lang) = code.attr("class").and_then(|value| {
            value
                .split_whitespace()
                .find_map(|code_element| code_element.strip_prefix("language-"))
        })
        && let Some(html) = highlight::to_html(&code.children.iter().map(super::tree::plain_text).collect::<String>(), lang)
    {
        return html;
    }
    let mut out = format!("<{}", element.tag);
    for (name, value) in &element.attrs {
        out.push(' ');
        out.push_str(name);
        if let Some(value) = value {
            out.push_str("=\"");
            out.push_str(&if element.component {
                component_escape(value)
            } else {
                escape(value, true)
            });
            out.push('"');
        }
    }
    out.push('>');
    if !element.component && (element.tag == "script" || element.tag == "style") {
        for node in &element.children {
            if let Node::Text(value) = node {
                out.push_str(value);
            } else {
                out.push_str(&node_html(node, highlighting));
            }
        }
    } else {
        out.push_str(&serialize(&element.children, highlighting));
    }
    let void = matches!(
        element.tag.to_ascii_lowercase().as_str(),
        "area"
            | "base"
            | "basefont"
            | "bgsound"
            | "link"
            | "meta"
            | "hr"
            | "br"
            | "wbr"
            | "img"
            | "input"
            | "embed"
            | "param"
            | "source"
            | "track"
            | "col"
            | "frame"
            | "keygen"
    );
    if element.component || !void || !element.children.is_empty() {
        out.push_str(&format!("</{}>", element.tag));
    }
    out
}
