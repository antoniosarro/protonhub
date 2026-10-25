//! Channels: tools kept at their latest release, and updating them.

use protonhub_core::channels::{self, Channel, Channels};
use protonhub_core::launcher;
use protonhub_core::manifests;

use crate::args::ChannelCommand;
use crate::common::*;
use crate::progress::reported;
use crate::timer;

pub fn channel(cx: &Cx, cmd: ChannelCommand) -> Result<()> {
    let path = channels_path()?;
    let mut channels = Channels::load(&path)?;
    match cmd {
        ChannelCommand::Add {
            tool,
            launcher,
            keep,
            move_games,
        } => {
            let registry = cx.registry()?;
            let tool = find_tool(&registry, &tool)?;
            let target = pick_launcher(tool, launcher.as_deref())?;
            let channel = Channel {
                tool: tool.id.clone(),
                launcher: launcher.unwrap_or_else(|| target.kind.name().to_owned()),
                keep: keep.max(1),
                move_games,
            };
            channels.set(channel.clone());
            channels.save(&path)?;
            let text = format!(
                "{} in {target} now updates to each new release, keeping {}{}",
                tool.name,
                plural(channel.keep, "version"),
                if move_games {
                    " and moving games onto it"
                } else {
                    ""
                }
            );
            cx.print(text, &channel)
        }
        ChannelCommand::List => {
            let text = channels
                .list
                .iter()
                .map(|c| {
                    let moves = if c.move_games { "\tmoves games" } else { "" };
                    format!("{}\t{}\tkeep {}{moves}", c.launcher, c.tool, c.keep)
                })
                .collect::<Vec<_>>()
                .join("\n");
            cx.print(text, &channels.list)
        }
        ChannelCommand::Remove { tool, launcher } => {
            let of_tool: Vec<_> = channels
                .list
                .iter()
                .filter(|c| c.tool == tool && launcher.as_ref().is_none_or(|l| *l == c.launcher))
                .map(|c| c.launcher.clone())
                .collect();
            let launcher = match &of_tool[..] {
                [one] => one.clone(),
                [] => return Err(format!("no channel for {tool}").into()),
                _ => {
                    return Err(format!(
                        "{tool} has channels in {}; choose one with --launcher",
                        of_tool.join(", ")
                    )
                    .into());
                }
            };
            channels.remove(&tool, &launcher);
            channels.save(&path)?;
            cx.print(
                format!("{tool} in {launcher} is no longer updated"),
                &launcher,
            )
        }
    }
}

pub fn update(cx: &Cx, check: bool, notify: bool) -> Result<()> {
    let path = channels_path()?;
    let channels = Channels::load(&path)?;
    if channels.list.is_empty() {
        return cx.print(
            "no channels; add one with `protonhub channel add TOOL`".into(),
            &channels.list,
        );
    }
    let client = client()?;
    // The repository's manifests first, as the timer runs this daily.
    if cx.registry.is_none()
        && manifests::used()
        && let Err(e) = manifests::update(&client, &cache_dir()?, false)
    {
        eprintln!("protonhub: warning: fetching the tool manifests: {e}");
    }
    let registry = cx.registry()?;
    let launchers = launcher::detect(&home()?);
    let downloads = cache_dir()?.join("downloads");
    let context = channels::Context {
        client: &client,
        registry: &registry,
        launchers: &launchers,
        arch: arch()?,
        downloads: &downloads,
    };
    let mut text = Vec::new();
    let mut changes = Vec::new();
    let mut errors = Vec::new();
    let mut rows = Vec::new();
    for ch in &channels.list {
        let what = format!("{} in {}", ch.tool, ch.launcher);
        if check {
            match channels::check(&context, ch) {
                Ok(st) => {
                    if !st.installed {
                        changes.push(format!("{} {}", ch.tool, st.latest));
                    }
                    let state = if st.installed {
                        "up to date"
                    } else {
                        "new release"
                    };
                    text.push(format!("{what}\t{}\t{state}", st.latest));
                    rows.push(serde_json::to_value(&st)?);
                }
                Err(e) => {
                    text.push(format!("{what}\tfailed: {e}"));
                    errors.push(format!("{what}: {e}"));
                }
            }
            continue;
        }
        let result = reported(|progress| channels::update(&context, ch, progress));
        match result {
            Ok(u) => {
                let mut did = Vec::new();
                if u.installed.is_some() {
                    did.push(format!("installed {}", u.latest));
                }
                if !u.moved.is_empty() {
                    did.push(format!("moved {}", plural(u.moved.len(), "game")));
                }
                if !u.removed.is_empty() {
                    did.push(format!(
                        "deleted {}",
                        plural(u.removed.len(), "old version")
                    ));
                }
                if did.is_empty() {
                    text.push(format!("{what}\t{} is up to date", u.latest));
                } else {
                    changes.push(format!("{}: {}", ch.tool, did.join(", ")));
                    text.push(format!("{what}\t{}", did.join("; ")));
                }
                text.extend(u.notes.iter().map(|n| format!("  {n}")));
                rows.push(serde_json::to_value(&u)?);
            }
            Err(e) => {
                text.push(format!("{what}\tfailed: {e}"));
                errors.push(format!("{what}: {e}"));
            }
        }
    }
    if notify {
        if !errors.is_empty() {
            timer::notify("ProtonHub couldn't update", &errors.join("\n"));
        } else if !changes.is_empty() {
            let summary = if check {
                "New releases"
            } else {
                "ProtonHub updated"
            };
            timer::notify(summary, &changes.join("\n"));
        }
    }
    cx.print(text.join("\n"), &rows)?;
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} channels failed",
            errors.len(),
            channels.list.len()
        )
        .into())
    }
}
