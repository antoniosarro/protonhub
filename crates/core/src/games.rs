//! Games and which compatibility tool each uses, for the launchers that
//! keep that per game (Steam and Heroic), and from it which installed tools
//! nothing uses.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use protonhub_manifest::{LauncherKind, Registry};
use serde::Serialize;

use crate::install::{self, Entry};
use crate::launcher::{Launcher, Packaging};
use crate::{heroic, steam, vdf};

/// A tool a game can use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Choice {
    /// What the launcher stores: Steam's internal name, Heroic's binary.
    pub id: String,
    pub name: String,
    /// The tool's directory, when it is known.
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Game {
    /// Steam's app id, Heroic's app name.
    pub id: String,
    pub name: String,
    /// Where it comes from: steam or shortcut; epic, gog, amazon or sideload.
    pub source: &'static str,
    /// The tool it runs with, its own or the launcher's default.
    pub tool: Option<Choice>,
    /// Whether the tool is the game's own setting, not the default.
    pub explicit: bool,
    /// An Epic game's namespace, which anti-cheat data is keyed by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epic_namespace: Option<String>,
    /// A picture of the game: a local file's path, or a URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Library {
    pub games: Vec<Game>,
    /// What games without a setting of their own use.
    pub default: Option<Choice>,
    pub choices: Vec<Choice>,
    /// Files that couldn't be read, which left something out.
    pub problems: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: {source}")]
    Vdf {
        path: PathBuf,
        source: vdf::ParseError,
    },
    #[error("{path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("{path}: {message}")]
    Format { path: PathBuf, message: String },
    #[error("{launcher} is running; quit it first, or it overwrites the change when it exits")]
    Running { launcher: &'static str },
    #[error(
        "{launcher} is running, and a game or its shader processing may be using this; quit it first"
    )]
    InUse { launcher: &'static str },
    #[error("no installed game {id}")]
    UnknownGame { id: String },
    #[error("no compatibility tool {id}")]
    UnknownTool { id: String },
    #[error(
        "Heroic has no settings file for {game} yet; open the game's settings in Heroic once, then try again"
    )]
    NotConfigured { game: String },
    #[error("Heroic games always name their Wine or Proton; choose one")]
    NeedsTool,
    #[error("ProtonHub can't set the tool of {launcher} games")]
    Unsupported { launcher: &'static str },
    #[error(
        "ProtonHub can't change a non-Steam game's launch options yet; set them in its properties in Steam"
    )]
    Shortcut,
    #[error(transparent)]
    Install(#[from] install::Error),
}

/// The other launchers in `launcher`'s home: Heroic runs their tools too.
fn beside(launcher: &Launcher) -> Vec<Launcher> {
    let found = launcher.home().map(|h| crate::launcher::detect(&h));
    found
        .into_iter()
        .flatten()
        .filter(|l| l.kind != launcher.kind)
        .collect()
}

/// The launcher's games, or none for a launcher that doesn't keep a tool
/// per game.
pub fn library(launcher: &Launcher) -> Result<Option<Library>, Error> {
    match launcher.kind {
        LauncherKind::Steam => steam::library(&launcher.root).map(Some),
        LauncherKind::Heroic => heroic::library(&launcher.root, &beside(launcher)).map(Some),
        LauncherKind::Lutris | LauncherKind::Bottles | LauncherKind::Winezgui => Ok(None),
    }
}

/// Sets which tool `game` uses: a choice's id, or `None` for the
/// launcher's default. Refused while the launcher runs, since both write
/// their settings back when they exit.
pub fn assign(launcher: &Launcher, game: &str, tool: Option<&str>) -> Result<(), Error> {
    assign_many(launcher, &[(game, tool)])
}

/// Sets the tools of several games at once, as `assign` does each.
pub fn assign_many(launcher: &Launcher, moves: &[(&str, Option<&str>)]) -> Result<(), Error> {
    if is_running(launcher) {
        return Err(Error::Running {
            launcher: launcher.kind.title(),
        });
    }
    match launcher.kind {
        LauncherKind::Steam => steam::assign(&launcher.root, moves),
        LauncherKind::Heroic => moves.iter().try_for_each(|&(game, tool)| {
            heroic::assign(&launcher.root, &beside(launcher), game, tool)
        }),
        kind => Err(Error::Unsupported {
            launcher: kind.title(),
        }),
    }
}

/// Moves every game set to a tool matching `from` (its id or name) to the
/// tool `to`, and returns their names. Games on the launcher's default
/// stay on it.
pub fn reassign(launcher: &Launcher, from: &regex::Regex, to: &str) -> Result<Vec<String>, Error> {
    let Some(lib) = library(launcher)? else {
        return Err(Error::Unsupported {
            launcher: launcher.kind.title(),
        });
    };
    let to = lib
        .choices
        .iter()
        .find(|c| c.id == to || c.name == to)
        .ok_or_else(|| Error::UnknownTool { id: to.to_owned() })?;
    let moving: Vec<_> = lib
        .games
        .iter()
        .filter(|g| g.explicit)
        .filter(|g| {
            g.tool
                .as_ref()
                .is_some_and(|t| t.id != to.id && (from.is_match(&t.id) || from.is_match(&t.name)))
        })
        .collect();
    let moves: Vec<_> = moving
        .iter()
        .map(|g| (g.id.as_str(), Some(to.id.as_str())))
        .collect();
    if !moves.is_empty() {
        assign_many(launcher, &moves)?;
    }
    Ok(moving.iter().map(|g| g.name.clone()).collect())
}

/// Whether the launcher runs, so its per-game settings can't be changed.
/// Any process of that name counts: Flatpak and native alike.
pub fn is_running(launcher: &Launcher) -> bool {
    match launcher.kind {
        LauncherKind::Steam => running("steam"),
        LauncherKind::Heroic => running("heroic"),
        LauncherKind::Lutris | LauncherKind::Bottles | LauncherKind::Winezgui => false,
    }
}

/// Whether processes outside ProtonHub can be seen: inside a Flatpak, only
/// the sandbox's are, so a running launcher looks stopped.
pub fn sees_launchers() -> bool {
    !std::path::Path::new("/.flatpak-info").exists()
}

/// Whether a process named `name` runs (its /proc comm, as `pgrep -x`).
fn running(name: &str) -> bool {
    // Tests mustn't depend on whether Steam is open on the machine.
    if cfg!(test) {
        return false;
    }
    let Ok(procs) = fs::read_dir("/proc") else {
        return false;
    };
    procs
        .flatten()
        .any(|p| fs::read_to_string(p.path().join("comm")).is_ok_and(|c| c.trim_end() == name))
}

/// Replaces `path` with `data`, keeping the previous version next to it as
/// `<name>.protonhub-backup`. The write is atomic: a crash leaves the old
/// file or the new one.
pub(crate) fn write_with_backup(path: &Path, data: &[u8]) -> Result<(), Error> {
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| Error::Io { path, source }
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if path.exists() {
        fs::copy(
            path,
            path.with_file_name(format!("{name}.protonhub-backup")),
        )
        .map_err(io(path))?;
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io(dir))?;
    }
    let tmp = path.with_file_name(format!(".{name}.protonhub-tmp"));
    fs::write(&tmp, data).map_err(io(&tmp))?;
    fs::rename(&tmp, path).map_err(io(path))
}

/// An installed tool, what it takes on disk, and what uses it.
#[derive(Debug, Clone, Serialize)]
pub struct Usage {
    #[serde(flatten)]
    pub entry: Entry,
    pub size: u64,
    /// None where ProtonHub can't tell (a launcher without per-game tools,
    /// or a directory like DXVK's that games don't name).
    pub users: Option<Users>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Users {
    /// The names of the games that run with it.
    pub games: Vec<String>,
    /// Whether it is the launcher's default.
    pub default: bool,
}

impl Usage {
    pub fn unused(&self) -> bool {
        self.users
            .as_ref()
            .is_some_and(|u| u.games.is_empty() && !u.default)
    }
}

/// For each launcher, every directory in its tool directories with its
/// size and, where known, the games using it. Usage is counted across all
/// of `launchers`, since a game in one can run a tool from another's
/// directory: Heroic can use Steam's.
pub fn usage(
    registry: &Registry,
    launchers: &[Launcher],
) -> Result<Vec<(Launcher, Vec<Usage>)>, Error> {
    let libraries = launchers
        .iter()
        .map(|l| Ok((l, library(l)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    let lutris = lutris_games(launchers)?;
    let mut all = Vec::new();
    for (launcher, own) in &libraries {
        // Only directories of tools games are set to can be judged; where
        // a launcher doesn't keep a tool per game, nothing can.
        let runner_dirs: Vec<PathBuf> = match launcher.kind {
            LauncherKind::Steam => vec![launcher.root.join("compatibilitytools.d")],
            LauncherKind::Heroic => heroic::RUNNER_DIRS
                .iter()
                .map(|d| launcher.root.join(d))
                .collect(),
            _ => Vec::new(),
        };
        let mut usage = Vec::new();
        for td in install::inventory(registry, launcher)? {
            let judged = own.is_some() && runner_dirs.contains(&td.dir);
            for entry in td.entries {
                let users = judged.then(|| users(&libraries, &lutris, launcher, &entry.dir));
                usage.push(Usage {
                    size: install::size(&entry.dir),
                    entry,
                    users,
                });
            }
        }
        all.push(((*launcher).clone(), usage));
    }
    Ok(all)
}

/// One of Valve's Protons, its size, and the games that run it.
#[derive(Debug, Clone, Serialize)]
pub struct OfficialUsage {
    #[serde(flatten)]
    pub official: steam::Official,
    pub size: u64,
    pub users: Users,
}

/// Valve's Protons in each Steam among `launchers`, with what uses them.
/// Apart from `usage`: they're Steam's apps, removed with
/// `steam::uninstall`, never by a cleanup.
pub fn officials(launchers: &[Launcher]) -> Result<Vec<(Launcher, Vec<OfficialUsage>)>, Error> {
    let libraries = launchers
        .iter()
        .map(|l| Ok((l, library(l)?)))
        .collect::<Result<Vec<_>, Error>>()?;
    let lutris = lutris_games(launchers)?;
    let mut all = Vec::new();
    for launcher in launchers.iter().filter(|l| l.kind == LauncherKind::Steam) {
        let found = steam::officials(&launcher.root)?
            .into_iter()
            .map(|official| OfficialUsage {
                size: install::size(&official.dir),
                users: users(&libraries, &lutris, launcher, &official.dir),
                official,
            })
            .collect();
        all.push((launcher.clone(), found));
    }
    Ok(all)
}

/// The games, in any launcher, that run the tool in `dir`. Games of
/// another launcher than `owner` are named with it.
fn users(
    libraries: &[(&Launcher, Option<Library>)],
    lutris: &[(String, String)],
    owner: &Launcher,
    dir: &Path,
) -> Users {
    let real = dir.canonicalize().ok();
    let runs = |c: &Option<Choice>| {
        c.as_ref()
            .and_then(|c| c.dir.as_ref())
            .is_some_and(|d| d == dir || real.is_some() && d.canonicalize().ok() == real)
    };
    let mut users = Users::default();
    for (launcher, lib) in libraries {
        let Some(lib) = lib else { continue };
        users.default |= launcher == &owner && runs(&lib.default);
        for g in lib.games.iter().filter(|g| runs(&g.tool)) {
            users.games.push(if launcher == &owner {
                g.name.clone()
            } else {
                format!("{} ({})", g.name, launcher.kind.name())
            });
        }
    }
    // Lutris games name their own runners by directory name; another
    // launcher's tool only by its path.
    let paths: Vec<String> = [Some(dir.to_owned()), real.clone()]
        .into_iter()
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    for (game, config) in lutris {
        if paths.iter().any(|p| mentions(config, p)) {
            users.games.push(format!("{game} (lutris)"));
        }
    }
    users
}

/// Whether `text` has `path` in it as a whole path or a prefix of one.
fn mentions(text: &str, path: &str) -> bool {
    text.match_indices(path).any(|(i, _)| {
        matches!(
            text[i + path.len()..].chars().next(),
            None | Some('/' | '"' | '\'' | '\n' | ' ')
        )
    })
}

/// The name and configuration text of every Lutris game, to find the ones
/// that run another launcher's tool by its path.
fn lutris_games(launchers: &[Launcher]) -> Result<Vec<(String, String)>, Error> {
    let mut games = Vec::new();
    for l in launchers.iter().filter(|l| l.kind == LauncherKind::Lutris) {
        // Lutris keeps game configurations in its data directory; older
        // versions kept them in ~/.config/lutris/games.
        let mut dirs = vec![l.root.join("games")];
        if l.packaging == Packaging::Native
            && let Some(home) = crate::paths::home()
        {
            dirs.push(home.join(".config/lutris/games"));
        }
        for dir in dirs {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for e in entries.flatten() {
                let path = e.path();
                if path.extension().is_none_or(|x| x != "yml") {
                    continue;
                }
                let text = fs::read_to_string(&path).map_err(|source| Error::Io {
                    path: path.clone(),
                    source,
                })?;
                // clair-obscur-expedition-33-1745603321.yml: the slug,
                // without the time Lutris added.
                let stem = path.file_stem().unwrap_or_default().to_string_lossy();
                let name = stem
                    .rsplit_once('-')
                    .filter(|(_, t)| t.bytes().all(|b| b.is_ascii_digit()))
                    .map_or(&*stem, |(n, _)| n);
                games.push((name.to_owned(), text));
            }
        }
    }
    Ok(games)
}

#[cfg(test)]
pub(crate) mod tests {
    use protonhub_manifest::Registry;

    use super::*;
    use crate::testutil::TempDir;

    pub(crate) fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn registry() -> Registry {
        let mut r = Registry::default();
        r.add(
            Path::new("t.toml"),
            r#"
            [ge]
            name = "GE"
            source = { github = "o/r" }
            asset = { x86_64 = 'x' }
            launchers = { steam = "compatibilitytools.d", heroic = "tools/proton" }
            [dxvk]
            name = "DXVK"
            source = { github = "o/d" }
            asset = { x86_64 = 'x' }
            launchers = { heroic = "tools/dxvk" }
            "#,
        )
        .unwrap();
        r
    }

    fn compat_tool(root: &Path, name: &str) {
        write(
            &root.join(format!("compatibilitytools.d/{name}/compatibilitytool.vdf")),
            &format!(
                "\"compatibilitytools\" {{ \"compat_tools\" {{ \"{name}\" // internal\n {{ \"install_path\" \".\" \"display_name\" \"{name} (GE)\" }} }} }}"
            ),
        );
    }

    /// A Steam with two games (one mapped to GE-Proton11-7, one on the
    /// default proton_9), a shortcut, a Proton 9.0 install, and an unused
    /// GE-Proton10-1.
    pub(crate) fn steam(home: &Path) -> Launcher {
        let root = home.join("Steam");
        let lib2 = home.join("Games");
        write(
            &root.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                root.display(),
                lib2.display()
            ),
        );
        for (lib, id, name) in [
            (&root, "10", "Zeta"),
            (&lib2, "20", "Alpha"),
            (&root, "1493710", "Proton Experimental"),
        ] {
            write(
                &lib.join(format!("steamapps/appmanifest_{id}.acf")),
                &format!(
                    "\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n}}\n"
                ),
            );
        }
        fs::create_dir_all(lib2.join("steamapps/common/Proton 9.0")).unwrap();
        compat_tool(&root, "GE-Proton11-7");
        compat_tool(&root, "GE-Proton10-1");
        write(
            &root.join("config/config.vdf"),
            "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"0\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton_9\"\n\t\t\t\t\t}\n\t\t\t\t\t\"10\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton11-7\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"Other\"\t\t\"kept\"\n}\n",
        );
        let mut shortcut = vec![0x00];
        shortcut.extend(b"shortcuts\0\x000\0\x02appid\0");
        shortcut.extend((-5i32).to_le_bytes());
        shortcut.extend(b"\x01AppName\0My Shortcut\0\x08\x08\x08");
        write(&root.join("userdata/1/config/shortcuts.vdf"), "");
        fs::write(root.join("userdata/1/config/shortcuts.vdf"), shortcut).unwrap();
        Launcher {
            kind: LauncherKind::Steam,
            packaging: Packaging::Native,
            root,
        }
    }

    #[test]
    fn valves_protons_and_uninstalling_one() {
        let home = TempDir::new("steam-officials");
        let l = steam(&home);
        let lib2 = home.join("Games");
        let manifest = |lib: &Path, id: &str, dir: &str| {
            write(
                &lib.join(format!("steamapps/appmanifest_{id}.acf")),
                &format!(
                    "\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"installdir\"\t\t\"{dir}\"\n}}\n"
                ),
            );
        };
        manifest(&lib2, "2805730", "Proton 9.0");
        manifest(&l.root, "3658110", "Proton 10.0");
        write(&l.root.join("steamapps/common/Proton 10.0/proton"), "");
        // Installed by manifest, but its files are gone: not listed.
        manifest(&l.root, "2180100", "Proton Hotfix");

        let found = &officials(std::slice::from_ref(&l)).unwrap()[0].1;
        let summary: Vec<_> = found
            .iter()
            .map(|o| {
                (
                    o.official.name.as_str(),
                    o.official.appid.as_str(),
                    o.users.default,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("Proton 10.0", "3658110", false),
                ("Proton 9.0", "2805730", true)
            ],
            "Proton 9.0 is the default"
        );

        let ten = &found[0].official;
        steam::uninstall(ten).unwrap();
        assert!(!ten.manifest.exists() && !ten.dir.exists());
        assert_eq!(steam::officials(&l.root).unwrap().len(), 1);
    }

    #[test]
    fn steam_games_and_their_tools() {
        let home = TempDir::new("steam-games");
        let l = steam(&home);
        let lib = library(&l).unwrap().unwrap();
        assert!(lib.problems.is_empty(), "{:?}", lib.problems);
        let games: Vec<_> = lib
            .games
            .iter()
            .map(|g| {
                (
                    g.id.as_str(),
                    g.name.as_str(),
                    g.source,
                    g.tool.as_ref().map(|t| t.id.as_str()),
                    g.explicit,
                )
            })
            .collect();
        assert_eq!(
            games,
            [
                ("20", "Alpha", "steam", Some("proton_9"), false),
                (
                    "4294967291",
                    "My Shortcut",
                    "shortcut",
                    Some("proton_9"),
                    false
                ),
                ("10", "Zeta", "steam", Some("GE-Proton11-7"), true),
            ]
        );
        let choices: Vec<_> = lib
            .choices
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect();
        assert_eq!(
            choices,
            [
                ("GE-Proton10-1", "GE-Proton10-1 (GE)"),
                ("GE-Proton11-7", "GE-Proton11-7 (GE)"),
                ("proton_9", "Proton 9.0"),
            ]
        );
    }

    #[test]
    fn steam_assign_rewrites_only_the_mapping() {
        let home = TempDir::new("steam-assign");
        let l = steam(&home);
        let config = l.root.join("config/config.vdf");
        let before = fs::read_to_string(&config).unwrap();

        steam::assign(&l.root, &[("20", Some("GE-Proton10-1"))]).unwrap();
        steam::assign(&l.root, &[("10", None)]).unwrap();
        let lib = library(&l).unwrap().unwrap();
        let tool = |id: &str| {
            let g = lib.games.iter().find(|g| g.id == id).unwrap();
            (g.tool.as_ref().unwrap().id.clone(), g.explicit)
        };
        assert_eq!(tool("20"), ("GE-Proton10-1".into(), true));
        assert_eq!(tool("10"), ("proton_9".into(), false));
        let after = fs::read_to_string(&config).unwrap();
        assert!(after.contains("\t\"Other\"\t\t\"kept\"\n"));
        assert!(after.contains("\t\t\t\t\t\"20\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton10-1\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t\t\"priority\"\t\t\"250\"\n"));
        // The backup is the version before the last write.
        let backup = fs::read_to_string(l.root.join("config/config.vdf.protonhub-backup")).unwrap();
        assert_ne!(backup, before);
        assert!(backup.contains("\"10\""));

        let unknown = steam::assign(&l.root, &[("20", Some("nope"))]).unwrap_err();
        assert!(matches!(unknown, Error::UnknownTool { .. }), "{unknown}");
        let game = steam::assign(&l.root, &[("99", None)]).unwrap_err();
        assert!(matches!(game, Error::UnknownGame { .. }), "{game}");
    }

    #[test]
    fn reassign_moves_only_games_set_to_a_matching_tool() {
        let home = TempDir::new("reassign");
        let l = steam(&home);
        let from = regex::Regex::new("^GE-Proton11-").unwrap();
        // Zeta is set to GE-Proton11-7; Alpha and the shortcut follow the
        // default, proton_9, and stay on it.
        assert_eq!(reassign(&l, &from, "GE-Proton10-1").unwrap(), ["Zeta"]);
        let lib = library(&l).unwrap().unwrap();
        let tools: Vec<_> = lib
            .games
            .iter()
            .map(|g| {
                (
                    g.name.as_str(),
                    g.tool.as_ref().unwrap().id.as_str(),
                    g.explicit,
                )
            })
            .collect();
        assert_eq!(
            tools,
            [
                ("Alpha", "proton_9", false),
                ("My Shortcut", "proton_9", false),
                ("Zeta", "GE-Proton10-1", true),
            ]
        );
        assert!(reassign(&l, &from, "GE-Proton10-1").unwrap().is_empty());
        assert!(matches!(
            reassign(&l, &from, "nope"),
            Err(Error::UnknownTool { .. })
        ));
    }

    #[test]
    fn steam_usage_flags_what_nothing_uses() {
        let home = TempDir::new("steam-usage");
        let l = steam(&home);
        let usage = usage(&registry(), std::slice::from_ref(&l))
            .unwrap()
            .remove(0)
            .1;
        let rows: Vec<_> = usage
            .iter()
            .map(|u| {
                (
                    u.entry
                        .dir
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    u.users.as_ref().map(|u| u.games.clone()),
                    u.unused(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("GE-Proton10-1".into(), Some(vec![]), true),
                ("GE-Proton11-7".into(), Some(vec!["Zeta".to_owned()]), false),
            ]
        );
        assert!(usage[0].size > 0);
    }

    /// A Heroic with an Epic game set to GE-Proton10-34, a sideloaded one on
    /// the default Wine-GE, a native Linux game, and GE-Proton9-1 unused.
    fn heroic(home: &Path) -> Launcher {
        let root = home.join("heroic");
        for bin in [
            "tools/proton/GE-Proton10-34/proton",
            "tools/proton/GE-Proton9-1/proton",
            "tools/wine/lutris-GE-Proton8-26-x86_64/bin/wine",
            "tools/dxvk/dxvk-2.0/x64/d3d11.dll",
        ] {
            write(&root.join(bin), "");
        }
        let wine = root.join("tools/wine/lutris-GE-Proton8-26-x86_64/bin/wine");
        let proton = root.join("tools/proton/GE-Proton10-34/proton");
        write(
            &root.join("config.json"),
            &serde_json::json!({ "defaultSettings": { "wineVersion": { "bin": wine, "name": "Wine-GE", "type": "wine" } } }).to_string(),
        );
        write(
            &root.join("store_cache/legendary_library.json"),
            &serde_json::json!({ "library": [
                { "app_name": "epic1", "title": "Epic Game", "is_installed": true, "install": { "platform": "Windows" } },
                { "app_name": "epic2", "title": "Native", "is_installed": true, "install": { "platform": "linux" } },
                { "app_name": "epic3", "title": "Not Here", "is_installed": false, "install": {} },
            ] })
            .to_string(),
        );
        write(
            &root.join("sideload_apps/library.json"),
            &serde_json::json!({ "games": [
                { "app_name": "side1", "title": "Sideloaded", "is_installed": true, "install": { "platform": "Windows" } },
            ] })
            .to_string(),
        );
        write(
            &root.join("GamesConfig/epic1.json"),
            &serde_json::to_string_pretty(&serde_json::json!({
                "epic1": { "zfirst": 1, "wineVersion": { "bin": proton, "name": "GE-Proton10-34", "type": "proton" }, "afterwards": true },
                "version": "v0",
            }))
            .unwrap(),
        );
        write(
            &root.join("GamesConfig/side1.json"),
            &serde_json::json!({ "side1": { "language": "" } }).to_string(),
        );
        Launcher {
            kind: LauncherKind::Heroic,
            packaging: Packaging::Native,
            root,
        }
    }

    // Heroic can run Steam's tools: one a Heroic game uses isn't unused,
    // though no Steam game does.
    #[test]
    fn usage_counts_games_of_other_launchers() {
        let home = TempDir::new("cross-usage");
        let steam = steam(&home);
        let heroic = heroic(&home);
        let steam_ge10 = steam.root.join("compatibilitytools.d/GE-Proton10-1/proton");
        let config = heroic.root.join("GamesConfig/side1.json");
        fs::write(
            &config,
            serde_json::json!({ "side1": { "wineVersion": { "bin": steam_ge10, "name": "GE-Proton10-1", "type": "proton" } } }).to_string(),
        )
        .unwrap();
        let all = usage(&registry(), &[steam, heroic]).unwrap();
        let ge10 = all[0]
            .1
            .iter()
            .find(|u| u.entry.dir.ends_with("GE-Proton10-1"))
            .unwrap();
        assert!(!ge10.unused());
        assert_eq!(ge10.users.as_ref().unwrap().games, ["Sideloaded (heroic)"]);
    }

    #[test]
    fn paths_are_matched_whole() {
        assert!(mentions("path: /a/GE-1/proton", "/a/GE-1"));
        assert!(mentions("x: /a/GE-1", "/a/GE-1"));
        assert!(!mentions("x: /a/GE-10/proton", "/a/GE-1"));
    }

    #[test]
    fn heroic_games_and_their_tools() {
        let home = TempDir::new("heroic-games");
        let l = heroic(&home);
        let lib = library(&l).unwrap().unwrap();
        assert!(lib.problems.is_empty(), "{:?}", lib.problems);
        let games: Vec<_> = lib
            .games
            .iter()
            .map(|g| {
                (
                    g.id.as_str(),
                    g.source,
                    g.tool.as_ref().map(|t| t.name.as_str()),
                    g.explicit,
                )
            })
            .collect();
        assert_eq!(
            games,
            [
                ("epic1", "epic", Some("GE-Proton10-34"), true),
                ("side1", "sideload", Some("Wine-GE"), false),
            ]
        );
        assert_eq!(lib.choices.len(), 3);
        assert_eq!(
            lib.default.unwrap().dir.unwrap(),
            l.root.join("tools/wine/lutris-GE-Proton8-26-x86_64")
        );
    }

    #[test]
    fn heroic_lists_other_launchers_tools_too() {
        let home = TempDir::new("heroic-beside");
        let l = heroic(&home);
        let steam = home.join("steam");
        let lutris = home.join("lutris");
        for bin in [
            "steam/compatibilitytools.d/GE-Proton10-34/proton",
            "steam/compatibilitytools.d/GE-Proton11-2/proton",
            "steam/steamapps/common/Proton 10.0/proton",
            "steam/steamapps/common/Some Game/game.exe",
            "lutris/runners/wine/wine-ge-8-26-x86_64/bin/wine",
            "lutris/runners/wine/ge-proton/files/bin/wine",
            // A partial download, not a build.
            "heroic/tools/wine/wine.tar.xz.$$16$PART",
        ] {
            write(&home.join(bin), "");
        }
        let other = |kind, root: &Path| Launcher {
            kind,
            packaging: crate::launcher::Packaging::Native,
            root: root.to_owned(),
        };
        let beside = [
            other(LauncherKind::Steam, &steam),
            other(LauncherKind::Lutris, &lutris),
        ];
        let names = || {
            let lib = heroic::library(&l.root, &beside).unwrap();
            lib.choices.into_iter().map(|c| c.name).collect::<Vec<_>>()
        };
        assert_eq!(
            names(),
            [
                "GE-Proton10-34",
                "GE-Proton9-1",
                "lutris-GE-Proton8-26-x86_64",
                "GE-Proton10-34 (Steam)",
                "GE-Proton11-2",
                "ge-proton",
                "wine-ge-8-26-x86_64",
            ],
            "Valve's Protons only when Heroic shows them"
        );
        write(
            &l.root.join("config.json"),
            &serde_json::json!({ "defaultSettings": { "showValveProton": true } }).to_string(),
        );
        assert!(names().contains(&"Proton 10.0".to_owned()));

        let ge11 = steam.join("compatibilitytools.d/GE-Proton11-2/proton");
        heroic::assign(&l.root, &beside, "epic1", Some(&ge11.to_string_lossy())).unwrap();
        let text = fs::read_to_string(l.root.join("GamesConfig/epic1.json")).unwrap();
        assert!(text.contains("GE-Proton11-2"), "{text}");
    }

    #[test]
    fn heroic_assign_keeps_the_other_settings() {
        let home = TempDir::new("heroic-assign");
        let l = heroic(&home);
        let ge9 = l.root.join("tools/proton/GE-Proton9-1/proton");
        heroic::assign(&l.root, &[], "epic1", Some(&ge9.to_string_lossy())).unwrap();
        let text = fs::read_to_string(l.root.join("GamesConfig/epic1.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["epic1"]["wineVersion"]["name"], "GE-Proton9-1");
        assert_eq!(v["epic1"]["wineVersion"]["type"], "proton");
        assert_eq!(v["version"], "v0");
        let keys: Vec<_> = v["epic1"].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["zfirst", "wineVersion", "afterwards"],
            "key order is kept"
        );

        let unconfigured =
            heroic::assign(&l.root, &[], "epic3", Some(&ge9.to_string_lossy())).unwrap_err();
        assert!(
            matches!(unconfigured, Error::NotConfigured { .. }),
            "{unconfigured}"
        );
        assert!(matches!(
            heroic::assign(&l.root, &[], "epic1", None),
            Err(Error::NeedsTool)
        ));
    }

    #[test]
    fn heroic_usage_judges_only_runner_directories() {
        let home = TempDir::new("heroic-usage");
        let l = heroic(&home);
        let rows: Vec<_> = usage(&registry(), std::slice::from_ref(&l))
            .unwrap()
            .remove(0)
            .1
            .into_iter()
            .map(|u| {
                (
                    u.entry
                        .dir
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    u.users.is_some(),
                    u.unused(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("dxvk-2.0".into(), false, false),
                ("GE-Proton10-34".into(), true, false),
                ("GE-Proton9-1".into(), true, true),
            ]
        );
    }
}
