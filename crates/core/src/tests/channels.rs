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
