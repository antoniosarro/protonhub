//! Lockfiles: what ProtonHub installed, written out and installed again.

use std::path::PathBuf;

use protonhub_core::install;
use protonhub_core::launcher;
use protonhub_core::lock::{self, Lock};
use protonhub_core::releases::Version;

use crate::common::*;
use crate::progress::reported;

pub fn export(cx: &Cx, output: Option<PathBuf>) -> Result<()> {
    let registry = cx.registry()?;
    let lock = Lock::export(&registry, &launcher::detect(&home()?))?;
    match output {
        Some(path) => {
            std::fs::write(&path, lock.to_string())
                .map_err(|e| format!("{}: {e}", path.display()))?;
            cx.print(
                format!(
                    "wrote {} versions to {}",
                    lock.entries.len(),
                    path.display()
                ),
                &lock,
            )
        }
        None if cx.json => cx.print(String::new(), &lock),
        None => {
            print!("{lock}");
            Ok(())
        }
    }
}

pub fn apply(cx: &Cx, file: PathBuf, dry_run: bool) -> Result<()> {
    let registry = cx.registry()?;
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let lock = Lock::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
    let plan = lock::plan(&lock, &registry, &launcher::detect(&home()?));
    let (mut lines, mut failed) = (Vec::new(), 0);
    let (client, arch) = (client()?, arch()?);
    let downloads = cache_dir()?.join("downloads");
    for p in &plan {
        let what = format!(
            "{} {} ({})",
            p.locked.tool, p.locked.version, p.locked.launcher
        );
        let target = match &p.target {
            Err(why) => {
                failed += 1;
                lines.push(format!("skipped {what}: {why}"));
                continue;
            }
            Ok(t) => t,
        };
        if p.present {
            lines.push(format!("have {what}"));
            continue;
        }
        if dry_run {
            lines.push(format!("would install {what}"));
            continue;
        }
        let tool = &registry.tools[&p.locked.tool];
        let version = Version::Tag(p.locked.version.clone());
        let result = reported(|progress| {
            install::install(&client, tool, &version, target, arch, &downloads, progress)
        });
        match result {
            // The lockfile names the files it had; a release whose
            // files changed since isn't the same version.
            Ok(entry)
                if p.locked.checksum.is_some()
                    && entry.record.as_ref().and_then(|r| r.checksum.as_ref())
                        != p.locked.checksum.as_ref() =>
            {
                install::delete(&entry.dir)?;
                failed += 1;
                lines.push(format!("removed {what} again: its checksum differs from the lockfile's, so the release changed since the export"));
            }
            Ok(entry) => lines.push(format!("installed {what} into {}", entry.dir.display())),
            Err(e) => {
                failed += 1;
                lines.push(format!("failed {what}: {e}"));
            }
        }
    }
    cx.print(lines.join("\n"), &plan)?;
    if failed > 0 {
        let not = if dry_run {
            "can't be installed here"
        } else {
            "weren't installed"
        };
        return Err(format!("{failed} of {} versions {not}", plan.len()).into());
    }
    Ok(())
}
