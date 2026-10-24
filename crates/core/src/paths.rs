//! Where protonhub keeps its files, by the XDG base directory spec.

use std::path::PathBuf;

/// The user's home directory.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `$XDG_CACHE_HOME/protonhub`: API responses, pictures and partial
/// downloads.
pub fn cache_dir() -> Option<PathBuf> {
    xdg("XDG_CACHE_HOME", ".cache").map(|d| d.join("protonhub"))
}

/// `$XDG_CONFIG_HOME/protonhub`: channels.
pub fn config_dir() -> Option<PathBuf> {
    config_home().map(|d| d.join("protonhub"))
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
pub fn config_home() -> Option<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// `$var` if it is an absolute path, else `fallback` in the home directory.
fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(fallback)))
}

/// The tool manifests: `$PROTONHUB_REGISTRY`, else the ones installed with
/// the binary (`<prefix>/share/protonhub/registry` for `<prefix>/bin`), else
/// `registry` in the current directory, as in a checkout.
pub fn registry_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PROTONHUB_REGISTRY") {
        return dir.into();
    }
    installed_registry().unwrap_or_else(|| PathBuf::from("registry"))
}

/// The manifests installed with the binary, if it was installed with them.
pub fn installed_registry() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.join("share/protonhub/registry")).filter(|d| d.is_dir())
}
