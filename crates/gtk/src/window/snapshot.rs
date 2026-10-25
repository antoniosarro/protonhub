//! What the window works with, and what its views show: the launchers
//! found, read on a worker thread, since finding sizes walks gigabytes of
//! files.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use protonhub_core::games;
use protonhub_core::info;
use protonhub_core::launch;
use protonhub_core::launcher::{self, Launcher};
use protonhub_core::manifest::{Arch, Registry};
use protonhub_core::manifests;
use protonhub_core::paths;
use protonhub_core::releases::{self, Client};
use protonhub_core::storage;

/// What the window works with; loading it can fail, and then the window
/// shows why instead of tabs.
pub struct Env {
    pub registry: Arc<Registry>,
    pub client: Arc<Client>,
    pub home: PathBuf,
    pub downloads: PathBuf,
    pub arch: Arch,
}

impl Env {
    pub(super) fn load() -> Result<Self, String> {
        let registry = manifests::load_default(None).map_err(|e| e.to_string())?;
        let home = paths::home().ok_or("HOME is not set")?;
        let cache = paths::cache_dir().ok_or("neither XDG_CACHE_HOME nor HOME is set")?;
        let arch = Arch::host().ok_or("this CPU architecture has no builds")?;
        let token = paths::github_token();
        Ok(Self {
            registry: Arc::new(registry),
            client: Arc::new(Client::new(releases::API, cache.clone(), token)),
            home,
            downloads: cache.join("downloads"),
            arch,
        })
    }
}

/// What the tabs show, read on a worker thread: finding sizes walks
/// gigabytes of files.
pub struct Snapshot {
    pub launchers: Vec<LauncherState>,
}

pub struct LauncherState {
    pub launcher: Launcher,
    pub usage: Result<Vec<games::Usage>, String>,
    /// None for a launcher without per-game tools.
    pub library: Result<Option<games::Library>, String>,
    pub running: bool,
    /// ProtonDB's rating and the anti-cheat status of each game, by id.
    pub info: Vec<(String, info::Info)>,
    /// Why some of that couldn't be read; the rest is still there.
    pub info_error: Option<String>,
    /// Valve's Protons, in a Steam, with what uses them.
    pub officials: Result<Vec<games::OfficialUsage>, String>,
    /// A Steam's prefixes and shader caches; none for another launcher.
    pub storage: Result<Option<storage::Storage>, String>,
    /// Each game's launch options, by id; none where they can't be read.
    pub launch: Result<Option<Vec<(String, String)>>, String>,
}

impl Snapshot {
    pub(super) fn read(registry: &Registry, home: &Path, client: &Client) -> Self {
        let found = launcher::detect(home);
        // One list for every launcher; it's cached for a day.
        let anticheat = info::AntiCheatList::fetch(client, info::AWACY).map_err(|e| e.to_string());
        let officials = games::officials(&found).map_err(|e| e.to_string());
        let mut usage = match games::usage(registry, &found) {
            Ok(u) => u.into_iter().map(|(_, u)| Ok(u)).collect(),
            Err(e) => vec![Err(e.to_string()); found.len()],
        };
        let launchers = found
            .into_iter()
            .map(|l| {
                let library = games::library(&l).map_err(|e| e.to_string());
                let mut info_error = None;
                let mut fail = |e: String| {
                    info_error.get_or_insert(e);
                };
                let ac = anticheat.as_ref().map_err(|e| fail(e.clone())).ok();
                let info = match &library {
                    Ok(Some(lib)) => lib
                        .games
                        .iter()
                        .map(|g| {
                            let protondb = info::protondb(client, info::PROTONDB, g)
                                .map_err(|e| fail(e.to_string()))
                                .ok()
                                .flatten();
                            let anticheat = ac.and_then(|ac| ac.find(g));
                            (
                                g.id.clone(),
                                info::Info {
                                    protondb,
                                    anticheat,
                                },
                            )
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                LauncherState {
                    usage: usage.remove(0),
                    library,
                    running: games::is_running(&l),
                    storage: storage::scan(&l).map_err(|e| e.to_string()),
                    launch: launch::all(&l).map_err(|e| e.to_string()),
                    info,
                    info_error,
                    officials: match &officials {
                        Ok(all) => Ok(all
                            .iter()
                            .find(|(o, _)| *o == l)
                            .map(|(_, found)| found.clone())
                            .unwrap_or_default()),
                        Err(e) => Err(e.clone()),
                    },
                    launcher: l,
                }
            })
            .collect();
        Self { launchers }
    }

    /// What was read of `launcher`, if it's still there.
    pub fn state(&self, launcher: &Launcher) -> Option<&LauncherState> {
        self.launchers.iter().find(|s| s.launcher == *launcher)
    }
}

impl LauncherState {
    /// The games, where the launcher has per-game tools and they were read.
    pub fn library(&self) -> Option<&games::Library> {
        self.library.as_ref().ok().and_then(Option::as_ref)
    }

    /// The game `id`'s ratings, where any are known.
    pub fn info(&self, id: &str) -> Option<&info::Info> {
        self.info.iter().find(|(i, _)| i == id).map(|(_, i)| i)
    }

    /// The launch options of the game `id`, as text.
    pub fn launch_options(&self, id: &str) -> &str {
        match &self.launch {
            Ok(Some(all)) => all.iter().find(|(g, _)| g == id).map_or("", |(_, o)| o),
            _ => "",
        }
    }
}
