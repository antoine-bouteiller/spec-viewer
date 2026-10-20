//! String and regular-expression primitives with JavaScript whitespace and line-terminator rules.
use std::cmp::Ordering;

/// Case-insensitive order, with byte order breaking ties so sorting stays deterministic.
pub(super) fn name_order(first: &str, second: &str) -> Ordering {
    first.to_lowercase().cmp(&second.to_lowercase()).then_with(|| first.cmp(second))
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
