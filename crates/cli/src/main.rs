use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use protonhub_core::channels::{self, Channel, Channels};
use protonhub_core::games;
use protonhub_core::info::{self, AntiCheatList};
use protonhub_core::install::{self, Step};
use protonhub_core::launcher::{self, Launcher};
use protonhub_core::lock::{self, Lock};
use protonhub_core::manifest::{Arch, LauncherKind, Registry, Tool};
use protonhub_core::paths;
use protonhub_core::releases::{self, Client, Version};
use serde::Serialize;

mod timer;

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

/// Install and manage Proton, Wine and DXVK builds across game launchers.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// The directory of tool manifests [default: $PROTONHUB_REGISTRY, the
    /// installed ones, or ./registry]
    #[arg(long, global = true)]
    registry: Option<PathBuf>,
    /// Print JSON instead of text
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the tools the registry defines
    Tools,
    /// List the launchers found for this user
    Launchers,
    /// List a tool's newest releases
    Releases {
        tool: String,
        /// How many
        #[arg(short, default_value_t = 10)]
        n: usize,
    },
    /// Install TOOL[@VERSION]; the version is a release tag, or latest
    Install {
        spec: String,
        /// The launcher, as KIND or KIND:PACKAGING (steam, heroic:flatpak);
        /// needed when more than one is found
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List what is in each launcher's tool directories
    List,
    /// Remove TOOL@VERSION, installed by protonhub
    Remove {
        spec: String,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List the games of Steam and Heroic and the tool each runs with
    Games {
        #[arg(long)]
        launcher: Option<String>,
        /// Add ProtonDB's rating and Are We Anti-Cheat Yet's status (network,
        /// cached for a day)
        #[arg(long)]
        info: bool,
    },
    /// Set the tool GAME (its id or name) runs with: a tool's id or name,
    /// or "default" for the launcher's default
    Assign {
        game: String,
        tool: String,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Show each installed tool's size and the games that use it
    Usage {
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List the tools no game uses, and with --yes delete them
    Cleanup {
        #[arg(long)]
        launcher: Option<String>,
        /// Delete them
        #[arg(long)]
        yes: bool,
    },
    /// Keep a tool at its latest release in a launcher
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// Update every channel: install new releases, move games if asked,
    /// delete old versions no game uses
    Update {
        /// Only say which channels have a new release
        #[arg(long)]
        check: bool,
        /// Send a desktop notification when something changed or failed
        #[arg(long)]
        notify: bool,
    },
    /// Move every game set to a tool matching FROM (a regex on its id or
    /// name) to the tool TO
    Reassign {
        from: String,
        to: String,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Print the notes of TOOL[@VERSION]'s release
    Changelog { spec: String },
    /// Write what ProtonHub installed here as a lockfile, to stdout or FILE
    Export {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Install what a lockfile lists and isn't here
    Apply {
        file: PathBuf,
        /// Only say what would be installed
        #[arg(long)]
        dry_run: bool,
    },
    /// Run `update --notify` daily, with a systemd user timer
    #[command(subcommand)]
    Timer(TimerCommand),
    /// Work with tool manifests
    #[command(subcommand)]
    Registry(RegistryCommand),
}

#[derive(Subcommand)]
enum ChannelCommand {
    /// Keep TOOL updated in a launcher
    Add {
        tool: String,
        #[arg(long)]
        launcher: Option<String>,
        /// How many versions to keep, newest first
        #[arg(long, default_value_t = 3)]
        keep: usize,
        /// Move games set to an older version to the new one
        #[arg(long)]
        move_games: bool,
    },
    /// List the channels
    List,
    /// Stop keeping TOOL updated
    Remove {
        tool: String,
        #[arg(long)]
        launcher: Option<String>,
    },
}

#[derive(Subcommand)]
enum TimerCommand {
    /// Install and start the timer
    Enable,
    /// Stop and remove the timer
    Disable,
}

#[derive(Subcommand)]
enum RegistryCommand {
    /// Load every manifest in DIR and report the first error
    Check { dir: PathBuf },
    /// Resolve every tool's latest release in DIR, for every arch, and
    /// report the asset and checksum each picks or why it can't (network)
    Verify { dir: PathBuf },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("protonhub: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let json = cli.json;
    let registry_dir = cli.registry.clone().unwrap_or_else(paths::registry_dir);
    match cli.command {
        Command::Tools => {
            let registry = Registry::load_dir(&registry_dir)?;
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
            print(json, text, &rows)
        }
        Command::Launchers => {
            let found = launcher::detect(&home()?);
            let text = found
                .iter()
                .map(|l| format!("{l}\t{}", l.root.display()))
                .collect::<Vec<_>>()
                .join("\n");
            print(json, text, &found)
        }
        Command::Releases { tool, n } => {
            let registry = Registry::load_dir(&registry_dir)?;
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
            print(json, text, &releases)
        }
        Command::Install { spec, launcher } => {
            let registry = Registry::load_dir(&registry_dir)?;
            let (id, v) = parse_spec(&spec);
            let tool = find_tool(&registry, id)?;
            let target = pick_launcher(tool, launcher.as_deref())?;
            let downloads = cache_dir()?.join("downloads");
            let mut report = Progress::new();
            let entry = install::install(
                &client()?,
                tool,
                &version(v),
                &target,
                arch()?,
                &downloads,
                &mut |step| report.step(step),
            );
            report.finish();
            let entry = entry?;
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
            print(json, text, &entry)
        }
        Command::List => {
            let registry = Registry::load_dir(&registry_dir)?;
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
            print(json, text.join("\n"), &rows)
        }
        Command::Remove { spec, launcher } => {
            let registry = Registry::load_dir(&registry_dir)?;
            let (id, Some(version)) = parse_spec(&spec) else {
                return Err("name the version to remove: TOOL@VERSION".into());
            };
            let tool = find_tool(&registry, id)?;
            let target = pick_launcher(tool, launcher.as_deref())?;
            let dir = target.tool_dir(tool).expect("pick_launcher checks support");
            let removed = install::remove(&dir, &tool.id, version)?;
            print(json, format!("removed {}", removed.display()), &removed)
        }
        Command::Games { launcher, info } => {
            #[derive(Serialize)]
            struct Row {
                launcher: Launcher,
                #[serde(flatten)]
                library: games::Library,
                /// By game id, with --info.
                #[serde(skip_serializing_if = "Vec::is_empty")]
                info: Vec<(String, info::Info)>,
            }
            let client = if info { Some(client()?) } else { None };
            let anticheat = match &client {
                Some(c) => Some(AntiCheatList::fetch(c, info::AWACY)?),
                None => None,
            };
            let mut rows = Vec::new();
            for l in launchers(launcher.as_deref(), has_games)?.1 {
                if let Some(library) = games::library(&l)? {
                    let mut infos = Vec::new();
                    if let (Some(c), Some(ac)) = (&client, &anticheat) {
                        for g in &library.games {
                            let rating = info::protondb(c, info::PROTONDB, g)?;
                            infos.push((
                                g.id.clone(),
                                info::Info {
                                    protondb: rating,
                                    anticheat: ac.find(g),
                                },
                            ));
                        }
                    }
                    rows.push(Row {
                        launcher: l,
                        library,
                        info: infos,
                    });
                }
            }
            let mut text = Vec::new();
            for r in &rows {
                for p in &r.library.problems {
                    eprintln!("protonhub: {}: {p}", r.launcher);
                }
                for g in &r.library.games {
                    let tool = match (&g.tool, g.explicit) {
                        (Some(t), true) => t.name.clone(),
                        (Some(t), false) => format!("{} (default)", t.name),
                        (None, _) => "-".into(),
                    };
                    let mut line = format!("{}\t{}\t{}\t{tool}", r.launcher, g.id, g.name);
                    if let Some((_, i)) = r.info.iter().find(|(id, _)| *id == g.id) {
                        let rating = i
                            .protondb
                            .as_ref()
                            .map_or("-".into(), |r| format!("ProtonDB {}", r.tier));
                        let ac = i.anticheat.as_ref().map_or("-".into(), |a| {
                            format!("anti-cheat {}: {}", a.status, a.anticheats.join(", "))
                        });
                        line.push_str(&format!("\t{rating}\t{ac}"));
                    }
                    text.push(line);
                }
            }
            print(json, text.join("\n"), &rows)
        }
        Command::Assign {
            game,
            tool,
            launcher,
        } => {
            let target = pick_one(
                launcher.as_deref(),
                has_games,
                "keep a tool per game (steam, heroic)",
            )?;
            let library = games::library(&target)?.expect("steam and heroic have games");
            let g = find_game(&library, &game)?;
            let choice = if tool == "default" {
                None
            } else {
                Some(find_choice(&library, &tool)?)
            };
            if !games::sees_launchers() {
                eprintln!(
                    "protonhub: warning: inside the Flatpak, ProtonHub can't tell whether {target} is running; \
                     if it is, it overwrites this change when it exits"
                );
            }
            games::assign(&target, &g.id, choice.map(|c| c.id.as_str()))?;
            let to = choice.map_or("the default".into(), |c| c.name.clone());
            print(json, format!("{} now runs with {to}", g.name), &choice)
        }
        Command::Usage { launcher } => {
            let registry = Registry::load_dir(&registry_dir)?;
            #[derive(Serialize)]
            struct Row {
                launcher: Launcher,
                tools: Vec<games::Usage>,
            }
            // Usage is counted across every launcher, then shown for the
            // ones asked for.
            let shown = launchers(launcher.as_deref(), |_| true)?.1;
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
            print(json, text.join("\n"), &rows)
        }
        Command::Cleanup { launcher, yes } => {
            let registry = Registry::load_dir(&registry_dir)?;
            let shown = launchers(launcher.as_deref(), has_games)?.1;
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
            text.push(match (unused.is_empty(), yes) {
                (true, _) => "no unused tools".into(),
                (false, true) => format!("freed {}", install::human_size(total)),
                (false, false) => format!(
                    "{} unused; run with --yes to delete them",
                    install::human_size(total)
                ),
            });
            print(json, text.join("\n"), &unused)
        }
        Command::Channel(cmd) => {
            let path = channels_path()?;
            let mut channels = Channels::load(&path)?;
            match cmd {
                ChannelCommand::Add {
                    tool,
                    launcher,
                    keep,
                    move_games,
                } => {
                    let registry = Registry::load_dir(&registry_dir)?;
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
                    print(json, text, &channel)
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
                    print(json, text, &channels.list)
                }
                ChannelCommand::Remove { tool, launcher } => {
                    let of_tool: Vec<_> = channels
                        .list
                        .iter()
                        .filter(|c| {
                            c.tool == tool && launcher.as_ref().is_none_or(|l| *l == c.launcher)
                        })
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
                    print(
                        json,
                        format!("{tool} in {launcher} is no longer updated"),
                        &launcher,
                    )
                }
            }
        }
        Command::Update { check, notify } => {
            let path = channels_path()?;
            let channels = Channels::load(&path)?;
            if channels.list.is_empty() {
                return print(
                    json,
                    "no channels; add one with `protonhub channel add TOOL`".into(),
                    &channels.list,
                );
            }
            let registry = Registry::load_dir(&registry_dir)?;
            let client = client()?;
            let launchers = launcher::detect(&home()?);
            let downloads = cache_dir()?.join("downloads");
            let cx = channels::Context {
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
                    match channels::check(&cx, ch) {
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
                let mut report = Progress::new();
                let result = channels::update(&cx, ch, &mut |step| report.step(step));
                report.finish();
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
            print(json, text.join("\n"), &rows)?;
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
        Command::Reassign { from, to, launcher } => {
            let target = pick_one(
                launcher.as_deref(),
                has_games,
                "keep a tool per game (steam, heroic)",
            )?;
            let from = regex::Regex::new(&from)?;
            let moved = games::reassign(&target, &from, &to)?;
            let text = match moved.len() {
                0 => format!("no game in {target} is set to a tool matching {from}"),
                n => format!("moved {} to {to}: {}", plural(n, "game"), moved.join(", ")),
            };
            print(json, text, &moved)
        }
        Command::Changelog { spec } => {
            let registry = Registry::load_dir(&registry_dir)?;
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
            print(
                json,
                format!("{} {}\n\n{body}", release.tag, date),
                &release,
            )
        }
        Command::Export { output } => {
            let registry = Registry::load_dir(&registry_dir)?;
            let lock = Lock::export(&registry, &launcher::detect(&home()?))?;
            match output {
                Some(path) => {
                    std::fs::write(&path, lock.to_string())
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    print(
                        json,
                        format!(
                            "wrote {} versions to {}",
                            lock.entries.len(),
                            path.display()
                        ),
                        &lock,
                    )
                }
                None if json => print(json, String::new(), &lock),
                None => {
                    print!("{lock}");
                    Ok(())
                }
            }
        }
        Command::Apply { file, dry_run } => {
            let registry = Registry::load_dir(&registry_dir)?;
            let text =
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let lock = Lock::parse(&text).map_err(|e| format!("{}: {e}", file.display()))?;
            let plan = lock::plan(&lock, &registry, &launcher::detect(&home()?));
            let (mut lines, mut failed) = (Vec::new(), 0);
            let client = client()?;
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
                let mut report = Progress::new();
                let result = install::install(
                    &client,
                    tool,
                    &Version::Tag(p.locked.version.clone()),
                    target,
                    arch()?,
                    &downloads,
                    &mut |step| report.step(step),
                );
                report.finish();
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
                    Ok(entry) => {
                        lines.push(format!("installed {what} into {}", entry.dir.display()))
                    }
                    Err(e) => {
                        failed += 1;
                        lines.push(format!("failed {what}: {e}"));
                    }
                }
            }
            print(json, lines.join("\n"), &plan)?;
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
        Command::Timer(TimerCommand::Enable) => {
            let dir = timer::enable(&registry_dir)?;
            print(
                json,
                format!(
                    "enabled protonhub-update.timer ({}); it runs `protonhub update --notify` daily",
                    dir.display()
                ),
                &dir,
            )
        }
        Command::Timer(TimerCommand::Disable) => {
            timer::disable()?;
            print(json, "disabled protonhub-update.timer".into(), &())
        }
        Command::Registry(RegistryCommand::Verify { dir }) => {
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
            print(json, text, &rows)?;
            let failed = rows.iter().filter(|r| r.error.is_some()).count();
            if failed > 0 {
                return Err(format!("{failed} of {} checks failed", rows.len()).into());
            }
            Ok(())
        }
        Command::Registry(RegistryCommand::Check { dir }) => {
            let n = Registry::load_dir(&dir)?.tools.len();
            print(json, format!("{n} tools, all valid"), &n)
        }
    }
}

/// Prints `text`, or `value` as JSON with --json.
fn print<T: Serialize + ?Sized>(json: bool, text: String, value: &T) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else if !text.is_empty() {
        println!("{text}");
    }
    Ok(())
}

fn arch() -> Result<Arch> {
    Arch::host().ok_or_else(|| "this CPU architecture has no builds".into())
}

fn channels_path() -> Result<PathBuf> {
    Channels::path().ok_or_else(|| "neither XDG_CONFIG_HOME nor HOME is set".into())
}

fn home() -> Result<PathBuf> {
    paths::home().ok_or_else(|| "HOME is not set".into())
}

fn cache_dir() -> Result<PathBuf> {
    paths::cache_dir().ok_or_else(|| "neither XDG_CACHE_HOME nor HOME is set".into())
}

fn client() -> Result<Client> {
    Ok(Client::new(
        releases::API,
        cache_dir()?,
        std::env::var("GITHUB_TOKEN").ok(),
    ))
}

/// `tool@version` → ("tool", Some("version")); `tool` and `tool@latest`
/// have no version.
fn parse_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once('@') {
        Some((tool, "latest")) | Some((tool, "")) => (tool, None),
        Some((tool, version)) => (tool, Some(version)),
        None => (spec, None),
    }
}

fn version(tag: Option<&str>) -> Version {
    tag.map_or(Version::Latest, |t| Version::Tag(t.into()))
}

/// The day of an ISO 8601 time: 2026-10-01T12:00:00Z is 2026-10-01.
fn date(time: Option<&str>) -> &str {
    time.and_then(|t| t.split('T').next()).unwrap_or("")
}

/// "1 game", "3 games".
fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn find_tool<'a>(registry: &'a Registry, id: &str) -> Result<&'a Tool> {
    registry.tools.get(id).ok_or_else(|| {
        let known: Vec<_> = registry.tools.keys().map(String::as_str).collect();
        format!("no tool {id} in the registry; it has: {}", known.join(", ")).into()
    })
}

/// The one detected launcher that `spec` (KIND or KIND:PACKAGING, or none
/// at all) names and `tool` supports.
fn pick_launcher(tool: &Tool, spec: Option<&str>) -> Result<Launcher> {
    let kinds: Vec<_> = tool.launchers.keys().map(|k| k.name()).collect();
    pick_one(
        spec,
        |l| l.tool_dir(tool).is_some(),
        &format!("can take {} ({})", tool.id, kinds.join(", ")),
    )
}

/// The one detected launcher that `spec` names among those `keep` accepts;
/// `what` says what they have in common, for the errors.
fn pick_one(spec: Option<&str>, keep: impl Fn(&Launcher) -> bool, what: &str) -> Result<Launcher> {
    let (supported, mut matching) = launchers(spec, keep)?;
    let names = |ls: &[Launcher]| {
        ls.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match matching.len() {
        1 => Ok(matching.remove(0)),
        0 if supported.is_empty() => Err(format!("no launcher was found that {what}").into()),
        0 => Err(format!(
            "no launcher matches {}; found: {}",
            spec.unwrap_or_default(),
            names(&supported)
        )
        .into()),
        _ => Err(format!(
            "several launchers {what}: {}; choose one with --launcher",
            names(&matching)
        )
        .into()),
    }
}

/// The detected launchers `keep` accepts, and of those the ones `spec`
/// (KIND or KIND:PACKAGING, or none at all) names.
fn launchers(
    spec: Option<&str>,
    keep: impl Fn(&Launcher) -> bool,
) -> Result<(Vec<Launcher>, Vec<Launcher>)> {
    let supported: Vec<_> = launcher::detect(&home()?)
        .into_iter()
        .filter(keep)
        .collect();
    let matching = supported
        .iter()
        .filter(|l| spec.is_none_or(|s| l.matches(s)))
        .cloned()
        .collect();
    Ok((supported, matching))
}

/// Launchers that keep a compatibility tool per game.
fn has_games(l: &Launcher) -> bool {
    matches!(l.kind, LauncherKind::Steam | LauncherKind::Heroic)
}

/// Progress on stderr: a line per step, redrawn in place on a terminal.
struct Progress {
    tty: bool,
    label: &'static str,
    percent: Option<u64>,
}

impl Progress {
    fn new() -> Self {
        Self {
            tty: std::io::stderr().is_terminal(),
            label: "",
            percent: None,
        }
    }

    fn step(&mut self, step: Step) {
        let (label, fraction) = match step {
            Step::Resolve => ("resolving", None),
            Step::Download { done, total } => ("downloading", Some((done, total))),
            Step::Verify => ("verifying", None),
            Step::Extract { done, total } => ("extracting", Some((done, total))),
        };
        // Of an unknown total (0), only the MiB so far.
        let percent = fraction.map(|(done, total)| match total {
            0 => done >> 20,
            _ => done * 100 / total,
        });
        if label == self.label && (percent == self.percent || !self.tty) {
            return;
        }
        let mut err = std::io::stderr().lock();
        if label != self.label && !self.label.is_empty() && self.tty {
            let _ = writeln!(err);
        }
        // Off a terminal each step is one line, so a percentage would be
        // stuck at its first value.
        let line = match (percent, fraction) {
            (_, Some((done, 0))) if self.tty => format!("{label} {}", mib(done)),
            (_, Some((_, 0))) | (_, None) => label.to_owned(),
            (Some(p), Some((_, total))) if self.tty => format!("{label} {p:>3}% of {}", mib(total)),
            (_, Some((_, total))) => format!("{label} {}", mib(total)),
        };
        let _ = if self.tty {
            write!(err, "\r{line}")
        } else {
            writeln!(err, "{line}")
        };
        self.label = label;
        self.percent = percent;
    }

    fn finish(&self) {
        if self.tty && !self.label.is_empty() {
            eprintln!();
        }
    }
}

fn find_game<'a>(library: &'a games::Library, game: &str) -> Result<&'a games::Game> {
    let found: Vec<_> = library
        .games
        .iter()
        .filter(|g| g.id == game || g.name.eq_ignore_ascii_case(game))
        .collect();
    match found[..] {
        [one] => Ok(one),
        [] => Err(format!("no installed game {game}; `protonhub games` lists them").into()),
        _ => Err(format!("several games are called {game}; use its id").into()),
    }
}

fn find_choice<'a>(library: &'a games::Library, tool: &str) -> Result<&'a games::Choice> {
    library
        .choices
        .iter()
        .find(|c| c.id == tool || c.name.eq_ignore_ascii_case(tool))
        .ok_or_else(|| {
            let names: Vec<_> = library.choices.iter().map(|c| c.name.as_str()).collect();
            format!("no tool {tool}; the launcher has: {}", names.join(", ")).into()
        })
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

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}
