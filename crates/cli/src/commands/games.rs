//! The games: listing them, one's details, the tool each runs with, the
//! launchers' defaults, and Steam's shortcuts and users.

use protonhub_core::games;
use protonhub_core::info::{self, AntiCheatList};
use protonhub_core::install;
use protonhub_core::launch;
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::LauncherKind;
use protonhub_core::paths;
use protonhub_core::steam;
use protonhub_core::storage;
use serde::Serialize;

use super::storage::kind;
use crate::common::*;

pub fn games(cx: &Cx, launcher: Option<String>, info: bool) -> Result<()> {
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
    for l in detected(launcher.as_deref(), has_games)?.1 {
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
            let mut line = format!("{}\t{}\t{}\t{}", r.launcher, g.id, g.name, tool_label(g));
            if let Some((_, i)) = r.info.iter().find(|(id, _)| *id == g.id) {
                let rating = i
                    .protondb
                    .as_ref()
                    .map_or("-".into(), |r| format!("ProtonDB {}", r.tier));
                let ac = i.anticheat.as_ref().map_or("-".into(), |a| {
                    format!("anti-cheat {}: {}", a.status, a.anticheats.join(", "))
                });
                let deck = g.deck.as_ref().map_or("-".into(), |d| {
                    let rating = serde_json::to_value(d.rating).expect("ratings serialize");
                    match &d.runtime {
                        Some(r) => {
                            format!("Deck {} ({r})", rating.as_str().unwrap_or_default())
                        }
                        None => format!("Deck {}", rating.as_str().unwrap_or_default()),
                    }
                });
                line.push_str(&format!("\t{rating}\t{ac}\t{deck}"));
            }
            text.push(line);
        }
    }
    cx.print(text.join("\n"), &rows)
}

pub fn game(cx: &Cx, game: String, launcher: Option<String>, info: bool) -> Result<()> {
    let (target, g) = locate(launcher.as_deref(), &game)?;
    let g = &g;
    let details = if info {
        let c = client()?;
        let anticheat = AntiCheatList::fetch(&c, info::AWACY)?;
        Some(info::Info {
            protondb: info::protondb(&c, info::PROTONDB, g)?,
            anticheat: anticheat.find(g),
        })
    } else {
        None
    };
    let launch_options = launch::all(&target)?
        .into_iter()
        .flatten()
        .find(|(id, _)| *id == g.id)
        .map(|(_, o)| o);
    let storage: Vec<_> = storage::scan(&target)?
        .into_iter()
        .flat_map(|s| s.items)
        .filter(|i| i.app == g.id)
        .collect();
    #[derive(Serialize)]
    struct Out<'a> {
        launcher: &'a Launcher,
        #[serde(flatten)]
        game: &'a games::Game,
        #[serde(skip_serializing_if = "Option::is_none")]
        info: Option<info::Info>,
        #[serde(skip_serializing_if = "Option::is_none")]
        launch_options: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        storage: Vec<storage::Item>,
    }
    let mut text = vec![
        format!("name\t{}", g.name),
        format!("id\t{}", g.id),
        format!("source\t{}", g.source),
        format!("tool\t{}", tool_label(g)),
    ];
    if let Some(o) = &launch_options {
        text.push(format!("launch\t{o}"));
    }
    if let Some(i) = &details {
        text.push(format!(
            "protondb\t{}",
            i.protondb
                .as_ref()
                .map_or("-".into(), |r| format!("{} ({} reports)", r.tier, r.total))
        ));
        text.push(format!(
            "anti-cheat\t{}",
            i.anticheat.as_ref().map_or("-".into(), |a| format!(
                "{}: {}",
                a.status,
                a.anticheats.join(", ")
            ))
        ));
    }
    for i in &storage {
        text.push(format!(
            "{}\t{}\t{}",
            kind(i.kind),
            install::human_size(i.size),
            i.dir.display()
        ));
    }
    let out = Out {
        launcher: &target,
        game: g,
        info: details,
        launch_options,
        storage,
    };
    cx.print(text.join("\n"), &out)
}

pub fn assign(cx: &Cx, game: String, tool: String, launcher: Option<String>) -> Result<()> {
    let target = pick_one(
        launcher.as_deref(),
        has_games,
        "keep a tool per game (steam, heroic, faugus)",
    )?;
    let library = games::library(&target)?.expect("steam and heroic have games");
    let g = find_game(&library, &game)?;
    let choice = if tool == "default" {
        None
    } else {
        Some(find_choice(&library, &tool)?)
    };
    warn_unseen(&target);
    games::assign(&target, &g.id, choice.map(|c| c.id.as_str()))?;
    let to = choice.map_or("the default".into(), |c| c.name.clone());
    cx.print(format!("{} now runs with {to}", g.name), &choice)
}

pub fn default(cx: &Cx, tool: Option<String>, launcher: Option<String>) -> Result<()> {
    let target = pick_one(
        launcher.as_deref(),
        |l| matches!(l.kind, LauncherKind::Steam | LauncherKind::Faugus),
        "has a default tool (steam, faugus)",
    )?;
    let library = games::library(&target)?.expect("steam and faugus have games");
    let Some(tool) = tool else {
        let text = library
            .default
            .as_ref()
            .map_or("none".into(), |c| c.name.clone());
        return cx.print(text, &library.default);
    };
    let choice = if tool == "none" {
        None
    } else {
        Some(find_choice(&library, &tool)?)
    };
    warn_unseen(&target);
    games::set_default(&target, choice.map(|c| c.id.as_str()))?;
    let text = match choice {
        Some(c) => format!("Games without a tool of their own now run with {}", c.name),
        None => "Steam Play is off for games without a tool of their own".into(),
    };
    cx.print(text, &choice)
}

pub fn reassign(cx: &Cx, from: String, to: String, launcher: Option<String>) -> Result<()> {
    let target = pick_one(
        launcher.as_deref(),
        has_games,
        "keep a tool per game (steam, heroic, faugus)",
    )?;
    let from = regex::Regex::new(&from)?;
    let moved = games::reassign(&target, &from, &to)?;
    let text = match moved.len() {
        0 => format!("no game in {target} is set to a tool matching {from}"),
        n => format!("moved {} to {to}: {}", plural(n, "game"), moved.join(", ")),
    };
    cx.print(text, &moved)
}

pub fn shortcut(
    cx: &Cx,
    game: String,
    name: Option<String>,
    target: Option<String>,
    start_in: Option<String>,
    launcher: Option<String>,
) -> Result<()> {
    let target_launcher = pick_one(launcher.as_deref(), is_steam, "has shortcuts (steam)")?;
    let library = games::library(&target_launcher)?.expect("steam has games");
    let g = find_game(&library, &game)?;
    let (_, mut s) = steam::shortcuts(&target_launcher.root)?
        .into_iter()
        .find(|(id, _)| *id == g.id)
        .ok_or_else(|| format!("{} isn't a non-Steam shortcut", g.name))?;
    if name.is_some() || target.is_some() || start_in.is_some() {
        s.name = name.unwrap_or(s.name);
        s.target = target.unwrap_or(s.target);
        s.start_dir = start_in.unwrap_or(s.start_dir);
        games::edit_shortcut(&target_launcher, &g.id, &s.name, &s.target, &s.start_dir)?;
    }
    let text = format!(
        "name\t{}\ntarget\t{}\nstart in\t{}\nlaunch options\t{}",
        s.name, s.target, s.start_dir, s.launch_options
    );
    cx.print(text, &s)
}

pub fn steam_user(cx: &Cx, user: Option<String>, launcher: Option<String>) -> Result<()> {
    let target = pick_one(launcher.as_deref(), is_steam, "has users (steam)")?;
    if let Some(user) = user {
        let users = steam::users(&target.root)?;
        let account = match user.as_str() {
            "last" => None,
            _ => Some(
                users
                    .iter()
                    .find(|u| u.account == user || u.name.as_deref() == Some(&user))
                    .map(|u| u.account.as_str())
                    .ok_or_else(|| format!("no Steam user {user} with settings here"))?,
            ),
        };
        paths::save_steam_user(account)?;
    }
    let users = steam::users(&target.root)?;
    let text: Vec<_> = users
        .iter()
        .map(|u| {
            let mark = if u.current { "\t(launch options)" } else { "" };
            format!("{}\t{}{mark}", u.account, u.name.as_deref().unwrap_or("-"))
        })
        .collect();
    cx.print(text.join("\n"), &users)
}

pub fn add_to_steam(cx: &Cx, launcher: Option<String>) -> Result<()> {
    let target = pick_one(launcher.as_deref(), is_steam, "has a library (steam)")?;
    let (shortcut, icon) = steam::own_shortcut().ok_or(
        "protonhub-gtk is neither on PATH nor beside this program, so Steam couldn't start it",
    )?;
    let text = if games::add_shortcut(&target, &shortcut, &icon)? {
        format!("added {} to Steam's library", shortcut.target)
    } else {
        "ProtonHub is in Steam's library already".into()
    };
    cx.print(text, &shortcut)
}
