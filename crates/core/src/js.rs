//! String and regular-expression primitives with JavaScript whitespace and line-terminator rules.
use std::cmp::Ordering;

/// Case-insensitive order, with byte order breaking ties so sorting stays deterministic.
pub(super) fn name_order(first: &str, second: &str) -> Ordering {
    first.to_lowercase().cmp(&second.to_lowercase()).then_with(|| first.cmp(second))
}

pub(super) fn label(path: &std::path::Path) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name.strip_suffix(".md").unwrap_or(&name).to_owned()
}

pub(super) fn spec_stem(path: &str) -> &str {
    for kind in ["spec", "discovery", "examples"] {
        for ext in ["md", "mdx"] {
            if let Some(stem) = path.strip_suffix(&format!(".{kind}.{ext}")) {
                return stem;
            }
        }
    }
    path
}
