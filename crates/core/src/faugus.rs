//! Faugus Launcher's games and the Proton each runs with.
//!
//! Its games are games.json in its data directory, each naming its runner:
//! a directory in a Steam's compatibilitytools.d, its own "… Latest" ones
//! among them, or "Default", the default-runner in its config.json. The
//! Protons it downloads go in that compatibilitytools.d too, with Steam
//! installed or not.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

use crate::games::{Choice, Error, Game, Library};
use crate::heroic::read_json;
use crate::launcher::{Launcher, Packaging};

/// A game's runner when it runs with the default.
const DEFAULT: &str = "Default";

/// The default runner until one is set.
const FAUGUS_DEFAULT: &str = "Proton-CachyOS Latest";

/// Runners Faugus downloads and keeps up to date itself, offered before
/// they're there.
const LATEST: [&str; 4] = [
    "Proton-CachyOS Latest",
    "Proton-GE Latest",
    "DW-Proton Latest",
    "Proton-Wineland Latest",
];

/// Directories in compatibilitytools.d that aren't runners to choose.
const HIDDEN: [&str; 2] = ["UMU-Latest", "LegacyRuntime"];

/// Runners that aren't Proton: those games are left out.
const NOT_PROTON: [&str; 2] = ["Linux-Native", "Steam"];

/// Proton-CachyOS from the system's package, named apart.
const SYSTEM_CACHYOS: (&str, &str) = (
    "Proton-CachyOS (System)",
    "/usr/share/steam/compatibilitytools.d/proton-cachyos-slr",
);

/// Whether `dir` in compatibilitytools.d is one Faugus downloads and keeps
/// itself: no game naming it doesn't make it unused.
pub fn manages(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| LATEST.contains(&n) || HIDDEN.contains(&n))
}

/// The Steam data directory whose compatibilitytools.d Faugus uses: the
/// native Steam's, unless only a Flatpak Steam's is there.
pub fn steam_dir(home: &Path) -> PathBuf {
    let native = home.join(".local/share/Steam");
    let flatpak = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
    if !native.is_dir() && flatpak.is_dir() {
        flatpak
    } else {
        native
    }
}

pub fn library(launcher: &Launcher) -> Result<Library, Error> {
    let choices = choices(launcher);
    let config = read_json(&config_path(launcher))?;
    let default = config
        .as_ref()
        .and_then(|c| c["default-runner"].as_str())
        .unwrap_or(FAUGUS_DEFAULT);
    let default = Some(choice(&choices, default.trim_matches('"')));

    let mut problems = Vec::new();
    let path = launcher.root.join("games.json");
    let list = read_json(&path)?.unwrap_or_else(|| json!([]));
    let Some(list) = list.as_array() else {
        problems.push(format!("{}: not a list of games", path.display()));
        return Ok(Library {
            games: Vec::new(),
            default,
            choices,
            problems,
        });
    };
    let mut games = Vec::new();
    for g in list {
        let (Some(id), Some(name)) = (g["gameid"].as_str(), g["title"].as_str()) else {
            continue;
        };
        let runner = g["runner"].as_str().unwrap_or(DEFAULT);
        if NOT_PROTON.contains(&runner) {
            continue;
        }
        let explicit = !matches!(runner, DEFAULT | "");
        let cover = launcher.root.join(format!("covers/{id}.png"));
        games.push(Game {
            id: id.to_owned(),
            name: name.to_owned(),
            source: "faugus",
            tool: if explicit {
                Some(choice(&choices, runner))
            } else {
                default.clone()
            },
            explicit,
            epic_namespace: None,
            deck: None,
            art: cover
                .is_file()
                .then(|| cover.to_string_lossy().into_owned()),
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

/// `game`'s prefix, as its settings name it; none if they name none.
pub fn prefix(launcher: &Launcher, game: &str) -> Result<Option<PathBuf>, Error> {
    let list = read_json(&launcher.root.join("games.json"))?.unwrap_or_else(|| json!([]));
    let home = launcher.home().unwrap_or_default();
    Ok(list
        .as_array()
        .into_iter()
        .flatten()
        .find(|g| g["gameid"] == json!(game))
        .and_then(|g| g["prefix"].as_str())
        .filter(|p| !p.is_empty())
        .map(|p| match p.strip_prefix("~/") {
            Some(rest) => home.join(rest),
            None => PathBuf::from(p),
        }))
}

/// Sets `game`'s runner to the choice `tool`, or with `None` to the
/// default.
pub fn assign(launcher: &Launcher, game: &str, tool: Option<&str>) -> Result<(), Error> {
    let runner = match tool {
        Some(tool) => known(launcher, tool)?,
        None => DEFAULT,
    };
    let path = launcher.root.join("games.json");
    let mut list = read_json(&path)?.unwrap_or_else(|| json!([]));
    let entry = list
        .as_array_mut()
        .into_iter()
        .flatten()
        .find(|g| g["gameid"] == json!(game))
        .ok_or_else(|| Error::UnknownGame {
            id: game.to_owned(),
        })?;
    entry["runner"] = json!(runner);
    write(&path, &list)
}

/// Sets the runner games set to "Default" run with.
pub fn set_default(launcher: &Launcher, tool: &str) -> Result<(), Error> {
    let tool = known(launcher, tool)?;
    let path = config_path(launcher);
    // Faugus fills in what's missing from its defaults.
    let mut config = read_json(&path)?.unwrap_or_else(|| json!({}));
    let Some(settings) = config.as_object_mut() else {
        return Err(Error::Format {
            path,
            message: "not a settings object".into(),
        });
    };
    settings.insert("default-runner".into(), json!(tool));
    write(&path, &config)
}

/// `tool`, if it's one of the choices.
fn known<'a>(launcher: &Launcher, tool: &'a str) -> Result<&'a str, Error> {
    if choices(launcher).iter().any(|c| c.id == tool) {
        Ok(tool)
    } else {
        Err(Error::UnknownTool {
            id: tool.to_owned(),
        })
    }
}

/// As Faugus writes its files: indented by four, not escaping non-ASCII.
fn write(path: &Path, value: &Value) -> Result<(), Error> {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    value
        .serialize(&mut serializer)
        .expect("JSON values serialize");
    crate::games::write_with_backup(path, &out)
}

fn config_path(launcher: &Launcher) -> PathBuf {
    let home = launcher.home().expect("detected launchers have a home");
    home.join(match launcher.packaging {
        Packaging::Flatpak => ".var/app/io.github.Faugus.faugus-launcher/config",
        _ => ".config",
    })
    .join("faugus-launcher/config.json")
}

/// The runners a game can be set to: the directories in compatibilitytools.d
/// by name, Faugus's own Latest ones, and the system's Proton-CachyOS.
fn choices(launcher: &Launcher) -> Vec<Choice> {
    let home = launcher.home().expect("detected launchers have a home");
    let dir = steam_dir(&home).join("compatibilitytools.d");
    let mut choices: Vec<Choice> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let name = p.file_name()?.to_str()?.to_owned();
            (!HIDDEN.contains(&name.as_str())).then(|| Choice {
                id: name.clone(),
                name,
                dir: Some(p),
            })
        })
        .collect();
    choices.sort_by(|a, b| a.name.cmp(&b.name));
    for latest in LATEST {
        if !choices.iter().any(|c| c.id == latest) {
            choices.push(Choice {
                id: latest.into(),
                name: latest.into(),
                dir: None,
            });
        }
    }
    let (name, path) = SYSTEM_CACHYOS;
    if Path::new(path).is_dir() {
        choices.push(Choice {
            id: name.into(),
            name: name.into(),
            dir: Some(path.into()),
        });
    }
    choices
}

/// The choice a runner names; one that isn't there is still shown by name.
fn choice(choices: &[Choice], runner: &str) -> Choice {
    choices
        .iter()
        .find(|c| c.id == runner)
        .cloned()
        .unwrap_or_else(|| Choice {
            id: runner.to_owned(),
            name: runner.to_owned(),
            dir: None,
        })
}

#[cfg(test)]
mod tests {
    use protonhub_manifest::{LauncherKind, Registry};

    use super::*;
    use crate::games::tests::write;
    use crate::testutil::TempDir;

    /// Faugus with three games, its default set, and Steam's
    /// compatibilitytools.d beside it.
    fn faugus(home: &Path) -> Launcher {
        let root = home.join(".local/share/faugus-launcher");
        write(
            &root.join("games.json"),
            r#"[
    {"gameid": "witcher", "title": "The Witcher", "runner": "GE-Proton10-34", "prefix": "/p/w"},
    {"gameid": "celeste", "title": "Celeste", "runner": "Default"},
    {"gameid": "native", "title": "A Native Game", "runner": "Linux-Native"}
]"#,
        );
        write(&root.join("covers/celeste.png"), "");
        write(
            &home.join(".config/faugus-launcher/config.json"),
            r#"{"default-runner": "Proton-GE Latest", "mangohud": "False"}"#,
        );
        let tools = home.join(".local/share/Steam/compatibilitytools.d");
        for dir in ["GE-Proton10-34", "Proton-GE Latest", "UMU-Latest"] {
            fs::create_dir_all(tools.join(dir)).unwrap();
        }
        Launcher {
            kind: LauncherKind::Faugus,
            packaging: Packaging::Native,
            root,
        }
    }

    #[test]
    fn games_their_runners_and_the_default() {
        let home = TempDir::new("faugus-library");
        let l = faugus(&home);
        let lib = library(&l).unwrap();
        let games: Vec<_> = lib
            .games
            .iter()
            .map(|g| {
                (
                    g.id.as_str(),
                    g.tool.as_ref().unwrap().id.as_str(),
                    g.explicit,
                )
            })
            .collect();
        assert_eq!(
            games,
            [
                ("celeste", "Proton-GE Latest", false),
                ("witcher", "GE-Proton10-34", true)
            ],
            "the native game is left out"
        );
        assert!(
            lib.games[0]
                .art
                .as_ref()
                .unwrap()
                .ends_with("covers/celeste.png")
        );
        let witcher = lib.games[1].tool.as_ref().unwrap();
        assert_eq!(
            witcher.dir.as_deref(),
            Some(&*home.join(".local/share/Steam/compatibilitytools.d/GE-Proton10-34"))
        );
        let ids: Vec<_> = lib.choices.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "GE-Proton10-34",
                "Proton-GE Latest",
                "Proton-CachyOS Latest",
                "DW-Proton Latest",
                "Proton-Wineland Latest"
            ],
            "UMU-Latest isn't one"
        );
        assert_eq!(
            l.tool_dir(&registry().tools["ge"]),
            Some(home.join(".local/share/Steam/compatibilitytools.d")),
            "its tools go in Steam's directory"
        );
    }

    #[test]
    fn a_games_prefix_runs_with_its_runner() {
        let home = TempDir::new("faugus-prefix");
        let l = faugus(&home);
        let pfx = home.join("Faugus/witcher");
        fs::create_dir_all(pfx.join("drive_c")).unwrap();
        write(
            &l.root.join("games.json"),
            r#"[{"gameid": "witcher", "title": "The Witcher", "runner": "GE-Proton10-34", "prefix": "~/Faugus/witcher"},
                {"gameid": "celeste", "title": "Celeste", "runner": "Default", "prefix": ""}]"#,
        );
        let wine =
            home.join(".local/share/Steam/compatibilitytools.d/GE-Proton10-34/files/bin/wine");
        write(&wine, "");
        let lib = library(&l).unwrap();
        let game = |id: &str| lib.games.iter().find(|g| g.id == id).unwrap();
        let found = crate::prefix::find(&l, game("witcher")).unwrap();
        assert_eq!((found.dir, found.wine), (pfx, wine));
        assert!(matches!(
            crate::prefix::find(&l, game("celeste")),
            Err(Error::NoPrefix { .. })
        ));
    }

    #[test]
    fn sets_a_games_runner_and_the_default() {
        let home = TempDir::new("faugus-assign");
        let l = faugus(&home);
        assign(&l, "celeste", Some("GE-Proton10-34")).unwrap();
        assign(&l, "witcher", None).unwrap();
        set_default(&l, "GE-Proton10-34").unwrap();
        let lib = library(&l).unwrap();
        let tools: Vec<_> = lib
            .games
            .iter()
            .map(|g| (g.id.as_str(), g.explicit))
            .collect();
        assert_eq!(tools, [("celeste", true), ("witcher", false)]);
        assert_eq!(lib.default.unwrap().id, "GE-Proton10-34");

        let games = fs::read_to_string(l.root.join("games.json")).unwrap();
        assert!(
            games.contains("\n        \"prefix\": \"/p/w\""),
            "kept, indented by four: {games}"
        );
        assert!(games.contains("\"runner\": \"Default\""));
        let config = fs::read_to_string(home.join(".config/faugus-launcher/config.json")).unwrap();
        assert!(config.contains("\"mangohud\": \"False\""), "{config}");
        assert!(l.root.join("games.json.protonhub-backup").is_file());

        let unknown = assign(&l, "celeste", Some("nope")).unwrap_err();
        assert!(matches!(unknown, Error::UnknownTool { .. }), "{unknown}");
        let game = assign(&l, "nope", None).unwrap_err();
        assert!(matches!(game, Error::UnknownGame { .. }), "{game}");
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
            launchers = { steam = "compatibilitytools.d", faugus = "compatibilitytools.d" }
            "#,
        )
        .unwrap();
        r
    }

    #[test]
    fn steams_tools_count_faugus_games_and_default_as_users() {
        let home = TempDir::new("faugus-usage");
        let faugus = faugus(&home);
        let steam = Launcher {
            kind: LauncherKind::Steam,
            packaging: Packaging::Native,
            root: home.join(".local/share/Steam"),
        };
        set_default(&faugus, "GE-Proton10-34").unwrap();
        let all = crate::games::usage(&registry(), &[steam, faugus]).unwrap();
        let users = |launcher: usize, dir: &str| {
            let u = all[launcher]
                .1
                .iter()
                .find(|u| u.entry.dir.ends_with(dir))
                .unwrap();
            u.users.clone()
        };
        assert_eq!(
            users(0, "GE-Proton10-34").unwrap().games,
            [
                "default (faugus)",
                "Celeste (faugus)",
                "The Witcher (faugus)"
            ]
        );
        let own = users(1, "GE-Proton10-34").unwrap();
        assert_eq!(own.games, ["Celeste", "The Witcher"]);
        assert!(own.default);
        for dir in ["Proton-GE Latest", "UMU-Latest"] {
            assert!(
                users(0, dir).is_none(),
                "{dir} is Faugus's own, never unused"
            );
        }
    }
}
