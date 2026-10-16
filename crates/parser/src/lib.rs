//! Pure metadata and markdown/MDX parsing into a semantic document.
mod ast;
mod components;
mod js;
mod mdast;
mod metadata;
mod plugins;
mod types;

pub use ast::*;
pub use metadata::{body_below_header, meta_of, meta_of as parse_meta};
pub use types::{Counts, Meta};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TocEntry {
    pub id: String,
    pub text: String,
    pub depth: u8,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Degraded {
    pub reason: String,
    pub message: String,
    pub line: usize,
}

/// Parse the body below the document header; metadata describes the complete source.
/// JavaScript in MDX is never executed, and parser failures never escape as panics.
pub fn parse(source: &str, path: &str) -> Document {
    let mut document = parse_with(&body_below_header(source), path, pipeline);
    document.meta = parse_meta(source, std::path::Path::new(path));
    document
}
fn pipeline(source: &str, path: &str) -> Result<Document, Degraded> {
    let mut options = markdown::ParseOptions::gfm();
    if path.ends_with(".mdx") {
        options.constructs.html_flow = false;
        options.constructs.html_text = false;
        options.constructs.autolink = false;
        options.constructs.code_indented = false;
        options.constructs.mdx_jsx_flow = true;
        options.constructs.mdx_jsx_text = true;
        options.constructs.mdx_expression_flow = true;
        options.constructs.mdx_expression_text = true;
        options.constructs.mdx_esm = true;
        // Only lexical boundaries are recognized, not full JavaScript validation.
        options.mdx_esm_parse = Some(Box::new(lexical_boundary));
        options.mdx_expression_parse = Some(Box::new(|value, _| lexical_boundary(value)));
    }
    let mut root = markdown::to_mdast(source, &options).map_err(failure_details)?;
    if path.ends_with(".mdx") {
        validate_trailing_jsx_marker(&root, source)?;
    }
    repair_line_endings(&mut root, source);
    let mut state = plugins::State::default();
    let blocks = mdast::convert_children(root.children().map(Vec::as_slice).unwrap_or(&[]), &mut state, path);
    let toc = plugins::ordered_toc(&state.toc, &blocks);
    Ok(Document {
        meta: parse_meta(source, std::path::Path::new(path)),
        blocks,
        toc,
        degraded: None,
    })
}
fn lexical_boundary(value: &str) -> markdown::MdxSignal {
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut block = false;
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if comment {
            if character == '\n' {
                comment = false;
            }
            continue;
        }
        if block {
            if character == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block = false;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }
        if character == '/' && chars.peek() == Some(&'/') {
            chars.next();
            comment = true;
            continue;
        }
        if character == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block = true;
            continue;
        }
        if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
        } else if matches!(character, '{' | '[' | '(') {
            stack.push(character);
        } else if matches!(character, '}' | ']' | ')') {
            stack.pop();
        }
    }
    if quote.is_some() || block || !stack.is_empty() {
        markdown::MdxSignal::Eof(
            "Unexpected end of file in expression".into(),
            Box::new("micromark-extension-mdx-expression".into()),
            Box::new("unexpected-eof".into()),
        )
    } else {
        markdown::MdxSignal::Ok
    }
}
fn parse_with(source: &str, path: &str, run: impl Fn(&str, &str) -> Result<Document, Degraded>) -> Document {
    let safe_run = |path: &str| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(source, path))).unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|value| value.to_string()))
                .unwrap_or_else(|| "Unknown render failure".into());
            Err(Degraded {
                reason: "mdx-parse".into(),
                message,
                line: 0,
            })
        })
    };
    match safe_run(path) {
        Ok(document) => document,
        Err(degraded) => {
            let markdown_path = if path.ends_with(".mdx") { &path[..path.len() - 1] } else { path };
            let mut document = safe_run(markdown_path).unwrap_or_else(|_| Document {
                meta: parse_meta(source, std::path::Path::new(path)),
                blocks: vec![Block::Code {
                    lang: None,
                    source: source.into(),
                }],
                toc: vec![],
                degraded: None,
            });
            document.blocks.insert(0, Block::Degraded);
            document.degraded = Some(degraded);
            document
        }
    }
}

#[cfg(test)]
mod tests;

// markdown-rs 1.0.0 drops soft line endings directly inside JSX and links.
// Re-decode only affected Text spans with markdown's own escape/entity tokenizer.
fn repair_line_endings(node: &mut markdown::mdast::Node, source: &str) {
    if let markdown::mdast::Node::Text(text_node) = node
        && let Some(position) = &text_node.position
        && let Some(raw) = source.get(position.start.offset..position.end.offset)
        && line_ending_count(raw) > line_ending_count(&text_node.value)
    {
        let mut options = markdown::ParseOptions::default();
        let constructs = &mut options.constructs;
        constructs.attention = false;
        constructs.autolink = false;
        constructs.block_quote = false;
        constructs.code_indented = false;
        constructs.code_fenced = false;
        constructs.code_text = false;
        constructs.definition = false;
        constructs.hard_break_escape = false;
        constructs.hard_break_trailing = false;
        constructs.heading_atx = false;
        constructs.heading_setext = false;
        constructs.html_flow = false;
        constructs.html_text = false;
        constructs.label_start_image = false;
        constructs.label_start_link = false;
        constructs.label_end = false;
        constructs.list_item = false;
        constructs.thematic_break = false;
        if let Ok(decoded) = markdown::to_mdast(raw, &options) {
            text_node.value = decoded.to_string();
        }
    }
    if let Some(children) = node.children_mut() {
        for child in children {
            repair_line_endings(child, source);
        }
    }
    let boundaries = if let markdown::mdast::Node::MdxJsxTextElement(element) = node {
        element.position.as_ref().and_then(|position| {
            let raw = source.get(position.start.offset..position.end.offset)?;
            Some((
                position.start.offset + opening_tag_length(raw)?,
                position.start.offset + raw.rfind("</")?,
            ))
        })
    } else {
        None
    };
    if matches!(
        node,
        markdown::mdast::Node::MdxJsxTextElement(_) | markdown::mdast::Node::Link(_) | markdown::mdast::Node::LinkReference(_)
    ) && let Some(children) = node.children_mut()
    {
        repair_sibling_gaps(children, source, boundaries);
    }
}

fn failure_details(error: markdown::message::Message) -> Degraded {
    let line = match error.place.as_deref() {
        Some(markdown::message::Place::Point(point)) => point.line,
        Some(markdown::message::Place::Position(position)) => position.start.line,
        None => 0,
    };
    Degraded {
        reason: "mdx-parse".into(),
        message: error.reason,
        line,
    }
}
fn opening_tag_length(source: &str) -> Option<usize> {
    let mut quote = None;
    let mut braces = 0usize;
    for (index, character) in source.char_indices() {
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            }
        } else if matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if character == '{' {
            braces += 1;
        } else if character == '}' {
            braces = braces.saturating_sub(1);
        } else if character == '>' && braces == 0 {
            return Some(index + 1);
        }
    }
    None
}

fn line_ending_count(value: &str) -> usize {
    value
        .replace("\r\n", "\n")
        .bytes()
        .filter(|byte| matches!(byte, b'\r' | b'\n'))
        .count()
}
fn repair_sibling_gaps(children: &mut Vec<markdown::mdast::Node>, source: &str, boundaries: Option<(usize, usize)>) {
    use markdown::mdast::{Node as Md, Text};
    let mut previous_end = boundaries.map(|(start, _)| start);
    let mut output = Vec::new();
    for mut child in std::mem::take(children) {
        let position = child.position().cloned();
        if let Some(previous_end) = previous_end
            && let Some(position) = &position
            && let Some(gap) = source.get(previous_end..position.start.offset)
            && line_ending_count(gap) > 0
            && gap.chars().all(char::is_whitespace)
        {
            if let Some(Md::Text(previous)) = output.last_mut() {
                previous.value.push_str(gap);
            } else if let Md::Text(current) = &mut child {
                current.value.insert_str(0, gap);
            } else {
                output.push(Md::Text(Text {
                    value: gap.into(),
                    position: None,
                }));
            }
        }
        previous_end = position.map(|position| position.end.offset);
        output.push(child);
    }
    if let Some(previous_end) = previous_end
        && let Some((_, end)) = boundaries
        && let Some(gap) = source.get(previous_end..end)
        && line_ending_count(gap) > 0
        && gap.chars().all(char::is_whitespace)
    {
        if let Some(Md::Text(previous)) = output.last_mut() {
            previous.value.push_str(gap);
        } else {
            output.push(Md::Text(Text {
                value: gap.into(),
                position: None,
            }));
        }
    }
    *children = output;
}

fn validate_trailing_jsx_marker(node: &markdown::mdast::Node, source: &str) -> Result<(), Degraded> {
    // markdown-rs treats an EOF JSX start marker as prose; remark-mdx errors on it.
    if let markdown::mdast::Node::Text(text) = node
        && text.value.ends_with('<')
        && let Some(position) = &text.position
        && position.end.offset == source.len()
        && source.ends_with('<')
    {
        let backslashes = source[..source.len() - 1]
            .chars()
            .rev()
            .take_while(|character| *character == '\\')
            .count();
        if backslashes % 2 == 0 {
            return Err(Degraded {
                reason: "mdx-parse".into(),
                message: "Unexpected end of file before name, expected a character that can start a name, such as a letter, `$`, or `_`"
                    .into(),
                line: position.end.line,
            });
        }
    }
    if let Some(children) = node.children() {
        for child in children {
            validate_trailing_jsx_marker(child, source)?;
        }
    }
    Ok(())
}
