//! The installer pipeline: resolve → download → verify → extract into a
//! staging directory → atomic rename → record. Each step fails with its own
//! error, and a failure leaves nothing half-installed behind.
//!
//! An installed version is recorded in a `.protonhub.json` inside its own
//! directory, written before the rename, so a directory either holds a
//! complete install and its record or isn't there. Launchers ignore the file.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use protonhub_manifest::{Algo, Arch, Registry, ResolveError, Tool};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

use crate::launcher::Launcher;
use crate::releases::{self, Client, Release, Version, hex};

pub const RECORD: &str = ".protonhub.json";
const STAGING: &str = ".protonhub-staging";

/// Where the pipeline is, for progress reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Resolve,
    Download {
        done: u64,
        total: u64,
    },
    Verify,
    /// `done` and `total` count bytes of the archive read.
    Extract {
        done: u64,
        total: u64,
    },
}

/// What protonhub knows about a version it installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub tool: String,
    /// The release's tag.
    pub version: String,
    pub asset: String,
    /// The checksum the download was verified against; none when the
    /// manifest names no checksum file.
    pub checksum: Option<String>,
    /// Seconds since the Unix epoch.
    pub installed_at: u64,
}

/// A directory in a launcher's tool directory.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub dir: PathBuf,
    /// None for a directory protonhub didn't install.
    pub record: Option<Record>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Source(#[from] releases::Error),
    #[error(transparent)]
    Resolve(#[from] ResolveError),
    #[error("{tool} can't be installed into {launcher:?}")]
    Unsupported {
        tool: String,
        launcher: protonhub_manifest::LauncherKind,
    },
    #[error("{dir} already exists")]
    AlreadyInstalled { dir: PathBuf },
    #[error("{file} has no checksum for {asset}")]
    NoChecksumLine { file: String, asset: String },
    #[error("{asset}: checksum mismatch: {file} says {expected}, the download is {actual}")]
    ChecksumMismatch {
        asset: String,
        file: String,
        expected: String,
        actual: String,
    },
    #[error("{asset}: unsupported archive format")]
    UnsupportedArchive { asset: String },
    #[error("{asset}: extracting: {source}")]
    Extract { asset: String, source: io::Error },
    #[error("{asset}: the archive has an entry outside its directory: {entry}")]
    UnsafeEntry { asset: String, entry: String },
    #[error("{asset}: expected one top-level directory in the archive, found {found:?}")]
    Layout { asset: String, found: Vec<String> },
    #[error("{tool} {version} isn't installed in {dir}")]
    NotInstalled {
        tool: String,
        version: String,
        dir: PathBuf,
    },
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_owned(),
        source,
    }
}

/// Installs `version` of `tool` into `launcher` and returns its entry.
/// Downloads go to `downloads`, and are deleted once installed or found bad;
/// an interrupted download stays there to resume.
pub fn install(
    client: &Client,
    tool: &Tool,
    version: &Version,
    launcher: &Launcher,
    arch: Arch,
    downloads: &Path,
    progress: &mut dyn FnMut(Step),
) -> Result<Entry, Error> {
    progress(Step::Resolve);
    let release = client.release(tool, version)?;
    install_release(client, tool, release, launcher, arch, downloads, progress)
}

/// `install` for a release already looked up.
pub fn install_release(
    client: &Client,
    tool: &Tool,
    release: Release,
    launcher: &Launcher,
    arch: Arch,
    downloads: &Path,
    progress: &mut dyn FnMut(Step),
) -> Result<Entry, Error> {
    let tools_dir = launcher.tool_dir(tool).ok_or(Error::Unsupported {
        tool: tool.id.clone(),
        launcher: launcher.kind,
    })?;
    if let Some(e) = find(&tools_dir, &tool.id, &release.tag)? {
        return Err(Error::AlreadyInstalled { dir: e.dir });
    }
    let resolved = tool.resolve(&release.assets, arch)?;
    let asset = resolved.asset;
    let expected = match resolved.checksum {
        Some((file, algo)) => {
            let text = client.fetch_text(&file.url)?;
            let per_asset = tool.checksum.as_ref().is_some_and(|c| c.per_asset());
            let sum = checksum_for(&text, &asset.name, per_asset).ok_or_else(|| {
                Error::NoChecksumLine {
                    file: file.name.clone(),
                    asset: asset.name.clone(),
                }
            })?;
            Some((sum, algo, &file.name))
        }
        None => None,
    };

    let archive = client.download(asset, downloads, &mut |done, total| {
        progress(Step::Download { done, total })
    })?;
    // From here on the download is consumed, whatever happens.
    let _archive = Cleanup(archive.clone());

    if let Some((want, algo, file)) = &expected {
        progress(Step::Verify);
        let got = digest(&archive, *algo)?;
        if &got != want {
            return Err(Error::ChecksumMismatch {
                asset: asset.name.clone(),
                file: (*file).clone(),
                expected: want.clone(),
                actual: got,
            });
        }
    }

    fs::create_dir_all(&tools_dir).map_err(io_err(&tools_dir))?;
    let staging = tools_dir.join(format!("{STAGING}-{}", std::process::id()));
    let _staging = Cleanup(staging.clone());
    let unpacked = staging.join("unpacked");
    fs::create_dir_all(&unpacked).map_err(io_err(&unpacked))?;
    extract(&archive, &asset.name, &unpacked, &mut |done, total| {
        progress(Step::Extract { done, total })
    })?;

    // The archive's one top-level directory, or with a `dir` in the
    // manifest, its contents under that name.
    let top = match &tool.dir {
        None => single_dir(&unpacked, &asset.name)?,
        Some(template) => {
            let name = template.replace("{tag}", &release.tag.replace('/', "-"));
            let contents = single_dir(&unpacked, &asset.name).unwrap_or_else(|_| unpacked.clone());
            let named = staging.join(name);
            fs::rename(&contents, &named).map_err(io_err(&named))?;
            named
        }
    };
    let target = tools_dir.join(top.file_name().expect("a directory entry has a name"));
    // A tool whose directory doesn't change between versions (Boxtron's is
    // always boxtron) replaces its older version; anything else there stays.
    let replaces = if target.exists() {
        let ours = list(&tools_dir)?
            .into_iter()
            .any(|e| e.dir == target && e.record.is_some_and(|r| r.tool == tool.id));
        if !ours {
            return Err(Error::AlreadyInstalled { dir: target });
        }
        true
    } else {
        false
    };
    let record = Record {
        tool: tool.id.clone(),
        version: release.tag,
        asset: asset.name.clone(),
        checksum: expected.map(|(sum, _, _)| sum),
        installed_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let record_path = top.join(RECORD);
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&record).expect("records serialize"),
    )
    .map_err(io_err(&record_path))?;
    if replaces {
        // Out of the way first; the staging cleanup deletes it.
        let old = staging.join("replaced");
        fs::rename(&target, &old).map_err(io_err(&target))?;
    }
    fs::rename(&top, &target).map_err(io_err(&target))?;
    Ok(Entry {
        dir: target,
        record: Some(record),
    })
}

/// The directories in a launcher's tool directory, with protonhub's record
/// where it has one. A directory that doesn't exist yet holds nothing.
pub fn list(tools_dir: &Path) -> Result<Vec<Entry>, Error> {
    let entries = match fs::read_dir(tools_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(io_err(tools_dir)(source)),
    };
    let mut list = Vec::new();
    for entry in entries {
        let dir = entry.map_err(io_err(tools_dir))?.path();
        let hidden = dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        if hidden || !dir.is_dir() {
            continue;
        }
        let record_path = dir.join(RECORD);
        let record = match fs::read(&record_path) {
            Ok(bytes) => Some(serde_json::from_slice(&bytes).map_err(|e| Error::Io {
                path: record_path,
                source: e.into(),
            })?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(source) => return Err(io_err(&record_path)(source)),
        };
        list.push(Entry { dir, record });
    }
    list.sort_by(|a, b| a.dir.cmp(&b.dir));
    Ok(list)
}

/// One of a launcher's tool directories and what is in it.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDir {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
}

/// Every directory the registry's tools install into in `launcher`, once
/// each: several tools can share one, like Steam's compatibilitytools.d.
pub fn inventory(registry: &Registry, launcher: &Launcher) -> Result<Vec<ToolDir>, Error> {
    let mut dirs: Vec<_> = registry
        .tools
        .values()
        .filter_map(|t| launcher.tool_dir(t))
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs.into_iter()
        .map(|dir| {
            Ok(ToolDir {
                entries: list(&dir)?,
                dir,
            })
        })
        .collect()
}

fn find(tools_dir: &Path, tool: &str, version: &str) -> Result<Option<Entry>, Error> {
    Ok(list(tools_dir)?.into_iter().find(|e| {
        e.record
            .as_ref()
            .is_some_and(|r| r.tool == tool && r.version == version)
    }))
}

/// The programs `tool` requires that aren't on PATH; the tool installs
/// without them, but won't run.
pub fn missing(tool: &Tool) -> Vec<String> {
    tool.requires
        .iter()
        .filter(|cmd| !on_path(cmd))
        .cloned()
        .collect()
}

/// Whether the program `cmd` is in a directory of `$PATH`.
pub fn on_path(cmd: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|dir| dir.join(cmd).is_file())
}

/// Removes a version protonhub installed, and returns its directory. It is
/// renamed out of the launcher's sight first, so an interrupted removal
/// never leaves a broken tool for the launcher to pick.
pub fn remove(tools_dir: &Path, tool: &str, version: &str) -> Result<PathBuf, Error> {
    let entry = find(tools_dir, tool, version)?.ok_or_else(|| Error::NotInstalled {
        tool: tool.to_owned(),
        version: version.to_owned(),
        dir: tools_dir.to_owned(),
    })?;
    delete(&entry.dir)?;
    Ok(entry.dir)
}

/// Deletes a directory in a tool directory, whoever installed it, renaming
/// it out of the launcher's sight first so an interrupted deletion never
/// leaves a broken tool behind. Cleanup uses it for tools nobody uses.
pub fn delete(dir: &Path) -> Result<(), Error> {
    let tools_dir = dir.parent().expect("a tool is inside a tool directory");
    let doomed = tools_dir.join(format!("{STAGING}-remove-{}", std::process::id()));
    fs::rename(dir, &doomed).map_err(io_err(dir))?;
    fs::remove_dir_all(&doomed).map_err(io_err(&doomed))
}

/// The bytes of the files under `dir`, without following symlinks. It is
/// for display: what can't be read counts as nothing.
pub fn size(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

/// A size as people read it: 12 KiB, 340 MiB, 1.2 GiB.
pub fn human_size(bytes: u64) -> String {
    const MIB: f64 = (1 << 20) as f64;
    if bytes >= 1 << 30 {
        format!("{:.1} GiB", bytes as f64 / (MIB * 1024.0))
    } else if bytes >= 1 << 20 {
        format!("{:.0} MiB", bytes as f64 / MIB)
    } else {
        format!("{} KiB", bytes.div_ceil(1024))
    }
}

/// The checksum for `asset` in a `sha*sum`-style file: lines of
/// `<hex>  <name>` (or `<hex> *<name>`), or, in a file of the asset's own
/// (`per_asset`), a lone hash.
fn checksum_for(text: &str, asset: &str, per_asset: bool) -> Option<String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let named = lines.clone().find_map(|line| {
        let (sum, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        let name = name.rsplit('/').next().unwrap_or(name);
        (name == asset).then(|| sum.to_ascii_lowercase())
    });
    named.or_else(|| {
        if !per_asset {
            return None;
        }
        match (lines.next(), lines.next()) {
            (Some(only), None) if only.chars().all(|c| c.is_ascii_hexdigit()) => {
                Some(only.to_ascii_lowercase())
            }
            _ => None,
        }
    })
}

fn digest(path: &Path, algo: Algo) -> Result<String, Error> {
    fn run<D: Digest>(path: &Path) -> Result<String, Error> {
        let mut file = File::open(path).map_err(io_err(path))?;
        let mut d = D::new();
        let mut buf = vec![0; 1 << 16];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => d.update(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(source) => return Err(io_err(path)(source)),
            }
        }
        Ok(hex(&d.finalize()))
    }
    match algo {
        Algo::Sha256 => run::<Sha256>(path),
        Algo::Sha512 => run::<Sha512>(path),
    }
}

fn extract(
    archive: &Path,
    name: &str,
    into: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<(), Error> {
    let file = File::open(archive).map_err(io_err(archive))?;
    let total = file.metadata().map_err(io_err(archive))?.len();
    let counted = Counting {
        inner: file,
        done: 0,
        total,
        progress,
    };
    let decoded: Box<dyn Read> = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        Box::new(flate2::read::GzDecoder::new(counted))
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        Box::new(liblzma::read::XzDecoder::new(counted))
    } else if name.ends_with(".tar.zst") {
        let decoder =
            ruzstd::decoding::StreamingDecoder::new(counted).map_err(|e| Error::Extract {
                asset: name.to_owned(),
                source: io::Error::new(io::ErrorKind::InvalidData, e.to_string()),
            })?;
        Box::new(decoder)
    } else {
        return Err(Error::UnsupportedArchive {
            asset: name.to_owned(),
        });
    };
    let failed = |source| Error::Extract {
        asset: name.to_owned(),
        source,
    };
    let mut tar = tar::Archive::new(decoded);
    for entry in tar.entries().map_err(failed)? {
        let mut entry = entry.map_err(failed)?;
        // unpack_in refuses paths that leave `into`, and says so with false.
        if !entry.unpack_in(into).map_err(failed)? {
            return Err(Error::UnsafeEntry {
                asset: name.to_owned(),
                entry: entry.path_bytes().escape_ascii().to_string(),
            });
        }
    }
    Ok(())
}

/// The one directory an archive extracted into `staging`.
fn single_dir(staging: &Path, asset: &str) -> Result<PathBuf, Error> {
    let found = fs::read_dir(staging)
        .map_err(io_err(staging))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_err(staging))?;
    match &found[..] {
        [one] if one.is_dir() => Ok(one.clone()),
        _ => Err(Error::Layout {
            asset: asset.to_owned(),
            found: found
                .iter()
                .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into())
                .collect(),
        }),
    }
}

struct Counting<'a, R> {
    inner: R,
    done: u64,
    total: u64,
    progress: &'a mut dyn FnMut(u64, u64),
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        (self.progress)(self.done, self.total);
        Ok(n)
    }
}

/// Removes a temporary file or directory when dropped. Removal is best
/// effort: it runs while another error, the one worth reporting, unwinds.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = if self.0.is_dir() {
            fs::remove_dir_all(&self.0)
        } else {
            fs::remove_file(&self.0)
        };
    }
}

#[cfg(test)]
mod tests {
    use protonhub_manifest::{LauncherKind, Registry};

    use super::*;
    use crate::launcher::Packaging;
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

    // Steam-Play-None's branch archive holds Steam-Play-None-main/.
    #[test]
    fn dir_renames_the_top_directory() {
        let home = TempDir::new("rename");
        let archive = gz(&tar(&[("Steam-Play-None-main/launch.sh", "#!/bin/sh")]));
        let (server, _) = github(("main", vec![("main.tar.gz", archive)]));
        let t = tool(
            "source = { github = \"o/r\" }\nasset = { any = '^main' }\ndir = \"Steam-Play-None\"",
        );
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
        let archive = ruzstd::encoding::compress_to_vec(
            &raw[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
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
}
