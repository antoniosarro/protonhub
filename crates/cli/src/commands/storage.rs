//! `storage`: Steam games' Wine prefixes and shader caches, and what
//! uninstalled games left behind.

use protonhub_core::install;
use protonhub_core::launcher::Launcher;
use protonhub_core::storage::{self, Kind};
use serde::Serialize;

use crate::common::*;

pub fn list(cx: &Cx, launcher: Option<String>) -> Result<()> {
    #[derive(Serialize)]
    struct Row {
        launcher: Launcher,
        storage: storage::Storage,
    }
    let rows = storages(launcher.as_deref())?
        .into_iter()
        .map(|(launcher, storage)| Row { launcher, storage })
        .collect::<Vec<_>>();
    let mut text = Vec::new();
    for r in &rows {
        warn(&r.storage);
        for i in &r.storage.items {
            text.push(format!(
                "{}	{}	{}	{}	{}",
                r.launcher,
                kind(i.kind),
                install::human_size(i.size),
                i.app,
                label(i)
            ));
        }
    }
    cx.print(text.join("\n"), &rows)
}

pub fn cleanup(cx: &Cx, launcher: Option<String>, yes: bool) -> Result<()> {
    let mut deleted = Vec::new();
    for (l, s) in storages(launcher.as_deref())? {
        warn(&s);
        let orphaned: Vec<_> = s.orphaned().cloned().collect();
        if yes && !orphaned.is_empty() {
            let dirs: Vec<_> = orphaned.iter().map(|i| i.dir.as_path()).collect();
            storage::delete(&l, &dirs)?;
        }
        deleted.extend(orphaned);
    }
    let total: u64 = deleted.iter().map(|i| i.size).sum();
    let mut text: Vec<_> = deleted
        .iter()
        .map(|i| {
            format!(
                "{}\t{}\t{}",
                i.dir.display(),
                install::human_size(i.size),
                label(i)
            )
        })
        .collect();
    text.push(freed(
        deleted.is_empty(),
        total,
        yes,
        "nothing left over",
        "left over; run with --yes to delete it",
    ));
    cx.print(text.join("\n"), &deleted)
}

pub fn remove(cx: &Cx, game: String, prefix: bool, launcher: Option<String>) -> Result<()> {
    let target = pick_one(launcher.as_deref(), is_steam, "has prefixes")?;
    let s = storage::scan(&target)?.expect("a Steam");
    let want = if prefix {
        Kind::Prefix
    } else {
        Kind::ShaderCache
    };
    let item = s
        .items
        .iter()
        .find(|i| i.kind == want && (i.app == game || i.name.as_deref() == Some(&game)))
        .ok_or_else(|| format!("{target} has no {} for {game}", kind(want)))?;
    storage::delete(&target, &[&item.dir])?;
    cx.print(
        format!(
            "deleted {} ({})",
            item.dir.display(),
            install::human_size(item.size)
        ),
        item,
    )
}

/// Warns of what couldn't be read: nothing is judged left over, or apps
/// no longer installed are shown without names.
fn warn(s: &storage::Storage) {
    for p in &s.problems {
        eprintln!("protonhub: warning: {p}; no folder is judged left over");
    }
    if let Some(e) = &s.names_error {
        eprintln!("protonhub: warning: {e}; apps no longer installed have no names");
    }
}

/// The prefixes and shader caches of each Steam `spec` names.
fn storages(spec: Option<&str>) -> Result<Vec<(Launcher, storage::Storage)>> {
    let mut all = Vec::new();
    for l in detected(spec, is_steam)?.1 {
        let s = storage::scan(&l)?.expect("a Steam");
        all.push((l, s));
    }
    Ok(all)
}

/// An item's app by name, and for a leftover why it is one.
fn label(i: &storage::Item) -> String {
    let name = match &i.name {
        Some(n) => n.clone(),
        None if i.shortcut() => "non-Steam game".into(),
        None => "-".into(),
    };
    match (i.orphaned, i.shortcut()) {
        (false, _) => name,
        (true, true) => format!("{name} (removed from Steam)"),
        (true, false) => format!("{name} (uninstalled)"),
    }
}

pub fn kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Prefix => "prefix",
        Kind::ShaderCache => "shader cache",
    }
}
