//! What protonhub does with a tool: find the launchers on this machine, and
//! download, verify, and install its releases into them.

pub mod channels;
pub mod games;
pub mod heroic;
pub mod info;
pub mod install;
pub mod launcher;
pub mod lock;
pub mod paths;
pub mod releases;
pub mod steam;
pub mod storage;
pub mod vdf;

pub use protonhub_manifest as manifest;

#[cfg(test)]
mod testutil;
