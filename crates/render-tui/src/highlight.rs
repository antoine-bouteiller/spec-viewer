//! Syntect tokens for the terminal emitter, coloured by ANSI slot.
use std::sync::LazyLock;
use syntect::{
    easy::HighlightLines,
    highlighting::{Style, Theme},
    parsing::{SyntaxReference, SyntaxSet},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub style: Style,
    pub text: String,
}
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);
// Foregrounds are encoded as alpha 0 with the ANSI slot index in red, or alpha 1 for the default.
static ANSI: LazyLock<Theme> = LazyLock::new(|| two_face::theme::extra().get(two_face::theme::EmbeddedThemeName::Ansi).clone());

/// The ANSI slot index a token foreground names, or None for the default foreground.
pub fn slot(color: syntect::highlighting::Color) -> Option<usize> {
    (color.a == 0).then_some(usize::from(color.r)).filter(|index| *index < 16)
}

fn syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let ext = match lang {
        "ts" | "typescript" | "cts" | "mts" => "ts",
        "tsx" => "tsx",
        "js" | "javascript" | "cjs" | "mjs" => "js",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "bash" | "sh" | "shell" | "zsh" | "shellscript" => "sh",
        "java" => "java",
        "kotlin" | "kt" | "kts" => "kt",
        "sql" => "sql",
        "rs" | "rust" => "rs",
        "diff" => "diff",
        "md" | "markdown" => "md",
        "text" | "txt" | "plain" | "plaintext" => return Some(SYNTAXES.find_syntax_plain_text()),
        _ => return None,
    };
    SYNTAXES.find_syntax_by_extension(ext)
}
/// Returns one token vector per source line, including a final empty line after a newline.
/// Unsupported languages or a syntax engine failure return None (plain code fallback).
pub fn tokens(source: &str, lang: &str) -> Option<Vec<Vec<Token>>> {
    let syntax = syntax(lang)?;
    let mut highlighter = HighlightLines::new(syntax, &ANSI);
    let mut lines = Vec::new();
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    for line in source.split_inclusive('\n') {
        let line_tokens = highlighter.highlight_line(line, &SYNTAXES).ok()?;
        let mut tokens = Vec::new();
        for (style, text) in line_tokens {
            let text = text.strip_suffix('\n').unwrap_or(text);
            let text = text.strip_suffix('\r').unwrap_or(text);
            if !text.is_empty() {
                tokens.push(Token { style, text: text.into() });
            }
        }
        lines.push(tokens);
    }
    if source.is_empty() || source.ends_with('\n') {
        lines.push(Vec::new());
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use crate::highlight;

    #[test]
    fn highlight_aliases_and_terminal_tokens() {
        let source = "const value = \"hello\"\n\n// comment\n";
        for language in [
            "ts",
            "tsx",
            "js",
            "json",
            "yaml",
            "bash",
            "java",
            "kotlin",
            "sql",
            "diff",
            "md",
            "typescript",
            "javascript",
            "cts",
            "mts",
            "cjs",
            "mjs",
            "sh",
            "shell",
            "shellscript",
            "zsh",
            "yml",
            "markdown",
            "kt",
            "kts",
            "rs",
            "rust",
        ] {
            let lines = highlight::tokens(source, language).unwrap();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.iter().map(|token| token.text.as_str()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n"),
                source
            );
        }
        let tokens = highlight::tokens(source, "ts").unwrap();
        assert!(
            tokens
                .iter()
                .flatten()
                .any(|token| highlight::slot(token.style.foreground).is_some())
        );
        assert!(highlight::tokens(source, "brainfuck").is_none());
        assert!(highlight::tokens(source, "cobol").is_none());
    }
}
