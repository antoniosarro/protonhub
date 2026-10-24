//! A game's page, pushed over its launcher's games with a back button: its
//! picture and ratings, the tool it runs with, its prefix and shader cache,
//! and its pages on the web.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::games::{Game, Library};
use protonhub_core::install::human_size;
use protonhub_core::launch::{self, Options};
use protonhub_core::launcher::Launcher;

use super::game_row::{options, source, thumbnail, tier_name};
use super::storage::{self, kind_name};
use super::tilde;
use crate::details::open_button;
use crate::errors;
use crate::i18n::{tr, trn};
use crate::window::{Env, LauncherState, Window};

/// Shows the game `id` of `launcher` over the launcher's page, which
/// `from` is in.
pub(super) fn open(win: &Rc<Window>, from: &impl IsA<gtk::Widget>, launcher: &Launcher, id: &str) {
    let nav = from
        .ancestor(adw::NavigationView::static_type())
        .and_downcast::<adw::NavigationView>();
    if let (Some(nav), Some(page)) = (nav, page(win, launcher, id)) {
        win.game.replace(Some((launcher.clone(), id.to_owned())));
        nav.push(&page);
    }
}

/// The page of the game `id` of `launcher`, as last read; none once it is
/// no longer installed.
pub(super) fn page(win: &Rc<Window>, launcher: &Launcher, id: &str) -> Option<adw::NavigationPage> {
    let (snapshot, Ok(env)) = (win.snapshot()?, &win.env) else {
        return None;
    };
    let state = snapshot
        .launchers
        .iter()
        .find(|s| s.launcher == *launcher)?;
    let Ok(Some(lib)) = &state.library else {
        return None;
    };
    let game = lib.games.iter().find(|g| g.id == id)?;
    Some(build(win, env, state, lib, game))
}

fn build(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    lib: &Library,
    game: &Game,
) -> adw::NavigationPage {
    let page = adw::PreferencesPage::new();

    let hero = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    let picture = thumbnail(env, game.art.as_deref(), 460);
    picture.add_css_class("hero");
    picture.set_margin_bottom(6);
    hero.append(&picture);
    let title = gtk::Label::builder()
        .label(&game.name)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    title.add_css_class("title-2");
    hero.append(&title);
    let from = gtk::Label::new(Some(&source(game.source)));
    from.add_css_class("dim-label");
    hero.append(&from);
    let top = adw::PreferencesGroup::new();
    top.add(&hero);
    page.add(&top);

    if let Some(group) = ratings(state, game) {
        page.add(&group);
    }
    page.add(&tool_group(win, state, lib, game));
    if let Some(group) = launch_group(win, state, game) {
        page.add(&group);
    }
    if let Ok(Some(storage)) = &state.storage {
        let group = adw::PreferencesGroup::builder()
            .title(tr("Storage", &[]))
            .build();
        let items: Vec<_> = storage.items.iter().filter(|i| i.app == game.id).collect();
        if items.is_empty() {
            let row = adw::ActionRow::builder()
                .title(tr("No prefixes or shader caches", &[]))
                .subtitle(tr("Steam makes them when a game first runs", &[]))
                .build();
            row.add_css_class("dim-label");
            group.add(&row);
        }
        for item in items {
            let row = adw::ActionRow::builder()
                .title(kind_name(item.kind))
                .subtitle(glib::markup_escape_text(&format!(
                    "{} · {}",
                    human_size(item.size),
                    tilde(&item.dir, &env.home)
                )))
                .build();
            row.add_suffix(&open_button(win, &item.dir));
            let (label, ask) = storage::ask(&state.launcher, item);
            let delete = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(label.trim_end_matches('…'))
                .valign(gtk::Align::Center)
                .sensitive(!win.busy() && !state.running)
                .build();
            delete.add_css_class("flat");
            delete.connect_clicked(glib::clone!(
                #[strong]
                win,
                move |_| storage::confirm(&win, &ask)
            ));
            row.add_suffix(&delete);
            group.add(&row);
        }
        page.add(&group);
    }
    if let Some(group) = web_group(win, game) {
        page.add(&group);
    }
    let about = adw::PreferencesGroup::new();
    about.add(
        &adw::ActionRow::builder()
            .title(match game.source {
                "steam" | "shortcut" => tr("Steam App ID", &[]),
                _ => tr("Heroic App Name", &[]),
            })
            .subtitle(glib::markup_escape_text(&game.id))
            .subtitle_selectable(true)
            .build(),
    );
    page.add(&about);

    // The header bar shows the back button, as the page isn't the first.
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    adw::NavigationPage::builder()
        .title(&game.name)
        .child(&view)
        .build()
}

/// ProtonDB's rating and the anti-cheat status, where either is known.
fn ratings(state: &LauncherState, game: &Game) -> Option<adw::PreferencesGroup> {
    let info = state.info.iter().find(|(id, _)| *id == game.id)?;
    let info = &info.1;
    if info.protondb.is_none() && info.anticheat.is_none() {
        return None;
    }
    let group = adw::PreferencesGroup::builder()
        .title(tr("On Linux", &[]))
        .build();
    if let Some(r) = &info.protondb {
        let row = adw::ActionRow::builder()
            .title(tr("ProtonDB", &[]))
            .subtitle(trn(
                "{tier}, from {n} report",
                "{tier}, from {n} reports",
                r.total as usize,
                &[("tier", &tier_name(&r.tier))],
            ))
            .build();
        let dot = gtk::Box::builder()
            .width_request(12)
            .height_request(12)
            .valign(gtk::Align::Center)
            .build();
        dot.add_css_class("dot");
        dot.add_css_class(&format!("tier-{}", r.tier));
        row.add_prefix(&dot);
        group.add(&row);
    }
    if let Some(a) = &info.anticheat {
        let row = adw::ActionRow::builder()
            .title(tr("Anti-cheat", &[]))
            .subtitle(glib::markup_escape_text(&format!(
                "{}: {}",
                a.status,
                a.anticheats.join(", ")
            )))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("security-high-symbolic"));
        group.add(&row);
    }
    Some(group)
}

/// The tool it runs with, to change, as its row's list does.
fn tool_group(
    win: &Rc<Window>,
    state: &LauncherState,
    lib: &Library,
    game: &Game,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Compatibility Tool", &[]))
        .build();
    let (options, labels, selected) = options(state.launcher.kind, lib, game);
    let row = adw::ComboRow::builder()
        .title(tr("Runs With", &[]))
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .sensitive(!state.running)
        .build();
    if let Some(i) = selected {
        row.set_selected(i as u32);
    }
    if state.running {
        row.set_subtitle(&tr(
            "Quit {launcher} to change it",
            &[("launcher", &state.launcher.title())],
        ));
    }
    let game = game.clone();
    row.connect_selected_notify(glib::clone!(
        #[strong]
        win,
        #[strong(rename_to = launcher)]
        state.launcher,
        move |r| {
            if let Some(choice) = options.get(r.selected() as usize) {
                win.assign(&launcher, &game, choice.as_ref());
            }
        }
    ));
    group.add(&row);
    group
}

/// The game's launch options as text, and a switch per preset that edits
/// it; Apply saves it. None for a launcher ProtonHub can't read them from.
fn launch_group(
    win: &Rc<Window>,
    state: &LauncherState,
    game: &Game,
) -> Option<adw::PreferencesGroup> {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Launch Options", &[]))
        .build();
    let all = match &state.launch {
        Ok(Some(all)) => all,
        Ok(None) => return None,
        Err(e) => {
            group.add(&errors::row(&tr("Can't read the launch options", &[]), e));
            return Some(group);
        }
    };
    if game.source == "shortcut" {
        let row = adw::ActionRow::builder()
            .title(tr(
                "Set a non-Steam game's launch options in its properties in Steam",
                &[],
            ))
            .build();
        row.add_css_class("dim-label");
        group.add(&row);
        return Some(group);
    }
    group.set_description(Some(&tr(
        "The switches edit the text; save it to change the game. Not every Proton build reads every option.",
        &[],
    )));
    let running = state.running;
    let current = all
        .iter()
        .find(|(id, _)| *id == game.id)
        .map_or("", |(_, o)| o.as_str());
    let entry = adw::EntryRow::builder()
        .title(tr("Launch options", &[]))
        .text(current)
        .sensitive(!running)
        .build();
    entry.add_css_class("monospace");
    // Not the row's own apply button: it shows only after typing, not
    // after a switch changed the text.
    let save = gtk::Button::builder()
        .label(tr("Save", &[]))
        .valign(gtk::Align::Center)
        .sensitive(false)
        .build();
    save.add_css_class("suggested-action");
    entry.add_suffix(&save);
    let apply = glib::clone!(
        #[strong]
        win,
        #[strong(rename_to = launcher)]
        state.launcher,
        #[strong]
        game,
        #[weak]
        entry,
        move || win.set_launch(&launcher, &game, &entry.text())
    );
    save.connect_clicked(glib::clone!(
        #[strong]
        apply,
        move |_| apply()
    ));
    entry.connect_entry_activated(glib::clone!(
        #[weak]
        save,
        move |_| {
            if save.is_sensitive() {
                apply();
            }
        }
    ));
    let saved = current.to_owned();
    entry.connect_changed(glib::clone!(
        #[weak]
        save,
        move |e| save.set_sensitive(!running && e.text() != saved)
    ));
    group.add(&entry);

    let mut switches = Vec::new();
    for preset in launch::presets() {
        // Inside a Flatpak, the host's programs aren't on PATH to look for.
        let missing = preset.wrapper.as_deref().filter(|w| {
            protonhub_core::games::sees_launchers() && !protonhub_core::install::on_path(w)
        });
        let subtitle = match missing {
            Some(w) => tr(
                "{program} isn't installed: the game won't start with it",
                &[("program", w)],
            ),
            None => preset.description.clone(),
        };
        let row = adw::SwitchRow::builder()
            .title(glib::markup_escape_text(&preset.name))
            .subtitle(glib::markup_escape_text(&subtitle))
            .build();
        group.add(&row);
        switches.push((preset, row));
    }
    // The switches show what the text sets, and change it; while they're
    // set to match it, changing them changes nothing.
    let switches = Rc::new(switches);
    let syncing = Rc::new(Cell::new(false));
    let sync = glib::clone!(
        #[strong]
        switches,
        #[strong]
        syncing,
        move |text: &str| {
            let options = Options::parse(text);
            syncing.set(true);
            for (preset, row) in switches.iter() {
                row.set_sensitive(!running && options.is_some());
                row.set_active(options.as_ref().is_some_and(|o| o.has(preset)));
            }
            syncing.set(false);
        }
    );
    sync(current);
    entry.connect_changed(move |e| sync(&e.text()));
    for (preset, row) in switches.iter() {
        row.connect_active_notify(glib::clone!(
            #[weak]
            entry,
            #[strong]
            syncing,
            #[strong]
            preset,
            move |row| {
                if syncing.get() {
                    return;
                }
                if let Some(mut options) = Options::parse(&entry.text()) {
                    options.set(&preset, row.is_active());
                    entry.set_text(&options.to_string());
                }
            }
        ));
    }
    Some(group)
}

/// A Steam game's pages on ProtonDB, SteamDB and the store; none for other
/// games, which those sites don't know.
fn web_group(win: &Rc<Window>, game: &Game) -> Option<adw::PreferencesGroup> {
    if game.source != "steam" {
        return None;
    }
    let id = &game.id;
    let group = adw::PreferencesGroup::builder()
        .title(tr("On the Web", &[]))
        .build();
    for (title, uri) in [
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
    ] {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(&uri)
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("web-browser-symbolic"));
        row.connect_activated(glib::clone!(
            #[strong]
            win,
            move |_| {
                gtk::UriLauncher::new(&uri).launch(
                    Some(&win.window),
                    None::<&gio::Cancellable>,
                    |_| {},
                );
            }
        ));
        group.add(&row);
    }
    Some(group)
}
