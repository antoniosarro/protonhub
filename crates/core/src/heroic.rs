//! Heroic's games and per-game Wine or Proton.
//!
//! Installed games come from Heroic's library caches (Epic, GOG, Amazon,
//! and sideloaded apps). A game's settings, its `wineVersion` among them,
//! are in GamesConfig/<app>.json once its settings were saved; until then
//! it uses the default in config.json.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::games::{Choice, Error, Game, Library};
use crate::launcher::Launcher;
use protonhub_manifest::LauncherKind;

/// Library caches, the store each is for, and the key of its game list.
const LIBRARIES: [(&str, &str, &str); 4] = [
    ("store_cache/legendary_library.json", "epic", "library"),
    ("store_cache/gog_library.json", "gog", "games"),
    ("store_cache/nile_library.json", "amazon", "library"),
    ("sideload_apps/library.json", "sideload", "games"),
];

/// The directories Heroic keeps its Proton and Wine builds in.
pub const RUNNER_DIRS: [&str; 2] = ["tools/proton", "tools/wine"];

/// `beside` are the other launchers in Heroic's home, whose tools it lists
/// too.
pub fn library(root: &Path, beside: &[Launcher]) -> Result<Library, Error> {
    let mut problems = Vec::new();
    let choices = tools(root, beside);
    let default = read_json(&root.join("config.json"))?
        .and_then(|c| choice(&c["defaultSettings"]["wineVersion"]));

    let mut games = Vec::new();
    for (file, source, key) in LIBRARIES {
        let path = root.join(file);
        let Some(cache) = read_json(&path)? else {
            continue;
        };
        let Some(list) = cache[key].as_array() else {
            if !cache.as_object().is_some_and(|o| o.is_empty()) {
                problems.push(format!("{}: no {key} list", path.display()));
            }
            continue;
        };
        for g in list {
            let (Some(id), Some(name)) = (g["app_name"].as_str(), g["title"].as_str()) else {
                continue;
            };
            if g["is_installed"] != json!(true) || is_native(&g["install"]["platform"]) {
                continue;
            }
            let config = read_json(&config_path(root, id))?;
            let own = config
                .as_ref()
                .and_then(|c| choice(&c[id]["wineVersion"]))
                .map(|own| listed(&choices, own));
            let explicit = own.is_some();
            games.push(Game {
                id: id.to_owned(),
                name: name.to_owned(),
                source,
                tool: own.or_else(|| default.clone()),
                explicit,
                epic_namespace: g["namespace"].as_str().map(str::to_owned),
                art: ["art_cover", "art_square"]
                    .iter()
                    .find_map(|k| g[*k].as_str().filter(|s| !s.is_empty()))
                    .map(str::to_owned),
            });
        }
    }
    games.sort_by_key(|g| g.name.to_lowercase());
    Ok(Library {
        games,
        default,
        choices,
        problems,
    })
}

/// Sets `game`'s Wine or Proton to the choice whose id (its binary) is
/// `tool`. The game must have a settings file: ProtonHub doesn't invent
/// one, since Heroic fills in the rest of the settings when it writes it.
pub fn assign(
    root: &Path,
    beside: &[Launcher],
    game: &str,
    tool: Option<&str>,
) -> Result<(), Error> {
    let tool = tool.ok_or(Error::NeedsTool)?;
    let choice = tools(root, beside)
        .into_iter()
        .find(|c| c.id == tool)
        .ok_or_else(|| Error::UnknownTool {
            id: tool.to_owned(),
        })?;
    let path = config_path(root, game);
    let mut config = read_json(&path)?.ok_or_else(|| Error::NotConfigured {
        game: game.to_owned(),
    })?;
    let settings = config
        .get_mut(game)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::Format {
            path: path.clone(),
            message: format!("no settings object for {game}"),
        })?;
    let kind = if choice.id.ends_with("/proton") {
        "proton"
    } else {
        "wine"
    };
    settings.insert(
        "wineVersion".into(),
        json!({ "bin": choice.id, "name": choice.name, "type": kind }),
    );
    let text = serde_json::to_string_pretty(&config).expect("JSON values serialize");
    crate::games::write_with_backup(&path, text.as_bytes())
}

/// The Proton and Wine builds in Heroic's tool directories, then those
/// of the launchers `beside` it that Heroic lists too: Steam's Protons in
/// compatibilitytools.d (and Valve's own, in its libraries, when Heroic's
/// showValveProton setting is on) and Lutris's Wine builds. A choice's id
/// is the binary Heroic runs, as its settings store it.
fn tools(root: &Path, beside: &[Launcher]) -> Vec<Choice> {
    let valve = read_json(&root.join("config.json"))
        .ok()
        .flatten()
        .is_some_and(|c| c["defaultSettings"]["showValveProton"] == json!(true));
    // Where to look, the binary there, and whose it is if not Heroic's. A
    // Wine is in bin/, or in files/bin/ in a Proton build used as a Wine.
    const WINE: &[&str] = &["bin/wine", "files/bin/wine"];
    const PROTON: &[&str] = &["proton"];
    let mut places = vec![
        (root.join("tools/proton"), PROTON, None),
        (root.join("tools/wine"), WINE, None),
    ];
    for other in beside {
        match other.kind {
            LauncherKind::Steam => {
                places.push((
                    other.root.join("compatibilitytools.d"),
                    PROTON,
                    Some("Steam"),
                ));
                if valve {
                    for lib in crate::steam::libraries(&other.root).unwrap_or_default() {
                        places.push((lib.join("steamapps/common"), PROTON, Some("Steam")));
                    }
                }
            }
            LauncherKind::Lutris => {
                places.push((other.root.join("runners/wine"), WINE, Some("Lutris")));
            }
            _ => {}
        }
    }
    let mut choices: Vec<Choice> = Vec::new();
    for (dir, bins, whose) in places {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut found: Vec<_> = entries
            .flatten()
            .map(|e| e.path())
            .filter_map(|p| {
                let bin = bins.iter().map(|b| p.join(b)).find(|b| b.is_file())?;
                Some((p, bin))
            })
            .collect();
        found.sort();
        for (path, bin) in found {
            // A library reached twice, through a symlink.
            if choices.iter().any(|c| {
                c.dir
                    .as_deref()
                    .is_some_and(|d| crate::steam::same_dir(d, &path))
            }) {
                continue;
            }
            let mut name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            // The same version in Heroic's tools and another launcher's.
            if let Some(whose) = whose
                && choices.iter().any(|c| c.name == name)
            {
                name.push_str(&format!(" ({whose})"));
            }
            choices.push(Choice {
                id: bin.to_string_lossy().into_owned(),
                name,
                dir: Some(path),
            });
        }
    }
    choices
}

/// The listed choice that is `own`'s tool, found by its directory when
/// the settings name it by another path (a symlink); else `own`.
fn listed(choices: &[Choice], own: Choice) -> Choice {
    if choices.iter().any(|c| c.id == own.id) {
        return own;
    }
    own.dir
        .as_deref()
        .and_then(|d| {
            choices.iter().find(|c| {
                c.dir
                    .as_deref()
                    .is_some_and(|cd| crate::steam::same_dir(cd, d))
            })
        })
        .cloned()
        .unwrap_or(own)
}

/// A `wineVersion` setting as a choice; none when it names no binary.
fn choice(wine: &Value) -> Option<Choice> {
    let bin = wine["bin"].as_str().filter(|b| !b.is_empty())?;
    let name = wine["name"].as_str().unwrap_or(bin);
    // The tool's directory, wherever it is: Heroic can run Steam's tools.
    // Proton's binary is at its top, Wine's in bin/.
    let bin = Path::new(bin);
    let dir = match bin.parent() {
        Some(p) if p.file_name().is_some_and(|n| n == "bin") => p.parent(),
        p => p,
    };
    Some(Choice {
        id: bin.to_string_lossy().into_owned(),
        name: name.to_owned(),
        dir: dir.map(Path::to_path_buf),
    })
}

fn is_native(platform: &Value) -> bool {
    platform
        .as_str()
        .is_some_and(|p| p.eq_ignore_ascii_case("linux"))
}

fn config_path(root: &Path, game: &str) -> PathBuf {
    root.join("GamesConfig").join(format!("{game}.json"))
}

fn read_json(path: &Path) -> Result<Option<Value>, Error> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| Error::Json {
            path: path.to_owned(),
            source,
        })
}
