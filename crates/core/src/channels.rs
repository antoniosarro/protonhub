//! Release channels: a tool kept at its latest release in a launcher.
//!
//! An update installs the latest release if it isn't there, can move the
//! games set to the tool's older versions onto it, and deletes the
//! versions past the newest `keep` that no game in any launcher uses.
//! Channels are listed in `$XDG_CONFIG_HOME/protonhub/channels.toml`.

use std::cmp::Reverse;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use protonhub_manifest::{Arch, Registry, Tool};
use serde::{Deserialize, Serialize};

use crate::games;
use crate::install::{self, Entry, Step};
use crate::launcher::{self, Launcher};
use crate::releases::{self, Client, Version};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub tool: String,
    /// A launcher kind (`steam`), or kind and packaging (`steam:flatpak`).
    pub launcher: String,
    /// How many versions to keep, newest first; at least the newest.
    #[serde(default = "default_keep")]
    pub keep: usize,
    /// Whether games set to an older version move to the new one.
    #[serde(default)]
    pub move_games: bool,
}

fn default_keep() -> usize {
    3
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channels {
    #[serde(default, rename = "channel")]
    pub list: Vec<Channel>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: {message}")]
    Config { path: PathBuf, message: String },
    #[error("no tool {tool} in the registry")]
    UnknownTool { tool: String },
    #[error("no launcher {launcher} that can take {tool} was found")]
    NoLauncher { tool: String, launcher: String },
    #[error("several launchers match {launcher}: {found}; name one as kind:packaging")]
    Ambiguous { launcher: String, found: String },
    #[error(transparent)]
    Source(#[from] releases::Error),
    #[error(transparent)]
    Install(#[from] install::Error),
    #[error(transparent)]
    Games(#[from] games::Error),
}

impl Channels {
    /// `$XDG_CONFIG_HOME/protonhub/channels.toml`.
    pub fn path() -> Option<PathBuf> {
        crate::paths::config_dir().map(|d| d.join("channels.toml"))
    }

    /// The channels in `path`; none if it doesn't exist.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(Error::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|e| Error::Config {
            path: path.to_owned(),
            message: e.to_string(),
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| Error::Io { path, source }
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io(dir))?;
        }
        let text = toml::to_string_pretty(self).expect("channels serialize");
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(io(&tmp))?;
        fs::rename(&tmp, path).map_err(io(path))
    }

    /// Adds `channel`, replacing one of the same tool and launcher.
    pub fn set(&mut self, channel: Channel) {
        match self
            .list
            .iter_mut()
            .find(|c| c.tool == channel.tool && c.launcher == channel.launcher)
        {
            Some(c) => *c = channel,
            None => self.list.push(channel),
        }
    }

    /// Removes the channel of `tool` in `launcher`; whether there was one.
    pub fn remove(&mut self, tool: &str, launcher: &str) -> bool {
        let before = self.list.len();
        self.list
            .retain(|c| !(c.tool == tool && c.launcher == launcher));
        self.list.len() != before
    }
}

/// What an update or a check works with.
pub struct Context<'a> {
    pub client: &'a Client,
    pub registry: &'a Registry,
    /// Every launcher found: usage is counted across all of them.
    pub launchers: &'a [Launcher],
    pub arch: Arch,
    pub downloads: &'a Path,
}

/// Whether a channel's latest release is installed.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub channel: Channel,
    pub launcher: Launcher,
    pub latest: String,
    pub installed: bool,
}

/// What an update did.
#[derive(Debug, Clone, Serialize)]
pub struct Update {
    pub channel: Channel,
    pub launcher: Launcher,
    pub latest: String,
    /// The new version's directory, when it was installed now.
    pub installed: Option<PathBuf>,
    /// The games moved onto it.
    pub moved: Vec<String>,
    /// Older versions deleted.
    pub removed: Vec<PathBuf>,
    /// What it left alone, and why.
    pub notes: Vec<String>,
}

impl Update {
    pub fn changed(&self) -> bool {
        self.installed.is_some() || !self.moved.is_empty() || !self.removed.is_empty()
    }
}

pub fn check(cx: &Context, channel: &Channel) -> Result<Status, Error> {
    let (tool, launcher) = resolve(cx, channel)?;
    let latest = cx.client.release(tool, &Version::Latest)?.tag;
    let dir = launcher
        .tool_dir(tool)
        .expect("the launcher takes the tool");
    let installed = ours(tool, install::list(&dir)?)
        .iter()
        .any(|e| version(e) == latest);
    Ok(Status {
        channel: channel.clone(),
        launcher,
        latest,
        installed,
    })
}

pub fn update(
    cx: &Context,
    channel: &Channel,
    progress: &mut dyn FnMut(Step),
) -> Result<Update, Error> {
    let (tool, launcher) = resolve(cx, channel)?;
    let dir = launcher
        .tool_dir(tool)
        .expect("the launcher takes the tool");
    let release = cx.client.release(tool, &Version::Latest)?;
    let latest = release.tag.clone();
    let mut update = Update {
        channel: channel.clone(),
        launcher: launcher.clone(),
        latest: latest.clone(),
        installed: None,
        moved: Vec::new(),
        removed: Vec::new(),
        notes: Vec::new(),
    };

    let newest = match ours(tool, install::list(&dir)?)
        .into_iter()
        .find(|e| version(e) == latest)
    {
        Some(e) => e.dir,
        None => {
            let e = install::install_release(
                cx.client,
                tool,
                release,
                &launcher,
                cx.arch,
                cx.downloads,
                progress,
            )?;
            update.installed = Some(e.dir.clone());
            e.dir
        }
    };

    if channel.move_games {
        move_games(tool, &launcher, &dir, &newest, &mut update)?;
    }
    prune(cx, tool, &launcher, &dir, channel.keep, &mut update)?;
    Ok(update)
}

/// Moves the games set to an older version of `tool` to `newest`.
fn move_games(
    tool: &Tool,
    launcher: &Launcher,
    dir: &Path,
    newest: &Path,
    update: &mut Update,
) -> Result<(), Error> {
    let Some(lib) = games::library(launcher)? else {
        return Ok(());
    };
    let older: Vec<PathBuf> = ours(tool, install::list(dir)?)
        .into_iter()
        .map(|e| e.dir)
        .filter(|d| d != newest)
        .collect();
    let moving: Vec<_> = lib
        .games
        .iter()
        .filter(|g| g.explicit)
        .filter(|g| {
            g.tool
                .as_ref()
                .and_then(|t| t.dir.as_ref())
                .is_some_and(|d| older.contains(d))
        })
        .collect();
    if moving.is_empty() {
        return Ok(());
    }
    let Some(to) = lib
        .choices
        .iter()
        .find(|c| c.dir.as_deref() == Some(newest))
    else {
        update.notes.push(format!(
            "{} isn't among {launcher}'s tools; {} games not moved",
            newest.display(),
            moving.len()
        ));
        return Ok(());
    };
    if games::is_running(launcher) {
        update.notes.push(format!(
            "{launcher} is running; {} games not moved",
            moving.len()
        ));
        return Ok(());
    }
    let moves: Vec<_> = moving
        .iter()
        .map(|g| (g.id.as_str(), Some(to.id.as_str())))
        .collect();
    games::assign_many(launcher, &moves)?;
    update.moved = moving.iter().map(|g| g.name.clone()).collect();
    Ok(())
}

/// Deletes the versions of `tool` past the newest `keep` that nothing uses.
fn prune(
    cx: &Context,
    tool: &Tool,
    launcher: &Launcher,
    dir: &Path,
    keep: usize,
    update: &mut Update,
) -> Result<(), Error> {
    let mut versions = ours(tool, install::list(dir)?);
    if versions.len() <= keep.max(1) {
        return Ok(());
    }
    // Newest first by the source's own order; versions it no longer lists
    // after those, by their tags compared as versions, then by when they
    // were installed.
    let order: Vec<String> = cx
        .client
        .releases(tool, 50)?
        .into_iter()
        .map(|r| r.tag)
        .collect();
    versions.sort_by_key(|e| {
        let r = e.record.as_ref().expect("ours have records");
        (
            order
                .iter()
                .position(|t| *t == r.version)
                .unwrap_or(usize::MAX),
            Reverse(natural(&r.version)),
            Reverse(r.installed_at),
        )
    });
    let usage = games::usage(cx.registry, cx.launchers)?;
    let usage: Vec<_> = usage
        .into_iter()
        .filter(|(l, _)| l == launcher)
        .flat_map(|(_, u)| u)
        .collect();
    for e in versions.iter().skip(keep.max(1)) {
        let name = version(e);
        match usage.iter().find(|u| u.entry.dir == e.dir) {
            Some(u) if u.unused() => {
                install::delete(&e.dir)?;
                update.removed.push(e.dir.clone());
            }
            Some(u) if u.users.is_none() => update.notes.push(format!(
                "kept {name}: ProtonHub can't tell what uses tools in {launcher}"
            )),
            _ => update
                .notes
                .push(format!("kept {name}: games still use it")),
        }
    }
    Ok(())
}

/// The channel's tool and the one launcher it names that takes it.
fn resolve<'a>(cx: &Context<'a>, channel: &Channel) -> Result<(&'a Tool, Launcher), Error> {
    let tool = cx
        .registry
        .tools
        .get(&channel.tool)
        .ok_or_else(|| Error::UnknownTool {
            tool: channel.tool.clone(),
        })?;
    match launcher::pick(cx.launchers, &channel.launcher, tool) {
        Ok(l) => Ok((tool, l.clone())),
        Err(found) if found.is_empty() => Err(Error::NoLauncher {
            tool: tool.id.clone(),
            launcher: channel.launcher.clone(),
        }),
        Err(found) => Err(Error::Ambiguous {
            launcher: channel.launcher.clone(),
            found: launcher::list(&found),
        }),
    }
}

/// The versions of `tool` among `entries`, which ProtonHub installed.
fn ours(tool: &Tool, entries: Vec<Entry>) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|e| e.record.as_ref().is_some_and(|r| r.tool == tool.id))
        .collect()
}

/// A tag as runs of digits (compared as numbers) and of other characters,
/// so GE-Proton11-7 sorts after GE-Proton10-34.
fn natural(tag: &str) -> Vec<(u64, String)> {
    let mut parts = Vec::new();
    let mut chars = tag.chars().peekable();
    while let Some(&c) = chars.peek() {
        let digit = c.is_ascii_digit();
        let mut run = String::new();
        while let Some(c) = chars.next_if(|c| c.is_ascii_digit() == digit) {
            run.push(c);
        }
        parts.push(if digit {
            (run.parse().unwrap_or(u64::MAX), String::new())
        } else {
            (0, run)
        });
    }
    parts
}

fn version(e: &Entry) -> &str {
    e.record.as_ref().map_or("", |r| r.version.as_str())
}

#[cfg(test)]
#[path = "tests/channels.rs"]
mod tests;
