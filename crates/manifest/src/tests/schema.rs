use super::*;

const GE: &str = r#"
    [ge-proton]
    name      = "GE-Proton"
    source    = { github = "GloriousEggroll/proton-ge-custom" }
    asset     = { x86_64 = '^GE-Proton.*-x86_64\.tar\.gz$' }
    checksum  = { pattern = "{asset_stem}.sha512sum", algo = "sha512" }
    launchers = { steam = "compatibilitytools.d", heroic = "tools/proton" }
"#;

fn add(text: &str) -> Result<Registry, LoadError> {
    let mut r = Registry::default();
    r.add(Path::new("t.toml"), text)?;
    Ok(r)
}

#[test]
fn loads_a_tool() {
    let r = add(GE).unwrap();
    let t = &r.tools["ge-proton"];
    assert_eq!(
        t.source,
        Source::Github("GloriousEggroll/proton-ge-custom".into())
    );
    assert!(t.asset[&Arch::X86_64].is_match("GE-Proton11-7-x86_64.tar.gz"));
    assert_eq!(
        t.launchers[&LauncherKind::Heroic],
        Path::new("tools/proton")
    );
}

#[test]
fn rejects_bad_manifests() {
    let cases = [
        ("unknown field", GE.replace("name ", "nmae ")),
        ("bad regex", GE.replace(r"\.tar", r"(\.tar")),
        ("bad tag regex", GE.replace("name ", "tag = '(' \n name ")),
        ("unknown arch", GE.replace("x86_64 =", "riscv =")),
        ("unknown launcher", GE.replace("heroic =", "origin =")),
        (
            "dir with a slash",
            GE.replace("name ", "dir = 'a/b'\n name "),
        ),
        (
            "two sources",
            GE.replace("source    = {", "source    = { forgejo = 'https://x/o/r',"),
        ),
        (
            "forgejo without a URL",
            GE.replace(
                "github = \"GloriousEggroll/proton-ge-custom\"",
                "forgejo = \"codeberg.org/o/r\"",
            ),
        ),
        (
            "url without a version",
            GE.replace(
                "github = \"GloriousEggroll/proton-ge-custom\"",
                "url = \"https://x/a.tar.gz\"",
            ),
        ),
        (
            "subdir without a dir",
            GE.replace("name ", "subdir = 'build'\n name "),
        ),
        (
            "escaping subdir",
            GE.replace("name ", "dir = 'a'\n subdir = '../b'\n name "),
        ),
        ("escaping dir", GE.replace("tools/proton", "../proton")),
        ("absolute dir", GE.replace("tools/proton", "/opt/proton")),
        (
            "no arch",
            GE.replace(r#"x86_64 = '^GE-Proton.*-x86_64\.tar\.gz$'"#, ""),
        ),
    ];
    for (what, text) in cases {
        assert!(add(&text).is_err(), "{what}: accepted");
    }
}

#[test]
fn any_arch_and_other_sources() {
    let r = add(r#"
        [spn]
        name = "Steam-Play-None"
        source = { url = "https://example.org/main.tar.gz", version = "main" }
        asset = { any = '^main\.tar\.gz$', aarch64 = '^other$' }
        dir = "Steam-Play-None"
        requires = ["sh"]
        launchers = { steam = "compatibilitytools.d" }
        [lux]
        name = "Luxtorpeda"
        source = { forgejo = "https://codeberg.org/luxtorpeda/luxtorpeda/" }
        asset = { x86_64 = 'x' }
        launchers = { steam = "compatibilitytools.d" }
        "#)
    .unwrap();
    let spn = &r.tools["spn"];
    assert!(spn.asset[&Arch::X86_64].is_match("main.tar.gz"));
    assert!(
        spn.asset[&Arch::Aarch64].is_match("other"),
        "an explicit arch wins"
    );
    assert_eq!(spn.dir.as_deref(), Some("Steam-Play-None"));
    assert_eq!(spn.requires, ["sh"]);
    assert_eq!(
        r.tools["lux"].source,
        Source::Forgejo("https://codeberg.org/luxtorpeda/luxtorpeda".into())
    );
}

#[test]
fn rejects_a_tool_defined_twice() {
    let mut r = add(GE).unwrap();
    let err = r.add(Path::new("other.toml"), GE).unwrap_err();
    assert!(matches!(err, LoadError::Duplicate { .. }), "{err}");
}
