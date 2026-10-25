//! The systemd user timer that runs `protonhub update --notify` daily: no
//! process stays running between updates.

use std::path::{Path, PathBuf};
use std::process::Command;

use protonhub_core::paths;

use crate::Result;

const SERVICE: &str = "protonhub-update.service";
const TIMER: &str = "protonhub-update.timer";

fn unit_dir() -> Result<PathBuf> {
    let config = paths::config_home().ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(config.join("systemd/user"))
}

/// Writes the units and enables the timer; returns the unit directory.
pub fn enable(registry: &Path) -> Result<PathBuf> {
    if !protonhub_core::games::sees_launchers() {
        return Err(
            "the timer can't be installed from inside the Flatpak; on the host, \
             create a systemd user timer that runs \
             `flatpak run --command=protonhub dev.antoniosarro.ProtonHub update --notify`"
                .into(),
        );
    }
    // `protonhub` as found on PATH, which stays put across upgrades (a
    // Nix store path doesn't), else this binary.
    let exe = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|d| d.join("protonhub"))
        .find(|p| p.is_file())
        .map_or_else(std::env::current_exe, Ok)?;
    // The installed manifests are found from the binary; others, like a
    // checkout's, are named.
    let env = if paths::installed_registry().is_some() {
        String::new()
    } else {
        let dir = registry
            .canonicalize()
            .map_err(|e| format!("{}: {e}", registry.display()))?;
        format!("Environment=PROTONHUB_REGISTRY={}\n", dir.display())
    };
    let dir = unit_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for (name, text) in units(&exe, &env) {
        let path = dir.join(name);
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", TIMER])?;
    Ok(dir)
}

/// The service and timer units, for `exe`, with `env` lines.
fn units(exe: &Path, env: &str) -> [(&'static str, String); 2] {
    [
        (
            SERVICE,
            format!(
                "[Unit]\nDescription=Update ProtonHub's release channels\n\n[Service]\nType=oneshot\n{env}ExecStart={} update --notify\n",
                exe.display()
            ),
        ),
        (
            TIMER,
            "[Unit]\nDescription=Update ProtonHub's release channels daily\n\n[Timer]\nOnCalendar=daily\nPersistent=true\nRandomizedDelaySec=1h\n\n[Install]\nWantedBy=timers.target\n".into(),
        ),
    ]
}

pub fn disable() -> Result<()> {
    let dir = unit_dir()?;
    if dir.join(TIMER).exists() {
        systemctl(&["disable", "--now", TIMER])?;
    }
    for name in [TIMER, SERVICE] {
        let path = dir.join(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{}: {e}", path.display()).into()),
        }
    }
    systemctl(&["daemon-reload"])
}

fn systemctl(args: &[&str]) -> Result<()> {
    let out = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .output()
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "systemctl --user {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into())
    }
}

/// A desktop notification through notify-send; without it, the message
/// goes to stderr (the journal, under the timer).
pub fn notify(summary: &str, body: &str) {
    let sent = Command::new("notify-send")
        .args(["--app-name=ProtonHub", summary, body])
        .status()
        .is_ok_and(|s| s.success());
    if !sent {
        eprintln!("protonhub: couldn't notify (is notify-send installed?): {summary}: {body}");
    }
}

#[cfg(test)]
#[path = "tests/timer.rs"]
mod tests;
