use super::user::local_config_of;
use super::*;
use crate::testutil::TempDir;

#[test]
fn adds_a_shortcut_once_for_the_user_signed_in_last() {
    let home = TempDir::new("steam-add");
    let root = home.join("Steam");
    let config = root.join("userdata/1/config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("localconfig.vdf"),
        "\"UserLocalConfigStore\"\n{\n}\n",
    )
    .unwrap();
    let add = Shortcut {
        name: "ProtonHub".into(),
        target: "\"/usr/bin/protonhub-gtk\"".into(),
        start_dir: "\"/usr/bin\"".into(),
        launch_options: String::new(),
    };
    assert!(add_shortcut(&root, &add, "").unwrap());
    assert!(!add_shortcut(&root, &add, "").unwrap(), "once");
    let all = shortcuts(&root).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].1, add);
    // The CRC-32 of target and name, with the top bit set.
    let mut crc = flate2::Crc::new();
    crc.update(b"\"/usr/bin/protonhub-gtk\"ProtonHub");
    assert_eq!(all[0].0, (crc.sum() | 0x8000_0000).to_string());
}

#[test]
fn a_chosen_user_gets_the_launch_options_where_they_have_settings() {
    let home = TempDir::new("steam-users");
    let root = home.join("Steam");
    for account in ["1", "2"] {
        let path = root.join(format!("userdata/{account}/config/localconfig.vdf"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "\"UserLocalConfigStore\"\n{\n}\n").unwrap();
    }
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(
        root.join("config/loginusers.vdf"),
        "\"users\"\n{\n\t\"76561197960265729\"\n\t{\n\t\t\"PersonaName\"\t\t\"Ann\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n",
    )
    .unwrap();
    let of = |chosen| local_config_of(&root, chosen).unwrap();
    assert!(
        of(None).ends_with("userdata/1/config/localconfig.vdf"),
        "signed in last"
    );
    assert!(of(Some("2")).ends_with("userdata/2/config/localconfig.vdf"));
    assert!(
        of(Some("9")).ends_with("userdata/1/config/localconfig.vdf"),
        "no settings: the last"
    );
    let users = users(&root).unwrap();
    assert_eq!(
        users
            .iter()
            .map(|u| (u.account.as_str(), u.name.as_deref()))
            .collect::<Vec<_>>(),
        [("1", Some("Ann")), ("2", None)]
    );
}

#[test]
fn a_shortcut_is_edited_or_its_file_left_alone() {
    let home = TempDir::new("steam-shortcut");
    let root = home.join("Steam");
    let file = |extra: &[u8]| {
        let mut b = vec![0x00];
        b.extend(b"shortcuts\0\x000\0\x02appid\0");
        b.extend((-5i32).to_le_bytes());
        b.extend(b"\x01AppName\0Game\0\x01Exe\0\"/g/game.exe\"\0");
        b.extend(extra);
        b.extend([0x08, 0x08, 0x08]);
        b
    };
    let path = root.join("userdata/1/config/shortcuts.vdf");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, file(&[])).unwrap();
    let (id, mut s) = shortcuts(&root).unwrap().remove(0);
    assert_eq!(
        (id.as_str(), s.target.as_str()),
        ("4294967291", "\"/g/game.exe\"")
    );
    s.name = "Renamed".into();
    s.start_dir = "\"/g\"".into();
    set_shortcut(&root, &id, &s).unwrap();
    assert_eq!(shortcuts(&root).unwrap()[0].1, s);

    // A float doesn't write back as it was: the file is left alone.
    let mut float = vec![0x03];
    float.extend(b"x\0");
    float.extend(1.5f32.to_le_bytes());
    fs::write(&path, file(&float)).unwrap();
    let err = set_shortcut(&root, &id, &s).unwrap_err();
    assert!(matches!(err, Error::Format { .. }), "{err}");
    assert_eq!(fs::read(&path).unwrap(), file(&float));
}

#[test]
fn art_from_the_library_cache_or_the_cdn() {
    let root = TempDir::new("steam-art");
    let cache = &root.join("appcache/librarycache");
    for file in ["10/header.jpg", "20/0ab1/header.jpg", "30_header.jpg"] {
        let path = cache.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"jpg").unwrap();
    }
    let local = |f: &str| cache.join(f).display().to_string();
    assert_eq!(art(&root, "10"), local("10/header.jpg"));
    assert_eq!(
        art(&root, "20"),
        local("20/0ab1/header.jpg"),
        "a directory deeper"
    );
    assert_eq!(art(&root, "30"), local("30_header.jpg"), "the older layout");
    assert_eq!(
        art(&root, "40"),
        "https://cdn.cloudflare.steamstatic.com/steam/apps/40/header.jpg"
    );
}

#[test]
fn official_names() {
    let cases = [
        ("Proton 9.0", Some("proton_9")),
        ("Proton 5.13", Some("proton_513")),
        ("Proton 10.0 (Beta)", Some("proton_10")),
        ("Proton - Experimental", Some("proton_experimental")),
        ("Proton Hotfix", Some("proton_hotfix")),
        ("Proton EasyAntiCheat Runtime", None),
        ("Portal 2", None),
    ];
    for (dir, want) in cases {
        assert_eq!(official_name(dir).as_deref(), want, "{dir}");
    }
}
