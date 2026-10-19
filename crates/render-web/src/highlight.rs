//! Class-based syntax highlighting, coloured by the configured palette per colour mode.
use std::sync::LazyLock;

use syntect::{
    highlighting::Color,
    html::{ClassStyle, ClassedHTMLGenerator, css_for_theme_with_class_style},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};
use two_face::theme::EmbeddedThemeName;

use crate::html::escape;

const STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: "hl-" };

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

// Keep in sync with the terminal's `highlight::syntax`.
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
        _ => return None,
    };
    SYNTAXES.find_syntax_by_extension(ext)
}

/// A `<pre>` block; unsupported languages and highlighter failures fall back to escaped source.
pub(crate) fn code(lang: Option<&str>, source: &str) -> String {
    let class = lang.map(|lang| format!(" class=\"language-{}\"", escape(lang))).unwrap_or_default();
    if let Some(highlighted) = lang.and_then(syntax).and_then(|syntax| highlighted(syntax, source)) {
        return format!("<pre class=\"hl\"><code{class}>{highlighted}</code></pre>");
    }
    format!("<pre><code{class}>{}</code></pre>", escape(source))
}

fn highlighted(syntax: &SyntaxReference, source: &str) -> Option<String> {
    let mut generator = ClassedHTMLGenerator::new_with_class_style(syntax, &SYNTAXES, STYLE);
    for line in LinesWithEndings::from(source) {
        generator.parse_html_for_line_which_includes_newline(line).ok()?;
    }
    Some(generator.finalize())
}

/// Token rules for one mode, colouring each token by its ANSI slot's CSS colour (`ANSI_SLOTS` order).
pub(crate) fn rules(ansi: &[String; 16]) -> String {
    // The ANSI theme stores the slot index in red with alpha 0, and syntect's CSS drops alpha,
    // so map slots to unique sentinels first and substitute the palette colours afterwards.
    let sentinel = |index: u8| Color {
        r: 0xab,
        g: 0xcd,
        b: index,
        a: 0xff,
    };
    let mut theme = two_face::theme::extra().get(EmbeddedThemeName::Ansi).clone();
    theme.settings = Default::default();
    for item in &mut theme.scopes {
        for color in [&mut item.style.foreground, &mut item.style.background] {
            *color = color.filter(|color| color.a == 0 && color.r < 16).map(|color| sentinel(color.r));
        }
    }
    let mut css = css_for_theme_with_class_style(&theme, STYLE).expect("embedded ANSI theme");
    for (index, color) in ansi.iter().enumerate() {
        css = css.replace(&format!("#abcd{index:02x}"), color);
    }
    css
}

#[cfg(test)]
mod tests {
    #[test]
    fn rules_colour_tokens_by_palette_slot() {
        let ansi = std::array::from_fn(|index| format!("#1020{index:02x}"));
        let css = super::rules(&ansi);
        assert!(css.contains(".hl-keyword {\n color: #102005;"), "{css}");
        assert!(css.contains(".hl-string, "), "{css}");
        assert!(!css.contains("#abcd"), "{css}");
    }

    #[test]
    fn rust_fences_are_highlighted() {
        for lang in ["rust", "rs"] {
            assert!(super::code(Some(lang), "fn main() {}").starts_with("<pre class=\"hl\">"), "{lang}");
        }
    }
}
