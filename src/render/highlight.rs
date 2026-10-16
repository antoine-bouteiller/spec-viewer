//! Shared syntect tokens for HTML and the future terminal emitter.
use super::html::escape;
use std::{io::Cursor, sync::LazyLock};
use syntect::{
    easy::HighlightLines,
    highlighting::{Color, FontStyle, Style, ThemeSet},
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
        ThemeSet::load_from_reader(&mut Cursor::new(include_bytes!("../../assets/themes/catppuccin-latte.tmTheme")))
            .expect("vendored latte theme"),
        ThemeSet::load_from_reader(&mut Cursor::new(include_bytes!("../../assets/themes/catppuccin-mocha.tmTheme")))
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
pub fn css_color(color: Color) -> String {
    let hex = format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    let value = match hex.as_str() {
        "#04a5e5" | "#89dceb" => "var(--status-in-progress)",
        "#179299" | "#94e2d5" => "var(--xref-g)",
        "#1e1e2e" | "#eff1f5" => "var(--code-bg)",
        "#1e66f5" | "#89b4fa" => "var(--status-ready)",
        "#209fb5" => "#326c7a",
        "#40a02b" | "#a6e3a1" => "var(--status-done)",
        "#4c4f69" | "#cdd6f4" => "var(--code-fg)",
        "#6c6f85" | "#7c7f93" | "#9399b2" | "#a6adc8" => "var(--muted)",
        "#7287fd" => "#606a9e",
        "#74c7ec" => "#92c3d9",
        "#8839ef" | "#cba6f7" => "var(--accent)",
        "#b4befe" => "#bdc3ed",
        "#d20f39" | "#f38ba8" => "var(--status-blocked)",
        "#dc8a78" => "#946459",
        "#dd7878" => "#9a5e56",
        "#df8e1d" | "#f9e2af" => "var(--status-draft)",
        "#e64553" => "#995366",
        "#ea76cb" | "#f5c2e7" => "var(--xref-oq)",
        "#eba0ac" => "#d99ca7",
        "#f2cdcd" => "#e6bfba",
        "#f5e0dc" => "#ead5cf",
        "#fab387" => "#f5b56f",
        "#fe640b" => "#995c22",
        _ => return hex,
    };
    value.into()
}
pub(crate) fn to_html(source: &str, lang: &str) -> Option<String> {
    let light = tokens(source, lang, Theme::Light)?;
    let dark = tokens(source, lang, Theme::Dark)?;
    let plain = matches!(lang, "text" | "txt" | "plain" | "plaintext");
    let mut html = String::from(
        "<pre class=\"shiki shiki-themes catppuccin-latte catppuccin-mocha\" style=\"background-color:var(--code-bg);--shiki-dark-bg:var(--code-bg);color:var(--code-fg);--shiki-dark:var(--code-fg)\" tabindex=\"0\"><code>",
    );
    for (line_index, (light, dark)) in light.iter().zip(&dark).enumerate() {
        if line_index > 0 {
            html.push('\n');
        }
        html.push_str("<span class=\"line\">");
        if plain {
            html.push_str("<span>");
            for token in light {
                html.push_str(&escape(&token.text, false));
            }
            html.push_str("</span></span>");
            continue;
        }
        // The two themes can coalesce different scopes. Split at both sets of boundaries.
        let mut li = 0;
        let mut di = 0;
        let mut lo = 0;
        let mut doff = 0;
        while li < light.len() && di < dark.len() {
            let light_token = &light[li];
            let dark_token = &dark[di];
            let len = (light_token.text.len() - lo).min(dark_token.text.len() - doff);
            let mut style = format!("color:{}", css_color(light_token.style.foreground));
            let flags = [
                (FontStyle::ITALIC, "font-style:italic"),
                (FontStyle::BOLD, "font-weight:bold"),
                (FontStyle::UNDERLINE, "text-decoration:underline"),
            ];
            for (flag, css) in flags {
                if light_token.style.font_style.contains(flag) {
                    style.push_str(&format!(";--shiki-light-{css}"));
                }
            }
            style.push_str(&format!(";--shiki-dark:{}", css_color(dark_token.style.foreground)));
            for (flag, css) in flags {
                if dark_token.style.font_style.contains(flag) {
                    style.push_str(&format!(";--shiki-dark-{css}"));
                }
            }
            html.push_str(&format!(
                "<span style=\"{style}\">{}</span>",
                escape(&light_token.text[lo..lo + len], false)
            ));
            lo += len;
            doff += len;
            if lo == light_token.text.len() {
                li += 1;
                lo = 0;
            }
            if doff == dark_token.text.len() {
                di += 1;
                doff = 0;
            }
        }
        html.push_str("</span>");
    }
    html.push_str("</code></pre>");
    Some(html)
}
