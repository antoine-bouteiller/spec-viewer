//! Syntect tokens for the terminal emitter.
use std::{io::Cursor, sync::LazyLock};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub style: Style,
    pub text: String,
}
struct Highlighter {
    syntaxes: SyntaxSet,
    themes: [syntect::highlighting::Theme; 2],
}
static HIGHLIGHTER: LazyLock<Highlighter> = LazyLock::new(|| Highlighter {
    syntaxes: two_face::syntax::extra_newlines(),
    themes: [
        ThemeSet::load_from_reader(&mut Cursor::new(include_bytes!("../assets/themes/catppuccin-latte.tmTheme")))
            .expect("vendored latte theme"),
        ThemeSet::load_from_reader(&mut Cursor::new(include_bytes!("../assets/themes/catppuccin-mocha.tmTheme")))
            .expect("vendored mocha theme"),
    ],
});
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
        "diff" => "diff",
        "md" | "markdown" => "md",
        "text" | "txt" | "plain" | "plaintext" => return Some(HIGHLIGHTER.syntaxes.find_syntax_plain_text()),
        _ => return None,
    };
    HIGHLIGHTER.syntaxes.find_syntax_by_extension(ext)
}
/// Returns one token vector per source line, including a final empty line after a newline.
/// Unsupported languages or a syntax engine failure return None (plain code fallback).
pub fn tokens(source: &str, lang: &str, theme: Theme) -> Option<Vec<Vec<Token>>> {
    let syntax = syntax(lang)?;
    let index = usize::from(theme == Theme::Dark);
    let mut highlighter = HighlightLines::new(syntax, &HIGHLIGHTER.themes[index]);
    let mut lines = Vec::new();
    let source = source.replace("\r\n", "\n").replace('\r', "\n");
    for line in source.split_inclusive('\n') {
        let line_tokens = highlighter.highlight_line(line, &HIGHLIGHTER.syntaxes).ok()?;
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
        ] {
            for theme in [highlight::Theme::Light, highlight::Theme::Dark] {
                let lines = highlight::tokens(source, language, theme).unwrap();
                assert_eq!(
                    lines
                        .iter()
                        .map(|line| line.iter().map(|token| token.text.as_str()).collect::<String>())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    source
                );
            }
        }
        assert!(highlight::tokens(source, "brainfuck", highlight::Theme::Light).is_none());
        assert!(highlight::tokens(source, "rust", highlight::Theme::Light).is_none());
    }
}
