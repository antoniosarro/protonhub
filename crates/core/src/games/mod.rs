//! Games and which compatibility tool each uses, for the launchers that
//! keep that per game (Steam and Heroic), and from it which installed tools
//! nothing uses.

use std::fs;
use std::io;
use std::path::PathBuf;

use protonhub_manifest::LauncherKind;
use serde::Serialize;

use crate::install;
use crate::launcher::Launcher;
use crate::{faugus, heroic, steam, vdf};

mod usage;

pub use usage::{OfficialUsage, Usage, Users, officials, usage};

/// A tool a game can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    /// What the launcher stores: Steam's internal name, Heroic's binary.
    pub id: String,
    pub name: String,
    /// The tool's directory, when it is known.
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Game {
    /// Steam's app id, Heroic's app name.
    pub id: String,
    pub name: String,
    /// Where it comes from: steam or shortcut; epic, gog, amazon or sideload.
    pub source: &'static str,
    /// The tool it runs with, its own or the launcher's default.
    pub tool: Option<Choice>,
    /// Whether the tool is the game's own setting, not the default.
    pub explicit: bool,
    /// An Epic game's namespace, which anti-cheat data is keyed by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epic_namespace: Option<String>,
    /// A picture of the game: a local file's path, or a URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
    /// Valve's Steam Deck rating, for a Steam game it rated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deck: Option<Deck>,
}

/// Valve's Steam Deck rating of a game, from Steam's app cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Deck {
    pub rating: DeckRating,
    /// What Valve rated it with: "native", "proton-stable" (Steam's
    /// default Proton), or a Proton's version, as "proton-8.0-2".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeckRating {
    Unsupported,
    Playable,
    Verified,
}

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub games: Vec<Game>,
    /// What games without a setting of their own use.
    pub default: Option<Choice>,
    pub choices: Vec<Choice>,
    /// Files that couldn't be read, which left something out.
    pub problems: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: {source}")]
    Vdf {
        path: PathBuf,
        source: vdf::ParseError,
    },
    #[error("{path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{path}: {message}")]
    Format { path: PathBuf, message: String },
    #[error("{launcher} is running; quit it first, or it overwrites the change when it exits")]
    Running { launcher: &'static str },
    #[error(
        "{launcher} is running, and a game or its shader processing may be using this; quit it first"
    )]
    InUse { launcher: &'static str },
    #[error("{launcher} didn't quit within a minute")]
    StillRunning { launcher: &'static str },
    #[error("no installed game {id}")]
    UnknownGame { id: String },
    #[error("no compatibility tool {id}")]
    UnknownTool { id: String },
    #[error(
        "Heroic has no settings file for {game} yet; open the game's settings in Heroic once, then try again"
    )]
    NotConfigured { game: String },
    #[error(
        "Heroic games, and Faugus Launcher's default, always name their Wine or Proton; choose one"
    )]
    NeedsTool,
    #[error("ProtonHub can't set the tool of {launcher} games")]
    Unsupported { launcher: &'static str },
    #[error("the launch options have an unclosed quote")]
    Unclosed,
    #[error("{game} has no Wine prefix yet; run it once, then try again")]
    NoPrefix { game: String },
    #[error(
        "can't find the Wine or Proton that runs {game}; run it once with the tool it should use"
    )]
    NoWine { game: String },
    #[error("{program} isn't installed")]
    Missing { program: &'static str },
    #[error(
        "Proton's 32-bit Wine needs /lib/ld-linux.so.2, which this system doesn't have; install steam-run, which provides it"
    )]
    NeedsFhs,
    #[error("{program} failed ({status}):\n{output}")]
    Failed {
        program: String,
        status: String,
        output: String,
    },
    #[error(transparent)]
    Install(#[from] install::Error),
}

/// The other launchers in `launcher`'s home: Heroic runs their tools too.
fn beside(launcher: &Launcher) -> Vec<Launcher> {
    let found = launcher.home().map(|h| crate::launcher::detect(&h));
    found
        .into_iter()
        .flatten()
        .filter(|l| l.kind != launcher.kind)
        .collect()
}

/// The launcher's games, or none for a launcher that doesn't keep a tool
/// per game.
pub fn library(launcher: &Launcher) -> Result<Option<Library>, Error> {
    match launcher.kind {
        LauncherKind::Steam => steam::library(&launcher.root).map(Some),
        LauncherKind::Heroic => heroic::library(&launcher.root, &beside(launcher)).map(Some),
        LauncherKind::Faugus => faugus::library(launcher).map(Some),
        LauncherKind::Lutris | LauncherKind::Bottles | LauncherKind::Winezgui => Ok(None),
    }
}

/// Sets which tool `game` uses: a choice's id, or `None` for the
/// launcher's default. Refused while the launcher runs, since both write
/// their settings back when they exit.
pub fn assign(launcher: &Launcher, game: &str, tool: Option<&str>) -> Result<(), Error> {
    assign_many(launcher, &[(game, tool)])
}

/// Sets the tools of several games at once, as `assign` does each.
pub fn assign_many(launcher: &Launcher, moves: &[(&str, Option<&str>)]) -> Result<(), Error> {
    if is_running(launcher) {
        return Err(Error::Running {
            launcher: launcher.kind.title(),
        });
    }
    match launcher.kind {
        LauncherKind::Steam => steam::assign(&launcher.root, moves),
        LauncherKind::Heroic => moves.iter().try_for_each(|&(game, tool)| {
            heroic::assign(&launcher.root, &beside(launcher), game, tool)
        }),
        LauncherKind::Faugus => moves
            .iter()
            .try_for_each(|&(game, tool)| faugus::assign(launcher, game, tool)),
        kind => Err(Error::Unsupported {
            launcher: kind.title(),
        }),
    }
}

/// Sets the tool games without one of their own run with, in Steam or
/// Faugus Launcher; with `None`, Steam Play off for them. Refused while the
/// launcher runs, as `assign` is.
pub fn set_default(launcher: &Launcher, tool: Option<&str>) -> Result<(), Error> {
    match launcher.kind {
        LauncherKind::Steam => assign_many(launcher, &[(steam::DEFAULT, tool)]),
        LauncherKind::Faugus if is_running(launcher) => Err(Error::Running {
            launcher: launcher.kind.title(),
        }),
        LauncherKind::Faugus => faugus::set_default(launcher, tool.ok_or(Error::NeedsTool)?),
        kind => Err(Error::Unsupported {
            launcher: kind.title(),
        }),
    }
}

/// Renames Steam shortcut `game` and sets what it runs and where, keeping
/// its launch options. Refused while Steam runs, which would write its own
/// back.
pub fn edit_shortcut(
    launcher: &Launcher,
    game: &str,
    name: &str,
    target: &str,
    start_dir: &str,
) -> Result<(), Error> {
    if is_running(launcher) {
        return Err(Error::Running {
            launcher: launcher.kind.title(),
        });
    }
    let (_, current) = steam::shortcuts(&launcher.root)?
        .into_iter()
        .find(|(id, _)| id == game)
        .ok_or_else(|| Error::UnknownGame {
            id: game.to_owned(),
        })?;
    steam::set_shortcut(
        &launcher.root,
        game,
        &steam::Shortcut {
            name: name.to_owned(),
            target: target.to_owned(),
            start_dir: start_dir.to_owned(),
            ..current
        },
    )
}

/// Adds a non-Steam shortcut to Steam, as `steam::add_shortcut` does;
/// refused while Steam runs, which would write its own list back.
pub fn add_shortcut(launcher: &Launcher, add: &steam::Shortcut, icon: &str) -> Result<bool, Error> {
    if is_running(launcher) {
        return Err(Error::Running {
            launcher: launcher.kind.title(),
        });
    }
    steam::add_shortcut(&launcher.root, add, icon)
}

/// Moves every game set to a tool matching `from` (its id or name) to the
/// tool `to`, and returns their names. Games on the launcher's default
/// stay on it.
pub fn reassign(launcher: &Launcher, from: &regex::Regex, to: &str) -> Result<Vec<String>, Error> {
    let Some(lib) = library(launcher)? else {
        return Err(Error::Unsupported {
            launcher: launcher.kind.title(),
        });
    };
    let to = lib
        .choices
        .iter()
        .find(|c| c.id == to || c.name == to)
        .ok_or_else(|| Error::UnknownTool { id: to.to_owned() })?;
    let moving: Vec<_> = lib
        .games
        .iter()
        .filter(|g| g.explicit)
        .filter(|g| {
            g.tool
                .as_ref()
                .is_some_and(|t| t.id != to.id && (from.is_match(&t.id) || from.is_match(&t.name)))
        })
        .collect();
    let moves: Vec<_> = moving
        .iter()
        .map(|g| (g.id.as_str(), Some(to.id.as_str())))
        .collect();
    if !moves.is_empty() {
        assign_many(launcher, &moves)?;
    }
    Ok(moving.iter().map(|g| g.name.clone()).collect())
}

/// Whether the launcher runs, so its per-game settings can't be changed.
/// Any process of that name counts: Flatpak and native alike.
pub fn is_running(launcher: &Launcher) -> bool {
    match launcher.kind {
        LauncherKind::Steam => running("steam"),
        LauncherKind::Heroic => running("heroic"),
        LauncherKind::Faugus => running("faugus-launcher"),
        LauncherKind::Lutris | LauncherKind::Bottles | LauncherKind::Winezgui => false,
    }
}

/// Whether processes outside ProtonHub can be seen: inside a Flatpak, only
/// the sandbox's are, so a running launcher looks stopped.
pub fn sees_launchers() -> bool {
    !std::path::Path::new("/.flatpak-info").exists()
}

/// Whether a process named `name` runs (its /proc comm, as `pgrep -x`).
fn running(name: &str) -> bool {
    // Tests mustn't depend on whether Steam is open on the machine.
    if cfg!(test) {
        return false;
    }
    let Ok(procs) = fs::read_dir("/proc") else {
        return false;
    };
    procs
        .flatten()
        .any(|p| fs::read_to_string(p.path().join("comm")).is_ok_and(|c| c.trim_end() == name))
}

#[cfg(test)]
#[path = "../tests/games.rs"]
mod tests;
