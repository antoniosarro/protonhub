//! Faugus Launcher's games and the Proton each runs with.
//!
//! Its games are games.json in its data directory, each naming its runner:
//! a directory in a Steam's compatibilitytools.d, its own "… Latest" ones
//! among them, or "Default", the default-runner in its config.json. The
//! Protons it downloads go in that compatibilitytools.d too, with Steam
//! installed or not.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::files::read_json;
use crate::games::{Choice, Error, Game, Library};
use crate::launcher::{Launcher, Packaging};

/// A game's runner when it runs with the default.
const DEFAULT: &str = "Default";

/// The default runner until one is set.
const FAUGUS_DEFAULT: &str = "Proton-CachyOS Latest";

/// Runners Faugus downloads and keeps up to date itself, offered before
/// they're there.
const LATEST: [&str; 4] = [
    "Proton-CachyOS Latest",
    "Proton-GE Latest",
    "DW-Proton Latest",
    "Proton-Wineland Latest",
];

/// Directories in compatibilitytools.d that aren't runners to choose.
const HIDDEN: [&str; 2] = ["UMU-Latest", "LegacyRuntime"];

/// Runners that aren't Proton: those games are left out.
const NOT_PROTON: [&str; 2] = ["Linux-Native", "Steam"];

/// Proton-CachyOS from the system's package, named apart.
const SYSTEM_CACHYOS: (&str, &str) = (
    "Proton-CachyOS (System)",
    "/usr/share/steam/compatibilitytools.d/proton-cachyos-slr",
);

/// Whether `dir` in compatibilitytools.d is one Faugus downloads and keeps
/// itself: no game naming it doesn't make it unused.
pub fn manages(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| LATEST.contains(&n) || HIDDEN.contains(&n))
}

/// The Steam data directory whose compatibilitytools.d Faugus uses: the
/// native Steam's, unless only a Flatpak Steam's is there.
pub fn steam_dir(home: &Path) -> PathBuf {
    let native = home.join(".local/share/Steam");
    let flatpak = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
    if !native.is_dir() && flatpak.is_dir() {
        flatpak
    } else {
        native
    }
}

pub fn library(launcher: &Launcher) -> Result<Library, Error> {
    let choices = choices(launcher);
    let config = read_json(&config_path(launcher))?;
    let default = config
        .as_ref()
        .and_then(|c| c["default-runner"].as_str())
        .unwrap_or(FAUGUS_DEFAULT);
    let default = Some(choice(&choices, default.trim_matches('"')));

    let mut problems = Vec::new();
    let path = launcher.root.join("games.json");
    let list = read_json(&path)?.unwrap_or_else(|| json!([]));
    let Some(list) = list.as_array() else {
        problems.push(format!("{}: not a list of games", path.display()));
        return Ok(Library {
            games: Vec::new(),
            default,
            choices,
            problems,
        });
    };
    let mut games = Vec::new();
    for g in list {
        let (Some(id), Some(name)) = (g["gameid"].as_str(), g["title"].as_str()) else {
            continue;
        };
        let runner = g["runner"].as_str().unwrap_or(DEFAULT);
        if NOT_PROTON.contains(&runner) {
            continue;
        }
        let explicit = !matches!(runner, DEFAULT | "");
        let cover = launcher.root.join(format!("covers/{id}.png"));
        games.push(Game {
            id: id.to_owned(),
            name: name.to_owned(),
            source: "faugus",
            tool: if explicit {
                Some(choice(&choices, runner))
            } else {
                default.clone()
            },
            explicit,
            epic_namespace: None,
            deck: None,
            art: cover
                .is_file()
                .then(|| cover.to_string_lossy().into_owned()),
        });
    }
    games.sort_by_key(|g| g.name.to_lowercase());
    Ok(Library {
        games,
        default,
        choices,
        problems,
    })
}

/// `game`'s prefix, as its settings name it; none if they name none.
pub fn prefix(launcher: &Launcher, game: &str) -> Result<Option<PathBuf>, Error> {
    let list = read_json(&launcher.root.join("games.json"))?.unwrap_or_else(|| json!([]));
    let home = launcher.home().unwrap_or_default();
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .find(|g| g["gameid"] == json!(game))
        .and_then(|g| g["prefix"].as_str())
        .filter(|p| !p.is_empty())
        .map(|p| match p.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(p),
        }))
}

/// Sets `game`'s runner to the choice `tool`, or with `None` to the
/// default.
pub fn assign(launcher: &Launcher, game: &str, tool: Option<&str>) -> Result<(), Error> {
    let runner = match tool {
        Some(tool) => known(launcher, tool)?,
        None => DEFAULT,
    };
    let path = launcher.root.join("games.json");
    let mut list = read_json(&path)?.unwrap_or_else(|| json!([]));
    let entry = list
        .as_array_mut()
        .into_iter()
        .flatten()
        .find(|g| g["gameid"] == json!(game))
        .ok_or_else(|| Error::UnknownGame {
            id: game.to_owned(),
        })?;
    entry["runner"] = json!(runner);
    write(&path, &list)
}

/// Sets the runner games set to "Default" run with.
pub fn set_default(launcher: &Launcher, tool: &str) -> Result<(), Error> {
    let tool = known(launcher, tool)?;
    let path = config_path(launcher);
    // Faugus fills in what's missing from its defaults.
    let mut config = read_json(&path)?.unwrap_or_else(|| json!({}));
    let Some(settings) = config.as_object_mut() else {
        return Err(Error::Format {
            path,
            message: "not a settings object".into(),
        });
    };
    settings.insert("default-runner".into(), json!(tool));
    write(&path, &config)
}

/// `tool`, if it's one of the choices.
fn known<'a>(launcher: &Launcher, tool: &'a str) -> Result<&'a str, Error> {
    if choices(launcher).iter().any(|c| c.id == tool) {
        Ok(tool)
    } else {
        Err(Error::UnknownTool {
            id: tool.to_owned(),
        })
    }
}

/// As Faugus writes its files: indented by four, not escaping non-ASCII.
fn write(path: &Path, value: &Value) -> Result<(), Error> {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    value
        .serialize(&mut serializer)
        .expect("JSON values serialize");
    crate::files::write_with_backup(path, &out)
}

fn config_path(launcher: &Launcher) -> PathBuf {
    let home = launcher.home().expect("detected launchers have a home");
    home.join(match launcher.packaging {
        Packaging::Flatpak => ".var/app/io.github.Faugus.faugus-launcher/config",
        _ => ".config",
    })
    .join("faugus-launcher/config.json")
}

/// The runners a game can be set to: the directories in compatibilitytools.d
/// by name, Faugus's own Latest ones, and the system's Proton-CachyOS.
fn choices(launcher: &Launcher) -> Vec<Choice> {
    let home = launcher.home().expect("detected launchers have a home");
    let dir = steam_dir(&home).join("compatibilitytools.d");
    let mut choices: Vec<Choice> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_owned();
            (!HIDDEN.contains(&name.as_str())).then(|| Choice {
                id: name.clone(),
                name,
                dir: Some(p),
            })
        })
        .collect();
    choices.sort_by(|a, b| a.name.cmp(&b.name));
    for latest in LATEST {
        if !choices.iter().any(|c| c.id == latest) {
            choices.push(Choice {
                id: latest.into(),
                name: latest.into(),
                dir: None,
            });
        }
    }
    let (name, path) = SYSTEM_CACHYOS;
    if Path::new(path).is_dir() {
        choices.push(Choice {
            id: name.into(),
            name: name.into(),
            dir: Some(path.into()),
        });
    }
    choices
}

/// The choice a runner names; one that isn't there is still shown by name.
fn choice(choices: &[Choice], runner: &str) -> Choice {
    choices
        .iter()
        .find(|c| c.id == runner)
        .cloned()
        .unwrap_or_else(|| Choice {
            id: runner.to_owned(),
            name: runner.to_owned(),
            dir: None,
        })
}

#[cfg(test)]
#[path = "tests/faugus.rs"]
mod tests;
