//! Owned HAST-like nodes. Bridge nodes retain structure but isolate the outer plugins,
//! just as the Bun component bridge's raw HTML nodes do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Text(String),
    Raw(String),
    /// Text escaped by components.ts rather than hast-util-to-html.
    ComponentText(String),
    /// Last-resort source text uses index.ts escaping (including decimal apostrophes).
    SourceText(String),
    Element(Element),
    Bridge(Vec<Node>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub tag: String,
    /// Ordered attributes; None means a boolean attribute.
    pub attrs: Vec<(String, Option<String>)>,
    pub children: Vec<Node>,
    /// Literal JSX/component markup uses component escaping and explicit end tags.
    pub component: bool,
}
impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(key, _)| key == name)
            .and_then(|(_, value)| value.as_deref())
    }
    pub(crate) fn set(&mut self, name: &str, value: impl Into<String>) {
        if let Some((_, attr)) = self.attrs.iter_mut().find(|(key, _)| key == name) {
            *attr = Some(value.into());
        } else {
            self.attrs.push((name.into(), Some(value.into())));
        }
    }
}
pub(crate) fn text(value: impl Into<String>) -> Node {
    Node::Text(value.into())
}
pub(crate) fn el(tag: &str, attrs: &[(&str, &str)], children: Vec<Node>) -> Node {
    Node::Element(Element {
        tag: tag.into(),
        attrs: attrs.iter().map(|(key, value)| ((*key).into(), Some((*value).into()))).collect(),
        children,
        component: false,
    })
}
pub(crate) fn wrap(nodes: Vec<Node>, loose: bool) -> Vec<Node> {
    let mut out = Vec::new();
    if loose {
        out.push(text("\n"));
    }
    for node in nodes {
        if out.len() > usize::from(loose) {
            out.push(text("\n"));
        }
        out.push(node);
    }
    if loose && out.len() > 1 {
        out.push(text("\n"));
    }
    out
}
pub fn plain_text(node: &Node) -> String {
    match node {
        Node::Text(value) | Node::ComponentText(value) | Node::SourceText(value) => value.clone(),
        Node::Element(element) => element.children.iter().map(plain_text).collect(),
        Node::Bridge(nodes) => nodes.iter().map(plain_text).collect(),
        Node::Raw(_) => String::new(),
    }
}
