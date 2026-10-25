//! The launchers' files: reading one that may not be there, the folders in
//! a folder, and writing one back with a backup of what it was.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::games::Error;

/// What `read` reads from `path`, or none where nothing is there.
pub(crate) fn optional<'a, T>(
    path: &'a Path,
    read: impl FnOnce(&'a Path) -> io::Result<T>,
) -> Result<Option<T>, Error> {
    match read(path) {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(Error::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// A JSON file, or none where there isn't one.
pub(crate) fn read_json(path: &Path) -> Result<Option<Value>, Error> {
    let Some(bytes) = optional(path, fs::read)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| Error::Json {
            path: path.to_owned(),
            source,
        })
}

/// The folders in `dir`, sorted; none where it isn't there.
pub(crate) fn subdirs(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let Some(entries) = optional(dir, fs::read_dir)? else {
        return Ok(Vec::new());
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// Whether `a` and `b` are the same folder, though named differently.
pub(crate) fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// Replaces `path` with `data`, keeping the previous version next to it as
/// `<name>.protonhub-backup`. The write is atomic: a crash leaves the old
/// file or the new one.
pub(crate) fn write_with_backup(path: &Path, data: &[u8]) -> Result<(), Error> {
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| Error::Io { path, source }
    };
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    if path.exists() {
        fs::copy(
            path,
            path.with_file_name(format!("{name}.protonhub-backup")),
        )
        .map_err(io(path))?;
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io(dir))?;
    }
    let tmp = path.with_file_name(format!(".{name}.protonhub-tmp"));
    fs::write(&tmp, data).map_err(io(&tmp))?;
    fs::rename(&tmp, path).map_err(io(path))
}
