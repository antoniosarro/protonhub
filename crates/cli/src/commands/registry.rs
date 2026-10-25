//! The tool manifests, and the daily timer.

use std::path::PathBuf;

use protonhub_core::manifest::{Arch, Registry};
use protonhub_core::manifests;
use protonhub_core::releases::Version;
use serde::Serialize;

use crate::common::*;
use crate::timer;

pub fn timer_enable(cx: &Cx) -> Result<()> {
    let dir = timer::enable(&cx.registry_dir())?;
    cx.print(
        format!(
            "enabled protonhub-update.timer ({}); it runs `protonhub update --notify` daily",
            dir.display()
        ),
        &dir,
    )
}

pub fn timer_disable(cx: &Cx) -> Result<()> {
    timer::disable()?;
    cx.print("disabled protonhub-update.timer".into(), &())
}

pub fn verify(cx: &Cx, dir: PathBuf) -> Result<()> {
    let registry = Registry::load_dir(&dir)?;
    let client = client()?;
    #[derive(Serialize)]
    struct Row {
        tool: String,
        arch: Option<Arch>,
        release: Option<String>,
        asset: Option<String>,
        checksum: Option<String>,
        error: Option<String>,
    }
    let mut rows = Vec::new();
    for tool in registry.tools.values() {
        let release = match client.release(tool, &Version::Latest) {
            Ok(r) => r,
            Err(e) => {
                rows.push(Row {
                    tool: tool.id.clone(),
                    arch: None,
                    release: None,
                    asset: None,
                    checksum: None,
                    error: Some(e.to_string()),
                });
                continue;
            }
        };
        for &arch in tool.asset.keys() {
            let picked = tool.resolve(&release.assets, arch);
            rows.push(Row {
                tool: tool.id.clone(),
                arch: Some(arch),
                release: Some(release.tag.clone()),
                asset: picked.as_ref().ok().map(|r| r.asset.name.clone()),
                checksum: picked
                    .as_ref()
                    .ok()
                    .and_then(|r| r.checksum.map(|(c, _)| c.name.clone())),
                error: picked.err().map(|e| e.to_string()),
            });
        }
    }
    let text = rows
        .iter()
        .map(|r| {
            let arch = r
                .arch
                .map_or("-".into(), |a| format!("{a:?}").to_lowercase());
            let release = r.release.as_deref().unwrap_or("-");
            match &r.error {
                Some(e) => format!("FAIL\t{}\t{release}\t{arch}\t{e}", r.tool),
                None => format!(
                    "ok\t{}\t{release}\t{arch}\t{}\t{}",
                    r.tool,
                    r.asset.as_deref().unwrap_or("-"),
                    r.checksum.as_deref().unwrap_or("no checksum")
                ),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    cx.print(text, &rows)?;
    let failed = rows.iter().filter(|r| r.error.is_some()).count();
    if failed > 0 {
        return Err(format!("{failed} of {} checks failed", rows.len()).into());
    }
    Ok(())
}

pub fn update(cx: &Cx) -> Result<()> {
    let cache = cache_dir()?;
    manifests::update(&client()?, &cache, true)?;
    let dir = manifests::dir(&cache);
    let n = Registry::load_dir(&dir)?.tools.len();
    let used = if manifests::used() {
        "used over the installed ones"
    } else {
        "not used: this ProtonHub runs from a checkout, or $PROTONHUB_REGISTRY names others"
    };
    cx.print(
        format!("fetched {n} tools into {}; {used}", dir.display()),
        &n,
    )
}

pub fn check(cx: &Cx, dir: PathBuf) -> Result<()> {
    let n = Registry::load_dir(&dir)?.tools.len();
    cx.print(format!("{n} tools, all valid"), &n)
}
