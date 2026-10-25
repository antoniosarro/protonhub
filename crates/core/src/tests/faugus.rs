use protonhub_manifest::{LauncherKind, Registry};

use super::*;
use crate::testutil::TempDir;
use crate::testutil::write;

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
    let wine = home.join(".local/share/Steam/compatibilitytools.d/GE-Proton10-34/files/bin/wine");
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
