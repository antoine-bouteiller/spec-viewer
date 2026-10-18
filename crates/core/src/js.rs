//! String and regular-expression primitives with JavaScript whitespace and line-terminator rules.
use std::cmp::Ordering;
use std::sync::OnceLock;

use icu_collator::{Collator, CollatorBorrowed};

pub(super) fn locale_compare(first: &str, second: &str) -> Ordering {
    static COLLATOR: OnceLock<CollatorBorrowed<'static>> = OnceLock::new();
    COLLATOR
        .get_or_init(|| Collator::try_new(Default::default(), Default::default()).expect("compiled ICU root collation"))
        .compare(first, second)
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
