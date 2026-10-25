use crate::{Algo, Arch, Tool};

/// A downloadable file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// The asset to install for one architecture, and the checksum file that
/// belongs to it.
#[derive(Debug, PartialEq, Eq)]
pub struct Resolved<'a> {
    pub asset: &'a ReleaseAsset,
    pub checksum: Option<(&'a ReleaseAsset, Algo)>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("{tool} has no build for {arch:?}")]
    NoBuildForArch { tool: String, arch: Arch },
    #[error("no asset of the release matches {pattern} ({tool}, {arch:?}); assets: {assets:?}")]
    NoAsset {
        tool: String,
        arch: Arch,
        pattern: String,
        assets: Vec<String>,
    },
    #[error("{} assets match {pattern} ({tool}, {arch:?}): {matches:?}", matches.len())]
    AmbiguousAsset {
        tool: String,
        arch: Arch,
        pattern: String,
        matches: Vec<String>,
    },
    #[error("the release has no checksum file {expected} for {asset} ({tool})")]
    NoChecksum {
        tool: String,
        asset: String,
        expected: String,
    },
}

/// The name without its archive extension: `GE-Proton11-7-x86_64.tar.gz`
/// becomes `GE-Proton11-7-x86_64`.
pub fn asset_stem(name: &str) -> &str {
    const ARCHIVES: [&str; 8] = [
        ".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2", ".tgz", ".txz", ".zip", ".7z",
    ];
    ARCHIVES
        .iter()
        .find_map(|ext| name.strip_suffix(ext))
        .unwrap_or(name)
}

impl Tool {
    /// Picks the release asset for `arch`, and its checksum file by name.
    ///
    /// Exactly one asset must match: none or several is an error naming
    /// them, never a guess. The checksum file is the one whose name the
    /// manifest's pattern gives for that asset, so a release that ships one
    /// checksum per architecture can't pair an asset with another's.
    pub fn resolve<'a>(
        &self,
        assets: &'a [ReleaseAsset],
        arch: Arch,
    ) -> Result<Resolved<'a>, ResolveError> {
        let re = self.asset.get(&arch).ok_or(ResolveError::NoBuildForArch {
            tool: self.id.clone(),
            arch,
        })?;
        let matches: Vec<_> = assets.iter().filter(|a| re.is_match(&a.name)).collect();
        let asset = match matches[..] {
            [one] => one,
            [] => {
                return Err(ResolveError::NoAsset {
                    tool: self.id.clone(),
                    arch,
                    pattern: re.to_string(),
                    assets: assets.iter().map(|a| a.name.clone()).collect(),
                });
            }
            _ => {
                return Err(ResolveError::AmbiguousAsset {
                    tool: self.id.clone(),
                    arch,
                    pattern: re.to_string(),
                    matches: matches.iter().map(|a| a.name.clone()).collect(),
                });
            }
        };

        let Some(c) = &self.checksum else {
            return Ok(Resolved {
                asset,
                checksum: None,
            });
        };
        let expected = c
            .pattern
            .replace("{asset_stem}", asset_stem(&asset.name))
            .replace("{asset}", &asset.name);
        let file =
            assets
                .iter()
                .find(|a| a.name == expected)
                .ok_or_else(|| ResolveError::NoChecksum {
                    tool: self.id.clone(),
                    asset: asset.name.clone(),
                    expected,
                })?;
        Ok(Resolved {
            asset,
            checksum: Some((file, c.algo)),
        })
    }
}

#[cfg(test)]
#[path = "tests/resolve.rs"]
mod tests;
