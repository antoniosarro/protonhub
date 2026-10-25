//! A switch per launch option preset on a game's page, the game's own
//! MangoHud settings and gamescope among them, each kept in step with the
//! text of its launch options.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Game;
use protonhub_core::launch::{self, Options};
use protonhub_core::mangohud;

use super::gamescope;
use crate::i18n::tr;
use crate::window::{LauncherState, Window};

/// Adds the switches to `group`; they show what `text`, the launch
/// options as text, has on, and change it. Enter in a field of theirs does
/// `on_enter`.
pub(super) fn add(
    win: &Rc<Window>,
    state: &LauncherState,
    game: &Game,
    group: &adw::PreferencesGroup,
    text: &adw::EntryRow,
    on_enter: &Rc<dyn Fn()>,
) {
    let running = state.running;
    let (text, on_enter) = (text.clone(), on_enter.clone());
    let current = text.text().to_string();
    let current = current.as_str();
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
        group: None,
    });
    let mut presets = launch::presets();
    if let Some(own) = &own {
        let at = presets
            .iter()
            .position(|p| p.id == "mangohud")
            .map_or(0, |i| i + 1);
        presets.insert(at, own.clone());
    }
    // Those with a heading are in a row of their own for it, after
    // gamescope.
    let mut switches = Vec::new();
    let mut groups: Vec<(String, adw::ExpanderRow)> = Vec::new();
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
        match &preset.group {
            None => group.add(&row),
            Some(title) => {
                if !groups.iter().any(|(t, _)| t == title) {
                    let expander = adw::ExpanderRow::builder()
                        .title(glib::markup_escape_text(title))
                        .subtitle_lines(1)
                        .build();
                    groups.push((title.clone(), expander));
                }
                let (_, expander) = groups.iter().find(|(t, _)| t == title).unwrap();
                expander.add_row(&row);
            }
        }
        switches.push((preset, row));
    }
    group.add(&gamescope::row(&text, running, &on_enter));
    for (_, expander) in &groups {
        group.add(expander);
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
            // A heading's row names the switches it has on.
            for (title, expander) in &groups {
                let on: Vec<_> = switches
                    .iter()
                    .filter(|(p, row)| p.group.as_ref() == Some(title) && row.is_active())
                    .map(|(p, _)| p.name.as_str())
                    .collect();
                expander.set_subtitle(&glib::markup_escape_text(&on.join(", ")));
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
        move |_| win.push(&crate::pages::mangohud::page(
            &win,
            &title,
            path.clone(),
            global.clone()
        ))
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
