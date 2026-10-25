//! What Steam keeps for each game besides the game: its Wine prefix
//! (`steamapps/compatdata/<id>`) and its shader cache
//! (`steamapps/shadercache/<id>`), in every library. Steam leaves both
//! behind when a game is uninstalled; those are what a cleanup deletes.

use std::path::{Path, PathBuf};

use protonhub_manifest::LauncherKind;
use serde::Serialize;

use crate::games::Error;
use crate::launcher::Launcher;
use crate::{install, steam};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Prefix,
    ShaderCache,
}

impl Kind {
    const ALL: [(Kind, &str); 2] = [
        (Kind::Prefix, "steamapps/compatdata"),
        (Kind::ShaderCache, "steamapps/shadercache"),
    ];
}

/// A prefix or shader cache.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub kind: Kind,
    /// The app id it is for, a game's or a shortcut's.
    pub app: String,
    /// The app's name: an installed game's, else what Steam's app cache
    /// has, which keeps the names of apps no longer installed. None for a
    /// shortcut no longer in Steam, or an app the cache doesn't know.
    pub name: Option<String>,
    pub dir: PathBuf,
    pub size: u64,
    /// Whether nothing installed has its app id. Never set when that
    /// can't be told: when a file listing the games couldn't be read.
    pub orphaned: bool,
}

impl Item {
    /// Whether it is a non-Steam shortcut's: their app ids have the high
    /// bit set, which Steam's own never do.
    pub fn shortcut(&self) -> bool {
        self.app.parse::<u32>().is_ok_and(|id| id >= 1 << 31)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Storage {
    /// Largest first.
    pub items: Vec<Item>,
    /// Files that couldn't be read, which leave nothing judged orphaned.
    pub problems: Vec<String>,
    /// Why Steam's app cache couldn't be read, which leaves apps not
    /// installed without names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub names_error: Option<String>,
}

impl Storage {
    pub fn orphaned(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|i| i.orphaned)
    }
}

/// The prefixes and shader caches of a Steam, or none for another
/// launcher.
pub fn scan(launcher: &Launcher) -> Result<Option<Storage>, Error> {
    if launcher.kind != LauncherKind::Steam {
        return Ok(None);
    }
    let root = &launcher.root;
    let lib = steam::library(root)?;
    // Every app, games or not: Steam's runtimes have shader caches too.
    let installed = steam::app_ids(root)?;
    let judged = lib.problems.is_empty();
    let mut items = Vec::new();
    for dir in steam::libraries(root)? {
        for (kind, sub) in Kind::ALL {
            for dir in crate::files::subdirs(&dir.join(sub))? {
                let app = dir.file_name().unwrap_or_default().to_string_lossy();
                // Only app ids; anything else there isn't Steam's.
                if !app.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                let game = lib.games.iter().find(|g| g.id == app);
                items.push(Item {
                    kind,
                    orphaned: judged && game.is_none() && !installed.iter().any(|i| *i == app),
                    app: app.into_owned(),
                    name: game.map(|g| g.name.clone()),
                    size: install::size(&dir),
                    dir,
                });
            }
        }
    }
    items.sort_by_key(|i| std::cmp::Reverse(i.size));
    let unnamed: Vec<&str> = items
        .iter()
        .filter(|i| i.name.is_none())
        .map(|i| i.app.as_str())
        .collect();
    let mut names_error = None;
    let names = steam::app_names(root, &unnamed).unwrap_or_else(|e| {
        names_error = Some(e.to_string());
        Vec::new()
    });
    for item in items.iter_mut().filter(|i| i.name.is_none()) {
        item.name = names
            .iter()
            .find(|(id, _)| *id == item.app)
            .map(|(_, n)| n.clone());
    }
    Ok(Some(Storage {
        items,
        problems: lib.problems,
        names_error,
    }))
}

/// Deletes prefixes and shader caches of `launcher`, refused while it
/// runs: a game could be running in a prefix, and Steam fills shader
/// caches in the background.
pub fn delete(launcher: &Launcher, dirs: &[&Path]) -> Result<(), Error> {
    if crate::games::is_running(launcher) {
        return Err(Error::InUse {
            launcher: launcher.kind.title(),
        });
    }
    for dir in dirs {
        install::delete(dir)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/storage.rs"]
mod tests;
