//! Colours are independent of any renderer or terminal library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ansi {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

pub const ANSI_SLOTS: [(&str, Ansi); 16] = [
    ("black", Ansi::Black),
    ("red", Ansi::Red),
    ("green", Ansi::Green),
    ("yellow", Ansi::Yellow),
    ("blue", Ansi::Blue),
    ("magenta", Ansi::Magenta),
    ("cyan", Ansi::Cyan),
    ("white", Ansi::White),
    ("bright_black", Ansi::BrightBlack),
    ("bright_red", Ansi::BrightRed),
    ("bright_green", Ansi::BrightGreen),
    ("bright_yellow", Ansi::BrightYellow),
    ("bright_blue", Ansi::BrightBlue),
    ("bright_magenta", Ansi::BrightMagenta),
    ("bright_cyan", Ansi::BrightCyan),
    ("bright_white", Ansi::BrightWhite),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Color {
    Ansi(Ansi),
    Rgb(u8, u8, u8),
    Reset,
}

impl Color {
    pub fn parse(value: &str) -> Option<Self> {
        if value == "reset" {
            return Some(Self::Reset);
        }
        if let Some((_, slot)) = ANSI_SLOTS.iter().find(|(name, _)| *name == value) {
            return Some(Self::Ansi(*slot));
        }
        if let Some(hex) = value.strip_prefix('#') {
            if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            return Some(Self::Rgb(
                u8::from_str_radix(&hex[0..2], 16).ok()?,
                u8::from_str_radix(&hex[2..4], 16).ok()?,
                u8::from_str_radix(&hex[4..6], 16).ok()?,
            ));
        }
        let rgb = value.strip_prefix("rgb(")?.strip_suffix(')')?;
        let channels = rgb
            .split(',')
            .map(|part| part.trim().parse::<u8>().ok())
            .collect::<Option<Vec<_>>>()?;
        match channels.as_slice() {
            [r, g, b] => Some(Self::Rgb(*r, *g, *b)),
            _ => None,
        }
    }
}
