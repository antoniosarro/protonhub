//! What's each user's: their settings, launch options among them, and
//! which user's settings ProtonHub changes.

use std::path::{Path, PathBuf};

use super::{Shortcut, library, read_text, set_shortcut, shortcuts};
use crate::files::{subdirs, write_with_backup};
use crate::games::Error;
use crate::vdf::{self, Value};

/// Where in a user's localconfig.vdf each app's settings are, its launch
/// options among them.
const APPS: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

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

/// Sets each game's launch options to its text, or with an empty one
/// removes them: a Steam game's in the user's settings, all in one write,
/// a shortcut's in its entry. Nothing is written unless every game is
/// known.
pub fn set_launch_options(root: &Path, changes: &[(&str, &str)]) -> Result<(), Error> {
    let lib = library(root)?;
    let mut own = Vec::new();
    let mut of_shortcuts = Vec::new();
    for &(game, text) in changes {
        let found = lib
            .games
            .iter()
            .find(|g| g.id == game)
            .ok_or_else(|| Error::UnknownGame {
                id: game.to_owned(),
            })?;
        if found.source == "shortcut" {
            of_shortcuts.push((game, text));
        } else {
            own.push((game, text));
        }
    }
    if !of_shortcuts.is_empty() {
        let all = shortcuts(root)?;
        for (game, text) in of_shortcuts {
            let (_, shortcut) =
                all.iter()
                    .find(|(id, _)| id == game)
                    .ok_or_else(|| Error::UnknownGame {
                        id: game.to_owned(),
                    })?;
            let shortcut = Shortcut {
                launch_options: text.to_owned(),
                ..shortcut.clone()
            };
            set_shortcut(root, game, &shortcut)?;
        }
    }
    if own.is_empty() {
        return Ok(());
    }
    let path = local_config(root)?;
    let mut config = read_text(&path)?.ok_or_else(|| Error::Format {
        path: path.clone(),
        message: "missing; start Steam once to make it".into(),
    })?;
    for (game, text) in own {
        let mut keys = APPS.to_vec();
        keys.push(game);
        let app = config.path_mut(&keys);
        if text.is_empty() {
            app.remove("LaunchOptions");
        } else {
            app.set("LaunchOptions", Value::Str(text.to_owned()));
        }
    }
    write_with_backup(&path, vdf::to_string(&config).as_bytes())
}

/// The localconfig.vdf of the user last signed in (MostRecent in
/// loginusers.vdf, keyed by SteamID64, whose low 32 bits are the account id
/// that names their userdata directory), else of the only user.
pub(super) fn local_config(root: &Path) -> Result<PathBuf, Error> {
    local_config_of(root, crate::paths::steam_user().as_deref())
}

/// The localconfig.vdf of `chosen`, an account id, where it has one; else
/// as `local_config` picks.
pub(super) fn local_config_of(root: &Path, chosen: Option<&str>) -> Result<PathBuf, Error> {
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
