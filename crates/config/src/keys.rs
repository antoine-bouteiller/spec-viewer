//! Keybindings in herdr's format: `action = "ctrl+k"` or `action = ["up", "k"]`, `[]` unbinds.
use serde::{Deserialize, Deserializer, de::Error as _};

/// A key, independent of the terminal or browser reporting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// The character as typed: `Char('Y')` is Shift+y.
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    /// Never set with `Code::Char`, whose character already carries Shift.
    pub shift: bool,
    pub cmd: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: Code,
    pub modifiers: Modifiers,
}
impl Key {
    pub fn new(code: Code, mut modifiers: Modifiers) -> Self {
        if matches!(code, Code::Char(_)) {
            modifiers.shift = false;
        }
        Self { code, modifiers }
    }
    /// The short label shown in key hints, like `C-u`, `S-Tab`, `↑`, or `Y`.
    pub fn label(&self) -> String {
        let Modifiers { ctrl, alt, shift, cmd } = self.modifiers;
        let mut label: String = [(cmd, "Cmd-"), (ctrl, "C-"), (alt, "M-"), (shift, "S-")]
            .iter()
            .filter(|(on, _)| *on)
            .map(|(_, prefix)| *prefix)
            .collect();
        label.push_str(&match self.code {
            Code::Char(' ') => "Space".into(),
            Code::Char(character) => character.to_string(),
            Code::Enter => "Enter".into(),
            Code::Tab => "Tab".into(),
            Code::Esc => "Esc".into(),
            Code::Backspace => "BS".into(),
            Code::Delete => "Del".into(),
            Code::Insert => "Ins".into(),
            Code::Up => "↑".into(),
            Code::Down => "↓".into(),
            Code::Left => "←".into(),
            Code::Right => "→".into(),
            Code::Home => "Home".into(),
            Code::End => "End".into(),
            Code::PageUp => "PgUp".into(),
            Code::PageDown => "PgDn".into(),
            Code::F(number) => format!("F{number}"),
        });
        label
    }
}

/// One configured key; `range` is herdr's `1..9`, matching the digits 1 to 9.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key: Key,
    pub range: bool,
}
impl Chord {
    /// The 0-based index this chord matches `key` at: always 0 without a range.
    pub fn index(&self, key: &Key) -> Option<usize> {
        if self.range {
            match key.code {
                Code::Char(digit @ '1'..='9') if key.modifiers == self.key.modifiers => Some(usize::from(digit as u8 - b'1')),
                _ => None,
            }
        } else {
            (*key == self.key).then_some(0)
        }
    }
    /// The key at 0-based `index` of a range, or the key itself.
    pub fn nth(&self, index: usize) -> Key {
        match self.range {
            true => Key::new(Code::Char(char::from(b'1' + index.min(8) as u8)), self.key.modifiers),
            false => self.key,
        }
    }
    /// A key both chords match.
    fn shared(&self, other: &Chord) -> Option<Key> {
        (0..if self.range { 9 } else { 1 })
            .map(|index| self.nth(index))
            .find(|key| other.index(key).is_some())
    }
}
impl std::str::FromStr for Chord {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, String> {
        let invalid = |why: &str| format!("invalid key {text:?}: {why}");
        // A trailing `+` is the key itself, as in `ctrl++`.
        let (modifiers, last) = match text.strip_suffix("++") {
            Some(rest) => (rest, "+"),
            None if text == "+" => ("", "+"),
            None => text.rsplit_once('+').map_or(("", text), |(modifiers, last)| (modifiers, last)),
        };
        let mut set = Modifiers::default();
        for modifier in modifiers.split('+').filter(|_| !modifiers.is_empty()) {
            let flag = match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut set.ctrl,
                "alt" | "option" | "meta" => &mut set.alt,
                "shift" => &mut set.shift,
                "cmd" | "super" | "command" => &mut set.cmd,
                _ => return Err(invalid(&format!("unknown modifier {modifier:?}"))),
            };
            if std::mem::replace(flag, true) {
                return Err(invalid(&format!("repeated modifier {modifier:?}")));
            }
        }
        if last == "1..9" {
            return Ok(Self {
                key: Key::new(Code::Char('1'), set),
                range: true,
            });
        }
        let mut characters = last.chars();
        let code = match (characters.next(), characters.next()) {
            (Some(character), None) => Code::Char(character),
            _ => match last.to_ascii_lowercase().as_str() {
                "enter" | "return" => Code::Enter,
                "tab" => Code::Tab,
                "esc" | "escape" => Code::Esc,
                "backspace" => Code::Backspace,
                "delete" | "del" => Code::Delete,
                "insert" => Code::Insert,
                "up" => Code::Up,
                "down" => Code::Down,
                "left" => Code::Left,
                "right" => Code::Right,
                "home" => Code::Home,
                "end" => Code::End,
                "pageup" | "pgup" => Code::PageUp,
                "pagedown" | "pgdown" | "pgdn" => Code::PageDown,
                "space" => Code::Char(' '),
                "plus" => Code::Char('+'),
                "minus" => Code::Char('-'),
                "comma" => Code::Char(','),
                "period" => Code::Char('.'),
                "slash" => Code::Char('/'),
                "backslash" => Code::Char('\\'),
                "backtick" => Code::Char('`'),
                "semicolon" => Code::Char(';'),
                name => match name.strip_prefix('f').and_then(|number| number.parse().ok()) {
                    Some(number @ 1..=24) => Code::F(number),
                    _ => return Err(invalid("unknown key name")),
                },
            },
        };
        let code = match code {
            Code::Char(character) if set.shift => {
                if !character.is_alphabetic() {
                    return Err(invalid("write the shifted character instead of shift+"));
                }
                Code::Char(character.to_uppercase().next().unwrap_or(character))
            }
            code => code,
        };
        Ok(Self {
            key: Key::new(code, set),
            range: false,
        })
    }
}

/// The keys bound to one action, in hint order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Binding(pub Vec<Chord>);
impl Binding {
    /// The 0-based index of the chord's match, `Some(0)` for a key without a range.
    pub fn index(&self, key: &Key) -> Option<usize> {
        self.0.iter().find_map(|chord| chord.index(key))
    }
    pub fn matches(&self, key: &Key) -> bool {
        self.index(key).is_some()
    }
    /// The first key's label; `None` when unbound.
    pub fn label(&self) -> Option<String> {
        self.0.first().map(|chord| chord.key.label())
    }
}
impl<'de> Deserialize<'de> for Binding {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            One(String),
            Many(Vec<String>),
        }
        let texts =
            match Raw::deserialize(deserializer).map_err(|_| D::Error::custom("expected a key string or an array of key strings"))? {
                Raw::One(text) => vec![text],
                Raw::Many(texts) => texts,
            };
        texts
            .iter()
            .map(|text| text.parse().map_err(D::Error::custom))
            .collect::<Result<_, _>>()
            .map(Binding)
    }
}

macro_rules! keymap {
    ($(#[$meta:meta])* $name:ident, $action:ident { $($field:ident => $variant:ident),* $(,)? } $(; $($table:ident: $type:ty),*)?) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            $(pub $field: Binding,)*
            $($(pub $table: $type,)*)?
        }
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $action {
            $($variant,)*
        }
        impl $name {
            /// The action `key` triggers and the chord's index (see `Chord::index`).
            pub fn action(&self, key: &Key) -> Option<($action, usize)> {
                $(if let Some(index) = self.$field.index(key) {
                    return Some(($action::$variant, index));
                })*
                None
            }
            /// `(config name, binding)` per action, in declaration order.
            pub fn entries(&self) -> Vec<(&'static str, &Binding)> {
                vec![$((stringify!($field), &self.$field),)*]
            }
        }
    };
}

keymap!(
    /// The `[keys]` table: the main screen's keys, then one table per dialog or mode.
    /// `force_quit` also applies in every dialog and mode.
    Keys, Action {
        force_quit => ForceQuit,
        quit => Quit,
        switch_tab => SwitchTab,
        worktrees => Worktrees,
        search => Search,
        diff => Diff,
        open_web => OpenWeb,
        toggle_files => ToggleFiles,
        toggle_panel => TogglePanel,
        toggle_focus => ToggleFocus,
        copy_path => CopyPath,
        copy_name => CopyName,
        hints => Hints,
        dismiss => Dismiss,
        page_up => PageUp,
        page_down => PageDown,
        top => Top,
        bottom => Bottom,
        up => Up,
        down => Down,
        left => Left,
        right => Right,
        open => Open,
    };
    search_dialog: SearchKeys,
    worktree_picker: PickerKeys,
    hint_mode: HintKeys
);
keymap!(
    /// Keys inside the search dialog; unbound printable characters type into the query.
    SearchKeys, SearchAction {
        up => Up,
        down => Down,
        open => Open,
        clear => Clear,
        delete_word => DeleteWord,
        delete_char => DeleteChar,
        close => Close,
    }
);
keymap!(
    PickerKeys, PickerAction {
        up => Up,
        down => Down,
        open => Open,
        close => Close,
    }
);
keymap!(
    /// Keys in hint mode; unbound characters type hint labels.
    HintKeys, HintAction {
        delete_char => DeleteChar,
        close => Close,
    }
);

impl Keys {
    /// `(key path, binding)` per context; each context also holds `force_quit`.
    fn contexts(&self) -> Vec<Vec<(String, &Binding)>> {
        let quit = || ("keys.force_quit".to_owned(), &self.force_quit);
        fn prefixed<'a>(prefix: &str, entries: Vec<(&'static str, &'a Binding)>) -> Vec<(String, &'a Binding)> {
            entries
                .into_iter()
                .map(|(name, binding)| (format!("{prefix}{name}"), binding))
                .collect()
        }
        let mut search = vec![quit()];
        search.extend(prefixed("keys.search_dialog.", self.search_dialog.entries()));
        let mut picker = vec![quit()];
        picker.extend(prefixed("keys.worktree_picker.", self.worktree_picker.entries()));
        let mut hints = vec![quit()];
        hints.extend(prefixed("keys.hint_mode.", self.hint_mode.entries()));
        vec![prefixed("keys.", self.entries()), search, picker, hints]
    }
    /// A misplaced range as `(path, None, message)`, or a key bound twice in one context as
    /// `(path, Some(other path), message)`.
    pub(crate) fn problem(&self) -> Option<(String, Option<String>, String)> {
        for context in self.contexts() {
            for (index, (path, binding)) in context.iter().enumerate() {
                let wants_range = path == "keys.switch_tab";
                if let Some(chord) = binding.0.iter().find(|chord| chord.range != wants_range) {
                    let message = if wants_range {
                        format!("expected a 1..9 range like \"alt+1..9\", got {:?}", chord.key.label())
                    } else {
                        "1..9 is only allowed in keys.switch_tab".to_owned()
                    };
                    return Some((path.clone(), None, message));
                }
                for (other, earlier) in &context[..index] {
                    if let Some(key) = binding
                        .0
                        .iter()
                        .find_map(|chord| earlier.0.iter().find_map(|old| old.shared(chord)))
                    {
                        return Some((path.clone(), Some(other.clone()), key.label()));
                    }
                }
            }
        }
        None
    }
}
