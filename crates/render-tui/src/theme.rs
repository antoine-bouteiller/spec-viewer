//! Resolve renderer-neutral configuration once, including syntect code themes.
use plan_viewer_config::{Ansi, Color as ConfigColor, Config, Error, Mode, ResolvedTheme, Roles};
use plan_viewer_core::Status;
use ratatui::style::Color;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Theme {
    pub mode: Mode,
    pub code: syntect::highlighting::Theme,
    pub palette: BTreeMap<String, Color>,
    pub roles: Roles<Color>,
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
        // Decode both configured code themes before terminal setup, including inactive paths.
        let code = |mode| {
            crate::highlight::load_theme(&config.code_theme(mode)).map_err(|error| {
                config.error(
                    if mode == Mode::Light {
                        "theme.code_light"
                    } else {
                        "theme.code_dark"
                    },
                    error,
                )
            })
        };
        let light_code = code(Mode::Light)?;
        let dark_code = code(Mode::Dark)?;
        match config.mode(override_mode) {
            Mode::Light => Self::resolved(config, Mode::Light, light_code),
            Mode::Dark => Self::resolved(config, Mode::Dark, dark_code),
            Mode::Auto => {
                let mut dark = Self::resolved(config, Mode::Dark, dark_code)?;
                dark.auto_light = Some(Box::new(Self::resolved(config, Mode::Light, light_code)?));
                Ok(dark)
            }
        }
    }
    fn resolved(config: &Config, mode: Mode, code: syntect::highlighting::Theme) -> Result<Self, Error> {
        let ResolvedTheme {
            palette, roles, statuses, ..
        } = config.resolve(mode)?;
        Ok(Self {
            mode,
            code,
            palette: palette.into_iter().map(|(name, color)| (name, color_to_terminal(color))).collect(),
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
    fn vc_8_ansi_slot_mapping_preserves_terminal_colours() {
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
    fn vc_8_modes_select_builtin_and_code_defaults_with_auto_fallback() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join(".plan-viewer.toml"),
            "[theme]\nname='terminal'\nlight_name='catppuccin-latte'\nmode='auto'",
        )
        .unwrap();
        let config = Config::load_layers(None, root.path(), None).unwrap();
        let auto = Theme::from_config(&config, None).unwrap();
        assert!(auto.is_auto());
        let dark = auto.clone().resolve_auto(None);
        assert_eq!(dark.mode, Mode::Dark);
        assert_eq!(dark.roles.accent, Color::Blue);
        assert_eq!(dark.code.name.as_deref(), Some("Catppuccin Mocha"));
        let light = auto.resolve_auto(Some(Mode::Light));
        assert_eq!(light.mode, Mode::Light);
        assert_eq!(light.roles.accent, Color::Rgb(0x1e, 0x66, 0xf5));
        assert_eq!(light.code.name.as_deref(), Some("Catppuccin Latte"));
        let forced = Theme::from_config(&config, Some(Mode::Light)).unwrap();
        assert!(!forced.is_auto());
        assert_eq!(forced.resolve_auto(Some(Mode::Dark)).mode, Mode::Light);
    }
}
