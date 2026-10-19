//! Resolve renderer-neutral configuration once.
use ratatui::style::Color;
use spec_viewer_config::{Ansi, Color as ConfigColor, Config, Error, Mode, ResolvedTheme, Roles};
use spec_viewer_core::Status;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Theme {
    pub mode: Mode,
    pub roles: Roles<Color>,
    /// Palette colour per ANSI slot; code tokens are coloured through it.
    pub ansi: [Color; 16],
    statuses: BTreeMap<String, Color>,
    auto_light: Option<Box<Theme>>,
}
impl Default for Theme {
    fn default() -> Self {
        Self::from_config(&Config::default(), Some(Mode::Dark)).expect("default theme")
    }
}
impl Theme {
    pub fn from_config(config: &Config, override_mode: Option<Mode>) -> Result<Self, Error> {
        match config.mode(override_mode) {
            Mode::Light => Self::resolved(config, Mode::Light),
            Mode::Dark => Self::resolved(config, Mode::Dark),
            Mode::Auto => {
                let mut dark = Self::resolved(config, Mode::Dark)?;
                dark.auto_light = Some(Box::new(Self::resolved(config, Mode::Light)?));
                Ok(dark)
            }
        }
    }
    fn resolved(config: &Config, mode: Mode) -> Result<Self, Error> {
        let ResolvedTheme { roles, ansi, statuses, .. } = config.resolve(mode)?;
        Ok(Self {
            mode,
            ansi: ansi.map(color_to_terminal),
            roles: roles
                .try_map(|_, color| Ok::<_, std::convert::Infallible>(color_to_terminal(*color)))
                .unwrap(),
            statuses: statuses.into_iter().map(|(name, color)| (name, color_to_terminal(color))).collect(),
            auto_light: None,
        })
    }
    pub(crate) fn is_auto(&self) -> bool {
        self.auto_light.is_some()
    }
    pub(crate) fn resolve_auto(mut self, detected: Option<Mode>) -> Self {
        if let Some(light) = self.auto_light.take()
            && detected == Some(Mode::Light)
        {
            return *light;
        }
        self
    }
    pub fn status_color(&self, status: Option<&Status>) -> Color {
        status
            .filter(|status| status.known)
            .and_then(|status| self.statuses.get(&status.name))
            .copied()
            .unwrap_or(self.roles.status_unknown)
    }
    pub fn family_color(&self, family: &str) -> Color {
        let xref = &self.roles.xref;
        match family {
            "g" => xref.g,
            "ng" => xref.ng,
            "pi" => xref.pi,
            "kd" => xref.kd,
            "so" => xref.so,
            "c" => xref.c,
            "ct" => xref.ct,
            "vc" => xref.vc,
            "oq" => xref.oq,
            "ex" => xref.ex,
            _ => xref.other,
        }
    }
    pub fn heading_color(&self, depth: u8) -> Color {
        self.roles.headings[usize::from(depth.saturating_sub(1)).min(self.roles.headings.len() - 1)]
    }
}

pub fn color_to_terminal(color: ConfigColor) -> Color {
    match color {
        ConfigColor::Reset => Color::Reset,
        ConfigColor::Rgb(r, g, b) => Color::Rgb(r, g, b),
        ConfigColor::Ansi(slot) => match slot {
            Ansi::Black => Color::Black,
            Ansi::Red => Color::Red,
            Ansi::Green => Color::Green,
            Ansi::Yellow => Color::Yellow,
            Ansi::Blue => Color::Blue,
            Ansi::Magenta => Color::Magenta,
            Ansi::Cyan => Color::Cyan,
            Ansi::White => Color::Gray,
            Ansi::BrightBlack => Color::DarkGray,
            Ansi::BrightRed => Color::LightRed,
            Ansi::BrightGreen => Color::LightGreen,
            Ansi::BrightYellow => Color::LightYellow,
            Ansi::BrightBlue => Color::LightBlue,
            Ansi::BrightMagenta => Color::LightMagenta,
            Ansi::BrightCyan => Color::LightCyan,
            Ansi::BrightWhite => Color::White,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vc_1_default_theme_reproduces_terminal_colours() {
        for mode in [Mode::Dark, Mode::Light] {
            let theme = Theme::from_config(&Config::default(), Some(mode)).unwrap();
            let roles = &theme.roles;
            for (name, actual, expected) in [
                ("accent", roles.accent, Color::Blue),
                ("muted", roles.muted, Color::DarkGray),
                ("inline_code", roles.inline_code, Color::Yellow),
                ("annotation", roles.annotation, Color::Yellow),
                ("warning", roles.warning, Color::Yellow),
                ("meter_progress", roles.meter_progress, Color::Cyan),
                ("meter_complete", roles.meter_complete, Color::Green),
                ("hint_fg", roles.hint_fg, Color::Black),
                ("hint_bg", roles.hint_bg, Color::Yellow),
                ("status_unknown", roles.status_unknown, Color::DarkGray),
            ] {
                assert_eq!(actual, expected, "{mode:?}: {name}");
            }
            assert_eq!(
                roles.headings,
                [Color::Magenta, Color::Blue, Color::Cyan, Color::Green, Color::Yellow, Color::Red]
            );
            let statuses = [
                ("draft", Color::Yellow),
                ("amended", Color::Yellow),
                ("ready", Color::Blue),
                ("accepted", Color::Blue),
                ("converging", Color::Blue),
                ("in-progress", Color::Cyan),
                ("implementing", Color::Cyan),
                ("blocked", Color::Red),
                ("done", Color::Green),
            ];
            assert_eq!(theme.statuses.len(), statuses.len());
            for (name, expected) in statuses {
                assert_eq!(
                    theme.status_color(Some(&Status {
                        name: name.into(),
                        known: true
                    })),
                    expected,
                    "{mode:?}: {name}"
                );
            }
            assert_eq!(theme.status_color(None), Color::DarkGray);
            for (family, expected) in [
                ("g", Color::Cyan),
                ("ng", Color::Red),
                ("pi", Color::Magenta),
                ("kd", Color::Blue),
                ("so", Color::Yellow),
                ("c", Color::Yellow),
                ("ct", Color::LightBlue),
                ("vc", Color::Green),
                ("oq", Color::LightMagenta),
                ("ex", Color::LightCyan),
                ("other", Color::DarkGray),
            ] {
                assert_eq!(theme.family_color(family), expected, "{mode:?}: {family}");
            }
        }
    }

    #[test]
    fn vc_6_ansi_slot_mapping_preserves_terminal_colours() {
        for (slot, expected) in [
            (Ansi::Black, Color::Black),
            (Ansi::Red, Color::Red),
            (Ansi::Green, Color::Green),
            (Ansi::Yellow, Color::Yellow),
            (Ansi::Blue, Color::Blue),
            (Ansi::Magenta, Color::Magenta),
            (Ansi::Cyan, Color::Cyan),
            (Ansi::White, Color::Gray),
            (Ansi::BrightBlack, Color::DarkGray),
            (Ansi::BrightRed, Color::LightRed),
            (Ansi::BrightGreen, Color::LightGreen),
            (Ansi::BrightYellow, Color::LightYellow),
            (Ansi::BrightBlue, Color::LightBlue),
            (Ansi::BrightMagenta, Color::LightMagenta),
            (Ansi::BrightCyan, Color::LightCyan),
            (Ansi::BrightWhite, Color::White),
        ] {
            assert_eq!(color_to_terminal(ConfigColor::Ansi(slot)), expected);
        }
        assert_eq!(color_to_terminal(ConfigColor::Reset), Color::Reset);
    }

    #[test]
    fn vc_6_modes_select_mode_palettes_with_auto_fallback() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(".spec-viewer.toml"),
            "[theme]\nmode='auto'\n[theme.palette.light]\nblue='#1e66f5'",
        )
        .unwrap();
        let config = Config::load_layers(None, root.path(), None).unwrap();
        let auto = Theme::from_config(&config, None).unwrap();
        assert!(auto.is_auto());
        let dark = auto.clone().resolve_auto(None);
        assert_eq!(dark.mode, Mode::Dark);
        assert_eq!(dark.roles.accent, Color::Blue);
        assert_eq!(dark.roles.code_bg, Color::Rgb(0x1f, 0x1f, 0x22));
        assert_eq!(dark.ansi[4], Color::Blue);
        let light = auto.resolve_auto(Some(Mode::Light));
        assert_eq!(light.mode, Mode::Light);
        assert_eq!(light.roles.accent, Color::Rgb(0x1e, 0x66, 0xf5));
        assert_eq!(light.roles.code_bg, Color::Rgb(0xea, 0xea, 0xee));
        assert_eq!(light.ansi[4], Color::Rgb(0x1e, 0x66, 0xf5));
        let forced = Theme::from_config(&config, Some(Mode::Light)).unwrap();
        assert!(!forced.is_auto());
        assert_eq!(forced.resolve_auto(Some(Mode::Dark)).mode, Mode::Light);
    }
}
