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
const APPS: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

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

/// The launch options of each game that has some, by app id, from the
/// settings of the user `local_config` picks.
pub fn launch_options(root: &Path) -> Result<Vec<(String, String)>, Error> {
    let Some(config) = read_text(&local_config(root)?)? else {
        return Ok(Vec::new());
    };
    let apps = config
        .path(&APPS)
        .map(|a| a.0.as_slice())
        .unwrap_or_default();
    Ok(apps
        .iter()
        .filter_map(|(id, app)| {
            let options = app.as_obj()?.str("LaunchOptions")?;
            (!options.is_empty()).then(|| (id.clone(), options.to_owned()))
        })
        .collect())
}

/// Sets `game`'s launch options to `text`, or with an empty one removes
/// them. A shortcut's are in shortcuts.vdf, which ProtonHub doesn't write.
pub fn set_launch_options(root: &Path, game: &str, text: &str) -> Result<(), Error> {
    let lib = library(root)?;
    let found = lib
        .games
        .iter()
        .find(|g| g.id == game)
        .ok_or_else(|| Error::UnknownGame {
            id: game.to_owned(),
        })?;
    if found.source == "shortcut" {
        return Err(Error::Shortcut);
    }
    let path = local_config(root)?;
    let mut config = read_text(&path)?.ok_or_else(|| Error::Format {
        path: path.clone(),
        message: "missing; start Steam once to make it".into(),
    })?;
    let mut keys = APPS.to_vec();
    keys.push(game);
    let app = config.path_mut(&keys);
    if text.is_empty() {
        app.remove("LaunchOptions");
    } else {
        app.set("LaunchOptions", Value::Str(text.to_owned()));
    }
    crate::games::write_with_backup(&path, vdf::to_string(&config).as_bytes())
}

/// The localconfig.vdf of the user last signed in (MostRecent in
/// loginusers.vdf, keyed by SteamID64, whose low 32 bits are the account id
/// that names their userdata directory), else of the only user.
fn local_config(root: &Path) -> Result<PathBuf, Error> {
    let file = |account: &str| root.join(format!("userdata/{account}/config/localconfig.vdf"));
    let logins = root.join("config/loginusers.vdf");
    let users = read_text(&logins)?;
    let recent = users
        .as_ref()
        .and_then(|u| u.obj("users"))
        .map(|u| u.0.as_slice())
        .unwrap_or_default()
        .iter()
        .find(|(_, u)| u.as_obj().and_then(|u| u.str("MostRecent")) == Some("1"))
        .and_then(|(id, _)| id.parse::<u64>().ok());
    if let Some(id) = recent {
        return Ok(file(&(id & 0xffff_ffff).to_string()));
    }
    // "0" is no one's: Steam's, signed out.
    let accounts: Vec<_> = subdirs(&root.join("userdata"))?
        .into_iter()
        .filter_map(|d| Some(d.file_name()?.to_string_lossy().into_owned()))
        .filter(|a| a != "0" && file(a).is_file())
        .collect();
    match &accounts[..] {
        [one] => Ok(file(one)),
        _ => Err(Error::Format {
            path: logins,
            message: format!(
                "no user is marked as signed in last, and there are {} with settings; sign in to Steam once",
                accounts.len()
            ),
        }),
    }
}

/// The id of every app installed in any library, games or not, from its
/// manifest's name: one that can't be read still counts.
pub(crate) fn app_ids(root: &Path) -> Result<Vec<String>, Error> {
    let mut ids = Vec::new();
    for lib in libraries(root)? {
        for path in app_manifests(&lib)? {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let id = &name["appmanifest_".len()..name.len() - ".acf".len()];
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

/// The names Steam's app cache (appcache/appinfo.vdf) has for the apps
/// `ids`, which it keeps for apps no longer installed too. Entries of other
/// apps are skipped unread. Formats 28 and 29 (2024 on, its keys in a table
/// at the end); an older one is an error.
pub(crate) fn app_names(root: &Path, ids: &[&str]) -> Result<Vec<(String, String)>, Error> {
    let path = root.join("appcache/appinfo.vdf");
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(Error::Io { path, source }),
    };
    let bad = |message: String| Error::Vdf {
        path: path.clone(),
        source: vdf::ParseError { line: 0, message },
    };
    let int = |at: usize, n: usize| -> Result<u64, Error> {
        let raw = bytes
            .get(at..at + n)
            .ok_or_else(|| bad(format!("byte {at}: truncated")))?;
        Ok(raw.iter().rev().fold(0, |v, &b| v << 8 | u64::from(b)))
    };
    let (mut pos, keys) = match int(0, 4)? {
        0x0756_4428 => (8, None),
        0x0756_4429 => {
            let table = int(8, 8)? as usize;
            let count = int(table, 4)? as usize;
            let keys: Vec<String> = bytes[table + 4..]
                .split(|&b| b == 0)
                .take(count)
                .map(|k| String::from_utf8_lossy(k).into_owned())
                .collect();
            if keys.len() < count {
                return Err(bad(format!("byte {table}: truncated key table")));
            }
            (16, Some(keys))
        }
        other => return Err(bad(format!("unknown format {other:#010x}"))),
    };
    let wanted: Vec<u64> = ids.iter().filter_map(|id| id.parse().ok()).collect();
    let mut names = Vec::new();
    loop {
        let id = int(pos, 4)?;
        if id == 0 {
            break;
        }
        let size = int(pos + 4, 4)? as usize;
        let end = pos + 8 + size;
        if wanted.contains(&id) {
            // Before the KeyValues: its state, update time, access token,
            // two SHA-1s and change number, 60 bytes.
            let kv = bytes
                .get(pos + 68..end)
                .ok_or_else(|| bad(format!("byte {pos}: app {id} is truncated")))?;
            let obj = match &keys {
                Some(keys) => vdf::parse_binary_keyed(kv, keys),
                None => vdf::parse_binary(kv),
            }
            .map_err(|e| bad(format!("app {id}: {}", e.message)))?;
            if let Some(name) = obj.path(&["appinfo", "common"]).and_then(|c| c.str("name")) {
                names.push((id.to_string(), name.to_owned()));
            }
        }
        pos = end;
    }
    Ok(names)
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

pub(crate) fn subdirs(dir: &Path) -> Result<Vec<PathBuf>, Error> {
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
