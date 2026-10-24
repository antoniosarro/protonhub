//! Release channels: a tool kept at its latest release in a launcher.
//!
//! An update installs the latest release if it isn't there, can move the
//! games set to the tool's older versions onto it, and deletes the
//! versions past the newest `keep` that no game in any launcher uses.
//! Channels are listed in `$XDG_CONFIG_HOME/protonhub/channels.toml`.

use std::cmp::Reverse;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use protonhub_manifest::{Arch, Registry, Tool};
use serde::{Deserialize, Serialize};

use crate::games;
use crate::install::{self, Entry, Step};
use crate::launcher::{self, Launcher};
use crate::releases::{self, Client, Version};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub tool: String,
    /// A launcher kind (`steam`), or kind and packaging (`steam:flatpak`).
    pub launcher: String,
    /// How many versions to keep, newest first; at least the newest.
    #[serde(default = "default_keep")]
    pub keep: usize,
    /// Whether games set to an older version move to the new one.
    #[serde(default)]
    pub move_games: bool,
}

fn default_keep() -> usize {
    3
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channels {
    #[serde(default, rename = "channel")]
    pub list: Vec<Channel>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{path}: {message}")]
    Config { path: PathBuf, message: String },
    #[error("no tool {tool} in the registry")]
    UnknownTool { tool: String },
    #[error("no launcher {launcher} that can take {tool} was found")]
    NoLauncher { tool: String, launcher: String },
    #[error("several launchers match {launcher}: {found}; name one as kind:packaging")]
    Ambiguous { launcher: String, found: String },
    #[error(transparent)]
    Source(#[from] releases::Error),
    #[error(transparent)]
    Install(#[from] install::Error),
    #[error(transparent)]
    Games(#[from] games::Error),
}

impl Channels {
    /// `$XDG_CONFIG_HOME/protonhub/channels.toml`.
    pub fn path() -> Option<PathBuf> {
        crate::paths::config_dir().map(|d| d.join("channels.toml"))
    }

    /// The channels in `path`; none if it doesn't exist.
    pub fn load(path: &Path) -> Result<Self, Error> {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(Error::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|e| Error::Config {
            path: path.to_owned(),
            message: e.to_string(),
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| Error::Io { path, source }
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io(dir))?;
        }
        let text = toml::to_string_pretty(self).expect("channels serialize");
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(io(&tmp))?;
        fs::rename(&tmp, path).map_err(io(path))
    }

    /// Adds `channel`, replacing one of the same tool and launcher.
    pub fn set(&mut self, channel: Channel) {
        match self
            .list
            .iter_mut()
            .find(|c| c.tool == channel.tool && c.launcher == channel.launcher)
        {
            Some(c) => *c = channel,
            None => self.list.push(channel),
        }
    }

    /// Removes the channel of `tool` in `launcher`; whether there was one.
    pub fn remove(&mut self, tool: &str, launcher: &str) -> bool {
        let before = self.list.len();
        self.list
            .retain(|c| !(c.tool == tool && c.launcher == launcher));
        self.list.len() != before
    }
}

/// What an update or a check works with.
pub struct Context<'a> {
    pub client: &'a Client,
    pub registry: &'a Registry,
    /// Every launcher found: usage is counted across all of them.
    pub launchers: &'a [Launcher],
    pub arch: Arch,
    pub downloads: &'a Path,
}

/// Whether a channel's latest release is installed.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub channel: Channel,
    pub launcher: Launcher,
    pub latest: String,
    pub installed: bool,
}

/// What an update did.
#[derive(Debug, Clone, Serialize)]
pub struct Update {
    pub channel: Channel,
    pub launcher: Launcher,
    pub latest: String,
    /// The new version's directory, when it was installed now.
    pub installed: Option<PathBuf>,
    /// The games moved onto it.
    pub moved: Vec<String>,
    /// Older versions deleted.
    pub removed: Vec<PathBuf>,
    /// What it left alone, and why.
    pub notes: Vec<String>,
}

impl Update {
    pub fn changed(&self) -> bool {
        self.installed.is_some() || !self.moved.is_empty() || !self.removed.is_empty()
    }
}

pub fn check(cx: &Context, channel: &Channel) -> Result<Status, Error> {
    let (tool, launcher) = resolve(cx, channel)?;
    let latest = cx.client.release(tool, &Version::Latest)?.tag;
    let dir = launcher
        .tool_dir(tool)
        .expect("the launcher takes the tool");
    let installed = ours(tool, install::list(&dir)?)
        .iter()
        .any(|e| version(e) == latest);
    Ok(Status {
        channel: channel.clone(),
        launcher,
        latest,
        installed,
    })
}

pub fn update(
    cx: &Context,
    channel: &Channel,
    progress: &mut dyn FnMut(Step),
) -> Result<Update, Error> {
    let (tool, launcher) = resolve(cx, channel)?;
    let dir = launcher
        .tool_dir(tool)
        .expect("the launcher takes the tool");
    let release = cx.client.release(tool, &Version::Latest)?;
    let latest = release.tag.clone();
    let mut update = Update {
        channel: channel.clone(),
        launcher: launcher.clone(),
        latest: latest.clone(),
        installed: None,
        moved: Vec::new(),
        removed: Vec::new(),
        notes: Vec::new(),
    };

    let newest = match ours(tool, install::list(&dir)?)
        .into_iter()
        .find(|e| version(e) == latest)
    {
        Some(e) => e.dir,
        None => {
            let e = install::install_release(
                cx.client,
                tool,
                release,
                &launcher,
                cx.arch,
                cx.downloads,
                progress,
            )?;
            update.installed = Some(e.dir.clone());
            e.dir
        }
    };

    if channel.move_games {
        move_games(tool, &launcher, &dir, &newest, &mut update)?;
    }
    prune(cx, tool, &launcher, &dir, channel.keep, &mut update)?;
    Ok(update)
}

/// Moves the games set to an older version of `tool` to `newest`.
fn move_games(
    tool: &Tool,
    launcher: &Launcher,
    dir: &Path,
    newest: &Path,
    update: &mut Update,
) -> Result<(), Error> {
    let Some(lib) = games::library(launcher)? else {
        return Ok(());
    };
    let older: Vec<PathBuf> = ours(tool, install::list(dir)?)
        .into_iter()
        .map(|e| e.dir)
        .filter(|d| d != newest)
        .collect();
    let moving: Vec<_> = lib
        .games
        .iter()
        .filter(|g| g.explicit)
        .filter(|g| {
            g.tool
                .as_ref()
                .and_then(|t| t.dir.as_ref())
                .is_some_and(|d| older.contains(d))
        })
        .collect();
    if moving.is_empty() {
        return Ok(());
    }
    let Some(to) = lib
        .choices
        .iter()
        .find(|c| c.dir.as_deref() == Some(newest))
    else {
        update.notes.push(format!(
            "{} isn't among {launcher}'s tools; {} games not moved",
            newest.display(),
            moving.len()
        ));
        return Ok(());
    };
    if games::is_running(launcher) {
        update.notes.push(format!(
            "{launcher} is running; {} games not moved",
            moving.len()
        ));
        return Ok(());
    }
    let moves: Vec<_> = moving
        .iter()
        .map(|g| (g.id.as_str(), Some(to.id.as_str())))
        .collect();
    games::assign_many(launcher, &moves)?;
    update.moved = moving.iter().map(|g| g.name.clone()).collect();
    Ok(())
}

/// Deletes the versions of `tool` past the newest `keep` that nothing uses.
fn prune(
    cx: &Context,
    tool: &Tool,
    launcher: &Launcher,
    dir: &Path,
    keep: usize,
    update: &mut Update,
) -> Result<(), Error> {
    let mut versions = ours(tool, install::list(dir)?);
    if versions.len() <= keep.max(1) {
        return Ok(());
    }
    // Newest first by the source's own order; versions it no longer lists
    // after those, by their tags compared as versions, then by when they
    // were installed.
    let order: Vec<String> = cx
        .client
        .releases(tool, 50)?
        .into_iter()
        .map(|r| r.tag)
        .collect();
    versions.sort_by_key(|e| {
        let r = e.record.as_ref().expect("ours have records");
        (
            order
                .iter()
                .position(|t| *t == r.version)
                .unwrap_or(usize::MAX),
            Reverse(natural(&r.version)),
            Reverse(r.installed_at),
        )
    });
    let usage = games::usage(cx.registry, cx.launchers)?;
    let usage: Vec<_> = usage
        .into_iter()
        .filter(|(l, _)| l == launcher)
        .flat_map(|(_, u)| u)
        .collect();
    for e in versions.iter().skip(keep.max(1)) {
        let name = version(e);
        match usage.iter().find(|u| u.entry.dir == e.dir) {
            Some(u) if u.unused() => {
                install::delete(&e.dir)?;
                update.removed.push(e.dir.clone());
            }
            Some(u) if u.users.is_none() => update.notes.push(format!(
                "kept {name}: ProtonHub can't tell what uses tools in {launcher}"
            )),
            _ => update
                .notes
                .push(format!("kept {name}: games still use it")),
        }
    }
    Ok(())
}

/// The channel's tool and the one launcher it names that takes it.
fn resolve<'a>(cx: &Context<'a>, channel: &Channel) -> Result<(&'a Tool, Launcher), Error> {
    let tool = cx
        .registry
        .tools
        .get(&channel.tool)
        .ok_or_else(|| Error::UnknownTool {
            tool: channel.tool.clone(),
        })?;
    match launcher::pick(cx.launchers, &channel.launcher, tool) {
        Ok(l) => Ok((tool, l.clone())),
        Err(found) if found.is_empty() => Err(Error::NoLauncher {
            tool: tool.id.clone(),
            launcher: channel.launcher.clone(),
        }),
        Err(found) => Err(Error::Ambiguous {
            launcher: channel.launcher.clone(),
            found: launcher::list(&found),
        }),
    }
}

/// The versions of `tool` among `entries`, which ProtonHub installed.
fn ours(tool: &Tool, entries: Vec<Entry>) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|e| e.record.as_ref().is_some_and(|r| r.tool == tool.id))
        .collect()
}

/// A tag as runs of digits (compared as numbers) and of other characters,
/// so GE-Proton11-7 sorts after GE-Proton10-34.
fn natural(tag: &str) -> Vec<(u64, String)> {
    let mut parts = Vec::new();
    let mut chars = tag.chars().peekable();
    while let Some(&c) = chars.peek() {
        let digit = c.is_ascii_digit();
        let mut run = String::new();
        while let Some(c) = chars.next_if(|c| c.is_ascii_digit() == digit) {
            run.push(c);
        }
        parts.push(if digit {
            (run.parse().unwrap_or(u64::MAX), String::new())
        } else {
            (0, run)
        });
    }
    parts
}

fn version(e: &Entry) -> &str {
    e.record.as_ref().map_or("", |r| r.version.as_str())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use protonhub_manifest::{LauncherKind, Registry};

    use super::*;
    use crate::launcher::Packaging;
    use crate::testutil::{Published, TempDir, github, gz, tar};

    /// A version of a Steam compatibility tool named Tool-<version>.
    fn tool_archive(version: &str) -> (String, Vec<u8>) {
        let vdf = format!(
            "\"compatibilitytools\" {{ \"compat_tools\" {{ \"Tool-{version}\" {{ \"install_path\" \".\" \"display_name\" \"Tool {version}\" }} }} }}"
        );
        let path = format!("Tool-{version}/compatibilitytool.vdf");
        (format!("tool-{version}.tar.gz"), gz(&tar(&[(&path, &vdf)])))
    }

    fn publish(published: &Published, version: &str) {
        let (name, bytes) = tool_archive(version);
        *published.lock().unwrap() = (version.into(), vec![(name, bytes)]);
    }

    struct Setup {
        home: TempDir,
        server: crate::testutil::Server,
        published: Published,
        registry: Registry,
        steam: Launcher,
    }

    /// A Steam with one game, and a tool whose source has release v1.
    fn setup(name: &str) -> Setup {
        let home = TempDir::new(name);
        let root = home.join("Steam");
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(
            root.join("steamapps/appmanifest_10.acf"),
            "\"AppState\"\n{\n\t\"appid\"\t\t\"10\"\n\t\"name\"\t\t\"Game\"\n}\n",
        )
        .unwrap();
        let (asset, bytes) = tool_archive("v1");
        let (server, published) = github(("v1", vec![(&asset, bytes)]));
        let mut registry = Registry::default();
        registry
            .add(
                Path::new("t.toml"),
                "[tool]\nname = \"Tool\"\nsource = { github = \"o/r\" }\nasset = { any = '^tool-' }\nlaunchers = { steam = \"compatibilitytools.d\" }",
            )
            .unwrap();
        Setup {
            server,
            published,
            registry,
            steam: Launcher {
                kind: LauncherKind::Steam,
                packaging: Packaging::Native,
                root,
            },
            home,
        }
    }

    impl Setup {
        fn with<T>(&self, f: impl FnOnce(&Context) -> T) -> T {
            let client = Client::new(&self.server.url, self.home.join("cache"), None);
            let launchers = [self.steam.clone()];
            f(&Context {
                client: &client,
                registry: &self.registry,
                launchers: &launchers,
                arch: Arch::X86_64,
                downloads: &self.home.join("downloads"),
            })
        }

        fn update(&self, channel: &Channel) -> Update {
            self.with(|cx| update(cx, channel, &mut |_| {}).unwrap())
        }

        fn check(&self, channel: &Channel) -> Status {
            self.with(|cx| check(cx, channel).unwrap())
        }

        fn game_tool(&self) -> String {
            let lib = games::library(&self.steam).unwrap().unwrap();
            lib.games[0].tool.as_ref().unwrap().id.clone()
        }
    }

    fn channel(keep: usize, move_games: bool) -> Channel {
        Channel {
            tool: "tool".into(),
            launcher: "steam".into(),
            keep,
            move_games,
        }
    }

    #[test]
    fn an_update_installs_moves_games_and_deletes_the_unused() {
        let s = setup("chan-move");
        let ch = channel(1, true);
        let first = s.update(&ch);
        assert!(first.installed.unwrap().ends_with("Tool-v1"));
        crate::steam::assign(&s.steam.root, &[("10", Some("Tool-v1"))]).unwrap();

        publish(&s.published, "v2");
        let second = s.update(&ch);
        assert!(second.installed.as_ref().unwrap().ends_with("Tool-v2"));
        assert_eq!(second.moved, ["Game"]);
        assert_eq!(s.game_tool(), "Tool-v2");
        assert_eq!(second.removed.len(), 1);
        assert!(second.removed[0].ends_with("Tool-v1") && !second.removed[0].exists());

        let again = s.update(&ch);
        assert!(!again.changed(), "{again:?}");
    }

    #[test]
    fn an_update_keeps_what_games_still_use() {
        let s = setup("chan-keep");
        let ch = channel(1, false);
        s.update(&ch);
        crate::steam::assign(&s.steam.root, &[("10", Some("Tool-v1"))]).unwrap();
        publish(&s.published, "v2");
        let u = s.update(&ch);
        assert!(u.installed.is_some());
        assert!(u.moved.is_empty() && u.removed.is_empty());
        assert_eq!(u.notes, ["kept v1: games still use it"]);
        assert_eq!(s.game_tool(), "Tool-v1");
    }

    #[test]
    fn keep_counts_newest_first() {
        let s = setup("chan-order");
        let ch = channel(2, false);
        s.update(&ch);
        for v in ["v2", "v3"] {
            publish(&s.published, v);
            s.update(&ch);
        }
        let left: Vec<_> = install::list(&s.steam.root.join("compatibilitytools.d"))
            .unwrap()
            .into_iter()
            .map(|e| e.record.unwrap().version)
            .collect();
        assert_eq!(left, ["v2", "v3"]);
    }

    #[test]
    fn check_says_whether_the_latest_is_installed() {
        let s = setup("chan-check");
        let ch = channel(3, false);
        assert!(!s.check(&ch).installed);
        s.update(&ch);
        assert!(s.check(&ch).installed);
        publish(&s.published, "v2");
        let st = s.check(&ch);
        assert_eq!((st.latest.as_str(), st.installed), ("v2", false));
    }

    #[test]
    fn tags_compare_as_versions() {
        let mut tags = [
            "GE-Proton9-1",
            "GE-Proton11-7",
            "GE-Proton10-34",
            "GE-Proton11-10",
        ];
        tags.sort_by_key(|t| Reverse(natural(t)));
        assert_eq!(
            tags,
            [
                "GE-Proton11-10",
                "GE-Proton11-7",
                "GE-Proton10-34",
                "GE-Proton9-1"
            ]
        );
    }

    #[test]
    fn channels_file_round_trips() {
        let home = TempDir::new("chan-file");
        let path = home.join("protonhub/channels.toml");
        assert!(Channels::load(&path).unwrap().list.is_empty());
        let mut c = Channels::default();
        c.set(channel(3, false));
        c.set(channel(2, true));
        assert_eq!(c.list.len(), 1, "same tool and launcher: replaced");
        c.save(&path).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[[channel]]"), "{text}");
        let back = Channels::load(&path).unwrap();
        assert_eq!(back.list, [channel(2, true)]);
        let mut back = back;
        assert!(back.remove("tool", "steam") && back.list.is_empty());
        fs::write(&path, "[[channel]]\ntool = 1").unwrap();
        assert!(matches!(Channels::load(&path), Err(Error::Config { .. })));
    }
}
