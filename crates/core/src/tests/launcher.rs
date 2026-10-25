use std::fs;

use super::*;
use crate::testutil::TempDir;

#[test]
fn a_launchers_home() {
    let l = |kind, packaging, root: &str| Launcher {
        kind,
        packaging,
        root: root.into(),
    };
    assert_eq!(
        l(
            LauncherKind::Heroic,
            Packaging::Flatpak,
            "/h/.var/app/com.heroicgameslauncher.hgl/config/heroic"
        )
        .home(),
        Some("/h".into())
    );
    assert_eq!(
        l(LauncherKind::Heroic, Packaging::Native, "/elsewhere").home(),
        None
    );
}

#[test]
fn detects_launchers_in_a_fixture_home() {
    let home = TempDir::new("detect");
    for rel in [
        ".local/share/Steam",
        ".var/app/com.heroicgameslauncher.hgl/config/heroic",
        ".local/share/lutris",
        ".var/app/io.github.fastrizwaan.WineZGUI/data/winezgui",
    ] {
        fs::create_dir_all(home.join(rel)).unwrap();
    }
    // Debian's path, linked to the same Steam: reported once.
    fs::create_dir_all(home.join(".steam")).unwrap();
    std::os::unix::fs::symlink(
        home.join(".local/share/Steam"),
        home.join(".steam/debian-installation"),
    )
    .unwrap();

    let found: Vec<_> = detect(&home)
        .into_iter()
        .map(|l| (l.kind, l.packaging))
        .collect();
    assert_eq!(
        found,
        [
            (LauncherKind::Steam, Packaging::Native),
            (LauncherKind::Heroic, Packaging::Flatpak),
            (LauncherKind::Lutris, Packaging::Native),
            (LauncherKind::Winezgui, Packaging::Flatpak),
        ]
    );
}
