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
            for dir in steam::subdirs(&dir.join(sub))? {
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
mod tests {
    use std::fs;

    use super::*;
    use crate::games::tests::{steam, write};
    use crate::testutil::TempDir;

    fn item<'a>(s: &'a Storage, kind: Kind, app: &str) -> &'a Item {
        s.items
            .iter()
            .find(|i| i.kind == kind && i.app == app)
            .unwrap_or_else(|| panic!("no {kind:?} for {app}"))
    }

    #[test]
    fn finds_what_uninstalled_games_left() {
        let tmp = TempDir::new("storage");
        let launcher = steam(&tmp);
        let root = &launcher.root;
        let lib2 = tmp.join("Games");
        // Zeta (10) in the Steam directory, Alpha (20) in the second
        // library, the shortcut's, Proton Experimental's, and 30's, which
        // nothing installed has.
        write(&root.join("steamapps/compatdata/10/pfx/user.reg"), "zeta");
        write(&lib2.join("steamapps/compatdata/20/pfx/user.reg"), "alpha!");
        write(
            &root.join("steamapps/compatdata/4294967291/pfx/user.reg"),
            "s",
        );
        write(&root.join("steamapps/shadercache/1493710/cache"), "runtime");
        write(
            &root.join("steamapps/compatdata/30/pfx/user.reg"),
            "gone, big",
        );
        write(&lib2.join("steamapps/shadercache/30/cache"), "gone");
        fs::create_dir_all(root.join("steamapps/compatdata/.staging")).unwrap();

        let s = scan(&launcher).unwrap().unwrap();
        assert_eq!(s.items.len(), 6, "{:?}", s.items);
        assert_eq!(s.items[0].app, "30", "largest first");
        assert_eq!(item(&s, Kind::Prefix, "10").name.as_deref(), Some("Zeta"));
        assert_eq!(item(&s, Kind::Prefix, "20").name.as_deref(), Some("Alpha"));
        assert_eq!(
            item(&s, Kind::Prefix, "4294967291").name.as_deref(),
            Some("My Shortcut")
        );
        let runtime = item(&s, Kind::ShaderCache, "1493710");
        assert!(!runtime.orphaned, "installed, though not a game");
        let orphaned: Vec<_> = s.orphaned().map(|i| (i.kind, i.app.as_str())).collect();
        assert_eq!(orphaned.len(), 2);
        assert!(orphaned.contains(&(Kind::Prefix, "30")));
        assert!(orphaned.contains(&(Kind::ShaderCache, "30")));
        assert_eq!(item(&s, Kind::Prefix, "30").size, 9);
    }

    /// An appinfo.vdf of `apps` (id, name), in format 29, or with `v28`
    /// the one before it, with keys inline.
    fn appinfo(apps: &[(u32, &str)], v28: bool) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(if v28 { 0x0756_4428u32 } else { 0x0756_4429 }.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        if !v28 {
            b.extend(0u64.to_le_bytes()); // the key table's offset, below
        }
        let key = |b: &mut Vec<u8>, i: u32, name: &str| {
            if v28 {
                b.extend(name.as_bytes());
                b.push(0);
            } else {
                b.extend(i.to_le_bytes());
            }
        };
        for &(id, name) in apps {
            let mut kv = vec![0x00];
            key(&mut kv, 0, "appinfo");
            kv.push(0x00);
            key(&mut kv, 1, "common");
            kv.push(0x01);
            key(&mut kv, 2, "name");
            kv.extend(name.as_bytes());
            kv.extend([0x00, 0x08, 0x08, 0x08]);
            b.extend(id.to_le_bytes());
            b.extend((60 + kv.len() as u32).to_le_bytes());
            b.extend([0; 60]);
            b.extend(kv);
        }
        b.extend(0u32.to_le_bytes());
        if !v28 {
            let table = b.len() as u64;
            b[8..16].copy_from_slice(&table.to_le_bytes());
            b.extend(3u32.to_le_bytes());
            b.extend(b"appinfo\0common\0name\0");
        }
        b
    }

    #[test]
    fn names_what_is_left_from_steams_app_cache() {
        for v28 in [false, true] {
            let tmp = TempDir::new("storage-names");
            let launcher = steam(&tmp);
            let root = &launcher.root;
            write(&root.join("steamapps/compatdata/30/pfx/user.reg"), "x");
            write(&root.join("steamapps/shadercache/1493710/cache"), "x");
            write(&root.join("steamapps/shadercache/3623460739/cache"), "x");
            write(&root.join("steamapps/compatdata/10/pfx/user.reg"), "x");
            let cache = appinfo(
                &[
                    (10, "Stale"),
                    (30, "Gone Game"),
                    (1_493_710, "Proton Experimental"),
                ],
                v28,
            );
            fs::create_dir_all(root.join("appcache")).unwrap();
            fs::write(root.join("appcache/appinfo.vdf"), cache).unwrap();

            let s = scan(&launcher).unwrap().unwrap();
            assert_eq!(s.names_error, None);
            let name = |kind, app| item(&s, kind, app).name.as_deref();
            assert_eq!(name(Kind::Prefix, "30"), Some("Gone Game"), "v28: {v28}");
            assert_eq!(
                name(Kind::ShaderCache, "1493710"),
                Some("Proton Experimental")
            );
            assert_eq!(
                name(Kind::Prefix, "10"),
                Some("Zeta"),
                "the manifest's name first"
            );
            let shortcut = item(&s, Kind::ShaderCache, "3623460739");
            assert!(shortcut.name.is_none() && shortcut.shortcut() && shortcut.orphaned);
            assert!(!item(&s, Kind::Prefix, "30").shortcut());
        }
    }

    #[test]
    fn an_unreadable_app_cache_leaves_names_out_not_the_scan() {
        let tmp = TempDir::new("storage-names-bad");
        let launcher = steam(&tmp);
        let root = &launcher.root;
        write(&root.join("steamapps/compatdata/30/pfx/user.reg"), "x");
        write(
            &root.join("appcache/appinfo.vdf"),
            "\x27DV\x07 an old format",
        );
        let s = scan(&launcher).unwrap().unwrap();
        let e = s.names_error.as_deref().unwrap();
        assert!(e.contains("unknown format"), "{e}");
        assert!(item(&s, Kind::Prefix, "30").orphaned);
    }

    #[test]
    fn judges_nothing_orphaned_when_the_shortcuts_cant_be_read() {
        let tmp = TempDir::new("storage-unread");
        let launcher = steam(&tmp);
        let root = &launcher.root;
        fs::write(
            root.join("userdata/1/config/shortcuts.vdf"),
            "not binary vdf",
        )
        .unwrap();
        write(
            &root.join("steamapps/compatdata/4294967291/pfx/user.reg"),
            "s",
        );
        let s = scan(&launcher).unwrap().unwrap();
        assert!(!s.problems.is_empty());
        assert_eq!(
            s.orphaned().count(),
            0,
            "the shortcut's prefix may be in use"
        );
    }

    #[test]
    fn deletes_and_other_launchers_have_none() {
        let tmp = TempDir::new("storage-delete");
        let launcher = steam(&tmp);
        let prefix = launcher.root.join("steamapps/compatdata/30");
        write(&prefix.join("pfx/user.reg"), "gone");
        delete(&launcher, &[&prefix]).unwrap();
        assert!(!prefix.exists());
        assert!(
            fs::read_dir(launcher.root.join("steamapps/compatdata"))
                .unwrap()
                .next()
                .is_none(),
            "no staging directory left"
        );

        let heroic = Launcher {
            kind: LauncherKind::Heroic,
            ..launcher
        };
        assert!(scan(&heroic).unwrap().is_none());
    }
}
