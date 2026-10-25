use std::fs;
use std::path::Path;

use super::*;
use crate::testutil::TempDir;
use crate::testutil::{heroic, steam, write};

fn preset(id: &str) -> Preset {
    presets().into_iter().find(|p| p.id == id).unwrap()
}

#[test]
fn presets_load_and_each_does_something() {
    let all = presets();
    assert!(all.len() >= 2);
    for p in &all {
        assert!(
            !p.env.is_empty() || p.wrapper.is_some(),
            "{} sets nothing",
            p.id
        );
        assert_eq!(
            all.iter().filter(|q| q.id == p.id).count(),
            1,
            "{} twice",
            p.id
        );
    }
}

#[test]
fn splits_and_keeps_unknown_words_in_place() {
    let o = Options::parse("PROTON_LOG=1 gamemoderun protonhax init %command% -dx11").unwrap();
    assert_eq!(o.env, [("PROTON_LOG".into(), "1".into())]);
    assert_eq!(o.wrappers, ["gamemoderun", "protonhax", "init"]);
    assert_eq!(o.args, ["-dx11"]);
    assert_eq!(
        o.to_string(),
        "PROTON_LOG=1 gamemoderun protonhax init %command% -dx11"
    );
    // An assignment after a program is the program's.
    let o = Options::parse("env FOO=1 %command%").unwrap();
    assert!(o.env.is_empty());
    assert_eq!(o.wrappers, ["env", "FOO=1"]);
}

#[test]
fn without_command_everything_is_an_argument() {
    let mut o = Options::parse("-novid -high").unwrap();
    assert_eq!(o.args, ["-novid", "-high"]);
    assert_eq!(o.to_string(), "-novid -high");
    o.set(&preset("log"), true);
    assert_eq!(o.to_string(), "PROTON_LOG=1 %command% -novid -high");
}

#[test]
fn switches_presets_on_and_off() {
    let mut o = Options::parse("gamemoderun protonhax init %command%").unwrap();
    let (mangohud, gamemode, wayland) = (preset("mangohud"), preset("gamemode"), preset("wayland"));
    assert!(o.has(&gamemode) && !o.has(&mangohud) && !o.has(&wayland));
    o.set(&mangohud, true);
    o.set(&wayland, true);
    assert_eq!(
        o.to_string(),
        "PROTON_ENABLE_WAYLAND=1 mangohud gamemoderun protonhax init %command%"
    );
    o.set(&mangohud, true);
    assert_eq!(o.wrappers.len(), 4, "on twice is on once");
    o.set(&gamemode, false);
    o.set(&wayland, false);
    o.set(&mangohud, false);
    assert_eq!(o.to_string(), "protonhax init %command%");
    o.set(&gamemode, false);
    assert_eq!(
        o.to_string(),
        "protonhax init %command%",
        "off twice is off"
    );
}

#[test]
fn a_preset_with_another_value_is_off_and_takes_its_value() {
    let mut o = Options::parse("PROTON_LOG=0 %command%").unwrap();
    assert!(!o.has(&preset("log")));
    o.set(&preset("log"), true);
    assert_eq!(o.to_string(), "PROTON_LOG=1 %command%");
}

#[test]
fn quotes_values_alone_and_rejects_unclosed_quotes() {
    let o =
        Options::parse("DXVK_HUD='fps,frametimes' WINEDLLOVERRIDES=\"a b\" %command% --name 'x y'")
            .unwrap();
    assert_eq!(o.env[1], ("WINEDLLOVERRIDES".into(), "a b".into()));
    let text = o.to_string();
    assert!(
        text.starts_with("DXVK_HUD='fps,frametimes' WINEDLLOVERRIDES='a b' %command%"),
        "{text}"
    );
    assert_eq!(Options::parse(&text), Some(o), "it reads back");
    assert_eq!(Options::parse("FOO='open %command%"), None);
    assert_eq!(Options::parse("").unwrap().to_string(), "");
}

#[test]
fn runs_the_game_in_gamescope_and_edits_its_options() {
    let mut o = Options::parse("mangohud %command% -novid").unwrap();
    assert_eq!(o.gamescope(), None);
    o.set_gamescope(Some(vec!["-f".into()]));
    assert_eq!(o.to_string(), "mangohud gamescope -f -- %command% -novid");

    let mut o =
        Options::parse("gamescope --output-width=1280 -r 60 -- protonhax init %command%").unwrap();
    let mut args = o.gamescope().unwrap().to_vec();
    assert_eq!(
        option(&args, &["-W", "--output-width"]).as_deref(),
        Some("1280")
    );
    assert_eq!(
        option(&args, &["-r", "--nested-refresh"]).as_deref(),
        Some("60")
    );
    assert_eq!(option(&args, &["-H", "--output-height"]), None);
    set_option(&mut args, &["-W", "--output-width"], Some("2560"));
    set_option(&mut args, &["-r", "--nested-refresh"], None);
    set_flag(&mut args, &["-f", "--fullscreen"], true);
    set_flag(&mut args, &["-f", "--fullscreen"], true);
    assert_eq!(args, ["-W", "2560", "-f"]);
    assert!(flag(&args, &["-f", "--fullscreen"]));
    o.set_gamescope(Some(args));
    assert_eq!(
        o.to_string(),
        "gamescope -W 2560 -f -- protonhax init %command%",
        "the programs after it are kept"
    );
    o.set_gamescope(None);
    assert_eq!(o.to_string(), "protonhax init %command%");

    // Without its --, the rest are gamescope's.
    let mut o = Options::parse("gamescope -f %command%").unwrap();
    assert_eq!(o.gamescope().unwrap(), ["-f"]);
    o.set_gamescope(None);
    assert_eq!(o.to_string(), "%command%");
}

fn local_config(root: &Path, account: &str, apps: &str) -> std::path::PathBuf {
    let path = root.join(format!("userdata/{account}/config/localconfig.vdf"));
    write(
        &path,
        &format!(
            "\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n{apps}\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n\t\"friends\"\t\t\"kept\"\n}}\n"
        ),
    );
    path
}

#[test]
fn reads_and_writes_steams_launch_options() {
    let home = TempDir::new("launch-steam");
    let l = steam(&home);
    let path = local_config(
        &l.root,
        "1",
        "\t\t\t\t\t\"10\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"5\"\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"gamemoderun %command%\"\n\t\t\t\t\t}\n\t\t\t\t\t\"30\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"\"\n\t\t\t\t\t}\n",
    );
    assert_eq!(
        all(&l).unwrap().unwrap(),
        [("10".to_owned(), "gamemoderun %command%".to_owned())],
        "empty ones left out"
    );

    set(&l, "20", "PROTON_LOG=1 %command%").unwrap();
    set(&l, "10", "").unwrap();
    let options = all(&l).unwrap().unwrap();
    assert_eq!(
        options,
        [("20".to_owned(), "PROTON_LOG=1 %command%".to_owned())]
    );
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("\"LastPlayed\"\t\t\"5\"") && after.contains("\"friends\"\t\t\"kept\""));
    let backup = path.with_file_name("localconfig.vdf.protonhub-backup");
    // The file before the last change: 20's set, 10's not yet removed.
    let backup = fs::read_to_string(backup).unwrap();
    assert!(backup.contains("gamemoderun %command%") && backup.contains("PROTON_LOG=1"));

    // A shortcut's are in its entry in shortcuts.vdf.
    set(&l, "4294967291", "-novid").unwrap();
    let options = all(&l).unwrap().unwrap();
    assert!(options.contains(&("4294967291".to_owned(), "-novid".to_owned())));
    let shortcuts = crate::steam::shortcuts(&l.root).unwrap();
    assert_eq!(shortcuts[0].1.name, "My Shortcut", "the rest kept");
    assert!(matches!(set(&l, "99", "x"), Err(Error::UnknownGame { .. })));
}

#[test]
fn sets_many_games_in_one_write() {
    let home = TempDir::new("launch-many");
    let l = steam(&home);
    let path = local_config(&l.root, "1", "");
    set_many(
        &l,
        &[("10", "-a %command%"), ("20", "MANGOHUD=1 %command%")],
    )
    .unwrap();
    let options = all(&l).unwrap().unwrap();
    assert!(options.contains(&("10".into(), "-a %command%".into())));
    assert!(options.contains(&("20".into(), "MANGOHUD=1 %command%".into())));
    let backup =
        fs::read_to_string(path.with_file_name("localconfig.vdf.protonhub-backup")).unwrap();
    assert!(
        !backup.contains("LaunchOptions"),
        "the backup is from before them all"
    );
    assert!(matches!(
        set_many(&l, &[("10", "x"), ("99", "y")]),
        Err(Error::UnknownGame { .. })
    ));
    assert!(
        all(&l)
            .unwrap()
            .unwrap()
            .contains(&("10".into(), "-a %command%".into())),
        "nothing written"
    );
}

#[test]
fn steams_user_is_the_one_signed_in_last_or_the_only_one() {
    let home = TempDir::new("launch-users");
    let l = steam(&home);
    local_config(
        &l.root,
        "1",
        "\t\t\t\t\t\"10\" { \"LaunchOptions\" \"one\" }\n",
    );
    local_config(
        &l.root,
        "2",
        "\t\t\t\t\t\"10\" { \"LaunchOptions\" \"two\" }\n",
    );
    assert!(
        matches!(all(&l), Err(Error::Format { .. })),
        "two users, neither last"
    );
    write(
        &l.root.join("config/loginusers.vdf"),
        "\"users\"\n{\n\t\"76561197960265729\"\n\t{\n\t\t\"MostRecent\"\t\t\"0\"\n\t}\n\t\"76561197960265730\"\n\t{\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n",
    );
    assert_eq!(all(&l).unwrap().unwrap()[0].1, "two");
    fs::remove_file(l.root.join("config/loginusers.vdf")).unwrap();
    fs::remove_dir_all(l.root.join("userdata/2")).unwrap();
    assert_eq!(all(&l).unwrap().unwrap()[0].1, "one", "the only one");
}

#[test]
fn reads_and_writes_heroics_launch_options() {
    let home = TempDir::new("launch-heroic");
    let l = heroic(&home);
    let path = l.root.join("GamesConfig/side1.json");
    write(
        &path,
        &serde_json::json!({ "side1": {
            "language": "kept",
            "showMangohud": false,
            "useGameMode": true,
            "enviromentOptions": [{ "key": "DXVK_HUD", "value": "fps,gpu" }],
            "wrapperOptions": [{ "exe": "gamescope", "args": "-W 1920 --" }, { "exe": "protonhax", "args": "init" }],
            "launcherArgs": "-novid 'a b'",
        } })
        .to_string(),
    );
    let text =
        "DXVK_HUD='fps,gpu' gamemoderun gamescope -W 1920 -- protonhax init %command% -novid 'a b'";
    assert_eq!(
        all(&l).unwrap().unwrap(),
        [("side1".to_owned(), text.to_owned())],
        "epic1 has none"
    );

    // MangoHud on, GameMode off: Heroic's own switches. A new word
    // after a program is that program's.
    let mut o = Options::parse(text).unwrap();
    o.set(&preset("mangohud"), true);
    o.set(&preset("gamemode"), false);
    o.set(&preset("wayland"), true);
    o.wrappers.push("--fullscreen".into());
    set(&l, "side1", &o.to_string()).unwrap();
    let after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let s = &after["side1"];
    assert_eq!(s["language"], "kept");
    assert_eq!(
        (&s["showMangohud"], &s["useGameMode"]),
        (&true.into(), &false.into())
    );
    assert_eq!(
        s["enviromentOptions"],
        serde_json::json!([
            { "key": "DXVK_HUD", "value": "fps,gpu" },
            { "key": "PROTON_ENABLE_WAYLAND", "value": "1" },
        ])
    );
    assert_eq!(
        s["wrapperOptions"],
        serde_json::json!([
            { "exe": "gamescope", "args": "-W 1920 --" },
            { "exe": "protonhax", "args": "init --fullscreen" },
        ])
    );
    assert_eq!(s["launcherArgs"], "-novid 'a b'");
    assert!(path.with_file_name("side1.json.protonhub-backup").is_file());

    set(&l, "side1", "").unwrap();
    assert!(all(&l).unwrap().unwrap().is_empty(), "all removed");
    assert!(matches!(
        set(&l, "epic3", "x"),
        Err(Error::NotConfigured { .. })
    ));
    assert!(matches!(set(&l, "side1", "'x"), Err(Error::Unclosed)));
}
