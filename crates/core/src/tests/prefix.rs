use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::*;
use crate::testutil::TempDir;
use crate::testutil::{heroic, steam, write};

fn game(id: &str, tool_dir: Option<PathBuf>) -> Game {
    Game {
        id: id.into(),
        name: format!("Game {id}"),
        source: "steam",
        tool: tool_dir.map(|dir| crate::games::Choice {
            id: "x".into(),
            name: "x".into(),
            dir: Some(dir),
        }),
        explicit: false,
        epic_namespace: None,
        deck: None,
        art: None,
    }
}

#[test]
fn verbs_load_and_are_verbs() {
    let all: Vec<Verb> = verbs().into_iter().flat_map(|g| g.verb).collect();
    assert!(all.len() > 20);
    for v in &all {
        assert!(is_verb(&v.id), "{}", v.id);
        assert_eq!(
            all.iter().filter(|w| w.id == v.id).count(),
            1,
            "{} twice",
            v.id
        );
    }
    assert!(!is_verb("-q") && !is_verb("a b") && !is_verb("") && is_verb("sound=pulse"));
}

#[test]
fn steams_prefix_runs_with_the_proton_that_last_ran_it() {
    let home = TempDir::new("prefix-steam");
    let l = steam(&home);
    let last = home.join("protons/GE-Proton10-34");
    let set = home.join("protons/GE-Proton11-1");
    for p in [&last, &set] {
        write(&p.join("files/bin/wine"), "");
    }
    let data = l.root.join("steamapps/compatdata/10");
    write(&data.join("pfx/user.reg"), "");
    assert!(matches!(
        find(&l, &game("20", Some(set.clone()))),
        Err(Error::NoPrefix { .. })
    ));

    // No config_info: the game's tool.
    let p = find(&l, &game("10", Some(set.clone()))).unwrap();
    assert_eq!(p.dir, data.join("pfx"));
    assert_eq!(p.wine, set.join("files/bin/wine"));
    write(
        &data.join("config_info"),
        &format!("GE-Proton10-34\n{}/files/share/fonts/\n", last.display()),
    );
    let p = find(&l, &game("10", Some(set))).unwrap();
    assert_eq!(p.wine, last.join("files/bin/wine"), "the one that ran it");
    assert!(matches!(find(&l, &game("10", None)).map(|_| ()), Ok(())));
    fs::remove_dir_all(&last).unwrap();
    assert!(matches!(
        find(&l, &game("10", None)),
        Err(Error::NoWine { .. })
    ));
}

#[test]
fn heroics_prefix_is_in_its_settings() {
    let home = TempDir::new("prefix-heroic");
    let l = heroic(&home);
    let proton = l.root.join("tools/proton/GE-Proton10-34");
    write(&proton.join("files/bin/wine"), "");
    let pfx = home.join("Games/Heroic/Prefixes/epic1");
    fs::create_dir_all(pfx.join("pfx")).unwrap();
    write(
        &l.root.join("GamesConfig/epic1.json"),
        &serde_json::json!({ "epic1": {
            "winePrefix": pfx,
            "wineVersion": { "bin": proton.join("proton"), "name": "GE-Proton10-34", "type": "proton" },
        } })
        .to_string(),
    );
    let p = find(&l, &game("epic1", None)).unwrap();
    assert_eq!(p.dir, pfx.join("pfx"));
    assert_eq!(p.wine, proton.join("files/bin/wine"));
    assert!(matches!(
        find(&l, &game("epic3", None)),
        Err(Error::NotConfigured { .. })
    ));
}

#[test]
fn installed_verbs_and_the_environment() {
    let home = TempDir::new("prefix-env");
    let tool = home.join("GE-Proton10-34/files");
    write(&tool.join("bin/wine"), "");
    fs::create_dir_all(tool.join("lib/x86_64-linux-gnu")).unwrap();
    fs::create_dir_all(tool.join("lib/wine")).unwrap();
    let p = Prefix {
        dir: home.join("pfx"),
        wine: tool.join("bin/wine"),
    };
    assert!(p.installed().is_empty());
    write(
        &p.dir.join("winetricks.log"),
        "vcrun2022\n\nd3dx9\nvcrun2022\n",
    );
    assert_eq!(p.installed(), ["vcrun2022", "d3dx9"]);
    let env = p.env();
    let get = |k: &str| {
        env.iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get("WINESERVER"), Some(tool.join("bin/wineserver").into()));
    assert_eq!(
        get("LD_LIBRARY_PATH"),
        Some(std::env::join_paths([tool.join("lib/x86_64-linux-gnu"), tool.join("lib")]).unwrap())
    );
    assert_eq!(get("WINEDLLPATH"), Some(tool.join("lib/wine").into()));
    assert!(
        get("PATH")
            .unwrap()
            .to_string_lossy()
            .starts_with(&*tool.join("bin").to_string_lossy())
    );
    // The system's Wine gets no library paths.
    let system = Prefix {
        dir: p.dir.clone(),
        wine: "/usr/bin/wine".into(),
    };
    assert!(system.env().iter().all(|(k, _)| *k != "LD_LIBRARY_PATH"));
}

#[test]
fn runs_report_lines_and_fail_with_their_tail() {
    let mut cmd = Command::new("sh");
    cmd.args([
        "-c",
        "echo one; printf 'two\\r50%%\\r100%%\\n'; echo three >&2; exit 3",
    ]);
    let mut lines = Vec::new();
    let err = run(cmd, |_| {}, &mut |l| lines.push(l.to_owned())).unwrap_err();
    assert_eq!(lines, ["one", "100%", "three"]);
    match err {
        Error::Failed {
            program, output, ..
        } => {
            assert_eq!(program, "sh");
            assert_eq!(output, "one\n100%\nthree");
        }
        e => panic!("{e}"),
    }
}

#[test]
fn a_cancelled_run_ends_with_its_children() {
    let mut cmd = Command::new("sh");
    // A child that would hold the pipe open, as an installer would.
    cmd.args(["-c", "sleep 30 & sleep 30"]);
    let (tx, rx) = mpsc::channel();
    let started = Instant::now();
    let runner = std::thread::spawn(move || run(cmd, |pid| tx.send(pid).unwrap(), &mut |_| {}));
    let pid = rx.recv().unwrap();
    let prefix = Prefix {
        dir: "/nonexistent".into(),
        wine: "/nonexistent/bin/wine".into(),
    };
    cancel(&prefix, pid);
    assert!(runner.join().unwrap().is_err());
    assert!(started.elapsed() < Duration::from_secs(10));
}
