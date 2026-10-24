//! Tool manifests (registry/*.toml): what a tool is, where its releases come
//! from, which asset to download for each architecture, how to find that
//! asset's checksum, and where each launcher expects it.
//!
//! Tools are data so that an upstream rename is fixed by editing a manifest,
//! not by shipping a new release of the app.

mod resolve;
mod schema;

pub use resolve::{ReleaseAsset, ResolveError, Resolved, asset_stem};
pub use schema::{Algo, Arch, Checksum, LauncherKind, LoadError, Registry, Source, Tool};
