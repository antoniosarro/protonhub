//! What protonhub does with a tool: find the launchers on this machine, and
//! download, verify, and install its releases into them.

pub mod channels;
pub mod faugus;
mod files;
pub mod games;
pub mod heroic;
pub mod info;
pub mod install;
pub mod launch;
pub mod launcher;
pub mod lock;
pub mod mangohud;
pub mod manifests;
pub mod paths;
pub mod prefix;
pub mod releases;
pub mod steam;
pub mod storage;
pub mod vdf;

pub use protonhub_manifest as manifest;

#[cfg(test)]
#[path = "tests/testutil.rs"]
mod testutil;
