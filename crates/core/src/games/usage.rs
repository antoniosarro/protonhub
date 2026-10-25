//! Which installed tools games use, across every launcher, and so which
//! nothing does.

use std::fs;
use std::path::{Path, PathBuf};

use protonhub_manifest::{LauncherKind, Registry};
use serde::Serialize;

use super::{Choice, Error, Library, library};
use crate::install::{self, Entry};
use crate::launcher::{Launcher, Packaging};
use crate::{faugus, heroic, steam};

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
    let faugus = launchers.iter().any(|l| l.kind == LauncherKind::Faugus);
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
            LauncherKind::Faugus => launcher
                .home()
                .map(|h| faugus::steam_dir(&h).join("compatibilitytools.d"))
                .into_iter()
                .collect(),
            _ => Vec::new(),
        };
        let mut usage = Vec::new();
        for td in install::inventory(registry, launcher)? {
            let judged = own.is_some() && runner_dirs.contains(&td.dir);
            for entry in td.entries {
                let judged = judged && !(faugus && faugus::manages(&entry.dir));
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
        // Another launcher's default is one of its users too: Heroic's or
        // Faugus's can be a tool in Steam's directory.
        if runs(&lib.default) {
            if launcher == &owner {
                users.default = true;
            } else {
                users
                    .games
                    .push(format!("default ({})", launcher.kind.name()));
            }
        }
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
pub(super) fn mentions(text: &str, path: &str) -> bool {
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
