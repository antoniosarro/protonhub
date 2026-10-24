//! The storage tab of a Steam: each game's Wine prefix and shader cache,
//! largest first, and what games no longer installed left behind.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::install::human_size;
use protonhub_core::launcher::Launcher;
use protonhub_core::storage::{Item, Kind, Storage};

use super::game_row::thumbnail;
use super::{boxed, scrolled};
use crate::errors;
use crate::i18n::{tr, trn};
use crate::window::{Env, LauncherState, Window};

pub(super) fn view(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    storage: &Storage,
) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    for problem in &storage.problems {
        content.append(&boxed(&errors::row(
            &tr("Nothing is marked left over: a file couldn't be read", &[]),
            problem,
        )));
    }
    if let Some(e) = &storage.names_error {
        content.append(&boxed(&errors::row(
            &tr("Games no longer installed are shown without names", &[]),
            e,
        )));
    }
    if let Some(group) = leftover_group(win, state, storage) {
        content.append(&group);
    }
    content.append(&games_group(win, env, state, storage));
    scrolled(&content)
}

/// What uninstalled games left, in a row that opens to list it, with a
/// button to delete it all.
fn leftover_group(
    win: &Rc<Window>,
    state: &LauncherState,
    storage: &Storage,
) -> Option<adw::PreferencesGroup> {
    let orphaned: Vec<_> = storage.orphaned().collect();
    if orphaned.is_empty() {
        return None;
    }
    let total: u64 = orphaned.iter().map(|i| i.size).sum();
    let row = adw::ExpanderRow::builder()
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
    for item in &orphaned {
        // Why it can go: what it was for isn't there any more.
        let why = if item.shortcut() {
            tr("Removed from Steam", &[])
        } else {
            tr("No longer installed", &[])
        };
        let sub = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&app_name(item)))
            .subtitle(format!(
                "{why} · {} · {}",
                kind_name(item.kind),
                human_size(item.size)
            ))
            .tooltip_text(item.dir.display().to_string())
            .build();
        row.add_row(&sub);
    }
    let clean = delete_button(win, state, &tr("Clean Up…", &[]));
    // Built out here: xgettext doesn't look for strings inside macros.
    let ask = Confirm {
        heading: trn(
            "Delete {n} leftover folder?",
            "Delete {n} leftover folders?",
            orphaned.len(),
            &[],
        ),
        body: tr(
            "Their games are no longer installed. Saves not in Steam Cloud are lost with their prefixes.",
            &[],
        ),
        launcher: state.launcher.clone(),
        dirs: orphaned.iter().map(|i| i.dir.clone()).collect(),
        size: total,
    };
    clean.connect_clicked(glib::clone!(
        #[strong]
        win,
        move |_| confirm(&win, &ask)
    ));
    row.add_suffix(&clean);
    let group = adw::PreferencesGroup::new();
    group.add(&row);
    Some(group)
}

/// Each installed app's prefix and shader cache, the largest first, with a
/// menu to delete either.
fn games_group(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    storage: &Storage,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Games", &[]))
        .build();
    // Grouped by app, in the order of their largest item.
    let mut apps: Vec<(&str, Vec<&Item>)> = Vec::new();
    for item in storage.items.iter().filter(|i| !i.orphaned) {
        match apps.iter_mut().find(|(app, _)| *app == item.app) {
            Some((_, items)) => items.push(item),
            None => apps.push((&item.app, vec![item])),
        }
    }
    if apps.is_empty() {
        let row = adw::ActionRow::builder()
            .title(tr("No prefixes or shader caches", &[]))
            .subtitle(tr("Steam makes them when a game first runs", &[]))
            .build();
        row.add_css_class("dim-label");
        group.add(&row);
        return group;
    }
    apps.sort_by_key(|(_, items)| std::cmp::Reverse(items.iter().map(|i| i.size).sum::<u64>()));
    let total: u64 = apps.iter().flat_map(|(_, i)| i).map(|i| i.size).sum();
    group.set_description(Some(&tr("{size} in all", &[("size", &human_size(total))])));
    let games = state.library.as_ref().ok().and_then(Option::as_ref);
    for (app, items) in apps {
        let subtitle: Vec<_> = items
            .iter()
            .map(|i| format!("{} {}", kind_name(i.kind), human_size(i.size)))
            .collect();
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&app_name(items[0])))
            .subtitle(subtitle.join(" · "))
            .build();
        let game = games.and_then(|l| l.games.iter().find(|g| g.id == app));
        row.add_prefix(&thumbnail(env, game.and_then(|g| g.art.as_deref()), 192));
        row.add_suffix(&menu(win, state, &items));
        group.add(&row);
    }
    group
}

/// A menu to delete one of an app's prefix and shader cache.
fn menu(win: &Rc<Window>, state: &LauncherState, items: &[&Item]) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    let mut asks = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let (label, ask) = ask(&state.launcher, item);
        let entry = gio::MenuItem::new(Some(&label), None);
        entry.set_action_and_target_value(Some("storage.delete"), Some(&(i as u32).to_variant()));
        menu.append_item(&entry);
        asks.push(ask);
    }
    let delete = gio::SimpleAction::new("delete", Some(glib::VariantTy::UINT32));
    delete.connect_activate(glib::clone!(
        #[strong]
        win,
        move |_, i| {
            if let Some(ask) = i
                .and_then(|i| i.get::<u32>())
                .and_then(|i| asks.get(i as usize))
            {
                confirm(&win, ask);
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

/// A destructive button, which can't be used while Steam runs.
fn delete_button(win: &Rc<Window>, state: &LauncherState, label: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .sensitive(!win.busy() && !state.running)
        .build();
    if state.running {
        button.set_tooltip_text(Some(&tr("Quit Steam first", &[])));
    }
    button.add_css_class("destructive-action");
    button
}

/// How deleting one of an app's prefix and shader cache is offered: a
/// button's label, and the confirmation.
pub(super) fn ask(launcher: &Launcher, item: &Item) -> (String, Confirm) {
    let name = app_name(item);
    let (label, heading, body) = match item.kind {
        Kind::Prefix => (
            tr("Reset Prefix…", &[]),
            tr("Reset the prefix of {game}?", &[("game", &name)]),
            tr(
                "It starts with a new prefix the next time it runs. Saves and settings kept in it, not in Steam Cloud, are lost.",
                &[],
            ),
        ),
        Kind::ShaderCache => (
            tr("Delete Shader Cache…", &[]),
            tr("Delete the shader cache of {game}?", &[("game", &name)]),
            tr(
                "Steam builds it again; until it does, the game may stutter.",
                &[],
            ),
        ),
    };
    let ask = Confirm {
        heading,
        body,
        launcher: launcher.clone(),
        dirs: vec![item.dir.clone()],
        size: item.size,
    };
    (label, ask)
}

/// A deletion to confirm: what the dialog says, and what it deletes.
pub(super) struct Confirm {
    heading: String,
    body: String,
    launcher: Launcher,
    dirs: Vec<PathBuf>,
    size: u64,
}

pub(super) fn confirm(win: &Rc<Window>, ask: &Confirm) {
    let dialog = adw::AlertDialog::new(
        Some(&ask.heading),
        Some(&format!(
            "{}\n\n{}",
            ask.body,
            tr("This frees {size}.", &[("size", &human_size(ask.size))])
        )),
    );
    dialog.add_responses(&[
        ("cancel", &tr("Cancel", &[])),
        ("delete", &tr("Delete", &[])),
    ]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.connect_response(
        Some("delete"),
        glib::clone!(
            #[strong]
            win,
            #[strong(rename_to = launcher)]
            ask.launcher,
            #[strong(rename_to = dirs)]
            ask.dirs,
            #[strong(rename_to = size)]
            ask.size,
            move |_, _| win.delete_storage(&launcher, dirs.clone(), size)
        ),
    );
    dialog.present(Some(&win.window));
}

/// The app's name, or where Steam doesn't know it, what it is.
fn app_name(item: &Item) -> String {
    match &item.name {
        Some(name) => name.clone(),
        None if item.shortcut() => tr("Non-Steam game", &[]),
        None => tr("Steam app {id}", &[("id", &item.app)]),
    }
}

pub(super) fn kind_name(kind: Kind) -> String {
    match kind {
        Kind::Prefix => tr("Prefix", &[]),
        Kind::ShaderCache => tr("Shader cache", &[]),
    }
}
