//! The tools: what the registry has and their releases, installing and
//! removing them, and what each launcher has and uses.

use std::path::PathBuf;

use protonhub_core::games;
use protonhub_core::install;
use protonhub_core::launcher::{self, Launcher};
use serde::Serialize;

use crate::common::*;
use crate::progress::reported;

pub fn tools(cx: &Cx) -> Result<()> {
    let registry = cx.registry()?;
    #[derive(Serialize)]
    struct Row<'a> {
        id: &'a str,
        name: &'a str,
        launchers: Vec<&'static str>,
    }
    let rows: Vec<_> = registry
        .tools
        .values()
        .map(|t| Row {
            id: &t.id,
            name: &t.name,
            launchers: t.launchers.keys().map(|k| k.name()).collect(),
        })
        .collect();
    let text = rows
        .iter()
        .map(|r| format!("{}\t{}\t{}", r.id, r.name, r.launchers.join(",")))
        .collect::<Vec<_>>()
        .join("\n");
    cx.print(text, &rows)
}

pub fn launchers(cx: &Cx) -> Result<()> {
    let found = launcher::detect(&home()?);
    let text = found
        .iter()
        .map(|l| format!("{l}\t{}", l.root.display()))
        .collect::<Vec<_>>()
        .join("\n");
    cx.print(text, &found)
}

pub fn releases(cx: &Cx, tool: String, n: usize) -> Result<()> {
    let registry = cx.registry()?;
    let tool = find_tool(&registry, &tool)?;
    let releases = client()?.releases(tool, n)?;
    let text = releases
        .iter()
        .map(|r| {
            let date = date(r.published_at.as_deref());
            let pre = if r.prerelease { "\tprerelease" } else { "" };
            format!("{}\t{date}{pre}", r.tag)
        })
        .collect::<Vec<_>>()
        .join("\n");
    cx.print(text, &releases)
}

pub fn install(cx: &Cx, spec: String, launcher: Option<String>) -> Result<()> {
    let registry = cx.registry()?;
    let (id, v) = parse_spec(&spec);
    let tool = find_tool(&registry, id)?;
    let target = pick_launcher(tool, launcher.as_deref())?;
    let downloads = cache_dir()?.join("downloads");
    let (client, arch) = (client()?, arch()?);
    let entry = reported(|progress| {
        install::install(
            &client,
            tool,
            &version(v),
            &target,
            arch,
            &downloads,
            progress,
        )
    })?;
    let missing = install::missing(tool);
    if !missing.is_empty() {
        eprintln!(
            "protonhub: warning: {} needs {}, which isn't installed",
            tool.name,
            missing.join(" and ")
        );
    }
    let record = entry.record.as_ref().expect("an install has a record");
    let verified = if record.checksum.is_some() {
        "checksum verified"
    } else {
        "not verified: the manifest names no checksum"
    };
    let text = format!(
        "installed {} {} into {target}: {} ({verified})",
        tool.id,
        record.version,
        entry.dir.display()
    );
    cx.print(text, &entry)
}

pub fn list(cx: &Cx) -> Result<()> {
    let registry = cx.registry()?;
    #[derive(Serialize)]
    struct Row {
        launcher: Launcher,
        dir: PathBuf,
        entries: Vec<install::Entry>,
    }
    let mut rows = Vec::new();
    for l in launcher::detect(&home()?) {
        for td in install::inventory(&registry, &l)? {
            rows.push(Row {
                launcher: l.clone(),
                dir: td.dir,
                entries: td.entries,
            });
        }
    }
    let mut text = Vec::new();
    for r in &rows {
        for e in &r.entries {
            let name = e.dir.file_name().unwrap_or_default().to_string_lossy();
            let by = match &e.record {
                Some(rec) => format!("{} {}", rec.tool, rec.version),
                None => "(not installed by protonhub)".into(),
            };
            text.push(format!("{}\t{name}\t{by}", r.launcher));
        }
    }
    cx.print(text.join("\n"), &rows)
}

pub fn remove(cx: &Cx, spec: String, launcher: Option<String>) -> Result<()> {
    let registry = cx.registry()?;
    let (id, Some(version)) = parse_spec(&spec) else {
        return Err("name the version to remove: TOOL@VERSION".into());
    };
    let tool = find_tool(&registry, id)?;
    let target = pick_launcher(tool, launcher.as_deref())?;
    let dir = target.tool_dir(tool).expect("pick_launcher checks support");
    let removed = install::remove(&dir, &tool.id, version)?;
    cx.print(format!("removed {}", removed.display()), &removed)
}

pub fn usage(cx: &Cx, launcher: Option<String>) -> Result<()> {
    let registry = cx.registry()?;
    #[derive(Serialize)]
    struct Row {
        launcher: Launcher,
        tools: Vec<games::Usage>,
    }
    // Usage is counted across every launcher, then shown for the
    // ones asked for.
    let shown = detected(launcher.as_deref(), |_| true)?.1;
    let rows: Vec<_> = games::usage(&registry, &launcher::detect(&home()?))?
        .into_iter()
        .filter(|(l, _)| shown.contains(l))
        .map(|(launcher, tools)| Row { launcher, tools })
        .collect();
    let mut text = Vec::new();
    for r in &rows {
        for u in &r.tools {
            let name = u
                .entry
                .dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            text.push(format!(
                "{}\t{name}\t{}\t{}",
                r.launcher,
                install::human_size(u.size),
                users(u)
            ));
        }
    }
    cx.print(text.join("\n"), &rows)
}

pub fn cleanup(cx: &Cx, launcher: Option<String>, yes: bool) -> Result<()> {
    let registry = cx.registry()?;
    let shown = detected(launcher.as_deref(), has_games)?.1;
    let unused: Vec<_> = games::usage(&registry, &launcher::detect(&home()?))?
        .into_iter()
        .filter(|(l, _)| shown.contains(l))
        .flat_map(|(_, tools)| tools)
        .filter(games::Usage::unused)
        .collect();
    let total: u64 = unused.iter().map(|u| u.size).sum();
    if yes {
        for u in &unused {
            install::delete(&u.entry.dir)?;
            eprintln!(
                "deleted {} ({})",
                u.entry.dir.display(),
                install::human_size(u.size)
            );
        }
    }
    let mut text: Vec<_> = unused
        .iter()
        .map(|u| format!("{}\t{}", u.entry.dir.display(), install::human_size(u.size)))
        .collect();
    text.push(freed(
        unused.is_empty(),
        total,
        yes,
        "no unused tools",
        "unused; run with --yes to delete them",
    ));
    cx.print(text.join("\n"), &unused)
}

pub fn changelog(cx: &Cx, spec: String) -> Result<()> {
    let registry = cx.registry()?;
    let (id, v) = parse_spec(&spec);
    let tool = find_tool(&registry, id)?;
    let release = client()?.release(tool, &version(v))?;
    let date = date(release.published_at.as_deref());
    let body = release
        .body
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or("(no notes)");
    cx.print(format!("{} {}\n\n{body}", release.tag, date), &release)
}

/// Who uses a tool, in a few words.
fn users(u: &games::Usage) -> String {
    let Some(users) = &u.users else {
        return "-".into();
    };
    let mut parts = Vec::new();
    if users.default {
        parts.push("the default".to_owned());
    }
    match users.games.len() {
        0 => {}
        1 => parts.push(users.games[0].clone()),
        n => parts.push(format!("{n} games")),
    }
    if parts.is_empty() {
        "unused".into()
    } else {
        parts.join(", ")
    }
}
