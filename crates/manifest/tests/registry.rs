//! The real registry/ against asset lists recorded from the upstream
//! releases, so a pattern edit that breaks a known release fails here.

use std::path::Path;

use protonhub_manifest::{Arch, Registry, ReleaseAsset};

fn registry() -> Registry {
    Registry::load_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../registry")).unwrap()
}

fn release(names: &[&str]) -> Vec<ReleaseAsset> {
    names
        .iter()
        .map(|n| ReleaseAsset {
            name: (*n).into(),
            url: String::new(),
            size: 0,
        })
        .collect()
}

/// (asset, checksum) the registry picks, by name.
fn pick(tool: &str, assets: &[&str], arch: Arch) -> (String, Option<String>) {
    let assets = release(assets);
    let r = registry().tools[tool].resolve(&assets, arch).unwrap();
    (
        r.asset.name.clone(),
        r.checksum.map(|(c, _)| c.name.clone()),
    )
}

#[test]
fn ge_proton_11_ships_per_arch() {
    let assets = [
        "GE-Proton11-7-aarch64.sha512sum",
        "GE-Proton11-7-aarch64.tar.gz",
        "GE-Proton11-7-x86_64.sha512sum",
        "GE-Proton11-7-x86_64.tar.gz",
    ];
    assert_eq!(
        pick("ge-proton", &assets, Arch::X86_64),
        (
            "GE-Proton11-7-x86_64.tar.gz".into(),
            Some("GE-Proton11-7-x86_64.sha512sum".into())
        )
    );
    assert_eq!(
        pick("ge-proton", &assets, Arch::Aarch64),
        (
            "GE-Proton11-7-aarch64.tar.gz".into(),
            Some("GE-Proton11-7-aarch64.sha512sum".into())
        )
    );
}

#[test]
fn ge_proton_10_ships_x86_64_only() {
    let assets = ["GE-Proton10-34.sha512sum", "GE-Proton10-34.tar.gz"];
    assert_eq!(
        pick("ge-proton", &assets, Arch::X86_64),
        (
            "GE-Proton10-34.tar.gz".into(),
            Some("GE-Proton10-34.sha512sum".into())
        )
    );
}

#[test]
fn dxvk_skips_the_native_build() {
    let assets = [
        "dxvk-3.1.1.tar.gz",
        "dxvk-native-3.1.1-steamrt-sniper.tar.gz",
    ];
    assert_eq!(
        pick("dxvk", &assets, Arch::X86_64),
        ("dxvk-3.1.1.tar.gz".into(), None)
    );
}

#[test]
fn wine_ge_skips_the_lol_releases() {
    let r = registry();
    let t = &r.tools["wine-ge"];
    assert!(t.is_version("GE-Proton8-26"));
    assert!(!t.is_version("GE-Proton8-27-LoL"));
    let assets = [
        "wine-lutris-GE-Proton8-26-x86_64.sha512sum",
        "wine-lutris-GE-Proton8-26-x86_64.tar.xz",
    ];
    assert_eq!(
        pick("wine-ge", &assets, Arch::X86_64),
        (
            "wine-lutris-GE-Proton8-26-x86_64.tar.xz".into(),
            Some("wine-lutris-GE-Proton8-26-x86_64.sha512sum".into())
        )
    );
}

/// (tool, arch, the release's assets, the asset and checksum it must pick).
type Case<'a> = (&'a str, Arch, &'a [&'a str], &'a str, Option<&'a str>);

/// Each tool's latest release, recorded on 2026-10-10.
#[test]
fn every_tool_picks_from_its_recorded_release() {
    let cases: &[Case] = &[
        (
            "proton-cachyos",
            Arch::X86_64,
            &[
                "proton-cachyos-11.0-20261005-slr-arm64.sha512sum",
                "proton-cachyos-11.0-20261005-slr-arm64.tar.xz",
                "proton-cachyos-11.0-20261005-slr-x86_64.sha512sum",
                "proton-cachyos-11.0-20261005-slr-x86_64.tar.xz",
                "proton-cachyos-11.0-20261005-slr-x86_64_v3.sha512sum",
                "proton-cachyos-11.0-20261005-slr-x86_64_v3.tar.xz",
            ],
            "proton-cachyos-11.0-20261005-slr-x86_64.tar.xz",
            Some("proton-cachyos-11.0-20261005-slr-x86_64.sha512sum"),
        ),
        (
            "proton-cachyos",
            Arch::Aarch64,
            &[
                "proton-cachyos-11.0-20261005-slr-arm64.sha512sum",
                "proton-cachyos-11.0-20261005-slr-arm64.tar.xz",
                "proton-cachyos-11.0-20261005-slr-x86_64.tar.xz",
            ],
            "proton-cachyos-11.0-20261005-slr-arm64.tar.xz",
            Some("proton-cachyos-11.0-20261005-slr-arm64.sha512sum"),
        ),
        (
            "proton-wineland",
            Arch::X86_64,
            &[
                "proton-wineland-11.0-20261005-x86_64.sha512sum",
                "proton-wineland-11.0-20261005-x86_64.tar.xz",
                "proton-wineland-11.0-20261005-x86_64_v3.sha512sum",
                "proton-wineland-11.0-20261005-x86_64_v3.tar.xz",
                "proton-wineland-11.0-20261005-x86_64_wow64.sha512sum",
                "proton-wineland-11.0-20261005-x86_64_wow64.tar.xz",
            ],
            "proton-wineland-11.0-20261005-x86_64.tar.xz",
            Some("proton-wineland-11.0-20261005-x86_64.sha512sum"),
        ),
        (
            "dw-proton",
            Arch::X86_64,
            &[
                "dwproton-11.0-14-x86_64.sha512sum",
                "dwproton-11.0-14-x86_64.tar.xz",
                "dwproton-11.0-14-x86_64.tar.xz.torrent",
            ],
            "dwproton-11.0-14-x86_64.tar.xz",
            Some("dwproton-11.0-14-x86_64.sha512sum"),
        ),
        (
            "proton-rtsp",
            Arch::X86_64,
            &[
                "proton-rtsp-11.0-20260609-4.tar.gz",
                "proton-rtsp-11.0-20260609-4.tar.gz.sha512sum",
            ],
            "proton-rtsp-11.0-20260609-4.tar.gz",
            Some("proton-rtsp-11.0-20260609-4.tar.gz.sha512sum"),
        ),
        (
            "proton-em",
            Arch::X86_64,
            &[
                "proton-EM-10.0-37-HDR.sha256sum",
                "proton-EM-10.0-37-HDR.tar.xz",
            ],
            "proton-EM-10.0-37-HDR.tar.xz",
            Some("proton-EM-10.0-37-HDR.sha256sum"),
        ),
        (
            "luxtorpeda",
            Arch::X86_64,
            &[
                "luxtorpeda-v77.1.0.tar.xz",
                "luxtorpeda-v77.1.0.tar.xz.sha512",
            ],
            "luxtorpeda-v77.1.0.tar.xz",
            Some("luxtorpeda-v77.1.0.tar.xz.sha512"),
        ),
        (
            "boxtron",
            Arch::X86_64,
            &["boxtron.tar.xz", "boxtron.zip"],
            "boxtron.tar.xz",
            None,
        ),
        (
            "roberta",
            Arch::Aarch64,
            &["roberta.tar.xz", "roberta.zip"],
            "roberta.tar.xz",
            None,
        ),
        (
            "steam-play-none",
            Arch::X86_64,
            &["main.tar.gz"],
            "main.tar.gz",
            None,
        ),
        (
            "lutris-wine",
            Arch::X86_64,
            &["wine-lutris-7.2-2-x86_64.tar.xz"],
            "wine-lutris-7.2-2-x86_64.tar.xz",
            None,
        ),
        (
            "kron4ek-wine",
            Arch::X86_64,
            &[
                "sha256sums.txt",
                "wine-11.19-amd64-wow64.tar.xz",
                "wine-11.19-amd64.tar.xz",
                "wine-11.19-staging-amd64-wow64.tar.xz",
                "wine-11.19-staging-amd64.tar.xz",
                "wine-11.19-staging-tkg-amd64-wow64.tar.xz",
                "wine-11.19-staging-tkg-amd64.tar.xz",
                "wine-11.19-staging-tkg-x86.tar.xz",
            ],
            "wine-11.19-amd64.tar.xz",
            Some("sha256sums.txt"),
        ),
        (
            "dxvk-gplasync",
            Arch::X86_64,
            &[
                "dxvk-gplasync-v3.1.1-1.tar.gz",
                "dxvk-gplasync-v3.1.1-1.zip",
            ],
            "dxvk-gplasync-v3.1.1-1.tar.gz",
            None,
        ),
        (
            "dxvk-nvapi",
            Arch::X86_64,
            &["dxvk-nvapi-v0.9.2.tar.gz"],
            "dxvk-nvapi-v0.9.2.tar.gz",
            None,
        ),
        (
            "vkd3d-proton",
            Arch::X86_64,
            &["vkd3d-proton-3.0.1.tar.zst"],
            "vkd3d-proton-3.0.1.tar.zst",
            None,
        ),
        (
            "vkd3d-lutris",
            Arch::X86_64,
            &["vkd3d-2.14.tar.xz"],
            "vkd3d-2.14.tar.xz",
            None,
        ),
    ];
    for (tool, arch, assets, asset, checksum) in cases {
        assert_eq!(
            pick(tool, assets, *arch),
            ((*asset).to_owned(), checksum.map(str::to_owned)),
            "{tool} {arch:?}"
        );
    }
    let r = registry();
    assert!(r.tools["lutris-wine"].is_version("lutris-wine-7.2-2"));
    assert!(r.tools["lutris-wine"].is_version("lutris-7.1"));
    assert!(!r.tools["lutris-wine"].is_version("lutris-ge-lol-6.16-4"));
}
