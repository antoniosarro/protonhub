//! A game's page, pushed over its launcher's games with a back button: its
//! picture and ratings, the tool it runs with, its prefix and shader cache,
//! and its pages on the web.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::games::{Game, Library};
use protonhub_core::install::human_size;
use protonhub_core::launch::{self, Options};
use protonhub_core::launcher::Launcher;
use protonhub_core::mangohud;

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
    if let Some(group) = super::prefix::group(win, state, game) {
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
    // The text is what's saved; the parts above it show it and change it.
    let text = adw::EntryRow::builder()
        .title(tr("As text", &[]))
        .text(current)
        .sensitive(!running)
        .build();
    let expander = adw::ExpanderRow::builder()
        .title(tr("Launch options", &[]))
        .subtitle_lines(1)
        .build();
    expander.add_css_class("launch-options");
    // Not the text row's own apply button: it shows only after typing
    // there, not after a part or a switch changed the text.
    let save = gtk::Button::builder()
        .label(tr("Save", &[]))
        .valign(gtk::Align::Center)
        .sensitive(false)
        .build();
    save.add_css_class("suggested-action");
    expander.add_suffix(&save);
    let apply = glib::clone!(
        #[strong]
        win,
        #[strong(rename_to = launcher)]
        state.launcher,
        #[strong]
        game,
        #[weak]
        text,
        move || win.set_launch(&launcher, &game, &text.text())
    );
    save.connect_clicked(glib::clone!(
        #[strong]
        apply,
        move |_| apply()
    ));
    let on_enter: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[weak]
        save,
        move || {
            if save.is_sensitive() {
                apply();
            }
        }
    ));

    // A row per variable, its name and value side by side; then the
    // programs it runs in and its arguments, each as words.
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let add = gtk::Button::builder()
        .label(tr("Add Variable", &[]))
        .halign(gtk::Align::Start)
        .build();
    add.add_css_class("flat");
    let vars = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(9)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    vars.append(&lines);
    vars.append(&add);
    let vars_row = gtk::ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .child(&vars)
        .build();
    let programs = adw::EntryRow::builder()
        .title(tr("Programs it runs in", &[]))
        .build();
    let args = adw::EntryRow::builder().title(tr("Arguments", &[])).build();
    for row in [&programs, &args, &text] {
        row.connect_entry_activated(glib::clone!(
            #[strong]
            on_enter,
            move |_| on_enter()
        ));
    }
    expander.add_row(&vars_row);
    expander.add_row(&programs);
    expander.add_row(&args);
    expander.add_row(&text);

    // Set while the parts are written to the text or read from it, so
    // neither writes back.
    let editing = Rc::new(Cell::new(false));
    let pairs: Rc<RefCell<Vec<(gtk::Box, gtk::Entry, gtk::Entry)>>> = Rc::default();
    let compose: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[strong]
        editing,
        #[strong]
        pairs,
        #[weak]
        text,
        #[weak]
        programs,
        #[weak]
        args,
        move || {
            if editing.get() {
                return;
            }
            let Some(mut options) = Options::parse(&text.text()) else {
                return;
            };
            // A pair without a name yet is left out, one with a name no
            // shell would set is marked.
            options.env = pairs
                .borrow()
                .iter()
                .filter_map(|(_, key, value)| {
                    let name = key.text();
                    let valid = name.is_empty() || launch::is_name(&name);
                    mark(key, !valid);
                    (valid && !name.is_empty()).then(|| (name.into(), value.text().into()))
                })
                .collect();
            for (row, words) in [
                (&programs, &mut options.wrappers),
                (&args, &mut options.args),
            ] {
                let split = launch::split(&row.text());
                mark(row, split.is_none());
                match split {
                    Some(split) => *words = split,
                    None => return,
                }
            }
            editing.set(true);
            text.set_text(&options.to_string());
            editing.set(false);
        }
    ));
    // Outside the macro below, where xgettext doesn't look.
    let hints = [tr("Name", &[]), tr("Value", &[]), tr("Remove", &[])];
    let add_pair: Rc<dyn Fn(&str, &str) -> gtk::Entry> = Rc::new(glib::clone!(
        #[strong]
        pairs,
        #[strong]
        compose,
        #[strong]
        on_enter,
        #[weak]
        lines,
        #[upgrade_or_panic]
        move |name: &str, value: &str| {
            let [name_hint, value_hint, remove_hint] = &hints;
            let key = gtk::Entry::builder()
                .text(name)
                .placeholder_text(name_hint.as_str())
                .hexpand(true)
                .build();
            let val = gtk::Entry::builder()
                .text(value)
                .placeholder_text(value_hint.as_str())
                .hexpand(true)
                .build();
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(remove_hint.as_str())
                .build();
            remove.add_css_class("flat");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.append(&key);
            line.append(&gtk::Label::new(Some("=")));
            line.append(&val);
            line.append(&remove);
            for entry in [&key, &val] {
                entry.connect_changed(glib::clone!(
                    #[strong]
                    compose,
                    move |_| compose()
                ));
                entry.connect_activate(glib::clone!(
                    #[strong]
                    on_enter,
                    move |_| on_enter()
                ));
            }
            remove.connect_clicked(glib::clone!(
                #[strong]
                pairs,
                #[strong]
                compose,
                #[weak]
                lines,
                #[weak]
                line,
                move |_| {
                    lines.remove(&line);
                    pairs.borrow_mut().retain(|(l, _, _)| *l != line);
                    compose();
                }
            ));
            lines.append(&line);
            pairs.borrow_mut().push((line, key.clone(), val));
            key
        }
    ));
    add.connect_clicked(glib::clone!(
        #[strong]
        add_pair,
        move |_| {
            add_pair("", "").grab_focus();
        }
    ));
    for row in [&programs, &args] {
        row.connect_changed(glib::clone!(
            #[strong]
            compose,
            move |_| compose()
        ));
    }
    // The parts, the summary and Save follow the text, however it changed.
    let none = tr("None", &[]);
    let saved = current.to_owned();
    let refresh = glib::clone!(
        #[strong]
        editing,
        #[strong]
        pairs,
        #[weak]
        lines,
        #[weak]
        vars_row,
        #[weak]
        programs,
        #[weak]
        args,
        #[weak]
        expander,
        #[weak]
        save,
        move |t: &str| {
            expander.set_subtitle(&glib::markup_escape_text(if t.is_empty() {
                &none
            } else {
                t
            }));
            save.set_sensitive(!running && t != saved);
            if editing.get() {
                return;
            }
            let options = Options::parse(t);
            for row in [
                vars_row.upcast_ref::<gtk::Widget>(),
                programs.upcast_ref(),
                args.upcast_ref(),
            ] {
                row.set_sensitive(!running && options.is_some());
            }
            editing.set(true);
            for (line, _, _) in pairs.take() {
                lines.remove(&line);
            }
            let o = options.unwrap_or_default();
            for (name, value) in &o.env {
                add_pair(name, value);
            }
            programs.set_text(&launch::join(&o.wrappers));
            args.set_text(&launch::join(&o.args));
            mark(&programs, false);
            mark(&args, false);
            editing.set(false);
        }
    );
    refresh(current);
    text.connect_changed(move |t| {
        mark(t, Options::parse(&t.text()).is_none());
        refresh(&t.text());
    });
    group.add(&expander);

    // The game's own MangoHud settings are a switch too, after MangoHud's.
    let own = mangohud::game_path(&state.launcher, &game.id).map(|path| launch::Preset {
        id: "mangohud-config".into(),
        name: tr("Own MangoHud Settings", &[]),
        description: tr(
            "This game's MangoHud settings instead of the global ones",
            &[],
        ),
        env: [(
            mangohud::CONFIG_VAR.into(),
            path.to_string_lossy().into_owned(),
        )]
        .into(),
        wrapper: None,
    });
    let mut presets = launch::presets();
    if let Some(own) = &own {
        let at = presets
            .iter()
            .position(|p| p.id == "mangohud")
            .map_or(0, |i| i + 1);
        presets.insert(at, own.clone());
    }
    let mut switches = Vec::new();
    for preset in presets {
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
    if let Some((path, row)) = own.as_ref().and_then(|own| {
        let path = mangohud::game_path(&state.launcher, &game.id)?;
        let (_, row) = switches.iter().find(|(p, _)| p.id == own.id)?;
        Some((path, row.clone()))
    }) {
        own_settings(win, game, path, &row);
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
    text.connect_changed(move |e| sync(&e.text()));
    for (preset, row) in switches.iter() {
        row.connect_active_notify(glib::clone!(
            #[weak]
            text,
            #[strong]
            syncing,
            #[strong]
            preset,
            move |row| {
                if syncing.get() {
                    return;
                }
                if let Some(mut options) = Options::parse(&text.text()) {
                    options.set(&preset, row.is_active());
                    // Setting the text empties it first, which switches
                    // this off and on again in the sync and notifies once
                    // more: unchanged, it stops there.
                    let new = options.to_string();
                    if new != text.text() {
                        text.set_text(&new);
                    }
                }
            }
        ));
    }
    Some(group)
}

/// An Edit button on the switch for the game's own MangoHud settings, in
/// `path`; switching them on starts them as a copy of the global ones.
fn own_settings(win: &Rc<Window>, game: &Game, path: std::path::PathBuf, row: &adw::SwitchRow) {
    let edit = gtk::Button::builder()
        .icon_name("document-edit-symbolic")
        .tooltip_text(tr("Edit", &[]))
        .valign(gtk::Align::Center)
        .build();
    edit.add_css_class("flat");
    row.add_suffix(&edit);
    let global = mangohud::global_path();
    // Outside the macros below, where xgettext doesn't look.
    let title = tr("MangoHud for {game}", &[("game", &game.name)]);
    let writing = tr("Writing {file}", &[("file", &path.display().to_string())]);
    edit.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        path,
        #[strong]
        global,
        move |_| crate::mangohud::show(&win, &title, path.clone(), global.clone())
    ));
    row.connect_active_notify(glib::clone!(
        #[strong]
        win,
        move |row| {
            if !row.is_active() || path.exists() {
                return;
            }
            let copy = global
                .as_deref()
                .map_or(Ok(Default::default()), mangohud::read)
                .and_then(|c| mangohud::write(&path, &c));
            if let Err(e) = copy {
                win.report(writing.clone(), e.to_string());
            }
        }
    ));
}

/// Marks `widget` as holding text that can't be used.
fn mark(widget: &impl IsA<gtk::Widget>, error: bool) {
    if error {
        widget.add_css_class("error");
    } else {
        widget.remove_css_class("error");
    }
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
