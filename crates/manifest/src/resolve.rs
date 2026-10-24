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
mod tests {
    use std::path::Path;

    use super::*;
    use crate::Registry;

    fn tool(asset: &str, checksum: &str) -> Tool {
        let text = format!(
            r#"
            [t]
            name = "T"
            source = {{ github = "o/r" }}
            asset = {asset}
            {checksum}
            launchers = {{ steam = "compatibilitytools.d" }}
            "#
        );
        let mut r = Registry::default();
        r.add(Path::new("t.toml"), &text).unwrap();
        r.tools.remove("t").unwrap()
    }

    fn release(names: &[&str]) -> Vec<ReleaseAsset> {
        names
            .iter()
            .map(|n| ReleaseAsset {
                name: (*n).into(),
                url: format!("https://example.invalid/{n}"),
                size: 1,
            })
            .collect()
    }

    fn ge_proton() -> Tool {
        tool(
            r#"{ x86_64 = '^GE-Proton.*-x86_64\.tar\.gz$', aarch64 = '^GE-Proton.*-aarch64\.tar\.gz$' }"#,
            r#"checksum = { pattern = "{asset_stem}.sha512sum", algo = "sha512" }"#,
        )
    }

    // GE-Proton11 ships a checksum per arch: taking the first .sha512sum
    // pairs the x86_64 tarball with aarch64's, and the check fails.
    #[test]
    fn pairs_each_arch_with_its_own_checksum() {
        let assets = release(&[
            "GE-Proton11-7-aarch64.sha512sum",
            "GE-Proton11-7-aarch64.tar.gz",
            "GE-Proton11-7-x86_64.sha512sum",
            "GE-Proton11-7-x86_64.tar.gz",
        ]);
        let t = ge_proton();
        for arch in [Arch::X86_64, Arch::Aarch64] {
            let r = t.resolve(&assets, arch).unwrap();
            let (sum, algo) = r.checksum.unwrap();
            assert_eq!(sum.name, format!("{}.sha512sum", asset_stem(&r.asset.name)));
            assert_eq!(algo, Algo::Sha512);
        }
    }

    #[test]
    fn a_missing_checksum_is_an_error_not_a_neighbour() {
        let assets = release(&[
            "GE-Proton11-7-aarch64.sha512sum",
            "GE-Proton11-7-x86_64.tar.gz",
        ]);
        let err = ge_proton().resolve(&assets, Arch::X86_64).unwrap_err();
        assert_eq!(
            err,
            ResolveError::NoChecksum {
                tool: "t".into(),
                asset: "GE-Proton11-7-x86_64.tar.gz".into(),
                expected: "GE-Proton11-7-x86_64.sha512sum".into(),
            }
        );
    }

    #[test]
    fn no_or_several_matching_assets_are_errors() {
        let t = tool(r#"{ x86_64 = '\.tar\.gz$' }"#, "");
        let none = t.resolve(&release(&["a.zip"]), Arch::X86_64).unwrap_err();
        assert!(matches!(none, ResolveError::NoAsset { .. }), "{none}");
        let two = t
            .resolve(&release(&["a.tar.gz", "b.tar.gz"]), Arch::X86_64)
            .unwrap_err();
        assert!(
            matches!(two, ResolveError::AmbiguousAsset { ref matches, .. } if matches.len() == 2),
            "{two}"
        );
        let arch = t
            .resolve(&release(&["a.tar.gz"]), Arch::Aarch64)
            .unwrap_err();
        assert!(
            matches!(arch, ResolveError::NoBuildForArch { .. }),
            "{arch}"
        );
    }

    #[test]
    fn checksum_pattern_can_use_the_full_name() {
        let t = tool(
            r#"{ x86_64 = '\.tar\.xz$' }"#,
            r#"checksum = { pattern = "{asset}.sha256", algo = "sha256" }"#,
        );
        let assets = release(&["w.tar.xz", "w.tar.xz.sha256", "other.sha256"]);
        let r = t.resolve(&assets, Arch::X86_64).unwrap();
        assert_eq!(r.checksum.unwrap().0.name, "w.tar.xz.sha256");
    }

    #[test]
    fn stems() {
        assert_eq!(
            asset_stem("GE-Proton11-7-x86_64.tar.gz"),
            "GE-Proton11-7-x86_64"
        );
        assert_eq!(asset_stem("dxvk-2.6.tar.gz"), "dxvk-2.6");
        assert_eq!(asset_stem("wine.tar.zst"), "wine");
        assert_eq!(asset_stem("plain"), "plain");
    }
}
