//! Finding the launchers installed for a user, in each way they are packaged.

use std::path::{Path, PathBuf};

use protonhub_manifest::{LauncherKind, Tool};
use serde::Serialize;

/// How a launcher is packaged; each keeps its data somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Packaging {
    Native,
    Flatpak,
    Snap,
}

impl Packaging {
    pub fn name(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Flatpak => "flatpak",
            Self::Snap => "snap",
        }
    }
}

/// A launcher found on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Launcher {
    pub kind: LauncherKind,
    pub packaging: Packaging,
    /// The launcher's data directory; manifests name tool directories
    /// relative to it.
    pub root: PathBuf,
}

impl Launcher {
    /// The home directory the launcher was found in.
    pub fn home(&self) -> Option<PathBuf> {
        LOCATIONS
            .iter()
            .filter(|&&(kind, packaging, _)| kind == self.kind && packaging == self.packaging)
            .find_map(|(_, _, rel)| {
                let base = self.root.to_str()?.strip_suffix(rel)?;
                Some(PathBuf::from(base.strip_suffix('/').unwrap_or(base)))
            })
    }

    /// Where `tool`'s versions go in this launcher, if the tool supports it.
    /// Faugus Launcher's go in a Steam's data directory, not its own.
    pub fn tool_dir(&self, tool: &Tool) -> Option<PathBuf> {
        let base = match self.kind {
            LauncherKind::Faugus => crate::faugus::steam_dir(&self.home()?),
            _ => self.root.clone(),
        };
        tool.launchers.get(&self.kind).map(|d| base.join(d))
    }

    /// Whether `spec` names this launcher: a kind (`steam`) or a kind and
    /// packaging (`steam:flatpak`).
    pub fn matches(&self, spec: &str) -> bool {
        match spec.split_once(':') {
            Some((kind, packaging)) => {
                self.kind.name() == kind && self.packaging.name() == packaging
            }
            None => self.kind.name() == spec,
        }
    }

    /// The spec that names this launcher among `all`: its kind, or
    /// kind:packaging when `all` has the kind twice (native and Flatpak).
    pub fn spec<'a>(&self, all: impl IntoIterator<Item = &'a Launcher>) -> String {
        if all.into_iter().filter(|o| o.kind == self.kind).count() > 1 {
            self.to_string()
        } else {
            self.kind.name().to_owned()
        }
    }

    /// Its name for people, with its packaging unless native.
    pub fn title(&self) -> String {
        let name = self.kind.title();
        match self.packaging {
            Packaging::Native => name.to_owned(),
            Packaging::Flatpak => format!("{name} (Flatpak)"),
            Packaging::Snap => format!("{name} (Snap)"),
        }
    }
}

/// The one launcher among `launchers` that `spec` names and that takes
/// `tool`; otherwise every match, none or several.
pub fn pick<'a>(
    launchers: &'a [Launcher],
    spec: &str,
    tool: &Tool,
) -> Result<&'a Launcher, Vec<&'a Launcher>> {
    let found: Vec<_> = launchers
        .iter()
        .filter(|l| l.matches(spec) && l.tool_dir(tool).is_some())
        .collect();
    match found[..] {
        [one] => Ok(one),
        _ => Err(found),
    }
}

/// Launchers as `steam:native, heroic:flatpak`, for messages.
pub fn list(launchers: &[&Launcher]) -> String {
    launchers
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl std::fmt::Display for Launcher {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind.name(), self.packaging.name())
    }
}

/// Every known data directory, relative to the home directory, in the order
/// they are reported.
const LOCATIONS: &[(LauncherKind, Packaging, &str)] = {
    use LauncherKind::*;
    use Packaging::*;
    &[
        (Steam, Native, ".local/share/Steam"),
        (Steam, Native, ".steam/debian-installation"),
        (
            Steam,
            Flatpak,
            ".var/app/com.valvesoftware.Steam/data/Steam",
        ),
        (Steam, Snap, "snap/steam/common/.local/share/Steam"),
        (Heroic, Native, ".config/heroic"),
        (
            Heroic,
            Flatpak,
            ".var/app/com.heroicgameslauncher.hgl/config/heroic",
        ),
        (Lutris, Native, ".local/share/lutris"),
        (Lutris, Flatpak, ".var/app/net.lutris.Lutris/data/lutris"),
        (Bottles, Native, ".local/share/bottles"),
        (
            Bottles,
            Flatpak,
            ".var/app/com.usebottles.bottles/data/bottles",
        ),
        (Winezgui, Native, ".local/share/winezgui"),
        (
            Winezgui,
            Flatpak,
            ".var/app/io.github.fastrizwaan.WineZGUI/data/winezgui",
        ),
        (Faugus, Native, ".local/share/faugus-launcher"),
        (
            Faugus,
            Flatpak,
            ".var/app/io.github.Faugus.faugus-launcher/data/faugus-launcher",
        ),
    ]
};

/// The launchers whose data directory exists under `home`. A directory
/// reached twice through symlinks (Steam's `~/.steam/root` aliases) is
/// reported once.
pub fn detect(home: &Path) -> Vec<Launcher> {
    let mut seen = Vec::new();
    let mut found = Vec::new();
    for &(kind, packaging, rel) in LOCATIONS {
        let root = home.join(rel);
        let Ok(real) = root.canonicalize() else {
            continue;
        };
        if !real.is_dir() || seen.contains(&real) {
            continue;
        }
        seen.push(real);
        found.push(Launcher {
            kind,
            packaging,
            root,
        });
    }
    found
}

#[cfg(test)]
#[path = "tests/launcher.rs"]
mod tests;
