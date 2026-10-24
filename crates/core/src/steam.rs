//! Steam's games and per-game compatibility tools.
//!
//! Games are the appmanifests in every library, plus the non-Steam
//! shortcuts. Which tool a game uses is `CompatToolMapping` in
//! config/config.vdf, keyed by app id, with "0" the default for every game
//! not listed; a tool is named by its internal name, which for a tool in
//! compatibilitytools.d is the key in its compatibilitytool.vdf.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::games::{Choice, Error, Game, Library};
use crate::vdf::{self, Obj, Value};

const MAPPING: [&str; 5] = [
    "InstallConfigStore",
    "Software",
    "Valve",
    "Steam",
    "CompatToolMapping",
];

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

    let mut games = Vec::new();
    for (id, name, source) in installed(root, &mut problems)? {
        let (tool, explicit) = match mapped(&id) {
            Some(name) => (Some(choice(name)), true),
            None => (default.clone(), false),
        };
        games.push(Game {
            art: (source == "steam").then(|| art(root, &id)),
            id,
            name,
            source,
            tool,
            explicit,
            epic_namespace: None,
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

/// Sets each game's tool, or with `None` leaves it to the default, in one
/// rewrite of config.vdf. Nothing is written unless every game and tool is
/// known.
pub fn assign(root: &Path, moves: &[(&str, Option<&str>)]) -> Result<(), Error> {
    let lib = library(root)?;
    for &(game, tool) in moves {
        if !lib.games.iter().any(|g| g.id == game) {
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
                // properties.
                let mut entry = Obj::default();
                entry.set("name", Value::Str(tool.to_owned()));
                entry.set("config", Value::Str(String::new()));
                entry.set("priority", Value::Str("250".into()));
                mapping.set(game, Value::Obj(entry));
            }
            None => {
                mapping.remove(game);
            }
        }
    }
    crate::games::write_with_backup(&path, vdf::to_string(&config).as_bytes())
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

/// A Proton build of Valve's, which Steam installs as an app of its own.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Official {
    pub appid: String,
    /// Its directory's name, as Steam shows it: "Proton 10.0".
    pub name: String,
    pub dir: PathBuf,
    /// The app manifest that tells Steam it's installed.
    pub manifest: PathBuf,
}

/// Valve's Proton builds installed in Steam's libraries.
pub fn officials(root: &Path) -> Result<Vec<Official>, Error> {
    let mut found = Vec::new();
    for lib in libraries(root)? {
        for manifest in app_manifests(&lib)? {
            let Some(app) = read_text(&manifest)? else {
                continue;
            };
            let Some(state) = app.obj("AppState") else {
                continue;
            };
            let (Some(appid), Some(installdir)) = (state.str("appid"), state.str("installdir"))
            else {
                continue;
            };
            let dir = lib.join("steamapps/common").join(installdir);
            if official_name(installdir).is_some() && dir.is_dir() {
                found.push(Official {
                    appid: appid.to_owned(),
                    name: installdir.to_owned(),
                    dir,
                    manifest,
                });
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
}

/// Uninstalls one of Valve's Protons as Steam does: its manifest first, so
/// Steam never sees an installed app with its files gone, then its files.
/// Only while Steam isn't running, which would write the manifest back.
pub fn uninstall(official: &Official) -> Result<(), Error> {
    fs::remove_file(&official.manifest).map_err(|source| Error::Io {
        path: official.manifest.clone(),
        source,
    })?;
    crate::install::delete(&official.dir)?;
    Ok(())
}

/// The internal name of a Valve Proton build from its directory name:
/// "Proton 9.0" is proton_9, "Proton 5.13" proton_513. Steam names them in
/// its app info, which this guesses rather than parses.
fn official_name(dir: &str) -> Option<String> {
    let version = dir.strip_prefix("Proton ")?;
    let version = version.strip_suffix(" (Beta)").unwrap_or(version);
    match version {
        "- Experimental" | "Experimental" => return Some("proton_experimental".into()),
        "Hotfix" => return Some("proton_hotfix".into()),
        _ => {}
    }
    let (major, minor) = version.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some(if minor == "0" {
        format!("proton_{major}")
    } else {
        format!("proton_{major}{minor}")
    })
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
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(Error::Io { path, source }),
        };
        let obj = match vdf::parse_binary(&bytes) {
            Ok(o) => o,
            Err(e) => {
                problems.push(format!("{}: {e}", path.display()));
                continue;
            }
        };
        for (_, s) in obj
            .obj("shortcuts")
            .map(|o| o.0.as_slice())
            .unwrap_or_default()
        {
            let Some(s) = s.as_obj() else { continue };
            let (Some(Value::Int(id)), Some(name)) = (s.get("appid"), s.str("AppName")) else {
                continue;
            };
            // Stored signed; Steam's mapping uses the unsigned value.
            games.push(((*id as u32).to_string(), name.to_owned(), "shortcut"));
        }
    }
    Ok(games)
}

/// The app manifests (`appmanifest_<id>.acf`) of a library; none on a
/// drive that isn't mounted.
fn app_manifests(lib: &Path) -> Result<Vec<PathBuf>, Error> {
    let apps = lib.join("steamapps");
    let entries = match fs::read_dir(&apps) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::Io { path: apps, source }),
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
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    vdf::parse(&text).map(Some).map_err(|source| Error::Vdf {
        path: path.to_owned(),
        source,
    })
}

fn subdirs(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(Error::Io {
                path: dir.to_owned(),
                source,
            });
        }
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    Ok(dirs)
}

pub(crate) fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::TempDir;

    #[test]
    fn art_from_the_library_cache_or_the_cdn() {
        let root = TempDir::new("steam-art");
        let cache = &root.join("appcache/librarycache");
        for file in ["10/header.jpg", "20/0ab1/header.jpg", "30_header.jpg"] {
            let path = cache.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"jpg").unwrap();
        }
        let local = |f: &str| cache.join(f).display().to_string();
        assert_eq!(art(&root, "10"), local("10/header.jpg"));
        assert_eq!(
            art(&root, "20"),
            local("20/0ab1/header.jpg"),
            "a directory deeper"
        );
        assert_eq!(art(&root, "30"), local("30_header.jpg"), "the older layout");
        assert_eq!(
            art(&root, "40"),
            "https://cdn.cloudflare.steamstatic.com/steam/apps/40/header.jpg"
        );
    }

    #[test]
    fn official_names() {
        let cases = [
            ("Proton 9.0", Some("proton_9")),
            ("Proton 5.13", Some("proton_513")),
            ("Proton 10.0 (Beta)", Some("proton_10")),
            ("Proton - Experimental", Some("proton_experimental")),
            ("Proton Hotfix", Some("proton_hotfix")),
            ("Proton EasyAntiCheat Runtime", None),
            ("Portal 2", None),
        ];
        for (dir, want) in cases {
            assert_eq!(official_name(dir).as_deref(), want, "{dir}");
        }
    }
}
