use protonhub_manifest::{LauncherKind, Registry};
use sha2::{Digest, Sha256, Sha512};

use super::*;
use crate::launcher::Packaging;
use crate::releases::hex;
use crate::testutil::{Server, TempDir, github, gz, tar};

struct Fixture {
    home: TempDir,
    server: Server,
    tool: Tool,
}

impl Fixture {
    /// A GitHub serving release v1 of a tool, with `archive` and a
    /// checksum file saying `sum` (the archive's real SHA-512 if None).
    fn new(name: &str, archive: Vec<u8>, sum: Option<&str>) -> Self {
        let sum = sum.map_or_else(|| hex(&Sha512::digest(&archive)), str::to_owned);
        let (server, _) = github((
            "v1",
            vec![
                (
                    "tool-v1-x86_64.sha512sum",
                    format!("{sum}  tool-v1-x86_64.tar.gz\n").into_bytes(),
                ),
                ("tool-v1-x86_64.tar.gz", archive),
            ],
        ));
        let tool = tool(
            "source = { github = \"o/r\" }\nasset = { x86_64 = '-x86_64\\.tar\\.gz$' }\nchecksum = { pattern = \"{asset_stem}.sha512sum\", algo = \"sha512\" }",
        );
        Self {
            home: TempDir::new(name),
            server,
            tool,
        }
    }

    fn install(&self) -> Result<Entry, Error> {
        install_into(&self.home, &self.server, &self.tool, &mut |_| {})
    }

    fn tools_dir(&self) -> PathBuf {
        self.home.join("Steam/compatibilitytools.d")
    }

    /// Everything in the tool directory and the downloads, hidden or not.
    fn leftovers(&self) -> Vec<String> {
        [self.tools_dir(), self.home.join("downloads")]
            .iter()
            .filter_map(|d| fs::read_dir(d).ok())
            .flatten()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }
}

#[test]
fn installs_lists_and_removes() {
    let f = Fixture::new(
        "install",
        gz(&tar(&[
            ("Tool-v1/proton", "#!/bin/sh\n"),
            ("Tool-v1/lib/x", "x"),
        ])),
        None,
    );
    let mut steps = Vec::new();
    let entry = install_into(&f.home, &f.server, &f.tool, &mut |s| {
        steps.push(std::mem::discriminant(&s))
    })
    .unwrap();
    steps.dedup();
    assert_eq!(
        steps,
        [
            Step::Resolve,
            Step::Download { done: 0, total: 0 },
            Step::Verify,
            Step::Extract { done: 0, total: 0 },
        ]
        .iter()
        .map(std::mem::discriminant)
        .collect::<Vec<_>>()
    );

    let dir = f.tools_dir().join("Tool-v1");
    assert_eq!(entry.dir, dir);
    assert_eq!(
        fs::read_to_string(dir.join("proton")).unwrap(),
        "#!/bin/sh\n"
    );
    let record = entry.record.unwrap();
    assert_eq!(
        (record.tool.as_str(), record.version.as_str()),
        ("tool", "v1")
    );
    assert_eq!(record.checksum.unwrap().len(), 128);
    assert_eq!(
        f.leftovers(),
        ["Tool-v1"],
        "the download and staging are gone"
    );

    // An unmanaged directory next to it is listed without a record.
    fs::create_dir(f.tools_dir().join("Proton-manual")).unwrap();
    let listed: Vec<_> = list(&f.tools_dir())
        .unwrap()
        .into_iter()
        .map(|e| (e.dir.file_name().unwrap().to_owned(), e.record.is_some()))
        .collect();
    assert_eq!(
        listed,
        [("Proton-manual".into(), false), ("Tool-v1".into(), true)]
    );

    let again = f.install().unwrap_err();
    assert!(matches!(again, Error::AlreadyInstalled { .. }), "{again}");

    assert_eq!(remove(&f.tools_dir(), "tool", "v1").unwrap(), dir);
    assert_eq!(f.leftovers(), ["Proton-manual"]);
    let gone = remove(&f.tools_dir(), "tool", "v1").unwrap_err();
    assert!(matches!(gone, Error::NotInstalled { .. }), "{gone}");
}

#[test]
fn a_checksum_mismatch_installs_nothing() {
    let f = Fixture::new(
        "mismatch",
        gz(&tar(&[("Tool-v1/proton", "x")])),
        Some("00ff"),
    );
    let err = f.install().unwrap_err();
    assert!(
        matches!(&err, Error::ChecksumMismatch { expected, .. } if expected == "00ff"),
        "{err}"
    );
    assert!(f.leftovers().is_empty(), "{:?}", f.leftovers());
}

#[test]
fn an_archive_without_one_top_directory_installs_nothing() {
    let f = Fixture::new("layout", gz(&tar(&[("a/x", "x"), ("b/y", "y")])), None);
    let err = f.install().unwrap_err();
    assert!(
        matches!(err, Error::Layout { ref found, .. } if found.len() == 2),
        "{err}"
    );
    assert!(f.leftovers().is_empty(), "{:?}", f.leftovers());
}

#[test]
fn an_archive_reaching_outside_installs_nothing() {
    let f = Fixture::new(
        "unsafe",
        gz(&tar(&[("Tool-v1/x", "x"), ("../escape", "y")])),
        None,
    );
    let err = f.install().unwrap_err();
    assert!(matches!(err, Error::UnsafeEntry { .. }), "{err}");
    assert!(!f.home.join("Steam/escape").exists());
    assert!(f.leftovers().is_empty(), "{:?}", f.leftovers());
}

#[test]
fn inventory_lists_a_shared_directory_once() {
    let mut r = Registry::default();
    r.add(
        Path::new("t.toml"),
        r#"
        [a]
        name = "A"
        source = { github = "o/a" }
        asset = { x86_64 = 'x' }
        launchers = { steam = "compatibilitytools.d", lutris = "runners/a" }
        [b]
        name = "B"
        source = { github = "o/b" }
        asset = { x86_64 = 'x' }
        launchers = { steam = "compatibilitytools.d" }
        "#,
    )
    .unwrap();
    let home = TempDir::new("inventory");
    fs::create_dir_all(home.join("compatibilitytools.d/Manual")).unwrap();
    let steam = Launcher {
        kind: LauncherKind::Steam,
        packaging: Packaging::Native,
        root: home.to_path_buf(),
    };
    let inv = inventory(&r, &steam).unwrap();
    assert_eq!(inv.len(), 1);
    assert_eq!(inv[0].dir, home.join("compatibilitytools.d"));
    assert_eq!(inv[0].entries.len(), 1);
    let lutris = Launcher {
        kind: LauncherKind::Lutris,
        ..steam
    };
    let inv = inventory(&r, &lutris).unwrap();
    assert_eq!(inv.len(), 1);
    assert!(
        inv[0].entries.is_empty(),
        "a missing directory holds nothing"
    );
}

#[test]
fn checksum_files() {
    let a = "GE-Proton11-7-x86_64.tar.gz";
    assert_eq!(
        checksum_for(&format!("ABC  {a}\n"), a, true).as_deref(),
        Some("abc")
    );
    assert_eq!(
        checksum_for(&format!("abc *./dist/{a}"), a, true).as_deref(),
        Some("abc")
    );
    assert_eq!(
        checksum_for("abc  other.tar.gz\ndef  x.tar.gz", a, true),
        None
    );
    assert_eq!(checksum_for("abc\n", a, true).as_deref(), Some("abc"));
    // A list shared by every asset must name it: a lone hash there
    // could be any asset's.
    assert_eq!(checksum_for("abc\n", a, false), None);
    assert_eq!(
        checksum_for(&format!("abc  other\ndef  {a}\n"), a, false).as_deref(),
        Some("def")
    );
}

fn tool(manifest: &str) -> Tool {
    let mut r = Registry::default();
    r.add(Path::new("t.toml"), &format!("[tool]\nname = \"Tool\"\nlaunchers = {{ steam = \"compatibilitytools.d\" }}\n{manifest}")).unwrap();
    r.tools.remove("tool").unwrap()
}

fn install_into(
    home: &Path,
    server: &Server,
    tool: &Tool,
    progress: &mut dyn FnMut(Step),
) -> Result<Entry, Error> {
    let client = Client::new(&server.url, home.join("cache"), None);
    let launcher = Launcher {
        kind: LauncherKind::Steam,
        packaging: Packaging::Native,
        root: home.join("Steam"),
    };
    install(
        &client,
        tool,
        &Version::Latest,
        &launcher,
        Arch::X86_64,
        &home.join("downloads"),
        progress,
    )
}

// dxvk-nvapi's archives hold x32/ and x64/ with no directory around them.
#[test]
fn a_flat_archive_goes_into_the_named_directory() {
    let home = TempDir::new("flat");
    let archive = gz(&tar(&[("./x64/nvapi64.dll", "dll"), ("./README.md", "hi")]));
    let (server, _) = github(("v0.9.2", vec![("dxvk-nvapi-v0.9.2.tar.gz", archive)]));
    let t = tool(
        "source = { github = \"o/r\" }\nasset = { any = '\\.tar\\.gz$' }\ndir = \"dxvk-nvapi-{tag}\"",
    );
    let entry = install_into(&home, &server, &t, &mut |_| {}).unwrap();
    assert_eq!(entry.dir.file_name().unwrap(), "dxvk-nvapi-v0.9.2");
    assert_eq!(
        fs::read_to_string(entry.dir.join("x64/nvapi64.dll")).unwrap(),
        "dll"
    );
}

// DXVK-Sarek's archives hold its source, with the DLLs in build/.
#[test]
fn subdir_is_what_goes_into_the_named_directory() {
    let home = TempDir::new("subdir");
    let archive = gz(&tar(&[
        ("dxvk-sarek-1.14.1/build/x64/d3d11.dll", "dll"),
        ("dxvk-sarek-1.14.1/src/dxvk.cpp", "src"),
    ]));
    let (server, _) = github(("v1.14.1", vec![("dxvk-sarek-1.14.1.tar.gz", archive)]));
    let t = tool(
        "source = { github = \"o/r\" }\nasset = { any = '\\.tar\\.gz$' }\ndir = \"dxvk-sarek-{tag}\"\nsubdir = \"build\"",
    );
    let entry = install_into(&home, &server, &t, &mut |_| {}).unwrap();
    assert_eq!(entry.dir.file_name().unwrap(), "dxvk-sarek-v1.14.1");
    assert_eq!(
        fs::read_to_string(entry.dir.join("x64/d3d11.dll")).unwrap(),
        "dll"
    );
    assert!(!entry.dir.join("src").exists());
}

// Steam-Play-None's branch archive holds Steam-Play-None-main/.
#[test]
fn dir_renames_the_top_directory() {
    let home = TempDir::new("rename");
    let archive = gz(&tar(&[("Steam-Play-None-main/launch.sh", "#!/bin/sh")]));
    let (server, _) = github(("main", vec![("main.tar.gz", archive)]));
    let t =
        tool("source = { github = \"o/r\" }\nasset = { any = '^main' }\ndir = \"Steam-Play-None\"");
    let entry = install_into(&home, &server, &t, &mut |_| {}).unwrap();
    assert_eq!(entry.dir.file_name().unwrap(), "Steam-Play-None");
    assert!(entry.dir.join("launch.sh").is_file());
}

// Boxtron's directory is boxtron in every version.
#[test]
fn a_new_version_replaces_the_old_in_the_same_directory() {
    let home = TempDir::new("replace");
    let v = |n: &str| gz(&tar(&[("boxtron/version", n)]));
    let (server, published) = github(("v1", vec![("boxtron.tar.gz", v("1"))]));
    let t = tool("source = { github = \"o/r\" }\nasset = { any = '^boxtron\\.tar\\.gz$' }");
    install_into(&home, &server, &t, &mut |_| {}).unwrap();
    *published.lock().unwrap() = ("v2".into(), vec![("boxtron.tar.gz".into(), v("2"))]);
    let entry = install_into(&home, &server, &t, &mut |_| {}).unwrap();
    assert_eq!(fs::read_to_string(entry.dir.join("version")).unwrap(), "2");
    assert_eq!(entry.record.unwrap().version, "v2");
    let tools_dir = home.join("Steam/compatibilitytools.d");
    let names: Vec<_> = fs::read_dir(&tools_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["boxtron"], "the old version and staging are gone");

    // Someone else's boxtron stays.
    fs::remove_file(entry.dir.join(RECORD)).unwrap();
    *published.lock().unwrap() = ("v3".into(), vec![("boxtron.tar.gz".into(), v("3"))]);
    let err = install_into(&home, &server, &t, &mut |_| {}).unwrap_err();
    assert!(matches!(err, Error::AlreadyInstalled { .. }), "{err}");
    assert_eq!(fs::read_to_string(entry.dir.join("version")).unwrap(), "2");
}

#[test]
fn extracts_zstandard_archives() {
    let home = TempDir::new("zstd");
    let raw = tar(&[("vkd3d-proton-3.0.1/x64/d3d12.dll", "dll")]);
    let archive =
        ruzstd::encoding::compress_to_vec(&raw[..], ruzstd::encoding::CompressionLevel::Fastest);
    let (server, _) = github(("v3.0.1", vec![("vkd3d-proton-3.0.1.tar.zst", archive)]));
    let t = tool("source = { github = \"o/r\" }\nasset = { any = '\\.tar\\.zst$' }");
    let entry = install_into(&home, &server, &t, &mut |_| {}).unwrap();
    assert!(entry.dir.join("x64/d3d12.dll").is_file());
}

// Kron4ek's releases have one sha256sums.txt for every asset.
#[test]
fn a_shared_checksum_list_is_read_by_name() {
    let home = TempDir::new("list");
    let a = gz(&tar(&[("wine-11-amd64/bin/wine", "a")]));
    let b = gz(&tar(&[("wine-11-x86/bin/wine", "b")]));
    let list = format!(
        "{}  wine-11-x86.tar.gz\n{}  wine-11-amd64.tar.gz\n",
        hex(&Sha256::digest(&b)),
        hex(&Sha256::digest(&a))
    );
    let (server, _) = github((
        "11",
        vec![
            ("sha256sums.txt", list.into_bytes()),
            ("wine-11-x86.tar.gz", b),
            ("wine-11-amd64.tar.gz", a),
        ],
    ));
    let t = tool(
        "source = { github = \"o/r\" }\nasset = { x86_64 = '-amd64\\.tar\\.gz$' }\nchecksum = { pattern = \"sha256sums.txt\", algo = \"sha256\" }",
    );
    let entry = install_into(&home, &server, &t, &mut |_| {}).unwrap();
    assert_eq!(
        entry.record.unwrap().checksum.unwrap(),
        hex(&Sha256::digest(gz(&tar(&[(
            "wine-11-amd64/bin/wine",
            "a"
        )]))))
    );
}

#[test]
fn reports_missing_programs() {
    let t = tool(
        "source = { github = \"o/r\" }\nasset = { any = 'x' }\nrequires = [\"sh\", \"no-such-program-protonhub\"]",
    );
    assert_eq!(missing(&t), ["no-such-program-protonhub"]);
}
