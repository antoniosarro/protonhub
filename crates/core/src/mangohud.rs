//! MangoHud's settings: its config files, read and written line by line so
//! lines ProtonHub doesn't know stay where they were, and the settings its
//! editor offers (`mangohud.toml`).
//!
//! The global file is `$XDG_CONFIG_HOME/MangoHud/MangoHud.conf`. A game's
//! own is in `MangoHud/protonhub/`, used through `MANGOHUD_CONFIGFILE` in
//! its launch options; MangoHud then reads that file instead of the
//! global one.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::launcher::Launcher;
use crate::paths;

/// The variable that names the file MangoHud reads instead of the global
/// one.
pub const CONFIG_VAR: &str = "MANGOHUD_CONFIGFILE";

/// A config file's lines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// `key=value`, or a bare `key`, which is `key=1`.
    Set { key: String, value: Option<String> },
    /// A comment or a blank line.
    Other(String),
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let lines = text
            .lines()
            .map(|line| {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    return Line::Other(line.to_owned());
                }
                match trimmed.split_once('=') {
                    Some((key, value)) => Line::Set {
                        key: key.trim().to_owned(),
                        value: Some(value.trim().to_owned()),
                    },
                    None => Line::Set {
                        key: trimmed.to_owned(),
                        value: None,
                    },
                }
            })
            .collect();
        Self { lines }
    }

    /// `key`'s value, the last line's if there are several, as MangoHud
    /// reads them; none without one.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|line| match line {
            Line::Set { key: k, value } if k == key => Some(value.as_deref().unwrap_or("1")),
            _ => None,
        })
    }

    /// Sets `key` to `value` in its first line, removing any others; none
    /// removes them all. `Some("1")` writes a bare `key`.
    pub fn set(&mut self, key: &str, value: Option<&str>) {
        let line = value.map(|v| Line::Set {
            key: key.to_owned(),
            value: (v != "1").then(|| v.to_owned()),
        });
        let at = self
            .lines
            .iter()
            .position(|l| matches!(l, Line::Set { key: k, .. } if k == key));
        self.lines
            .retain(|l| !matches!(l, Line::Set { key: k, .. } if k == key));
        if let Some(line) = line {
            self.lines.insert(at.unwrap_or(self.lines.len()), line);
        }
    }
}

impl std::fmt::Display for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        for line in &self.lines {
            match line {
                Line::Set { key, value: None } => writeln!(f, "{key}")?,
                Line::Set {
                    key,
                    value: Some(v),
                } => writeln!(f, "{key}={v}")?,
                Line::Other(text) => writeln!(f, "{text}")?,
            }
        }
        Ok(())
    }
}

/// A group of settings in the editor.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub title: String,
    /// Whether its settings are things the HUD shows, which in MangoHud's
    /// newer layout are off unless their line says on.
    #[serde(default)]
    pub elements: bool,
    pub setting: Vec<Setting>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub kind: Kind,
    /// What MangoHud does without the line.
    pub default: String,
    #[serde(default)]
    pub min: f64,
    #[serde(default)]
    pub max: f64,
    #[serde(default)]
    pub step: f64,
    #[serde(default)]
    pub choices: Vec<String>,
    /// Its group's `elements`.
    #[serde(skip)]
    pub element: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Switch,
    Number,
    Choice,
    Color,
    Text,
}

impl Setting {
    /// What MangoHud does without its line in `config`: with
    /// `legacy_layout=0`, the HUD shows only what's listed.
    pub fn default_in(&self, config: &Config) -> &str {
        let legacy = config.get("legacy_layout").is_none_or(is_on);
        if self.element && !legacy {
            "0"
        } else {
            &self.default
        }
    }

    /// Its value in `config`, else its default.
    pub fn value<'a>(&'a self, config: &'a Config) -> &'a str {
        config
            .get(&self.key)
            .unwrap_or_else(|| self.default_in(config))
    }

    /// Sets it to `value` in `config`, leaving it out at its default.
    pub fn set(&self, config: &mut Config, value: &str) {
        let default = self.default_in(config);
        let at_default = match self.kind {
            Kind::Number => value.parse::<f64>().ok() == default.parse().ok(),
            Kind::Color => value.eq_ignore_ascii_case(default),
            // An empty text is no setting.
            Kind::Text => value.is_empty() || value == default,
            Kind::Switch => is_on(value) == is_on(default),
            Kind::Choice => value == default,
        };
        config.set(&self.key, (!at_default).then_some(value));
    }
}

/// Whether a switch's value is on, read as MangoHud does (C's `strtol`):
/// by the number it starts with, so `false` is off.
pub fn is_on(value: &str) -> bool {
    let value = value.trim();
    let sign = value.starts_with(['-', '+']) as usize;
    let digits = value[sign..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(value.len(), |i| i + sign);
    value[sign..digits].parse::<u64>().is_ok_and(|n| n != 0)
}

/// The editor's settings, from mangohud.toml, in its order.
pub fn groups() -> Vec<Group> {
    #[derive(Deserialize)]
    struct File {
        group: Vec<Group>,
    }
    let mut groups = toml::from_str::<File>(include_str!("../mangohud.toml"))
        .expect("mangohud.toml is checked by the tests")
        .group;
    for group in &mut groups {
        for setting in &mut group.setting {
            setting.element = group.elements;
        }
    }
    groups
}

/// `$XDG_CONFIG_HOME/MangoHud/MangoHud.conf`.
pub fn global_path() -> Option<PathBuf> {
    paths::config_home().map(|d| d.join("MangoHud/MangoHud.conf"))
}

/// The file of `game`'s own settings.
pub fn game_path(launcher: &Launcher, game: &str) -> Option<PathBuf> {
    // An id names a file; a slash in one would name a directory.
    let name = format!("{}-{}.conf", launcher.kind.name(), game.replace('/', "_"));
    paths::config_home().map(|d| d.join("MangoHud/protonhub").join(name))
}

/// The settings in `path`; none for a missing file.
pub fn read(path: &Path) -> io::Result<Config> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Config::parse(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e),
    }
}

/// Writes `config` to `path` in place: MangoHud watches the file it read,
/// and a replaced file is a new one it doesn't see change.
pub fn write(path: &Path, config: &Config) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, config.to_string())
}

/// Whether MangoHud can be shown over vkcube, its preview.
pub fn can_preview() -> bool {
    crate::install::on_path("mangohud") && crate::install::on_path("vkcube")
}

#[cfg(test)]
#[path = "tests/mangohud.rs"]
mod tests;
