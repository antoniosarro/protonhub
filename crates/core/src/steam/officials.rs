//! Valve's Protons, which Steam installs as apps of their own.

use std::fs;
use std::path::{Path, PathBuf};

use super::{app_manifests, libraries, read_text};
use crate::games::Error;

/// A Proton build of Valve's, which Steam installs as an app of its own.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Official {
    pub appid: String,
    /// Its directory's name, as Steam shows it: "Proton 10.0".
    pub name: String,
    pub dir: PathBuf,
    /// The app manifest that tells Steam it's installed.
    pub manifest: PathBuf,
}

/// Valve's Proton builds installed in Steam's libraries.
pub fn officials(root: &Path) -> Result<Vec<Official>, Error> {
    let mut found = Vec::new();
    for lib in libraries(root)? {
        for manifest in app_manifests(&lib)? {
            let Some(app) = read_text(&manifest)? else {
                continue;
            };
            let Some(state) = app.obj("AppState") else {
                continue;
            };
            let (Some(appid), Some(installdir)) = (state.str("appid"), state.str("installdir"))
            else {
                continue;
            };
            let dir = lib.join("steamapps/common").join(installdir);
            if official_name(installdir).is_some() && dir.is_dir() {
                found.push(Official {
                    appid: appid.to_owned(),
                    name: installdir.to_owned(),
                    dir,
                    manifest,
                });
            }
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
}

/// Uninstalls one of Valve's Protons as Steam does: its manifest first, so
/// Steam never sees an installed app with its files gone, then its files.
/// Only while Steam isn't running, which would write the manifest back.
pub fn uninstall(official: &Official) -> Result<(), Error> {
    fs::remove_file(&official.manifest).map_err(|source| Error::Io {
        path: official.manifest.clone(),
        source,
    })?;
    crate::install::delete(&official.dir)?;
    Ok(())
}

/// The internal name of a Valve Proton build from its directory name:
/// "Proton 9.0" is proton_9, "Proton 5.13" proton_513. Steam names them in
/// its app info, which this guesses rather than parses.
pub(super) fn official_name(dir: &str) -> Option<String> {
    let version = dir.strip_prefix("Proton ")?;
    let version = version.strip_suffix(" (Beta)").unwrap_or(version);
    match version {
        "- Experimental" | "Experimental" => return Some("proton_experimental".into()),
        "Hotfix" => return Some("proton_hotfix".into()),
        _ => {}
    }
    let (major, minor) = version.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some(if minor == "0" {
        format!("proton_{major}")
    } else {
        format!("proton_{major}{minor}")
    })
}
