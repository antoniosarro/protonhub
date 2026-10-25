use std::fs;
use std::path::Path;

use protonhub_manifest::LauncherKind;

use super::*;
use crate::launcher::Packaging;
use crate::testutil::TempDir;

fn registry() -> Registry {
    let mut r = Registry::default();
    r.add(
        Path::new("t.toml"),
        "[ge]\nname = \"GE\"\nsource = { github = \"o/r\" }\nasset = { any = 'x' }\nlaunchers = { steam = \"compatibilitytools.d\", lutris = \"runners/proton\" }",
    )
    .unwrap();
    r
}

fn launcher(home: &Path, kind: LauncherKind, packaging: Packaging) -> Launcher {
    Launcher {
        kind,
        packaging,
        root: home.join(format!("{}-{}", kind.name(), packaging.name())),
    }
}

/// An installed version, with its record.
fn installed(l: &Launcher, dir: &str, version: &str) {
    let path = l.root.join(dir);
    fs::create_dir_all(&path).unwrap();
    let record = install::Record {
        tool: "ge".into(),
        version: version.into(),
        asset: "a.tar.gz".into(),
        checksum: Some(format!("sum-{version}")),
        installed_at: 0,
    };
    fs::write(
        path.join(install::RECORD),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

#[test]
fn exports_and_plans_on_another_machine() {
    let desktop = TempDir::new("lock-desktop");
    let steam = launcher(&desktop, LauncherKind::Steam, Packaging::Native);
    let lutris = launcher(&desktop, LauncherKind::Lutris, Packaging::Native);
    installed(&steam, "compatibilitytools.d/GE-11", "GE-Proton11-7");
    installed(&lutris, "runners/proton/GE-10", "GE-Proton10-34");
    fs::create_dir_all(steam.root.join("compatibilitytools.d/Manual")).unwrap();

    let lock = Lock::export(&registry(), &[steam.clone(), lutris]).unwrap();
    let text = lock.to_string();
    assert_eq!(Lock::parse(&text).unwrap(), lock);
    let entries: Vec<_> = lock
        .entries
        .iter()
        .map(|e| {
            (
                e.launcher.as_str(),
                e.version.as_str(),
                e.checksum.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        entries,
        [
            ("lutris", "GE-Proton10-34", Some("sum-GE-Proton10-34")),
            ("steam", "GE-Proton11-7", Some("sum-GE-Proton11-7"))
        ]
    );

    // The laptop has Steam as a Flatpak, GE-Proton11-7 already there,
    // and no Lutris.
    let laptop = TempDir::new("lock-laptop");
    let flatpak = launcher(&laptop, LauncherKind::Steam, Packaging::Flatpak);
    installed(&flatpak, "compatibilitytools.d/GE-11", "GE-Proton11-7");
    let plan = plan(&lock, &registry(), &[flatpak]);
    assert!(
        plan[0]
            .target
            .as_ref()
            .unwrap_err()
            .contains("no lutris launcher")
    );
    assert!(plan[1].target.is_ok() && plan[1].present);
}

#[test]
fn a_kind_on_a_machine_twice_is_exported_with_its_packaging() {
    let home = TempDir::new("lock-twice");
    let native = launcher(&home, LauncherKind::Steam, Packaging::Native);
    let flatpak = launcher(&home, LauncherKind::Steam, Packaging::Flatpak);
    installed(&flatpak, "compatibilitytools.d/GE-11", "GE-Proton11-7");
    let lock = Lock::export(&registry(), &[native.clone(), flatpak.clone()]).unwrap();
    assert_eq!(lock.entries[0].launcher, "steam:flatpak");
    let p = plan(&lock, &registry(), &[native.clone(), flatpak]);
    assert!(p[0].present);

    // A bare kind is ambiguous there.
    let mut bare = lock;
    bare.entries[0].launcher = "steam".into();
    let p = plan(
        &bare,
        &registry(),
        &[
            native,
            launcher(&home, LauncherKind::Steam, Packaging::Flatpak),
        ],
    );
    assert!(
        p[0].target
            .as_ref()
            .unwrap_err()
            .contains("several launchers")
    );
}
