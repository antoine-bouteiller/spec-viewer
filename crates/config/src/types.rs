use crate::Color;
use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusConfig {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub color: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChipKind {
    Checklist,
    Headings,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChipConfig {
    pub label: String,
    pub kind: ChipKind,
    pub sections: Vec<String>,
}

const EXTENSIONS: [&str; 2] = ["md", "mdx"];

/// A sidebar tab listing the `.md` and `.mdx` files below `folder`, as a path tree rooted there.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabConfig {
    pub name: String,
    /// Worktree-relative; `None` is the whole worktree, listed through Git so ignored files stay out.
    #[serde(default)]
    pub folder: Option<String>,
    /// The `<kind>` of `<name>.<kind>.md` files listed as documents; `None` lists every file.
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    /// Kinds nested under the document sharing their stem.
    #[serde(default)]
    pub companions: Vec<String>,
}
impl TabConfig {
    /// The worktree-relative folder, `""` for the whole worktree.
    pub fn root(&self) -> &str {
        self.folder.as_deref().unwrap_or("")
    }
    fn contains(&self, path: &str) -> bool {
        self.folder
            .as_ref()
            .is_none_or(|folder| path.strip_prefix(folder.as_str()).is_some_and(|rest| rest.starts_with('/')))
    }
    fn suffixed<'a>(path: &'a str, kinds: &[String]) -> Option<(&'a str, &'a str)> {
        kinds.iter().find_map(|kind| {
            EXTENSIONS.iter().find_map(|ext| {
                let stem = path.strip_suffix(&format!(".{kind}.{ext}"))?;
                Some((stem, &path[stem.len() + 1..stem.len() + 1 + kind.len()]))
            })
        })
    }
    /// The companion kind of `path`, if it is one.
    pub fn companion<'a>(&self, path: &'a str) -> Option<&'a str> {
        Self::suffixed(path, &self.companions).map(|(_, kind)| kind)
    }
    /// `path` without its `.<kind>.<ext>` or `.<ext>` suffix.
    pub fn stem<'a>(&self, path: &'a str) -> &'a str {
        if let Some((stem, _)) =
            Self::suffixed(path, &self.companions).or_else(|| self.kinds.as_deref().and_then(|kinds| Self::suffixed(path, kinds)))
        {
            return stem;
        }
        EXTENSIONS
            .iter()
            .find_map(|ext| path.strip_suffix(&format!(".{ext}")))
            .unwrap_or(path)
    }
    /// The paths a document with stem `stem` may have.
    pub fn documents(&self, stem: &str) -> Vec<String> {
        let kinds = self.kinds.iter().flatten().map(|kind| format!(".{kind}"));
        kinds
            .chain([String::new()])
            .flat_map(|kind| EXTENSIONS.map(|ext| format!("{stem}{kind}.{ext}")))
            .collect()
    }
    fn accepts(&self, path: &str) -> bool {
        match &self.kinds {
            Some(kinds) => Self::suffixed(path, kinds).is_some() || self.companion(path).is_some(),
            None => EXTENSIONS.iter().any(|ext| path.ends_with(&format!(".{ext}"))),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ParseConfig {
    pub statuses: Vec<StatusConfig>,
    pub chips: Vec<ChipConfig>,
    pub tabs: Vec<TabConfig>,
}
impl ParseConfig {
    /// The tab listing worktree-relative `path`: the one with the deepest folder containing it, if it accepts the file.
    pub fn tab_of(&self, path: &str) -> Option<usize> {
        let (index, tab) = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| tab.contains(path))
            .max_by_key(|(_, tab)| tab.root().len())?;
        tab.accepts(path).then_some(index)
    }
}
impl Default for ParseConfig {
    fn default() -> Self {
        crate::Config::default().parse
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSettings {
    pub mode: Mode,
    pub name: String,
    pub light_name: Option<String>,
    pub dark_name: Option<String>,
    pub palette: Palette,
    pub roles: Roles<String>,
}
// Arbitrary palette names are allowed; light and dark are the only nested tables.
#[derive(Clone, Debug, Deserialize)]
pub struct Palette {
    pub light: BTreeMap<String, String>,
    pub dark: BTreeMap<String, String>,
    #[serde(flatten)]
    pub colors: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roles<T> {
    pub accent: T,
    pub muted: T,
    pub inline_code: T,
    pub code_bg: T,
    pub annotation: T,
    pub warning: T,
    pub headings: Vec<T>,
    pub meter_progress: T,
    pub meter_complete: T,
    pub hint_fg: T,
    pub hint_bg: T,
    pub status_unknown: T,
    pub highlight_bg: T,
    pub current_bg: T,
    pub xref: XrefRoles<T>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XrefRoles<T> {
    pub g: T,
    pub ng: T,
    pub pi: T,
    pub kd: T,
    pub so: T,
    pub c: T,
    pub ct: T,
    pub vc: T,
    pub oq: T,
    pub ex: T,
    pub other: T,
}
impl<T> Roles<T> {
    pub fn try_map<U, E>(&self, mut f: impl FnMut(&str, &T) -> Result<U, E>) -> Result<Roles<U>, E> {
        Ok(Roles {
            accent: f("accent", &self.accent)?,
            muted: f("muted", &self.muted)?,
            inline_code: f("inline_code", &self.inline_code)?,
            code_bg: f("code_bg", &self.code_bg)?,
            annotation: f("annotation", &self.annotation)?,
            warning: f("warning", &self.warning)?,
            headings: self
                .headings
                .iter()
                .enumerate()
                .map(|(i, v)| f(&format!("headings[{i}]"), v))
                .collect::<Result<_, _>>()?,
            meter_progress: f("meter_progress", &self.meter_progress)?,
            meter_complete: f("meter_complete", &self.meter_complete)?,
            hint_fg: f("hint_fg", &self.hint_fg)?,
            hint_bg: f("hint_bg", &self.hint_bg)?,
            status_unknown: f("status_unknown", &self.status_unknown)?,
            highlight_bg: f("highlight_bg", &self.highlight_bg)?,
            current_bg: f("current_bg", &self.current_bg)?,
            xref: XrefRoles {
                g: f("xref.g", &self.xref.g)?,
                ng: f("xref.ng", &self.xref.ng)?,
                pi: f("xref.pi", &self.xref.pi)?,
                kd: f("xref.kd", &self.xref.kd)?,
                so: f("xref.so", &self.xref.so)?,
                c: f("xref.c", &self.xref.c)?,
                ct: f("xref.ct", &self.xref.ct)?,
                vc: f("xref.vc", &self.xref.vc)?,
                oq: f("xref.oq", &self.xref.oq)?,
                ex: f("xref.ex", &self.xref.ex)?,
                other: f("xref.other", &self.xref.other)?,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct ResolvedTheme {
    pub mode: Mode,
    pub roles: Roles<Color>,
    /// The palette's colour for each ANSI slot, in `ANSI_SLOTS` order.
    pub ansi: [Color; 16],
    pub statuses: BTreeMap<String, Color>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSettings {
    pub bind: String,
    pub cert: Option<PathBuf>,
    pub key: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiSettings {
    pub diff: bool,
}
