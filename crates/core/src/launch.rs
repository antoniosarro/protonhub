//! A game's launch options: what Steam runs it with, as
//! `VAR=value… wrapper… %command% arguments…`, and presets that each switch
//! part of that on or off.

use std::collections::BTreeMap;

use protonhub_manifest::LauncherKind;
use serde::{Deserialize, Serialize};

use crate::games::{self, Error};
use crate::launcher::Launcher;
use crate::{heroic, steam};

/// What Steam replaces with the game's own command line.
const COMMAND: &str = "%command%";

/// The compositor a game can run in, at a resolution and refresh rate of
/// its own: `gamescope ARGS… -- %command%`.
pub const GAMESCOPE: &str = "gamescope";

/// Launch options split into their parts. Words ProtonHub doesn't know are
/// kept where they were.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Options {
    /// Variables set before the game, in order.
    pub env: Vec<(String, String)>,
    /// The words between them and %command%: the programs the game runs
    /// in, with their arguments.
    pub wrappers: Vec<String>,
    /// Whether there is a %command%. Without one, Steam adds the options
    /// to the game's command line, so they're all arguments.
    pub command: bool,
    /// The game's own arguments.
    pub args: Vec<String>,
}

impl Options {
    /// None for text a shell couldn't split, like an unclosed quote.
    pub fn parse(text: &str) -> Option<Self> {
        let words = shlex::split(text)?;
        let Some(at) = words.iter().position(|w| w == COMMAND) else {
            return Some(Self {
                args: words,
                ..Self::default()
            });
        };
        let mut options = Self {
            command: true,
            args: words[at + 1..].to_vec(),
            ..Self::default()
        };
        for word in &words[..at] {
            // An assignment after a program is that program's argument.
            match assignment(word) {
                Some(pair) if options.wrappers.is_empty() => options.env.push(pair),
                _ => options.wrappers.push(word.clone()),
            }
        }
        Some(options)
    }

    /// Whether everything `preset` sets is set.
    pub fn has(&self, preset: &Preset) -> bool {
        preset
            .env
            .iter()
            .all(|(k, v)| self.env.iter().any(|(ek, ev)| ek == k && ev == v))
            && preset
                .wrapper
                .as_ref()
                .is_none_or(|w| self.wrappers.contains(w))
    }

    /// Switches `preset` on, or off: its variables and its program.
    pub fn set(&mut self, preset: &Preset, on: bool) {
        for (key, value) in &preset.env {
            match (self.env.iter().position(|(k, _)| k == key), on) {
                (Some(i), true) => self.env[i].1 = value.clone(),
                (None, true) => self.env.push((key.clone(), value.clone())),
                (_, false) => self.env.retain(|(k, _)| k != key),
            }
        }
        if let Some(wrapper) = &preset.wrapper {
            let at = self.wrappers.iter().position(|w| w == wrapper);
            match at {
                // Outermost: it runs whatever was there.
                None if on => self.wrappers.insert(0, wrapper.clone()),
                Some(i) if !on => {
                    self.wrappers.remove(i);
                }
                _ => {}
            }
        }
    }

    /// gamescope's arguments, when the game runs in it.
    pub fn gamescope(&self) -> Option<&[String]> {
        let (at, end) = self.gamescope_span()?;
        Some(&self.wrappers[at + 1..end])
    }

    /// Runs the game in gamescope with `args`, innermost, or with none not
    /// in it.
    pub fn set_gamescope(&mut self, args: Option<Vec<String>>) {
        let words: Vec<String> = args
            .map(|args| {
                let mut words = vec![GAMESCOPE.to_owned()];
                words.extend(args);
                words.push("--".to_owned());
                words
            })
            .unwrap_or_default();
        match self.gamescope_span() {
            Some((at, end)) => {
                let end = (end + 1).min(self.wrappers.len());
                self.wrappers.splice(at..end, words);
            }
            None => self.wrappers.extend(words),
        }
    }

    /// Where gamescope is among the programs, and where its `--` is, or the
    /// end.
    fn gamescope_span(&self) -> Option<(usize, usize)> {
        let at = self.wrappers.iter().position(|w| w == GAMESCOPE)?;
        let end = self.wrappers[at..]
            .iter()
            .position(|w| w == "--")
            .map_or(self.wrappers.len(), |i| at + i);
        Some((at, end))
    }
}

/// The value of a program's option among its `args`, written `-W 1920`,
/// `--output-width 1920` or `--output-width=1920`.
pub fn option(args: &[String], names: &[&str]) -> Option<String> {
    args.iter().enumerate().find_map(|(i, arg)| {
        if names.contains(&arg.as_str()) {
            return args.get(i + 1).cloned();
        }
        names
            .iter()
            .find_map(|n| arg.strip_prefix(n)?.strip_prefix('='))
            .map(str::to_owned)
    })
}

/// Sets a program's option to `value` where it was, written the first of
/// `names`' way, or with none takes it out, however it was written.
pub fn set_option(args: &mut Vec<String>, names: &[&str], value: Option<&str>) {
    let mut at = None;
    let mut i = 0;
    while i < args.len() {
        if names.contains(&args[i].as_str()) {
            at.get_or_insert(i);
            args.drain(i..(i + 2).min(args.len()));
        } else if names
            .iter()
            .any(|n| args[i].strip_prefix(n).is_some_and(|r| r.starts_with('=')))
        {
            at.get_or_insert(i);
            args.remove(i);
        } else {
            i += 1;
        }
    }
    if let Some(value) = value {
        let at = at.unwrap_or(args.len());
        args.splice(at..at, [names[0].to_owned(), value.to_owned()]);
    }
}

/// Whether a program's switch, an option without a value, is among its
/// `args`.
pub fn flag(args: &[String], names: &[&str]) -> bool {
    args.iter().any(|a| names.contains(&a.as_str()))
}

/// Switches a program's switch on, or off however it was written.
pub fn set_flag(args: &mut Vec<String>, names: &[&str], on: bool) {
    if !on {
        args.retain(|a| !names.contains(&a.as_str()));
    } else if !flag(args, names) {
        args.push(names[0].to_owned());
    }
}

/// Launch options as text Steam runs through a shell. Values are quoted
/// alone, since a quoted `'VAR=value'` is no assignment, and %command% is
/// left as it is for Steam to find.
impl std::fmt::Display for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut words: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{k}={}", quote(v)))
            .collect();
        words.extend(self.wrappers.iter().map(|w| quote(w)));
        if self.command || !words.is_empty() {
            words.push(COMMAND.to_owned());
        }
        words.extend(self.args.iter().map(|w| quote(w)));
        f.write_str(&words.join(" "))
    }
}

/// Words as a shell splits them; none for text it couldn't, like an
/// unclosed quote.
pub fn split(text: &str) -> Option<Vec<String>> {
    shlex::split(text)
}

/// Words quoted where needed, as a shell splits them back.
pub fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

fn quote(word: &str) -> String {
    // Quoting fails only on a NUL byte, which shlex never splits out.
    shlex::try_quote(word).map_or_else(|_| word.to_owned(), |q| q.into_owned())
}

/// Whether `key` can be a variable's name: a shell reads `key=value` as
/// setting it.
pub fn is_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `NAME=value` as a shell reads an assignment.
fn assignment(word: &str) -> Option<(String, String)> {
    let (key, value) = word.split_once('=')?;
    is_name(key).then(|| (key.to_owned(), value.to_owned()))
}

/// A switch on a game's page: variables to set, a program to run it in, or
/// both.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub wrapper: Option<String>,
    /// The heading it's under on the page; none for the first switches.
    pub group: Option<String>,
}

/// The presets, from presets.toml, in its order.
pub fn presets() -> Vec<Preset> {
    #[derive(Deserialize)]
    struct File {
        preset: Vec<Preset>,
    }
    toml::from_str::<File>(include_str!("../presets.toml"))
        .expect("presets.toml is checked by the tests")
        .preset
}

/// The launch options of each game that has some, by id, or none for a
/// launcher ProtonHub can't read them from.
pub fn all(launcher: &Launcher) -> Result<Option<Vec<(String, String)>>, Error> {
    match launcher.kind {
        LauncherKind::Steam => steam::launch_options(&launcher.root).map(Some),
        LauncherKind::Heroic => heroic::launch_options(&launcher.root).map(Some),
        _ => Ok(None),
    }
}

/// Sets `game`'s launch options to `text`, as it is; empty removes them.
/// Refused while the launcher runs, which would write its own back.
pub fn set(launcher: &Launcher, game: &str, text: &str) -> Result<(), Error> {
    set_many(launcher, &[(game, text)])
}

/// Sets several games' launch options, as `set` does each, writing each
/// file once.
pub fn set_many(launcher: &Launcher, changes: &[(&str, &str)]) -> Result<(), Error> {
    if games::is_running(launcher) {
        return Err(Error::Running {
            launcher: launcher.kind.title(),
        });
    }
    match launcher.kind {
        LauncherKind::Steam => steam::set_launch_options(&launcher.root, changes),
        LauncherKind::Heroic => changes
            .iter()
            .try_for_each(|&(game, text)| heroic::set_launch_options(&launcher.root, game, text)),
        kind => Err(Error::Unsupported {
            launcher: kind.title(),
        }),
    }
}

#[cfg(test)]
#[path = "tests/launch.rs"]
mod tests;
