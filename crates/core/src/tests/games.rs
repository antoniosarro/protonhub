use std::path::Path;

use protonhub_manifest::Registry;

use super::usage::mentions;
use super::*;
use crate::testutil::{TempDir, heroic, steam, write};

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
fn steam_deck_ratings_come_from_the_app_cache() {
    let home = TempDir::new("steam-deck");
    let l = steam(&home);
    // Format 29: games 10 (verified, native) and 20 (playable, with
    // the default runtime), with their keys in a table at the end.
    let mut b = Vec::new();
    b.extend(0x0756_4429u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    let app = |b: &mut Vec<u8>, id: u32, category: i32, runtime: Option<&str>| {
        let mut kv = vec![0x00];
        kv.extend(0u32.to_le_bytes()); // appinfo
        kv.push(0x00);
        kv.extend(1u32.to_le_bytes()); // common
        kv.push(0x00);
        kv.extend(2u32.to_le_bytes()); // steam_deck_compatibility
        kv.push(0x02);
        kv.extend(3u32.to_le_bytes()); // category
        kv.extend(category.to_le_bytes());
        if let Some(runtime) = runtime {
            kv.push(0x00);
            kv.extend(4u32.to_le_bytes()); // configuration
            kv.push(0x01);
            kv.extend(5u32.to_le_bytes()); // recommended_runtime
            kv.extend(runtime.as_bytes());
            kv.extend([0x00, 0x08]);
        }
        kv.extend([0x08, 0x08, 0x08, 0x08]);
        b.extend(id.to_le_bytes());
        b.extend((60 + kv.len() as u32).to_le_bytes());
        b.extend([0; 60]);
        b.extend(kv);
    };
    app(&mut b, 10, 3, Some("native"));
    app(&mut b, 20, 2, None);
    b.extend(0u32.to_le_bytes());
    let table = b.len() as u64;
    b[8..16].copy_from_slice(&table.to_le_bytes());
    b.extend(6u32.to_le_bytes());
    b.extend(b"appinfo\0common\0steam_deck_compatibility\0category\0configuration\0recommended_runtime\0");
    fs::create_dir_all(l.root.join("appcache")).unwrap();
    fs::write(l.root.join("appcache/appinfo.vdf"), b).unwrap();

    let lib = library(&l).unwrap().unwrap();
    assert!(lib.problems.is_empty(), "{:?}", lib.problems);
    let deck = |id: &str| lib.games.iter().find(|g| g.id == id).unwrap().deck.clone();
    assert_eq!(
        deck("10"),
        Some(Deck {
            rating: DeckRating::Verified,
            runtime: Some("native".into())
        })
    );
    assert_eq!(
        deck("20"),
        Some(Deck {
            rating: DeckRating::Playable,
            runtime: None
        })
    );
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
fn steam_default_is_set_and_turned_off() {
    let home = TempDir::new("steam-default");
    let l = steam(&home);
    let config = l.root.join("config/config.vdf");
    let default = |l: &Launcher| library(l).unwrap().unwrap().default.map(|c| c.id);

    steam::assign(&l.root, &[(steam::DEFAULT, Some("GE-Proton10-1"))]).unwrap();
    assert_eq!(default(&l).as_deref(), Some("GE-Proton10-1"));
    let lib = library(&l).unwrap().unwrap();
    let game = lib.games.iter().find(|g| g.id == "20").unwrap();
    assert_eq!(
        (game.tool.as_ref().unwrap().id.as_str(), game.explicit),
        ("GE-Proton10-1", false),
        "a game without its own follows"
    );
    assert!(
        fs::read_to_string(&config)
            .unwrap()
            .contains("\t\t\t\t\t\t\"priority\"\t\t\"75\"\n")
    );

    steam::assign(&l.root, &[(steam::DEFAULT, None)]).unwrap();
    assert_eq!(default(&l), None);
    let unknown = steam::assign(&l.root, &[(steam::DEFAULT, Some("nope"))]).unwrap_err();
    assert!(matches!(unknown, Error::UnknownTool { .. }), "{unknown}");
    let heroic = set_default(&heroic(&home), Some("x")).unwrap_err();
    assert!(matches!(heroic, Error::Unsupported { .. }), "{heroic}");
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
