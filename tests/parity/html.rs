use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
enum Node {
    Text(String),
    Element {
        name: String,
        attributes: BTreeMap<String, String>,
        children: Vec<Node>,
    },
}

struct Parser<'source> {
    source: &'source str,
    offset: usize,
}

impl<'source> Parser<'source> {
    fn remaining(&self) -> &'source str {
        &self.source[self.offset..]
    }

    fn error(&self, message: &str) -> String {
        format!(
            "HTML byte {}: {message}; next {:?}",
            self.offset,
            self.remaining().chars().take(80).collect::<String>()
        )
    }

    fn consume(&mut self, token: &str) -> Result<(), String> {
        if !self.remaining().starts_with(token) {
            return Err(self.error(&format!("expected {token:?}")));
        }
        self.offset += token.len();
        Ok(())
    }

    fn whitespace(&mut self) {
        while self.remaining().starts_with(|character: char| character.is_ascii_whitespace()) {
            self.offset += 1;
        }
    }

    fn name(&mut self) -> Result<String, String> {
        let length = self
            .remaining()
            .bytes()
            .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
            .count();
        if length == 0 {
            return Err(self.error("expected name"));
        }
        let name = self.remaining()[..length].to_owned();
        self.offset += length;
        Ok(name)
    }

    fn children(&mut self, parent: Option<&str>) -> Result<Vec<Node>, String> {
        let mut children = Vec::new();
        while !self.remaining().is_empty() {
            if self.remaining().starts_with("</") {
                self.consume("</")?;
                let closing = self.name()?;
                self.consume(">")?;
                if parent != Some(closing.as_str()) {
                    return Err(self.error(&format!("unexpected closing tag {closing:?}, parent {parent:?}")));
                }
                return Ok(children);
            }
            if self.remaining().starts_with('<') {
                children.push(self.element()?);
            } else {
                let length = self.remaining().find('<').unwrap_or(self.remaining().len());
                children.push(Node::Text(self.remaining()[..length].to_owned()));
                self.offset += length;
            }
        }
        if let Some(parent) = parent {
            return Err(self.error(&format!("unclosed {parent}")));
        }
        Ok(children)
    }

    fn element(&mut self) -> Result<Node, String> {
        self.consume("<")?;
        let name = self.name()?;
        let mut attributes = BTreeMap::new();
        loop {
            if self.remaining().starts_with('>') {
                self.consume(">")?;
                break;
            }
            if !self.remaining().starts_with(|character: char| character.is_ascii_whitespace()) {
                return Err(self.error("expected whitespace before attribute"));
            }
            self.whitespace();
            if self.remaining().starts_with('>') {
                continue;
            }
            let attribute = self.name()?;
            self.consume("=\"")?;
            let length = self.remaining().find('"').ok_or_else(|| self.error("unclosed attribute"))?;
            let value = self.remaining()[..length].to_owned();
            self.offset += length;
            self.consume("\"")?;
            if attributes.insert(attribute.clone(), value).is_some() {
                return Err(self.error(&format!("duplicate attribute {attribute}")));
            }
        }
        let children = if matches!(
            name.as_str(),
            "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "param" | "source" | "track" | "wbr"
        ) {
            Vec::new()
        } else {
            self.children(Some(&name))?
        };
        Ok(Node::Element {
            name,
            attributes,
            children,
        })
    }
}

fn text_content(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            Node::Text(text) => text.clone(),
            Node::Element { children, .. } => text_content(children),
        })
        .collect()
}

fn normalize_shiki(nodes: &mut [Node]) {
    for node in nodes {
        if let Node::Element {
            name,
            attributes,
            children,
        } = node
        {
            if name == "pre"
                && attributes
                    .get("class")
                    .is_some_and(|classes| classes.split_whitespace().any(|class| class == "shiki"))
            {
                let content = text_content(children);
                attributes.retain(|attribute, _value| attribute == "class");
                *children = vec![Node::Text(content)];
            } else {
                normalize_shiki(children);
            }
        }
    }
}

fn block(node: &Node) -> bool {
    matches!(node, Node::Element { name, .. } if matches!(name.as_str(),
        "address" | "article" | "aside" | "blockquote" | "div" | "dl" | "dt" | "dd" | "fieldset" | "figure" | "figcaption" |
        "footer" | "form" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "header" | "hr" | "li" | "main" | "nav" |
        "ol" | "p" | "pre" | "section" | "table" | "thead" | "tbody" | "tfoot" | "tr" | "td" | "th" | "ul"))
}

fn drop_block_whitespace(nodes: &mut Vec<Node>) {
    for node in nodes.iter_mut() {
        if let Node::Element { name, children, .. } = node
            && name != "pre"
        {
            drop_block_whitespace(children);
        }
    }
    let keep: Vec<bool> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            !(matches!(node, Node::Text(text) if text.chars().all(char::is_whitespace))
                && index > 0
                && index + 1 < nodes.len()
                && block(&nodes[index - 1])
                && block(&nodes[index + 1]))
        })
        .collect();
    let mut keep = keep.into_iter();
    nodes.retain(|_node| keep.next().unwrap());
}

fn difference(expected: &[Node], actual: &[Node], path: &str) -> Option<String> {
    for (index, (expected_node, actual_node)) in expected.iter().zip(actual).enumerate() {
        let path = format!("{path}/{index}");
        match (expected_node, actual_node) {
            (
                Node::Element {
                    name: expected_name,
                    attributes: expected_attributes,
                    children: expected_children,
                },
                Node::Element {
                    name: actual_name,
                    attributes: actual_attributes,
                    children: actual_children,
                },
            ) if expected_name == actual_name && expected_attributes == actual_attributes => {
                if let Some(diff) = difference(expected_children, actual_children, &format!("{path}<{expected_name}>")) {
                    return Some(diff);
                }
            }
            _ if expected_node == actual_node => {}
            _ => return Some(format!("{path}\nBun:  {expected_node:#?}\nRust: {actual_node:#?}")),
        }
    }
    (expected.len() != actual.len()).then(|| {
        let index = expected.len().min(actual.len());
        format!(
            "{path}/{index}: child count Bun={}, Rust={}\nBun: {:#?}\nRust: {:#?}",
            expected.len(),
            actual.len(),
            expected.get(index),
            actual.get(index)
        )
    })
}

pub fn compare(expected: &str, actual: &str) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    let mut expected_nodes = Parser {
        source: expected,
        offset: 0,
    }
    .children(None)
    .map_err(|error| format!("Bun: {error}"))?;
    let mut actual_nodes = Parser { source: actual, offset: 0 }
        .children(None)
        .map_err(|error| format!("Rust: {error}"))?;
    normalize_shiki(&mut expected_nodes);
    normalize_shiki(&mut actual_nodes);
    if expected_nodes == actual_nodes {
        return Ok(());
    }
    drop_block_whitespace(&mut expected_nodes);
    drop_block_whitespace(&mut actual_nodes);
    match difference(&expected_nodes, &actual_nodes, "html") {
        None => Ok(()),
        Some(diff) => Err(diff),
    }
}

#[test]
fn vc_1_html_normalizes_attribute_order_shiki_tokens_and_only_inter_block_whitespace() {
    for (expected, actual) in [
        (
            "<p id=\"title\" class=\"intro\" data-source=\"fixture\">A &amp; B<br>next</p>",
            "<p data-source=\"fixture\" class=\"intro\" id=\"title\">A &amp; B<br>next</p>",
        ),
        (
            "<pre class=\"shiki dual\" style=\"color:red\"><code><span>let </span><span>value</span>\n</code></pre>",
            "<pre style=\"color:blue\" class=\"shiki dual\"><code><span>let value\n</span></code></pre>",
        ),
        ("<div><h2>Title</h2>\n<p>Body</p></div>", "<div><h2>Title</h2><p>Body</p></div>"),
        ("<p>Body</p>", "<p>Body</p>"),
    ] {
        compare(expected, actual).unwrap_or_else(|diff| panic!("{expected:?} vs {actual:?}: {diff}"));
    }
}

#[test]
fn vc_1_html_rejects_significant_structure_attributes_entities_and_whitespace_changes() {
    for (expected, actual) in [
        ("<p>Body</p>", "<div>Body</div>"),
        ("<p>Body</p>", "<p>Body</p><hr>"),
        ("<p id=\"first\">Body</p>", "<p id=\"second\">Body</p>"),
        ("<p class=\"first\">Body</p>", "<p class=\"second\">Body</p>"),
        ("<p data-xref=\"G-1\">Body</p>", "<p data-xref=\"G-2\">Body</p>"),
        ("<p title=\"Body\">Body</p>", "<p>Body</p>"),
        ("<p>A &amp; B</p>", "<p>A &#38; B</p>"),
        (
            "<p><span>first</span> <span>second</span></p>",
            "<p><span>first</span><span>second</span></p>",
        ),
        ("<div><p>Body</p> </div>", "<div><p>Body</p></div>"),
        ("<p>Body </p>", "<p>Body</p>"),
        (
            "<pre class=\"shiki dual\"><code>Body</code></pre>",
            "<pre class=\"shiki\"><code>Body</code></pre>",
        ),
        (
            "<pre class=\"shiki\"><code>Body\n</code></pre>",
            "<pre class=\"shiki\"><code>Body</code></pre>",
        ),
        (
            "<pre><div>first</div> <div>second</div></pre>",
            "<pre><div>first</div><div>second</div></pre>",
        ),
        (
            "<code><span style=\"color:red\">Body</span></code>",
            "<code><span style=\"color:blue\">Body</span></code>",
        ),
    ] {
        let diff = compare(expected, actual).expect_err(&format!("accepted significant difference: {expected:?} vs {actual:?}"));
        assert!(diff.starts_with("html/"), "missing differing node path: {diff}");
        assert!(
            diff.contains("Bun:") && diff.contains("Rust:"),
            "missing readable comparison: {diff}"
        );
    }
}

#[test]
fn vc_1_html_tokenizer_rejects_malformed_or_unsupported_markup() {
    for malformed in [
        "<p>Body",
        "<p>Body</div>",
        "<p id='first'>Body</p>",
        "<p id=first>Body</p>",
        "<p id=\"first>Body</p>",
        "<p id=\"first\" id=\"second\">Body</p>",
        "<input disabled>",
        "<br></br>",
        "<!-- comment -->",
    ] {
        let diff = compare("<p>Body</p>", malformed).expect_err(&format!("accepted malformed markup: {malformed:?}"));
        assert!(diff.starts_with("Rust: HTML byte"), "missing tokenizer diagnostic: {diff}");
    }
}
