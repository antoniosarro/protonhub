//! A game's page: its picture, ratings, the tool it runs with and its
//! pages on the web across the top; under them its launch options beside
//! its shortcut, Wine prefix and storage.

mod gamescope;
mod launch;
mod parts;
mod prefix;
mod presets;
mod storage;

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::{Game, Library};
use protonhub_core::steam;

pub use storage::Deletion;

use crate::i18n::{tr, trn};
use crate::text::{anticheat_class, deck_rating, deck_text, source, tier_name};
use crate::window::{Env, LauncherState, Window};
use crate::{choices, ui};

pub fn page(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    lib: &Library,
    game: &Game,
) -> adw::NavigationPage {
    let id_title = match game.source {
        "steam" | "shortcut" => tr("Steam App ID", &[]),
        "faugus" => tr("Faugus Game ID", &[]),
        _ => tr("Heroic App Name", &[]),
    };
    let art = ui::art(env, game.art.as_deref(), 720, "hero-art");
    let (hero, extra) = ui::hero(
        &art,
        &game.name,
        &format!("{} · {id_title} {}", source(game.source), game.id),
    );
    let chips = chips(state, game);
    if !chips.is_empty() {
        extra.append(&ui::wrap(&chips));
    }
    extra.append(&tool(win, state, lib, game));
    if let Some(links) = links(win, game) {
        extra.append(&links);
    }

    let mut first: Vec<gtk::Widget> = Vec::new();
    let mut second: Vec<gtk::Widget> = Vec::new();
    if let Some(group) = launch::group(win, state, game) {
        first.push(group.upcast());
    }
    if let Some(group) = shortcut(win, state, game) {
        second.push(group.upcast());
    }
    if let Some(group) = prefix::group(win, state, game) {
        second.push(group.upcast());
    }
    if let Some(group) = storage::group(win, env, state, game) {
        second.push(group.upcast());
    }
    if first.is_empty() && second.len() > 1 {
        first.push(second.remove(0));
    }

    let columns = ui::columns(&first, &second);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 32);
    content.append(&hero);
    content.append(&columns);
    let key = format!("game/{}/{}", state.launcher, game.id);
    let flips = [(&hero, ui::HERO), (&columns, ui::COLUMNS)];
    ui::page(&game.name, &ui::scrolled(win, key, &content, 1200, &flips)).0
}

/// ProtonDB's rating, the anti-cheat status and Valve's Steam Deck
/// rating, where any is known.
fn chips(state: &LauncherState, game: &Game) -> Vec<gtk::Widget> {
    let mut chips = Vec::new();
    let info = state.info(&game.id);
    if let Some(r) = info.and_then(|i| i.protondb.as_ref()) {
        let reports = trn(
            "{tier}, from {n} report",
            "{tier}, from {n} reports",
            r.total as usize,
            &[("tier", &tier_name(&r.tier))],
        );
        let chip = ui::chip(
            "emblem-ok-symbolic",
            &format!("ProtonDB {}", tier_name(&r.tier)),
            &format!("tier-{}", r.tier),
            Some(&reports),
        );
        chips.push(chip);
    }
    if let Some(a) = info.and_then(|i| i.anticheat.as_ref()) {
        chips.push(ui::chip(
            "security-high-symbolic",
            &a.status,
            anticheat_class(&a.status),
            Some(&tr(
                "Anti-cheat: {names}",
                &[("names", &a.anticheats.join(", "))],
            )),
        ));
    }
    if let Some(d) = &game.deck {
        let (class, rating) = deck_rating(d.rating);
        chips.push(ui::chip(
            "input-gaming-symbolic",
            &tr("Deck {rating}", &[("rating", &rating)]),
            class,
            Some(&tr("Steam Deck: {rating}", &[("rating", &deck_text(d))])),
        ));
    }
    chips
}

/// The tool it runs with, to change, as its card's list does.
fn tool(win: &Rc<Window>, state: &LauncherState, lib: &Library, game: &Game) -> gtk::Box {
    let options = choices::for_game(state.launcher.kind, lib, game);
    let drop = ui::drop_down(&options.labels, options.selected, 28);
    drop.set_sensitive(!state.running);
    if state.running {
        drop.set_tooltip_text(Some(&tr(
            "Quit {launcher} to change it",
            &[("launcher", &state.launcher.title())],
        )));
    }
    let current = options.selected;
    drop.connect_selected_notify(glib::clone!(
        #[weak]
        win,
        #[strong(rename_to = launcher)]
        state.launcher,
        #[strong]
        game,
        move |d| {
            let at = d.selected() as usize;
            if Some(at) != current
                && let Some(choice) = options.choices.get(at)
            {
                win.assign(&launcher, &game, choice.as_ref());
            }
        }
    ));
    let row = gtk::Box::builder().spacing(12).build();
    row.append(&ui::line(&tr("Runs With", &[]), &["heading"]));
    row.append(&drop);
    row
}

/// A Steam game's pages on ProtonDB, SteamDB and the store; none for other
/// games, which those sites don't know.
fn links(win: &Rc<Window>, game: &Game) -> Option<gtk::FlowBox> {
    if game.source != "steam" {
        return None;
    }
    let id = &game.id;
    let buttons: Vec<gtk::Widget> = [
        (
            tr("ProtonDB Reports", &[]),
            format!("https://www.protondb.com/app/{id}"),
        ),
        (
            tr("SteamDB", &[]),
            format!("https://steamdb.info/app/{id}/"),
        ),
        (
            tr("Steam Store", &[]),
            format!("https://store.steampowered.com/app/{id}/"),
        ),
    ]
    .into_iter()
    .map(|(title, uri)| {
        let button = ui::button("web-browser-symbolic", &title);
        button.set_tooltip_text(Some(&uri));
        button.connect_clicked(glib::clone!(
            #[weak]
            win,
            move |_| ui::open_uri(&win, &uri)
        ));
        button.upcast()
    })
    .collect();
    Some(ui::wrap(&buttons))
}

/// A non-Steam shortcut's name, target and start folder, to change as its
/// properties in Steam do.
fn shortcut(win: &Rc<Window>, state: &LauncherState, game: &Game) -> Option<adw::PreferencesGroup> {
    if game.source != "shortcut" {
        return None;
    }
    let (_, shortcut) = steam::shortcuts(&state.launcher.root)
        .ok()?
        .into_iter()
        .find(|(id, _)| *id == game.id)?;
    let group = adw::PreferencesGroup::builder()
        .title(tr("Shortcut", &[]))
        .description(tr(
            "As its properties in Steam show them; a path with spaces is quoted",
            &[],
        ))
        .build();
    let rows: Vec<adw::EntryRow> = [
        (tr("Name", &[]), &shortcut.name),
        (tr("Target", &[]), &shortcut.target),
        (tr("Start In", &[]), &shortcut.start_dir),
    ]
    .into_iter()
    .map(|(title, text)| {
        let row = adw::EntryRow::builder()
            .title(title)
            .text(text.as_str())
            .show_apply_button(true)
            .sensitive(!state.running)
            .build();
        group.add(&row);
        row
    })
    .collect();
    let rows = Rc::new(rows);
    for row in rows.iter() {
        row.connect_apply(glib::clone!(
            #[weak]
            win,
            #[strong]
            rows,
            #[strong(rename_to = launcher)]
            state.launcher,
            #[strong]
            game,
            move |_| {
                let [name, target, start_dir] = [0, 1, 2].map(|i| rows[i].text().to_string());
                let to = steam::Shortcut {
                    name,
                    target,
                    start_dir,
                    launch_options: String::new(),
                };
                win.edit_shortcut(&launcher, &game, &to);
            }
        ));
    }
    Some(group)
}
