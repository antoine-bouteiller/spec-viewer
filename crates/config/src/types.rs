use crate::Color;
use serde::Deserialize;
use std::collections::BTreeMap;

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

#[derive(Clone, Debug)]
pub struct ParseConfig {
    pub statuses: Vec<StatusConfig>,
    pub chips: Vec<ChipConfig>,
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
    pub code_light: Option<String>,
    pub code_dark: Option<String>,
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
    pub annotation: T,
    pub warning: T,
    pub headings: Vec<T>,
    pub meter_progress: T,
    pub meter_complete: T,
    pub hint_fg: T,
    pub hint_bg: T,
    pub status_unknown: T,
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
    pub statuses: BTreeMap<String, Color>,
}
