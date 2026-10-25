//! Steam's games and per-game compatibility tools.
//!
//! Games are the appmanifests in every library, plus the non-Steam
//! shortcuts. Which tool a game uses is `CompatToolMapping` in
//! config/config.vdf, keyed by app id, with "0" the default for every game
//! not listed; a tool is named by its internal name, which for a tool in
//! compatibilitytools.d is the key in its compatibilitytool.vdf.

mod appinfo;
mod officials;
mod shortcuts;
mod user;

use std::fs;
use std::path::{Path, PathBuf};

pub use officials::{Official, officials, uninstall};
pub use shortcuts::{Shortcut, add_shortcut, own_shortcut, set_shortcut, shortcuts};
pub use user::{User, launch_options, set_launch_options, users};

use appinfo::deck_ratings;
pub(crate) use appinfo::{app_ids, app_names};
use officials::official_name;
use shortcuts::read_shortcut;
use user::local_config;

use crate::files::{optional, same_dir, subdirs};
use crate::games::{Choice, Error, Game, Library};
use crate::launcher::{Launcher, Packaging};
use crate::vdf::{self, Obj, Value};

const MAPPING: [&str; 5] = [
    "InstallConfigStore",
    "Software",
    "Valve",
    "Steam",
    "CompatToolMapping",
];

/// The key in `CompatToolMapping` of the default, what every game without
/// a tool of its own runs with.
pub const DEFAULT: &str = "0";

/// Where in a user's localconfig.vdf each app's settings are, its launch
/// options among them.
/// Steam apps that are tools, not games.
const NOT_GAMES: [&str; 3] = [
    "Proton ",
    "Steam Linux Runtime",
    "Steamworks Common Redistributables",
];

pub fn library(root: &Path) -> Result<Library, Error> {
    let mut problems = Vec::new();
    let choices = tools(root, &mut problems)?;
    let mapping = read_text(&root.join("config/config.vdf"))?
        .and_then(|c| c.path(&MAPPING).cloned())
        .unwrap_or_default();
    let mapped = |id: &str| {
        mapping
            .obj(id)
            .and_then(|m| m.str("name"))
            .filter(|n| !n.is_empty())
    };
    let choice = |name: &str| {
        choices
            .iter()
            .find(|c| c.id == name)
            .cloned()
            .unwrap_or(Choice {
                id: name.to_owned(),
                name: name.to_owned(),
                dir: None,
            })
    };
    let default = mapped("0").map(choice);

    let installed = installed(root, &mut problems)?;
    let ids: Vec<&str> = installed
        .iter()
        .filter(|(_, _, source)| *source == "steam")
        .map(|(id, _, _)| id.as_str())
        .collect();
    // Ratings are extra: an app cache that can't be read leaves them out,
    // not games, so it's no problem of the library's.
    let decks = deck_ratings(root, &ids).unwrap_or_default();
    let mut games = Vec::new();
    for (id, name, source) in installed {
        let (tool, explicit) = match mapped(&id) {
            Some(name) => (Some(choice(name)), true),
            None => (default.clone(), false),
        };
        let deck = decks
            .iter()
            .find(|(d, _)| *d == id)
            .map(|(_, deck)| deck.clone());
        games.push(Game {
            art: (source == "steam").then(|| art(root, &id)),
            id,
            name,
            source,
            tool,
            explicit,
            epic_namespace: None,
            deck,
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

/// The command that runs `launcher`, a Steam, as its packaging does.
fn command(launcher: &Launcher) -> std::process::Command {
    let (program, args): (&str, &[&str]) = match launcher.packaging {
        Packaging::Native => ("steam", &[]),
        Packaging::Flatpak => ("flatpak", &["run", "com.valvesoftware.Steam"]),
        Packaging::Snap => ("snap", &["run", "steam"]),
    };
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd
}

/// Asks Steam to quit, as its own Exit does, and waits up to a minute for
/// it to.
pub fn quit(launcher: &Launcher) -> Result<(), Error> {
    let mut cmd = command(launcher);
    cmd.arg("-shutdown");
    crate::prefix::launch(cmd)?;
    for _ in 0..120 {
        if !crate::games::is_running(launcher) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(Error::StillRunning { launcher: "Steam" })
}

/// Starts Steam, and leaves it running.
pub fn start(launcher: &Launcher) -> Result<(), Error> {
    crate::prefix::launch(command(launcher))
}

/// Sets each game's tool, or with `None` leaves it to the default, in one
/// rewrite of config.vdf; for [`DEFAULT`], the default's, or with `None`
/// none, which is Steam Play off for games without one. Nothing is written
/// unless every game and tool is known.
pub fn assign(root: &Path, moves: &[(&str, Option<&str>)]) -> Result<(), Error> {
    let lib = library(root)?;
    for &(game, tool) in moves {
        if game != DEFAULT && !lib.games.iter().any(|g| g.id == game) {
            return Err(Error::UnknownGame {
                id: game.to_owned(),
            });
        }
        if let Some(tool) = tool
            && !lib.choices.iter().any(|c| c.id == tool)
        {
            return Err(Error::UnknownTool {
                id: tool.to_owned(),
            });
        }
    }
    let path = root.join("config/config.vdf");
    let mut config = read_text(&path)?.unwrap_or_default();
    let mapping = config.path_mut(&MAPPING);
    for &(game, tool) in moves {
        match tool {
            Some(tool) => {
                // What Steam itself writes for a choice made in a game's
                // properties, or in its settings for the default.
                let priority = if game == DEFAULT { "75" } else { "250" };
                let mut entry = Obj::default();
                entry.set("name", Value::Str(tool.to_owned()));
                entry.set("config", Value::Str(String::new()));
                entry.set("priority", Value::Str(priority.into()));
                mapping.set(game, Value::Obj(entry));
            }
            None => {
                mapping.remove(game);
            }
        }
    }
    crate::files::write_with_backup(&path, vdf::to_string(&config).as_bytes())
}

/// The tools a game can be set to: those in compatibilitytools.d and
/// Valve's Proton builds installed in a library.
fn tools(root: &Path, problems: &mut Vec<String>) -> Result<Vec<Choice>, Error> {
    let mut choices = Vec::new();
    let custom = root.join("compatibilitytools.d");
    for dir in subdirs(&custom)? {
        let file = dir.join("compatibilitytool.vdf");
        let obj = match read_text(&file) {
            Ok(Some(obj)) => obj,
            Ok(None) => continue,
            Err(e) => {
                problems.push(e.to_string());
                continue;
            }
        };
        for (name, tool) in obj
            .path(&["compatibilitytools", "compat_tools"])
            .map(|o| o.0.as_slice())
            .unwrap_or_default()
        {
            let display = tool
                .as_obj()
                .and_then(|t| t.str("display_name"))
                .unwrap_or(name);
            choices.push(Choice {
                id: name.clone(),
                name: display.to_owned(),
                dir: Some(dir.clone()),
            });
        }
    }
    for lib in libraries(root)? {
        for dir in subdirs(&lib.join("steamapps/common"))? {
            let file_name = dir.file_name().unwrap_or_default().to_string_lossy();
            if let Some(id) = official_name(&file_name) {
                choices.push(Choice {
                    id,
                    name: file_name.into_owned(),
                    dir: Some(dir.clone()),
                });
            }
        }
    }
    choices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(choices)
}

/// The library folders, the Steam directory itself first.
pub(crate) fn libraries(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut libs = vec![root.to_owned()];
    let folders = read_text(&root.join("steamapps/libraryfolders.vdf"))?;
    let entries = folders
        .as_ref()
        .and_then(|f| f.obj("libraryfolders"))
        .map(|o| o.0.as_slice())
        .unwrap_or_default();
    for (_, v) in entries {
        // Older files have the path as the value itself.
        let path = match v {
            Value::Obj(o) => o.str("path"),
            Value::Str(s) => Some(s.as_str()),
            Value::Int(_) => None,
        };
        if let Some(path) = path.map(PathBuf::from)
            && !libs.iter().any(|l| same_dir(l, &path))
        {
            libs.push(path);
        }
    }
    Ok(libs)
}

/// A game's header picture: the one in Steam's library cache, either
/// layout (`<id>/header.jpg`, maybe a directory deeper, or the older
/// `<id>_header.jpg`), else Steam's CDN.
fn art(root: &Path, id: &str) -> String {
    let cache = root.join("appcache/librarycache");
    let dir = cache.join(id);
    let deeper = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("header.jpg"));
    [
        dir.join("header.jpg"),
        cache.join(format!("{id}_header.jpg")),
    ]
    .into_iter()
    .chain(deeper)
    .find(|p| p.is_file())
    .map(|p| p.display().to_string())
    .unwrap_or_else(|| format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/header.jpg"))
}

/// (app id, name, source) of every installed game and shortcut.
fn installed(
    root: &Path,
    problems: &mut Vec<String>,
) -> Result<Vec<(String, String, &'static str)>, Error> {
    let mut games = Vec::new();
    for lib in libraries(root)? {
        for path in app_manifests(&lib)? {
            let app = match read_text(&path) {
                Ok(Some(obj)) => obj,
                Ok(None) => continue,
                Err(e) => {
                    problems.push(e.to_string());
                    continue;
                }
            };
            let Some(state) = app.obj("AppState") else {
                problems.push(format!("{}: no AppState", path.display()));
                continue;
            };
            let (Some(id), Some(name)) = (state.str("appid"), state.str("name")) else {
                problems.push(format!("{}: no appid or name", path.display()));
                continue;
            };
            if NOT_GAMES.iter().any(|p| name.starts_with(p)) {
                continue;
            }
            games.push((id.to_owned(), name.to_owned(), "steam"));
        }
    }

    for user in subdirs(&root.join("userdata"))? {
        let path = user.join("config/shortcuts.vdf");
        let obj = match read_binary(&path) {
            Ok(Some(o)) => o,
            Ok(None) => continue,
            Err(Error::Vdf { source, .. }) => {
                problems.push(format!("{}: {source}", path.display()));
                continue;
            }
            Err(e) => return Err(e),
        };
        for (_, s) in obj
            .obj("shortcuts")
            .map(|o| o.0.as_slice())
            .unwrap_or_default()
        {
            if let Some((id, s)) = s.as_obj().and_then(read_shortcut) {
                games.push((id, s.name, "shortcut"));
            }
        }
    }
    Ok(games)
}

/// A binary KeyValues file, or none where there isn't one.
fn read_binary(path: &Path) -> Result<Option<Obj>, Error> {
    let Some(bytes) = optional(path, fs::read)? else {
        return Ok(None);
    };
    vdf::parse_binary(&bytes)
        .map(Some)
        .map_err(|source| Error::Vdf {
            path: path.to_owned(),
            source,
        })
}

/// The app manifests (`appmanifest_<id>.acf`) of a library; none on a
/// drive that isn't mounted.
fn app_manifests(lib: &Path) -> Result<Vec<PathBuf>, Error> {
    let Some(entries) = optional(&lib.join("steamapps"), fs::read_dir)? else {
        return Ok(Vec::new());
    };
    Ok(entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name.starts_with("appmanifest_") && name.ends_with(".acf")
        })
        .collect())
}

fn read_text(path: &Path) -> Result<Option<Obj>, Error> {
    let Some(text) = optional(path, fs::read_to_string)? else {
        return Ok(None);
    };
    vdf::parse(&text).map(Some).map_err(|source| Error::Vdf {
        path: path.to_owned(),
        source,
    })
}

#[cfg(test)]
#[path = "../tests/steam.rs"]
mod tests;
