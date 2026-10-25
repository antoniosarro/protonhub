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

/// The GitHub token to send: `$GITHUB_TOKEN`, else the one saved.
pub fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN").ok().or_else(saved_token)
}

/// The token saved in ProtonHub's settings, `github-token` there.
pub fn saved_token() -> Option<String> {
    let text = std::fs::read_to_string(config_dir()?.join("github-token")).ok()?;
    Some(text.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Saves `token`, readable by the user alone, or with none removes it.
pub fn save_token(token: Option<&str>) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = config_dir().ok_or_else(|| std::io::Error::other("HOME isn't set"))?;
    let path = dir.join("github-token");
    match token.map(str::trim).filter(|t| !t.is_empty()) {
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        Some(token) => {
            std::fs::create_dir_all(&dir)?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            writeln!(file, "{token}")
        }
    }
}

/// The Steam account chosen for launch options, `steam-user` in
/// ProtonHub's settings; none for the one signed in last.
pub fn steam_user() -> Option<String> {
    // Tests mustn't depend on the choice made on the machine.
    if cfg!(test) {
        return None;
    }
    let text = std::fs::read_to_string(config_dir()?.join("steam-user")).ok()?;
    Some(text.trim().to_owned()).filter(|a| !a.is_empty())
}

/// Saves the Steam account chosen, or with none goes back to the one
/// signed in last.
pub fn save_steam_user(account: Option<&str>) -> std::io::Result<()> {
    let dir = config_dir().ok_or_else(|| std::io::Error::other("HOME isn't set"))?;
    let path = dir.join("steam-user");
    match account {
        None => match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        Some(account) => {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&path, format!("{account}\n"))
        }
    }
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
