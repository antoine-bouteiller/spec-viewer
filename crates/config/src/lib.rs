//! Layered TOML configuration and renderer-neutral theme resolution.
mod color;
mod ports;
mod types;
pub use color::{ANSI_SLOTS, Ansi, Color};
pub use ports::Ports;
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
    tui: TuiSettings,
    theme: ThemeSettings,
    status: Vec<StatusConfig>,
    chip: Vec<ChipConfig>,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub web: WebSettings,
    pub tui: TuiSettings,
    pub theme: ThemeSettings,
    pub parse: ParseConfig,
    origins: BTreeMap<String, PathBuf>,
}
impl Default for Config {
    fn default() -> Self {
        Self::from_layers(&[]).expect("embedded default config")
    }
}
impl Config {
    /// The caller supplies the containing worktree, discovered by source::project_of.
    pub fn load(worktree: &Path, explicit: Option<&Path>) -> Result<Self, Error> {
        let user = user_dir().map(|dir| dir.join("config.toml"));
        Self::load_layers(user.as_deref(), worktree, explicit)
    }
    pub fn load_layers(user: Option<&Path>, worktree: &Path, explicit: Option<&Path>) -> Result<Self, Error> {
        let local = worktree.join(".spec-viewer.toml");
        let mut layers = Vec::new();
        let user = user.into_iter().map(|p| (p, false, true));
        for (path, required, is_user) in user.chain([(&*local, false, false)]).chain(explicit.map(|p| (p, true, false))) {
            match fs::read_to_string(path) {
                Ok(source) => layers.push((path.to_path_buf(), source, is_user)),
                Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::new(path, if required { "--config" } else { "config" }, error)),
            }
        }
        Self::from_layers(&layers)
    }
    /// Each layer is `(file, source, is_user)`; only the user layer may set `[web]`.
    fn from_layers(layers: &[(PathBuf, String, bool)]) -> Result<Self, Error> {
        let mut value = Value::Table(Default::default());
        let mut origins = BTreeMap::new();
        for (path, source, is_user) in std::iter::once((Path::new("crates/config/default.toml"), DEFAULT, true))
            .chain(layers.iter().map(|(p, s, u)| (p.as_path(), s.as_str(), *u)))
        {
            let layer = toml::from_str::<Value>(source).map_err(|e| {
                let offset = e.span().map_or(0, |span| span.start);
                let line = source.as_bytes()[..offset].iter().filter(|&&byte| byte == b'\n').count() + 1;
                Error::new(path, format!("line {line}"), e.message())
            })?;
            // A worktree file must not open the server to the network.
            if !is_user && layer.get("web").is_some() {
                return Err(Error::new(path, "web", "only allowed in the user config"));
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
        let raw: Raw = serde_path_to_error::deserialize(value).map_err(|e| {
            let mut key = e.path().to_string();
            let message = e.inner().to_string();
            if let Some(rest) = message.split("unknown field `").nth(1) {
                let field = rest.split('`').next().unwrap_or("");
                key = if key == "." { field.to_owned() } else { format!("{key}.{field}") };
            }
            Error::new(origin(&origins, &key), key, message)
        })?;
        let mut web = raw.web;
        for (key, file) in [("web.cert", &mut web.cert), ("web.key", &mut web.key)] {
            if let Some(file) = file.as_mut()
                && let Some(dir) = origin(&origins, key).parent()
            {
                *file = dir.join(&*file);
            }
        }
        let config = Self {
            web,
            tui: raw.tui,
            theme: raw.theme,
            parse: ParseConfig {
                statuses: raw.status,
                chips: raw.chip,
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
        if self.web.bind.trim().is_empty() {
            return Err(self.error("web.bind", "must not be empty"));
        }
        match (&self.web.cert, &self.web.key) {
            (Some(_), None) => return Err(self.error("web.cert", "requires web.key")),
            (None, Some(_)) => return Err(self.error("web.key", "requires web.cert")),
            _ => {}
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
        if self.theme.roles.headings.is_empty() {
            return Err(self.error("theme.roles.headings", "must contain at least one colour"));
        }
        let mut names = HashSet::new();
        for (index, status) in self.parse.statuses.iter().enumerate() {
            for (key, values) in [("name", std::slice::from_ref(&status.name)), ("aliases", status.aliases.as_slice())] {
                for value in values {
                    if value.trim().is_empty() || !names.insert(value.to_lowercase()) {
                        return Err(self.error(
                            &format!("status[{index}].{key}"),
                            format!("empty or duplicate status name/alias {value:?}"),
                        ));
                    }
                }
            }
        }
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
            None => return PathBuf::from("crates/config/default.toml"),
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
