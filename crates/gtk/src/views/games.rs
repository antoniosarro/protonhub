//! The games tab: a search, the launcher's default tool and what changes
//! many games at once in a bar, then a card per game.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Library;
use protonhub_core::manifest::LauncherKind;

use super::{game_card, key, opens};
use crate::i18n::tr;
use crate::pages::{self, Route};
use crate::window::{Env, LauncherState, Window};
use crate::{choices, errors, ui};

pub(super) fn view(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    lib: &Library,
) -> gtk::Widget {
    let launcher = &state.launcher;
    if lib.games.is_empty() && lib.problems.is_empty() {
        return ui::status(
            "applications-games-symbolic",
            &tr("No games installed", &[]),
            &tr(
                "Games installed with {launcher} show up here",
                &[("launcher", &launcher.title())],
            ),
        );
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search games", &[]))
        .hexpand(true)
        .build();
    let bar = gtk::Box::builder().spacing(12).build();
    bar.append(&search);
    bar.append(&actions(win, state, lib));
    content.append(&bar);

    // Not errors of ProtonHub's: a source was down, or a file was broken.
    let mut notices: Vec<gtk::Widget> = Vec::new();
    if let Some(e) = &state.info_error {
        let row = adw::ActionRow::builder()
            .title(tr("Some ratings or anti-cheat statuses are missing", &[]))
            .subtitle(glib::markup_escape_text(e))
            .subtitle_lines(1)
            .tooltip_text(e)
            .build();
        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        icon.add_css_class("warning");
        row.add_prefix(&icon);
        notices.push(row.upcast());
    }
    for problem in &lib.problems {
        notices.push(errors::row(&tr("Left out: a file couldn't be read", &[]), problem).upcast());
    }
    if !notices.is_empty() {
        content.append(&ui::boxed(&notices));
    }

    let grid = ui::Grid::new();
    for game in &lib.games {
        let open = opens(
            win,
            Route::Game {
                launcher: launcher.clone(),
                id: game.id.clone(),
            },
        );
        let child = grid.add(&game_card::card(win, env, state, lib, game), open);
        child.set_widget_name(&game.name.to_lowercase());
    }
    let none = ui::status("system-search-symbolic", &tr("No games match", &[]), "");
    none.set_visible(false);
    let names: Vec<String> = lib.games.iter().map(|g| g.name.to_lowercase()).collect();
    grid.flow.set_filter_func(glib::clone!(
        #[weak]
        search,
        #[upgrade_or]
        true,
        move |child| {
            let query = search.text().to_lowercase();
            child.widget_name().contains(query.trim())
        }
    ));
    let searches = key(state, "search");
    search.connect_search_changed(glib::clone!(
        #[weak]
        win,
        #[weak(rename_to = flow)]
        grid.flow,
        #[weak]
        none,
        move |search| {
            let query = search.text().to_lowercase();
            let query = query.trim();
            flow.invalidate_filter();
            none.set_visible(!names.iter().any(|n| n.contains(query)));
            win.searches
                .borrow_mut()
                .insert(searches.clone(), query.to_owned());
        }
    ));
    if let Some(query) = win.searches.borrow().get(&key(state, "search")).cloned() {
        search.set_text(&query);
    }
    content.append(&grid.flow);
    content.append(&none);
    ui::scrolled(win, key(state, "games"), &content, 1600, &[(&bar, 720)])
}

/// The bar's end: the default tool, and what changes many games at once,
/// each on a page of its own.
fn actions(win: &Rc<Window>, state: &LauncherState, lib: &Library) -> gtk::Box {
    let launcher = &state.launcher;
    let running = state.running;
    let actions = gtk::Box::builder().spacing(12).build();
    if matches!(launcher.kind, LauncherKind::Steam | LauncherKind::Faugus) {
        let options = choices::for_default(launcher.kind, lib);
        let label = ui::line(&tr("Default Tool", &[]), &["dim-label"]);
        let drop = ui::drop_down(&options.labels, options.selected, 16);
        drop.set_sensitive(!running);
        drop.set_tooltip_text(Some(&tr(
            "What games without a tool of their own run with",
            &[],
        )));
        let current = options.selected;
        drop.connect_selected_notify(glib::clone!(
            #[weak]
            win,
            #[strong]
            launcher,
            move |d| {
                let at = d.selected() as usize;
                if Some(at) != current {
                    win.set_default(&launcher, options.choices[at].as_ref());
                }
            }
        ));
        let default = gtk::Box::builder().spacing(8).build();
        default.append(&label);
        default.append(&drop);
        actions.append(&default);
    }
    let move_games = ui::button("media-playlist-shuffle-symbolic", &tr("Move Games…", &[]));
    move_games.set_tooltip_text(Some(&tr("Move every game set to one tool to another", &[])));
    move_games.set_sensitive(!running && lib.games.iter().any(|g| g.explicit));
    move_games.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[strong]
        launcher,
        #[strong]
        lib,
        move |_| win.push(&pages::move_games::page(&win, &launcher, &lib))
    ));
    actions.append(&move_games);
    if let Ok(Some(current)) = &state.launch {
        let edit = ui::button("document-edit-symbolic", &tr("Launch Options…", &[]));
        edit.set_tooltip_text(Some(&tr(
            "Switch launch options on or off for several games at once",
            &[],
        )));
        edit.set_sensitive(!running);
        edit.connect_clicked(glib::clone!(
            #[weak]
            win,
            #[strong]
            launcher,
            #[strong]
            lib,
            #[strong]
            current,
            move |_| win.push(&pages::bulk_launch::page(&win, &launcher, &lib, &current))
        ));
        actions.append(&edit);
    }
    actions
}
