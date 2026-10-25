//! Checking a download against its checksum, and unpacking it into the
//! staging directory.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use protonhub_manifest::Algo;
use sha2::{Digest, Sha256, Sha512};

use super::{Error, io_err};
use crate::releases::hex;

/// The checksum for `asset` in a `sha*sum`-style file: lines of
/// `<hex>  <name>` (or `<hex> *<name>`), or, in a file of the asset's own
/// (`per_asset`), a lone hash.
pub(super) fn checksum_for(text: &str, asset: &str, per_asset: bool) -> Option<String> {
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

pub(super) fn digest(path: &Path, algo: Algo) -> Result<String, Error> {
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

pub(super) fn extract(
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
pub(super) fn single_dir(staging: &Path, asset: &str) -> Result<PathBuf, Error> {
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

pub(super) struct Counting<'a, R> {
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
pub(super) struct Cleanup(pub(super) PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = if self.0.is_dir() {
            fs::remove_dir_all(&self.0)
        } else {
            fs::remove_file(&self.0)
        };
    }
}
