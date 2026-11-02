//! Layered TOML configuration and renderer-neutral theme resolution.
mod color;
mod keys;
mod types;
pub use color::{ANSI_SLOTS, Ansi, Color};
pub use keys::*;
pub use types::*;

use serde::Deserialize;
use std::{
    collections::{BTreeMap, HashSet},
    fmt, fs,
    path::{Path, PathBuf},
};
use toml::Value;

pub const DEFAULT: &str = include_str!("../default.toml");

#[derive(Clone, Debug)]
pub struct Error {
    pub file: PathBuf,
    pub key: String,
    message: String,
}
impl Error {
    pub fn new(file: impl Into<PathBuf>, key: impl Into<String>, message: impl ToString) -> Self {
        Self {
            file: file.into(),
            key: key.into(),
            message: message.to_string(),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        write!(
            f,
            "{}: {}: {}",
            one_line(&self.file.to_string_lossy()),
            one_line(&self.key),
            one_line(&self.message)
        )
    }
}
impl std::error::Error for Error {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    web: WebSettings,
    #[serde(default)]
    host: Vec<HostConfig>,
    tui: TuiSettings,
    keys: Keys,
    theme: ThemeSettings,
    status: Vec<StatusConfig>,
    chip: Vec<ChipConfig>,
    tab: Vec<TabConfig>,
}

/// The sections a worktree `.spec-viewer.toml` may set.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLayer {
    tab: Option<Vec<TabConfig>>,
    status: Option<Vec<StatusConfig>>,
    chip: Option<Vec<ChipConfig>>,
}

const DEFAULT_FILE: &str = "crates/config/default.toml";

/// The embedded default merged with one host's user config.
#[derive(Clone, Debug)]
pub struct Config {
    pub web: WebSettings,
    pub hosts: Vec<HostConfig>,
    pub tui: TuiSettings,
    pub keys: Keys,
    pub theme: ThemeSettings,
    /// The default's project sections replaced by the user config's.
    pub parse: ParseConfig,
    /// The project sections the user config sets.
    pub layer: Layer,
    origins: BTreeMap<String, PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        Self::from_layers(&[]).expect("embedded default config")
    }
}
impl Config {
    /// `$XDG_CONFIG_HOME/spec-viewer/config.toml`, else `~/.config/spec-viewer/config.toml`.
    pub fn user_file() -> Option<PathBuf> {
        user_dir().map(|dir| dir.join("config.toml"))
    }
    /// This host's user config.
    pub fn load() -> Result<Self, Error> {
        Self::load_file(Self::user_file().as_deref())
    }
    /// A missing file leaves the embedded default.
    pub fn load_file(user: Option<&Path>) -> Result<Self, Error> {
        let mut layers = Vec::new();
        if let Some(path) = user {
            match fs::read_to_string(path) {
                Ok(source) => layers.push((path.to_path_buf(), source)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::new(path, "config", error)),
            }
        }
        Self::from_layers(&layers)
    }
    fn from_layers(layers: &[(PathBuf, String)]) -> Result<Self, Error> {
        let mut value = Value::Table(Default::default());
        let mut origins = BTreeMap::new();
        let mut user = Value::Table(Default::default());
        for (path, source) in
            std::iter::once((Path::new(DEFAULT_FILE), DEFAULT)).chain(layers.iter().map(|(p, s)| (p.as_path(), s.as_str())))
        {
            let layer = parse_toml(path, source)?;
            if path != Path::new(DEFAULT_FILE) {
                user = layer.clone();
            }
            merge(&mut value, layer, "", path, &mut origins);
        }
        // serde's flatten loses the offending key for a dynamic palette name.
        // Validate these shapes on the merged value to retain file/key diagnostics.
        if let Some(palette) = value.get("theme").and_then(|theme| theme.get("palette")).and_then(Value::as_table) {
            for (name, colour) in palette {
                let key = format!("theme.palette.{name}");
                if matches!(name.as_str(), "light" | "dark") {
                    let table = colour
                        .as_table()
                        .ok_or_else(|| Error::new(origin(&origins, &key), &key, "expected a palette table"))?;
                    for (name, colour) in table {
                        let key = format!("{key}.{name}");
                        if !colour.is_str() {
                            return Err(Error::new(origin(&origins, &key), key, "expected a colour string"));
                        }
                    }
                } else if !colour.is_str() {
                    return Err(Error::new(origin(&origins, &key), key, "expected a colour string"));
                }
            }
        }
        let raw: Raw = deserialize(value, |key| origin(&origins, key))?;
        let set = |section: &str| user.get(section).is_some();
        let config = Self {
            web: raw.web,
            hosts: raw.host,
            tui: raw.tui,
            keys: raw.keys,
            theme: raw.theme,
            layer: Layer {
                tabs: set("tab").then(|| raw.tab.clone()),
                statuses: set("status").then(|| raw.status.clone()),
                chips: set("chip").then(|| raw.chip.clone()),
            },
            parse: ParseConfig {
                statuses: raw.status,
                chips: raw.chip,
                tabs: raw.tab,
            },
            origins,
        };
        config.validate()?;
        Ok(config)
    }
    /// Attach an adapter validation failure (e.g. syntect theme decoding) to its defining file/key.
    pub fn error(&self, key: &str, message: impl ToString) -> Error {
        Error::new(origin(&self.origins, key), key, message)
    }
    pub fn mode(&self, override_mode: Option<Mode>) -> Mode {
        override_mode.unwrap_or(self.theme.mode)
    }
    fn name(&self, mode: Mode) -> &str {
        match mode {
            Mode::Light => self.theme.light_name.as_deref(),
            _ => self.theme.dark_name.as_deref(),
        }
        .unwrap_or(&self.theme.name)
    }
    fn palette(&self, mode: Mode) -> BTreeMap<String, Color> {
        let mut palette = builtin(self.name(mode)).expect("validated built-in theme");
        let specific = if mode == Mode::Light {
            &self.theme.palette.light
        } else {
            &self.theme.palette.dark
        };
        for (name, value) in self.theme.palette.colors.iter().chain(specific) {
            palette.insert(name.clone(), Color::parse(value).expect("validated colour"));
        }
        palette
    }
    fn validate(&self) -> Result<(), Error> {
        let mut names = HashSet::new();
        for (index, host) in self.hosts.iter().enumerate() {
            match (&host.ssh, &host.command) {
                (Some(_), Some(_)) => return Err(self.error(&format!("host[{index}].command"), "set exactly one of ssh and command")),
                (None, None) => return Err(self.error(&format!("host[{index}]"), "set exactly one of ssh and command")),
                (None, Some(command)) if command.is_empty() => {
                    return Err(self.error(&format!("host[{index}].command"), "must not be empty"));
                }
                _ => {}
            }
            if host.name.trim().is_empty() || !names.insert(host.name.as_str()) {
                return Err(self.error(
                    &format!("host[{index}].name"),
                    format!("empty or duplicate host name {:?}", host.name),
                ));
            }
        }
        for (key, name) in [
            ("theme.name", Some(&self.theme.name)),
            ("theme.light_name", self.theme.light_name.as_ref()),
            ("theme.dark_name", self.theme.dark_name.as_ref()),
        ] {
            if let Some(name) = name
                && builtin(name).is_none()
            {
                return Err(self.error(key, format!("unknown theme {name:?}")));
            }
        }
        for (prefix, palette) in [
            ("theme.palette", &self.theme.palette.colors),
            ("theme.palette.light", &self.theme.palette.light),
            ("theme.palette.dark", &self.theme.palette.dark),
        ] {
            for (name, value) in palette {
                if Color::parse(value).is_none() {
                    return Err(self.error(&format!("{prefix}.{name}"), format!("malformed colour {value:?}")));
                }
            }
        }
        match self.keys.problem() {
            Some((key, None, message)) => return Err(self.error(&key, message)),
            Some((key, Some(other), label)) => {
                // Blame the binding a config file set, not the default it collides with.
                let (key, other) = if origin(&self.origins, &key) == Path::new(DEFAULT_FILE) {
                    (other, key)
                } else {
                    (key, other)
                };
                return Err(self.error(&key, format!("{label} is also bound to {other}")));
            }
            None => {}
        }
        if self.theme.roles.headings.is_empty() {
            return Err(self.error("theme.roles.headings", "must contain at least one colour"));
        }
        validate_statuses(&self.parse.statuses, |key, message| self.error(key, message))?;
        validate_tabs(&self.parse.tabs, |key, message| self.error(key, message))?;
        // A name may be added only for one mode. Validate that it exists here;
        // resolve() checks the palette for the mode actually selected (including CLI overrides).
        let palettes = [self.palette(Mode::Light), self.palette(Mode::Dark)];
        let exists = |name: &str| palettes.iter().any(|palette| palette.contains_key(name));
        self.theme.roles.try_map(|key, name| {
            if exists(name) {
                Ok(())
            } else {
                Err(self.error(&format!("theme.roles.{key}"), format!("unknown palette colour {name:?}")))
            }
        })?;
        for (index, status) in self.parse.statuses.iter().enumerate() {
            if !exists(&status.color) {
                return Err(self.error(
                    &format!("status[{index}].color"),
                    format!("unknown palette colour {:?}", status.color),
                ));
            }
        }
        Ok(())
    }
    /// The colour of each of `statuses` in `mode`, leaving out those naming no palette colour.
    pub fn status_colors(&self, mode: Mode, statuses: &[StatusConfig]) -> BTreeMap<String, Color> {
        let mode = if mode == Mode::Auto { Mode::Dark } else { mode };
        let palette = self.palette(mode);
        statuses
            .iter()
            .filter_map(|status| Some((status.name.clone(), *palette.get(&status.color)?)))
            .collect()
    }
    pub fn resolve(&self, mode: Mode) -> Result<ResolvedTheme, Error> {
        let mode = if mode == Mode::Auto { Mode::Dark } else { mode };
        let palette = self.palette(mode);
        let colour = |key: &str, name: &str| {
            palette
                .get(name)
                .copied()
                .ok_or_else(|| self.error(key, format!("unknown palette colour {name:?} in {mode:?} mode")))
        };
        let roles = self.theme.roles.try_map(|key, name| colour(&format!("theme.roles.{key}"), name))?;
        let statuses = self
            .parse
            .statuses
            .iter()
            .enumerate()
            .map(|(index, status)| Ok((status.name.clone(), colour(&format!("status[{index}].color"), &status.color)?)))
            .collect::<Result<_, Error>>()?;
        let ansi = ANSI_SLOTS.map(|(name, slot)| palette.get(name).copied().unwrap_or(Color::Ansi(slot)));
        Ok(ResolvedTheme {
            mode,
            roles,
            ansi,
            statuses,
        })
    }
}

impl Layer {
    /// The project sections of a worktree `.spec-viewer.toml`; `None` when `file` does not exist. Any section other than `[[tab]]`, `[[status]]`, and
    /// `[[chip]]` is an error naming the file.
    pub fn read(file: &Path) -> Result<Option<Self>, Error> {
        let source = match fs::read_to_string(file) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Error::new(file, "config", error)),
        };
        let value = parse_toml(file, &source)?;
        for key in value.as_table().into_iter().flat_map(|table| table.keys()) {
            if USER_SECTIONS.contains(&key.as_str()) {
                return Err(Error::new(file, key, "only allowed in the user config"));
            }
        }
        let raw: RawLayer = deserialize(value, |_| file.to_path_buf())?;
        let layer = Self {
            tabs: raw.tab,
            statuses: raw.status,
            chips: raw.chip,
        };
        let error = |key: &str, message: String| Error::new(file, key, message);
        if let Some(statuses) = &layer.statuses {
            validate_statuses(statuses, error)?;
        }
        if let Some(tabs) = &layer.tabs {
            validate_tabs(tabs, error)?;
        }
        Ok(Some(layer))
    }
}

/// The daemon and frontend sections, which a worktree file may not set.
const USER_SECTIONS: [&str; 5] = ["web", "host", "keys", "theme", "tui"];

fn parse_toml(path: &Path, source: &str) -> Result<Value, Error> {
    toml::from_str::<Value>(source).map_err(|e| {
        let offset = e.span().map_or(0, |span| span.start);
        let line = source.as_bytes()[..offset].iter().filter(|&&byte| byte == b'\n').count() + 1;
        Error::new(path, format!("line {line}"), e.message())
    })
}

fn deserialize<T: serde::de::DeserializeOwned>(value: Value, origin: impl Fn(&str) -> PathBuf) -> Result<T, Error> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        let mut key = e.path().to_string();
        let message = e.inner().to_string();
        if let Some(rest) = message.split("unknown field `").nth(1) {
            let field = rest.split('`').next().unwrap_or("");
            if key == "." {
                field.clone_into(&mut key);
            } else if !key.ends_with(&format!(".{field}")) && key != field {
                key = format!("{key}.{field}");
            }
        }
        Error::new(origin(&key), key, message)
    })
}

fn validate_statuses(statuses: &[StatusConfig], error: impl Fn(&str, String) -> Error) -> Result<(), Error> {
    let mut names = HashSet::new();
    for (index, status) in statuses.iter().enumerate() {
        for (key, values) in [("name", std::slice::from_ref(&status.name)), ("aliases", status.aliases.as_slice())] {
            for value in values {
                if value.trim().is_empty() || !names.insert(value.to_lowercase()) {
                    return Err(error(
                        &format!("status[{index}].{key}"),
                        format!("empty or duplicate status name/alias {value:?}"),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_tabs(tabs: &[TabConfig], error: impl Fn(&str, String) -> Error) -> Result<(), Error> {
    let mut tab_names = HashSet::new();
    for (index, tab) in tabs.iter().enumerate() {
        let name = tab.name.trim().to_lowercase();
        if name.is_empty() || !tab_names.insert(name) {
            return Err(error(
                &format!("tab[{index}].name"),
                format!("empty or duplicate tab name {:?}", tab.name),
            ));
        }
        if let Some(folder) = &tab.folder
            && !folder.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
        {
            return Err(error(
                &format!("tab[{index}].folder"),
                format!("expected a relative folder like \".plan\", got {folder:?}"),
            ));
        }
        if let Some(other) = tabs[..index].iter().position(|earlier| earlier.folder == tab.folder) {
            return Err(error(
                &format!("tab[{index}].folder"),
                format!("same folder as tab[{other}]: {:?}", tab.root()),
            ));
        }
        let mut kinds = HashSet::new();
        for (key, values) in [("kinds", tab.kinds.as_deref()), ("companions", Some(tab.companions.as_slice()))] {
            let values = values.unwrap_or_default();
            if key == "kinds" && tab.kinds.is_some() && values.is_empty() {
                return Err(error(&format!("tab[{index}].kinds"), "omit it to list every file".into()));
            }
            for value in values {
                if value.is_empty() || value.contains(['.', '/']) || !kinds.insert(value.as_str()) {
                    return Err(error(
                        &format!("tab[{index}].{key}"),
                        format!("empty, duplicate, or dotted kind {value:?}"),
                    ));
                }
            }
        }
    }
    // Keys 1-9 select tabs.
    if !(1..=9).contains(&tabs.len()) {
        return Err(error("tab", "expected 1 to 9 tabs".into()));
    }
    Ok(())
}

/// `$XDG_CONFIG_HOME/spec-viewer`, else `~/.config/spec-viewer`.
fn user_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|dir| dir.join("spec-viewer"))
}

fn origin(origins: &BTreeMap<String, PathBuf>, key: &str) -> PathBuf {
    let mut key = key;
    loop {
        if let Some(path) = origins.get(key) {
            return path.clone();
        }
        match key.rfind(['.', '[']) {
            Some(i) => key = &key[..i],
            None => return PathBuf::from(DEFAULT_FILE),
        }
    }
}
fn merge(lower: &mut Value, upper: Value, key: &str, file: &Path, origins: &mut BTreeMap<String, PathBuf>) {
    if let (Value::Table(low), Value::Table(high)) = (&mut *lower, &upper) {
        origins.insert(key.to_owned(), file.to_owned());
        for (name, value) in high {
            let child = if key.is_empty() { name.clone() } else { format!("{key}.{name}") };
            merge(
                low.entry(name.clone()).or_insert_with(|| Value::Table(Default::default())),
                value.clone(),
                &child,
                file,
                origins,
            );
        }
    } else {
        origins.retain(|name, _| name != key && !name.starts_with(&format!("{key}.")));
        origins.insert(key.to_owned(), file.to_owned());
        *lower = upper;
    }
}
fn builtin(name: &str) -> Option<BTreeMap<String, Color>> {
    (name == "terminal").then(|| {
        ANSI_SLOTS
            .iter()
            .map(|(name, slot)| (name.to_string(), Color::Ansi(*slot)))
            .collect()
    })
}
