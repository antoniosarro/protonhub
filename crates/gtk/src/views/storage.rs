//! The storage tab of a Steam: what games no longer installed left
//! behind, then a card per game with its Wine prefix and shader cache,
//! largest first.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::install::human_size;
use protonhub_core::storage::{Item, Storage};

use super::{key, opens};
use crate::i18n::{tr, trn};
use crate::pages::{self, Route};
use crate::text::{app_name, kind_name};
use crate::window::{Env, LauncherState, Window};
use crate::{errors, ui};

pub(super) fn view(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    storage: &Storage,
) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 32);
    let mut notices: Vec<gtk::Widget> = Vec::new();
    for problem in &storage.problems {
        notices.push(
            errors::row(
                &tr("Nothing is marked left over: a file couldn't be read", &[]),
                problem,
            )
            .upcast(),
        );
    }
    if let Some(e) = &storage.names_error {
        notices.push(
            errors::row(
                &tr("Games no longer installed are shown without names", &[]),
                e,
            )
            .upcast(),
        );
    }
    if !notices.is_empty() {
        content.append(&ui::boxed(&notices));
    }
    if let Some(strip) = leftovers(win, state, storage) {
        content.append(&strip);
    }
    content.append(&games(win, env, state, storage));
    ui::scrolled(win, key(state, "storage"), &content, 1600, &[])
}

/// How much uninstalled games left takes, and a button to the page that
/// lists and deletes it.
fn leftovers(win: &Rc<Window>, state: &LauncherState, storage: &Storage) -> Option<gtk::Widget> {
    let orphaned: Vec<_> = storage.orphaned().collect();
    if orphaned.is_empty() {
        return None;
    }
    let total: u64 = orphaned.iter().map(|i| i.size).sum();
    let row = adw::ActionRow::builder()
        .title(trn(
            "{n} folder of uninstalled games takes {size}",
            "{n} folders of uninstalled games take {size}",
            orphaned.len(),
            &[("size", &human_size(total))],
        ))
        .subtitle(tr(
            "Saves of games without Steam Cloud are in their prefixes",
            &[],
        ))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("user-trash-symbolic"));
    let review = gtk::Button::builder()
        .label(tr("Clean Up…", &[]))
        .valign(gtk::Align::Center)
        .sensitive(!win.busy() && !state.running)
        .build();
    review.add_css_class("destructive-action");
    if state.running {
        review.set_tooltip_text(Some(&tr("Quit Steam first", &[])));
    }
    review.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[strong(rename_to = launcher)]
        state.launcher,
        move |_| win.open(Route::Leftovers {
            launcher: launcher.clone()
        })
    ));
    row.add_suffix(&review);
    Some(ui::boxed(&[row.upcast()]).upcast())
}

/// A card per installed app: its prefix and shader cache, with a menu to
/// delete either; one of the launcher's games opens its page.
fn games(win: &Rc<Window>, env: &Env, state: &LauncherState, storage: &Storage) -> gtk::Box {
    let title = tr("Games", &[]);
    // Grouped by app, in the order of their largest item.
    let mut apps: Vec<(&str, Vec<&Item>)> = Vec::new();
    for item in storage.items.iter().filter(|i| !i.orphaned) {
        match apps.iter_mut().find(|(app, _)| *app == item.app) {
            Some((_, items)) => items.push(item),
            None => apps.push((&item.app, vec![item])),
        }
    }
    if apps.is_empty() {
        let none = ui::dim_row(
            &tr("No prefixes or shader caches", &[]),
            &tr("Steam makes them when a game first runs", &[]),
        );
        return ui::section(&title, None, None, &ui::boxed(&[none.upcast()]));
    }
    apps.sort_by_key(|(_, items)| std::cmp::Reverse(items.iter().map(|i| i.size).sum::<u64>()));
    let total: u64 = apps.iter().flat_map(|(_, i)| i).map(|i| i.size).sum();
    let grid = ui::Grid::new();
    for (app, items) in apps {
        let game = state
            .library()
            .and_then(|l| l.games.iter().find(|g| g.id == app));
        let art = ui::art(env, game.and_then(|g| g.art.as_deref()), 192, "thumbnail");
        art.set_valign(gtk::Align::Center);
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(3)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        let name = ui::line(&app_name(items[0]), &["heading"]);
        name.set_max_width_chars(14);
        text.append(&name);
        for item in &items {
            let size = ui::line(
                &format!("{} · {}", kind_name(item.kind), human_size(item.size)),
                &["caption", "dim-label"],
            );
            text.append(&size);
        }
        let card = ui::card(gtk::Orientation::Horizontal, 300);
        card.add_css_class("tile");
        card.set_spacing(12);
        card.append(&art);
        card.append(&text);
        card.append(&menu(win, state, &items));
        let open = game.and_then(|g| {
            opens(
                win,
                Route::Game {
                    launcher: state.launcher.clone(),
                    id: g.id.clone(),
                },
            )
        });
        grid.add(&card, open);
    }
    ui::section(
        &title,
        Some(&tr("{size} in all", &[("size", &human_size(total))])),
        None,
        &grid.flow,
    )
}

/// A menu to delete one of an app's prefix and shader cache.
fn menu(win: &Rc<Window>, state: &LauncherState, items: &[&Item]) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    let mut asks = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let ask = pages::game::Deletion::of(&state.launcher, item);
        let entry = gio::MenuItem::new(Some(&ask.label), None);
        entry.set_action_and_target_value(Some("storage.delete"), Some(&(i as u32).to_variant()));
        menu.append_item(&entry);
        asks.push(ask);
    }
    let delete = gio::SimpleAction::new("delete", Some(glib::VariantTy::UINT32));
    delete.connect_activate(glib::clone!(
        #[weak]
        win,
        move |_, i| {
            if let Some(ask) = i
                .and_then(|i| i.get::<u32>())
                .and_then(|i| asks.get(i as usize))
            {
                ask.confirm(&win);
            }
        }
    ));
    let actions = gio::SimpleActionGroup::new();
    actions.add_action(&delete);
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .tooltip_text(tr("Free space", &[]))
        .menu_model(&menu)
        .valign(gtk::Align::Center)
        .sensitive(!win.busy() && !state.running)
        .build();
    button.add_css_class("flat");
    button.insert_action_group("storage", Some(&actions));
    button
}
