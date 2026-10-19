//! Class-based syntax highlighting, with one Catppuccin stylesheet per colour mode.
use std::sync::LazyLock;

use syntect::{
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

/// Token rules for one mode; the caller scopes them so the two modes' specificities never compete.
pub(crate) fn rules(dark: bool) -> String {
    let name = if dark {
        EmbeddedThemeName::CatppuccinMocha
    } else {
        EmbeddedThemeName::CatppuccinLatte
    };
    css_for_theme_with_class_style(two_face::theme::extra().get(name), STYLE).expect("embedded Catppuccin theme")
}
