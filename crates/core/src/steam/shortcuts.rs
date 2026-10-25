//! Non-Steam shortcuts: reading them, changing one, and adding one as
//! Steam's "Add a Non-Steam Game" does.

use std::fs;
use std::path::Path;

use super::{local_config, read_binary};
use crate::files::{subdirs, write_with_backup};
use crate::games::Error;
use crate::vdf::{self, Obj, Value};

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

pub(super) fn read_shortcut(s: &Obj) -> Option<(String, Shortcut)> {
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
        return write_with_backup(&path, &vdf::to_binary(&obj));
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
    write_with_backup(&path, &vdf::to_binary(&obj))?;
    Ok(true)
}
