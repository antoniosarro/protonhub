//! Lockfiles: the versions ProtonHub installed on one machine, to install
//! the same on another (`protonhub export`, then `protonhub apply`).

use protonhub_manifest::Registry;
use serde::{Deserialize, Serialize};

use crate::install;
use crate::launcher::{self, Launcher};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Locked {
    pub tool: String,
    /// The release's tag.
    pub version: String,
    /// A launcher kind, or kind:packaging where the kind alone is
    /// ambiguous.
    pub launcher: String,
    /// The checksum it was verified against, so the same files install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    #[serde(default, rename = "install")]
    pub entries: Vec<Locked>,
}

impl Lock {
    /// What ProtonHub installed in `launchers`.
    pub fn export(registry: &Registry, launchers: &[Launcher]) -> Result<Self, install::Error> {
        let mut entries = Vec::new();
        for l in launchers {
            let spec = l.spec(launchers);
            for td in install::inventory(registry, l)? {
                for record in td.entries.into_iter().filter_map(|e| e.record) {
                    entries.push(Locked {
                        tool: record.tool,
                        version: record.version,
                        launcher: spec.clone(),
                        checksum: record.checksum,
                    });
                }
            }
        }
        entries.sort();
        entries.dedup();
        Ok(Self { entries })
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }
}

impl std::fmt::Display for Lock {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&toml::to_string_pretty(self).expect("locks serialize"))
    }
}

/// A lockfile entry, matched to this machine.
#[derive(Debug, Serialize)]
pub struct Planned {
    pub locked: Locked,
    /// The launcher to install into, or why there's none.
    pub target: Result<Launcher, String>,
    /// Whether that version is already installed there.
    pub present: bool,
}

pub fn plan(lock: &Lock, registry: &Registry, launchers: &[Launcher]) -> Vec<Planned> {
    lock.entries
        .iter()
        .map(|locked| {
            let target = target(locked, registry, launchers);
            let present = target.as_ref().is_ok_and(|l| {
                let tool = &registry.tools[&locked.tool];
                l.tool_dir(tool)
                    .and_then(|dir| install::list(&dir).ok())
                    .is_some_and(|entries| {
                        entries
                            .iter()
                            .filter_map(|e| e.record.as_ref())
                            .any(|r| r.tool == locked.tool && r.version == locked.version)
                    })
            });
            Planned {
                locked: locked.clone(),
                target,
                present,
            }
        })
        .collect()
}

fn target(
    locked: &Locked,
    registry: &Registry,
    launchers: &[Launcher],
) -> Result<Launcher, String> {
    let tool = registry
        .tools
        .get(&locked.tool)
        .ok_or_else(|| format!("no tool {} in the registry", locked.tool))?;
    match launcher::pick(launchers, &locked.launcher, tool) {
        Ok(l) => Ok(l.clone()),
        Err(found) if found.is_empty() => Err(format!("no {} launcher here", locked.launcher)),
        Err(found) => Err(format!(
            "several launchers match {}: {}; write it as kind:packaging",
            locked.launcher,
            launcher::list(&found)
        )),
    }
}

#[cfg(test)]
#[path = "tests/lock.rs"]
mod tests;
