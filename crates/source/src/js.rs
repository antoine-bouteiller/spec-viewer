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

// ECMAScript WhiteSpace + LineTerminator, not Rust's Unicode White_Space (which includes NEL).
pub(super) fn whitespace(character: char) -> bool {
    matches!(character, '\t'..='\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

pub(super) fn terminator(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

pub(super) fn trim(source: &str) -> &str {
    source.trim_matches(whitespace)
}

pub(super) fn starts(source: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            source
                .char_indices()
                .filter(|(_, character)| terminator(*character))
                .map(|(index, character)| index + character.len_utf8()),
        )
        .collect()
}

pub(super) fn take_while(source: &str, predicate: impl Fn(char) -> bool) -> usize {
    source
        .char_indices()
        .find(|(_, character)| !predicate(*character))
        .map_or(source.len(), |(index, _)| index)
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

pub(super) fn spec_path(path: &str) -> bool {
    spec_stem(path) != path
}
