use std::collections::HashMap;
use std::path::Path;

use super::js::{horizontal, label, starts, take_while, terminator, trim, whitespace};
use super::{Counts, Meta};

#[derive(Clone)]
enum Value {
    Text(String),
    List(Vec<String>),
}

fn unquote(value: &str) -> String {
    let value = trim(value);
    let start = usize::from(value.starts_with(['\'', '"']));
    let end = value.len() - usize::from(value.ends_with(['\'', '"']));
    trim(value.get(start..end).unwrap_or("")).to_owned()
}

fn parse_value(value: &str) -> Value {
    if value.is_empty() {
        Value::List(vec![])
    } else if let Some(value) = value.strip_prefix('[') {
        Value::List(
            value
                .strip_suffix(']')
                .unwrap_or(value)
                .split(',')
                .map(unquote)
                .filter(|source| !source.is_empty())
                .collect(),
        )
    } else {
        Value::Text(unquote(value))
    }
}

fn frontmatter_block(source: &str) -> Option<(&str, usize)> {
    let after = source.strip_prefix("---")?;
    let opening = take_while(after, horizontal);
    let after = after.get(opening..)?;
    let after = after.strip_prefix('\r').unwrap_or(after);
    let after = after.strip_prefix('\n')?;
    let from = source.len() - after.len();
    for (index, _) in after.match_indices("\n---") {
        let close = from + index + 4;
        let tail = &source[close..];
        let spacing = take_while(tail, horizontal);
        let tail = &tail[spacing..];
        let tail = tail.strip_prefix('\r').unwrap_or(tail);
        if tail.is_empty() || tail.starts_with('\n') {
            let content = source[from..from + index].strip_suffix('\r').unwrap_or(&source[from..from + index]);
            let end = source.len() - tail.len() + usize::from(tail.starts_with('\n'));
            return Some((content, end));
        }
    }
    None
}

fn strip_frontmatter(source: &str) -> &str {
    frontmatter_block(source).map_or(source, |(_, end)| &source[end..])
}

fn without_comment(line: &str) -> &str {
    for (index, _) in line.match_indices('#') {
        if line[..index].chars().next_back().is_some_and(whitespace)
            && line[index + 1..].chars().next().is_some_and(whitespace)
            && !line[index + 1..].chars().skip(1).any(terminator)
        {
            return line[..index].trim_end_matches(whitespace);
        }
    }
    line
}

fn parse_frontmatter(source: &str) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    let Some((content, _)) = frontmatter_block(source) else {
        return out;
    };
    let mut key: Option<String> = None;
    for raw in content.split('\n') {
        let line = without_comment(raw.strip_suffix('\r').unwrap_or(raw));
        if trim(line).is_empty() {
            continue;
        }
        let item = line.trim_start_matches(whitespace).strip_prefix('-');
        if let (Some(item), Some(key)) = (item, &key) {
            let spacing = take_while(item, whitespace);
            if spacing > 0 && !item[spacing..].chars().any(terminator) {
                let current = out.entry(key.clone()).or_insert_with(|| Value::List(vec![]));
                if !matches!(current, Value::List(_)) {
                    *current = Value::List(vec![]);
                }
                if let Value::List(items) = current {
                    items.push(unquote(&item[spacing..]));
                }
                continue;
            }
        }
        if let Some((parsed_key, value)) = line.split_once(':')
            && !parsed_key.is_empty()
            && parsed_key
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
        {
            let value = value.trim_start_matches(horizontal);
            if !value.chars().any(terminator) {
                key = Some(parsed_key.to_owned());
                out.insert(parsed_key.to_owned(), parse_value(trim(value)));
            }
        }
    }
    out
}

fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Text(value)) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn strip_fences(source: &str) -> String {
    let mut result = String::new();
    let mut consumed = 0;
    let anchors = starts(source);
    for &start in &anchors {
        if start < consumed {
            continue;
        }
        let spaces = take_while(&source[start..], |character| character == ' ');
        if spaces > 3 {
            continue;
        }
        let at = start + spaces;
        let fence = if source[at..].starts_with("```") {
            "```"
        } else if source[at..].starts_with("~~~") {
            "~~~"
        } else {
            continue;
        };
        if let Some(&close) = anchors.iter().find(|&&close| {
            if close < at + 3 {
                return false;
            }
            let spaces = take_while(&source[close..], |character| character == ' ');
            spaces <= 3 && source[close + spaces..].starts_with(fence)
        }) {
            // [^\n]* is greedy, including CR and Unicode line separators.
            let end = close + source[close..].find('\n').unwrap_or(source.len() - close);
            result.push_str(&source[consumed..start]);
            consumed = end;
        }
    }
    result.push_str(&source[consumed..]);
    result
}

// Matches a heading's ^prefix[^\S\n]+(.+) and optional lazy horizontal tail.
fn headings<'a>(source: &'a str, prefix: &str, lazy: bool) -> Vec<(usize, usize, &'a str)> {
    let mut found = vec![];
    let mut consumed = 0;
    for start in starts(source) {
        if start < consumed {
            continue;
        }
        let Some(after) = source[start..].strip_prefix(prefix) else {
            continue;
        };
        let spacing = take_while(after, horizontal);
        if spacing == 0 {
            continue;
        }
        let mut capture = spacing;
        loop {
            let rest = &after[capture..];
            let length = take_while(rest, |character| !terminator(character));
            if length > 0 {
                let name = &rest[..length];
                let name = if lazy {
                    let trimmed = name.trim_end_matches(horizontal);
                    if trimmed.is_empty() {
                        &name[..name.chars().next().unwrap().len_utf8()]
                    } else {
                        trimmed
                    }
                } else {
                    name
                };
                let end = start + prefix.len() + capture + length;
                let end = if lazy { end + take_while(&source[end..], horizontal) } else { end };
                found.push((start, end, name));
                consumed = end;
                break;
            }
            let Some((previous, _)) = after[..capture].char_indices().next_back() else {
                break;
            };
            if previous == 0 {
                break;
            }
            capture = previous;
        }
    }
    found
}

fn legacy_status(body: &str) -> Option<(usize, usize, &str)> {
    for start in starts(body) {
        let rest = &body[start..];
        if rest.get(..11).is_some_and(|source| source.eq_ignore_ascii_case("**Status:**")) {
            let after = &rest[11..];
            let mut spacing = take_while(after, horizontal);
            loop {
                let length = take_while(&after[spacing..], |character| !terminator(character));
                if length > 0 {
                    return Some((start, start + 11 + spacing + length, &after[spacing..spacing + length]));
                }
                let Some((previous, _)) = after[..spacing].char_indices().next_back() else {
                    break;
                };
                spacing = previous;
            }
        }
    }
    None
}

fn count_checks(source: Option<&str>) -> Counts {
    let Some(source) = source else { return Counts::default() };
    let mut counts = Counts::default();
    let mut consumed = 0;
    for start in starts(source) {
        if start < consumed {
            continue;
        }
        let rest = source[start..].trim_start_matches(whitespace);
        let Some(rest) = rest.strip_prefix(['-', '*']) else { continue };
        let spacing = take_while(rest, whitespace);
        if spacing == 0 {
            continue;
        }
        let check = &rest[spacing..];
        if check.starts_with("[ ]") || check.starts_with("[x]") || check.starts_with("[X]") {
            counts.total += 1;
            counts.done += usize::from(!check.starts_with("[ ]"));
            consumed = source.len() - check.len() + 3;
        }
    }
    counts
}

fn normalize_status(raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let lower = trim(&raw).to_lowercase();
    let cleaned = trim(lower.trim_end_matches(['.', ';', ',', '—', '-']));
    let head = cleaned
        .split([';', ',', '—'])
        .next()
        .unwrap_or("")
        .split(" - ")
        .next()
        .unwrap_or("");
    let head = trim(head);
    if head.is_empty() {
        return None;
    }
    Some(
        match head {
            "complete" | "completed" | "finished" | "implemented" => "done",
            "in progress" | "wip" => "in-progress",
            "planned" | "todo" => "draft",
            "review" => "ready",
            other => other,
        }
        .into(),
    )
}

pub fn meta_of(source: &str, file: &Path) -> Meta {
    let fm = parse_frontmatter(source);
    let raw = strip_frontmatter(source);
    let body = strip_fences(raw);
    let mut sections = HashMap::new();
    let headings = headings(&body, "##", true);
    for (index, &(_, from, name)) in headings.iter().enumerate() {
        let to = headings.get(index + 1).map_or(body.len(), |heading| heading.0);
        sections.insert(name.to_lowercase(), &body[from..to]);
    }
    let implementation = sections.get("implementation").or_else(|| sections.get("steps")).copied();
    let phases = implementation.map_or_else(Vec::new, |source| {
        self::headings(source, "###", true)
            .into_iter()
            .map(|heading| heading.2.to_owned())
            .collect()
    });
    let legacy_status = legacy_status(&body).map(|source| source.2.to_owned());
    let status = text(fm.get("status"));
    let title = text(fm.get("title"));
    let fallback = self::headings(&body, "#", false)
        .first()
        .map(|heading| trim(heading.2).to_owned())
        .unwrap_or_else(|| label(file));
    Meta {
        legacy: status.is_none() && title.is_none() && legacy_status.is_some(),
        title: title.unwrap_or(fallback),
        status: normalize_status(status.or(legacy_status)),
        author: text(fm.get("author")),
        date: text(fm.get("date")),
        related: match fm.get("related") {
            Some(Value::List(values)) => values.clone(),
            value => text(value).into_iter().collect(),
        },
        tasks: count_checks(implementation),
        acceptance: count_checks(sections.get("acceptance criteria").copied()),
        phases,
        lines: trim(raw).split('\n').count(),
        kind: text(fm.get("kind")),
        parent_spec: text(fm.get("parent-spec")),
        archived: text(fm.get("archived"))
            .filter(|value| value.to_lowercase() == "true")
            .map(|_| true),
    }
}

pub fn body_below_header(source: &str) -> String {
    let mut body = strip_frontmatter(source).to_owned();
    if let Some(&(start, end, _)) = headings(&body, "#", false).first() {
        body.replace_range(start..end, "");
    }
    for start in starts(&body) {
        if body[start..]
            .get(..11)
            .is_some_and(|source| source.eq_ignore_ascii_case("**Status:**"))
        {
            let end = start + take_while(&body[start..], |character| !terminator(character));
            body.replace_range(start..end, "");
            break;
        }
    }
    body.trim_start_matches(whitespace).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frontmatter_scalars_lists_and_comments() {
        let meta = meta_of(
            "---\ntitle: \" A title \" # comment\nstatus: completed\nauthor: 'Agent'\ndate: 2026-10-16\nrelated: [a.spec.md, 'b.spec.md', ]\nkind: unusual\nparent-spec: a.spec.md\narchived: TRUE\n---\n# Ignored\n",
            Path::new("fallback.md"),
        );
        assert_eq!(meta.title, "A title");
        assert_eq!(meta.status.as_deref(), Some("done"));
        assert_eq!(meta.author.as_deref(), Some("Agent"));
        assert_eq!(meta.date.as_deref(), Some("2026-10-16"));
        assert_eq!(meta.related, ["a.spec.md", "b.spec.md"]);
        assert_eq!(meta.kind.as_deref(), Some("unusual"));
        assert_eq!(meta.parent_spec.as_deref(), Some("a.spec.md"));
        assert_eq!(meta.archived, Some(true));
    }

    #[test]
    fn block_lists_and_scalar_related() {
        let meta = meta_of(
            "---\r\nrelated: replaced\r\n - a.spec.md\r\n # comment\r\n - 'b.spec.md' # note\r\n - ''\r\n---\r\n",
            Path::new("a.md"),
        );
        assert_eq!(meta.related, ["a.spec.md", "b.spec.md", ""]);
        assert_eq!(meta.lines, 1);
        let meta = meta_of("---\nrelated: a.spec.md\n---\n", Path::new("a.md"));
        assert_eq!(meta.related, ["a.spec.md"]);
    }
    #[test]
    fn legacy_identity_and_title_fallback() {
        let meta = meta_of("# Legacy title\n\n**STATUS:** WIP — note\n", Path::new("fallback.md"));
        assert_eq!(meta.title, "Legacy title");
        assert_eq!(meta.status.as_deref(), Some("in-progress"));
        assert!(meta.legacy);
        let meta = meta_of("---\ntitle: Front title\n---\n**Status:** finished\n", Path::new("fallback.md"));
        assert!(!meta.legacy);
        assert_eq!(meta.status.as_deref(), Some("done"));
        assert_eq!(meta_of("", Path::new("name.spec.mdx")).title, "name.spec.mdx");
        assert_eq!(meta_of("", Path::new("name.spec.md")).title, "name.spec");
    }
    #[test]
    fn whitespace_only_legacy_status_still_marks_legacy() {
        let meta = meta_of("**Status:**   ", Path::new("a.md"));
        assert!(meta.legacy);
        assert_eq!(meta.status, None);
    }

    #[test]
    fn aliases_and_status_normalization() {
        for (raw, expected) in [
            ("complete", "done"),
            ("completed", "done"),
            ("finished", "done"),
            ("implemented", "done"),
            ("in progress", "in-progress"),
            ("planned", "draft"),
            ("review", "ready"),
            ("todo", "draft"),
            ("wip", "in-progress"),
            (" BLOCKED; explanation.", "blocked"),
            ("Accepted - detail", "accepted"),
            ("custom, detail", "custom"),
            ("Ready—detail", "ready"),
            (" Converging.;,—- ", "converging"),
        ] {
            let meta = meta_of(&format!("---\nstatus: {raw}\n---\n"), Path::new("a.md"));
            assert_eq!(meta.status.as_deref(), Some(expected), "{raw}");
        }
        for raw in ["", "---", ";;;", ", detail"] {
            assert_eq!(meta_of(&format!("---\nstatus: '{raw}'\n---\n"), Path::new("a.md")).status, None);
        }
    }
    #[test]
    fn counts_and_phases_ignore_fences_and_unowned_checks() {
        let source = "# Plan\n- [x] Outside\n## Acceptance criteria\n- [X] done\n* [ ] pending\n```md\n- [x] fake\n## Implementation\n### fake\n```\n## Implementation\n### Phase one  \n- [x] task\n### Phase two\n  * [ ] task\n~~~md\n- [x] fake\n~~~\n## Other\n- [x] not a task\n";
        let meta = meta_of(source, Path::new("a.md"));
        assert_eq!(meta.acceptance, Counts { done: 1, total: 2 });
        assert_eq!(meta.tasks, Counts { done: 1, total: 2 });
        assert_eq!(meta.phases, ["Phase one", "Phase two"]);
        assert_eq!(meta.lines, 20);
    }
    #[test]
    fn steps_fallback_and_last_section_wins() {
        let meta = meta_of(
            "## Steps\n### Step\n- [x] done\n## Acceptance criteria\n- [x] old\n## Acceptance criteria\n- [ ] new\n",
            Path::new("a.md"),
        );
        assert_eq!(meta.phases, ["Step"]);
        assert_eq!(meta.tasks, Counts { done: 1, total: 1 });
        assert_eq!(meta.acceptance, Counts { done: 0, total: 1 });
        let meta = meta_of("## Steps\n- [x] done\n## Implementation\n", Path::new("a.md"));
        assert_eq!(meta.tasks, Counts::default());
    }
    #[test]
    fn unicode_whitespace_and_lines() {
        let meta = meta_of("\u{feff} \n", Path::new("a.md"));
        assert_eq!(meta.lines, 1);
        let meta = meta_of(
            "#\u{a0}Title\n##\u{2003}Implementation\n\u{feff}-\u{a0}[X] yes\n",
            Path::new("a.md"),
        );
        assert_eq!(meta.title, "Title");
        assert_eq!(meta.tasks, Counts { done: 1, total: 1 });
        assert_eq!(meta.lines, 3);
    }
    #[test]
    fn body_removes_only_first_header_and_legacy_status() {
        let body = body_below_header("---\ntitle: Plan\n---\n\n# Title\n**Status:** done\n\nBody\n# Second\n**Status:** remains\n");
        assert_eq!(body, "Body\n# Second\n**Status:** remains\n");
    }
}
