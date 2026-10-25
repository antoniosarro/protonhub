//! The installer pipeline: resolve → download → verify → extract into a
//! staging directory → atomic rename → record. Each step fails with its own
//! error, and a failure leaves nothing half-installed behind.
//!
//! An installed version is recorded in a `.protonhub.json` inside its own
//! directory, written before the rename, so a directory either holds a
//! complete install and its record or isn't there. Launchers ignore the file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use protonhub_manifest::{Arch, Registry, ResolveError, Tool};
use serde::{Deserialize, Serialize};

use crate::launcher::Launcher;
use crate::releases::{self, Client, Release, Version};

mod unpack;

use unpack::{Cleanup, checksum_for, digest, extract, single_dir};

pub const RECORD: &str = ".protonhub.json";
const STAGING: &str = ".protonhub-staging";

/// Where the pipeline is, for progress reports.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// A line a program printed, for a job that runs one rather than an
    /// install.
    Output(String),
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
    // manifest, its contents (or its `subdir`'s) under that name.
    let top = match &tool.dir {
        None => single_dir(&unpacked, &asset.name)?,
        Some(template) => {
            let name = template.replace("{tag}", &release.tag.replace('/', "-"));
            let mut contents =
                single_dir(&unpacked, &asset.name).unwrap_or_else(|_| unpacked.clone());
            if let Some(subdir) = &tool.subdir {
                contents.push(subdir);
            }
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

#[cfg(test)]
#[path = "../tests/install.rs"]
mod tests;
