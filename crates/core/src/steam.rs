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

use crate::games::{Choice, Deck, DeckRating, Error, Game, Library};
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

/// The launch options of each game that has some, by app id: a Steam
/// game's from the settings of the user `local_config` picks, a shortcut's
/// from its entry in shortcuts.vdf.
pub fn launch_options(root: &Path) -> Result<Vec<(String, String)>, Error> {
    let mut all: Vec<(String, String)> = shortcuts(root)?
        .into_iter()
        .filter(|(_, s)| !s.launch_options.is_empty())
        .map(|(id, s)| (id, s.launch_options))
        .collect();
    let Some(config) = read_text(&local_config(root)?)? else {
        return Ok(all);
    };
    let apps = config
        .path(&APPS)
        .map(|a| a.0.as_slice())
        .unwrap_or_default();
    all.extend(apps.iter().filter_map(|(id, app)| {
        let options = app.as_obj()?.str("LaunchOptions")?;
        (!options.is_empty()).then(|| (id.clone(), options.to_owned()))
    }));
    Ok(all)
}

/// A non-Steam shortcut's settings, as its properties in Steam show them.
/// The target and start folder are as Steam keeps them, often quoted.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct Shortcut {
    pub name: String,
    pub target: String,
    pub start_dir: String,
    pub launch_options: String,
}

/// Each user's shortcuts, by app id.
pub fn shortcuts(root: &Path) -> Result<Vec<(String, Shortcut)>, Error> {
    let mut all = Vec::new();
    for user in subdirs(&root.join("userdata"))? {
        let path = user.join("config/shortcuts.vdf");
        let Some(obj) = read_binary(&path)? else {
            continue;
        };
        for (_, s) in obj
            .obj("shortcuts")
            .map(|o| o.0.as_slice())
            .unwrap_or_default()
        {
            if let Some((id, shortcut)) = s.as_obj().and_then(read_shortcut) {
                all.push((id, shortcut));
            }
        }
    }
    Ok(all)
}

/// The fields of a shortcut, the keys Steam writes them under.
const SHORTCUT_KEYS: [&str; 4] = ["AppName", "Exe", "StartDir", "LaunchOptions"];

fn read_shortcut(s: &Obj) -> Option<(String, Shortcut)> {
    let Some(Value::Int(id)) = s.get("appid") else {
        return None;
    };
    let [name, target, start_dir, launch_options] =
        SHORTCUT_KEYS.map(|k| s.str(k).unwrap_or_default().to_owned());
    // Stored signed; Steam's mapping uses the unsigned value.
    Some((
        (*id as u32).to_string(),
        Shortcut {
            name,
            target,
            start_dir,
            launch_options,
        },
    ))
}

/// Sets shortcut `game`'s settings to `to`, in the shortcuts.vdf that has
/// it; refused for a file ProtonHub couldn't write back as it was.
pub fn set_shortcut(root: &Path, game: &str, to: &Shortcut) -> Result<(), Error> {
    for user in subdirs(&root.join("userdata"))? {
        let path = user.join("config/shortcuts.vdf");
        let Some(bytes) = fs::read(&path).ok() else {
            continue;
        };
        let mut obj = vdf::parse_binary(&bytes).map_err(|source| Error::Vdf {
            path: path.clone(),
            source,
        })?;
        let Some(entry) = obj
            .path_mut(&["shortcuts"])
            .0
            .iter_mut()
            .find_map(|(_, s)| {
                let Value::Obj(s) = s else { return None };
                read_shortcut(s)
                    .is_some_and(|(id, _)| id == game)
                    .then_some(s)
            })
        else {
            continue;
        };
        if vdf::to_binary(&vdf::parse_binary(&bytes).expect("parsed above")) != bytes {
            return Err(Error::Format {
                path,
                message: "it holds values ProtonHub can't write back as they were".into(),
            });
        }
        let values = [&to.name, &to.target, &to.start_dir, &to.launch_options];
        for (key, value) in SHORTCUT_KEYS.into_iter().zip(values) {
            entry.set(key, Value::Str(value.clone()));
        }
        return crate::games::write_with_backup(&path, &vdf::to_binary(&obj));
    }
    Err(Error::UnknownGame {
        id: game.to_owned(),
    })
}

/// ProtonHub's own shortcut, and its icon: its GUI as found on PATH, which
/// stays put across upgrades (a Nix store path doesn't), else beside this
/// binary; inside its Flatpak, run through Flatpak. None where the GUI
/// isn't to be found.
pub fn own_shortcut() -> Option<(Shortcut, String)> {
    const ID: &str = "dev.antoniosarro.ProtonHub";
    let name = "ProtonHub".to_owned();
    if std::env::var_os("FLATPAK_ID").is_some() {
        let shortcut = Shortcut {
            name,
            target: "\"/usr/bin/flatpak\"".into(),
            start_dir: "\"/usr/bin\"".into(),
            launch_options: format!("run {ID}"),
        };
        return Some((shortcut, String::new()));
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|e| Some(e.parent()?.join("protonhub-gtk")));
    let exe = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|d| d.join("protonhub-gtk"))
        .chain(beside)
        .find(|p| p.is_file())?;
    let icon = exe
        .parent()
        .and_then(Path::parent)
        .map(|prefix| prefix.join(format!("share/icons/hicolor/512x512/apps/{ID}.png")))
        .filter(|i| i.is_file())
        .map(|i| i.to_string_lossy().into_owned())
        .unwrap_or_default();
    let quoted = |p: &Path| format!("\"{}\"", p.display());
    let shortcut = Shortcut {
        name,
        target: quoted(&exe),
        start_dir: exe.parent().map(quoted).unwrap_or_default(),
        launch_options: String::new(),
    };
    Some((shortcut, icon))
}

/// Adds a non-Steam shortcut for the user `local_config` picks, as Steam's
/// "Add a Non-Steam Game" does, unless they have one with that name and
/// target already; whether it added it. `target` and `start_dir` are as
/// Steam keeps them, quoted.
pub fn add_shortcut(root: &Path, add: &Shortcut, icon: &str) -> Result<bool, Error> {
    let path = local_config(root)?.with_file_name("shortcuts.vdf");
    let bytes = fs::read(&path).unwrap_or_default();
    let mut obj = if bytes.is_empty() {
        Obj::default()
    } else {
        let obj = vdf::parse_binary(&bytes).map_err(|source| Error::Vdf {
            path: path.clone(),
            source,
        })?;
        if vdf::to_binary(&obj) != bytes {
            return Err(Error::Format {
                path,
                message: "it holds values ProtonHub can't write back as they were".into(),
            });
        }
        obj
    };
    let list = obj.path_mut(&["shortcuts"]);
    let there = list.0.iter().any(|(_, s)| {
        s.as_obj()
            .and_then(read_shortcut)
            .is_some_and(|(_, s)| s.name == add.name && s.target == add.target)
    });
    if there {
        return Ok(false);
    }
    // The id tools like Steam ROM Manager give a shortcut, the CRC-32 of
    // its target and name with the top bit set; Steam keeps the id it
    // finds.
    let mut crc = flate2::Crc::new();
    crc.update(format!("{}{}", add.target, add.name).as_bytes());
    let id = (crc.sum() | 0x8000_0000) as i32;
    let mut entry = Obj::default();
    let str = |s: &str| Value::Str(s.to_owned());
    for (key, value) in [
        ("appid", Value::Int(id.into())),
        ("AppName", str(&add.name)),
        ("Exe", str(&add.target)),
        ("StartDir", str(&add.start_dir)),
        ("icon", str(icon)),
        ("ShortcutPath", str("")),
        ("LaunchOptions", str(&add.launch_options)),
        ("IsHidden", Value::Int(0)),
        ("AllowDesktopConfig", Value::Int(1)),
        ("AllowOverlay", Value::Int(1)),
        ("OpenVR", Value::Int(0)),
        ("Devkit", Value::Int(0)),
        ("DevkitGameID", str("")),
        ("DevkitOverrideAppID", Value::Int(0)),
        ("LastPlayTime", Value::Int(0)),
        ("FlatpakAppID", str("")),
        ("sortas", str("")),
        ("tags", Value::Obj(Obj::default())),
    ] {
        entry.set(key, value);
    }
    // Entries are keyed 0, 1, 2…
    let key = (0..)
        .map(|n: usize| n.to_string())
        .find(|k| list.get(k).is_none())
        .expect("a free key");
    list.set(&key, Value::Obj(entry));
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_owned(),
            source,
        })?;
    }
    crate::games::write_with_backup(&path, &vdf::to_binary(&obj))?;
    Ok(true)
}

/// Sets `game`'s launch options to `text`, or with an empty one removes
/// them: a Steam game's in the user's settings, a shortcut's in its entry.
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
        let (_, mut shortcut) = shortcuts(root)?
            .into_iter()
            .find(|(id, _)| id == game)
            .ok_or_else(|| Error::UnknownGame {
                id: game.to_owned(),
            })?;
        shortcut.launch_options = text.to_owned();
        return set_shortcut(root, game, &shortcut);
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
    local_config_of(root, crate::paths::steam_user().as_deref())
}

/// The localconfig.vdf of `chosen`, an account id, where it has one; else
/// as `local_config` picks.
fn local_config_of(root: &Path, chosen: Option<&str>) -> Result<PathBuf, Error> {
    let file = |account: &str| root.join(format!("userdata/{account}/config/localconfig.vdf"));
    if let Some(chosen) = chosen
        && file(chosen).is_file()
    {
        return Ok(file(chosen));
    }
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

/// One of Steam's users with settings on this machine.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct User {
    /// The account id, which names its userdata directory.
    pub account: String,
    /// Its name in Steam, where loginusers.vdf has it.
    pub name: Option<String>,
    /// Whether its settings are the ones launch options go to.
    pub current: bool,
}

/// Steam's users with settings here, and which one launch options go to.
pub fn users(root: &Path) -> Result<Vec<User>, Error> {
    let logins = read_text(&root.join("config/loginusers.vdf"))?;
    let names: Vec<(u64, &str)> = logins
        .as_ref()
        .and_then(|u| u.obj("users"))
        .map(|u| u.0.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|(id, u)| {
            Some((
                id.parse::<u64>().ok()? & 0xffff_ffff,
                u.as_obj()?.str("PersonaName")?,
            ))
        })
        .collect();
    let current = local_config(root).ok();
    let mut users: Vec<User> = subdirs(&root.join("userdata"))?
        .into_iter()
        .filter(|d| d.join("config/localconfig.vdf").is_file())
        .filter_map(|d| {
            let account = d.file_name()?.to_string_lossy().into_owned();
            let id: u64 = account.parse().ok().filter(|&id| id != 0)?;
            Some(User {
                name: names
                    .iter()
                    .find(|(n, _)| *n == id)
                    .map(|(_, name)| (*name).to_owned()),
                current: current.as_deref() == Some(&*d.join("config/localconfig.vdf")),
                account,
            })
        })
        .collect();
    users.sort_by(|a, b| a.account.cmp(&b.account));
    Ok(users)
}

/// A binary KeyValues file, or none where there isn't one.
fn read_binary(path: &Path) -> Result<Option<Obj>, Error> {
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
    vdf::parse_binary(&bytes)
        .map(Some)
        .map_err(|source| Error::Vdf {
            path: path.to_owned(),
            source,
        })
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

/// The names Steam's app cache has for the apps `ids`, which it keeps for
/// apps no longer installed too.
pub(crate) fn app_names(root: &Path, ids: &[&str]) -> Result<Vec<(String, String)>, Error> {
    let mut names = Vec::new();
    app_info(root, ids, |id, info| {
        if let Some(name) = info
            .path(&["appinfo", "common"])
            .and_then(|c| c.str("name"))
        {
            names.push((id.to_string(), name.to_owned()));
        }
    })?;
    Ok(names)
}

/// Valve's Steam Deck ratings of the apps `ids` it rated, from Steam's app
/// cache.
fn deck_ratings(root: &Path, ids: &[&str]) -> Result<Vec<(String, Deck)>, Error> {
    let mut decks = Vec::new();
    app_info(root, ids, |id, info| {
        let Some(compat) = info.path(&["appinfo", "common", "steam_deck_compatibility"]) else {
            return;
        };
        let category = match compat.get("category") {
            Some(Value::Int(n)) => *n,
            Some(Value::Str(s)) => s.parse().unwrap_or(0),
            _ => 0,
        };
        let rating = match category {
            1 => DeckRating::Unsupported,
            2 => DeckRating::Playable,
            3 => DeckRating::Verified,
            _ => return,
        };
        let runtime = compat
            .obj("configuration")
            .and_then(|c| c.str("recommended_runtime"))
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        decks.push((id.to_string(), Deck { rating, runtime }));
    })?;
    Ok(decks)
}

/// Each of the apps `ids` in Steam's app cache (appcache/appinfo.vdf),
/// with its info. Entries of other apps are skipped unread. Formats 28 and
/// 29 (2024 on, its keys in a table at the end); an older one is an error.
fn app_info(root: &Path, ids: &[&str], mut each: impl FnMut(u64, &Obj)) -> Result<(), Error> {
    let path = root.join("appcache/appinfo.vdf");
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
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
            each(id, &obj);
        }
        pos = end;
    }
    Ok(())
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
    fn adds_a_shortcut_once_for_the_user_signed_in_last() {
        let home = TempDir::new("steam-add");
        let root = home.join("Steam");
        let config = root.join("userdata/1/config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("localconfig.vdf"),
            "\"UserLocalConfigStore\"\n{\n}\n",
        )
        .unwrap();
        let add = Shortcut {
            name: "ProtonHub".into(),
            target: "\"/usr/bin/protonhub-gtk\"".into(),
            start_dir: "\"/usr/bin\"".into(),
            launch_options: String::new(),
        };
        assert!(add_shortcut(&root, &add, "").unwrap());
        assert!(!add_shortcut(&root, &add, "").unwrap(), "once");
        let all = shortcuts(&root).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, add);
        // The CRC-32 of target and name, with the top bit set.
        let mut crc = flate2::Crc::new();
        crc.update(b"\"/usr/bin/protonhub-gtk\"ProtonHub");
        assert_eq!(all[0].0, (crc.sum() | 0x8000_0000).to_string());
    }

    #[test]
    fn a_chosen_user_gets_the_launch_options_where_they_have_settings() {
        let home = TempDir::new("steam-users");
        let root = home.join("Steam");
        for account in ["1", "2"] {
            let path = root.join(format!("userdata/{account}/config/localconfig.vdf"));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "\"UserLocalConfigStore\"\n{\n}\n").unwrap();
        }
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(
            root.join("config/loginusers.vdf"),
            "\"users\"\n{\n\t\"76561197960265729\"\n\t{\n\t\t\"PersonaName\"\t\t\"Ann\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n",
        )
        .unwrap();
        let of = |chosen| local_config_of(&root, chosen).unwrap();
        assert!(
            of(None).ends_with("userdata/1/config/localconfig.vdf"),
            "signed in last"
        );
        assert!(of(Some("2")).ends_with("userdata/2/config/localconfig.vdf"));
        assert!(
            of(Some("9")).ends_with("userdata/1/config/localconfig.vdf"),
            "no settings: the last"
        );
        let users = users(&root).unwrap();
        assert_eq!(
            users
                .iter()
                .map(|u| (u.account.as_str(), u.name.as_deref()))
                .collect::<Vec<_>>(),
            [("1", Some("Ann")), ("2", None)]
        );
    }

    #[test]
    fn a_shortcut_is_edited_or_its_file_left_alone() {
        let home = TempDir::new("steam-shortcut");
        let root = home.join("Steam");
        let file = |extra: &[u8]| {
            let mut b = vec![0x00];
            b.extend(b"shortcuts\0\x000\0\x02appid\0");
            b.extend((-5i32).to_le_bytes());
            b.extend(b"\x01AppName\0Game\0\x01Exe\0\"/g/game.exe\"\0");
            b.extend(extra);
            b.extend([0x08, 0x08, 0x08]);
            b
        };
        let path = root.join("userdata/1/config/shortcuts.vdf");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, file(&[])).unwrap();
        let (id, mut s) = shortcuts(&root).unwrap().remove(0);
        assert_eq!(
            (id.as_str(), s.target.as_str()),
            ("4294967291", "\"/g/game.exe\"")
        );
        s.name = "Renamed".into();
        s.start_dir = "\"/g\"".into();
        set_shortcut(&root, &id, &s).unwrap();
        assert_eq!(shortcuts(&root).unwrap()[0].1, s);

        // A float doesn't write back as it was: the file is left alone.
        let mut float = vec![0x03];
        float.extend(b"x\0");
        float.extend(1.5f32.to_le_bytes());
        fs::write(&path, file(&float)).unwrap();
        let err = set_shortcut(&root, &id, &s).unwrap_err();
        assert!(matches!(err, Error::Format { .. }), "{err}");
        assert_eq!(fs::read(&path).unwrap(), file(&float));
    }

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
