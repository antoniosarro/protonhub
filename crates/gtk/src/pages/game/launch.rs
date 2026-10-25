//! A game's launch options on its page: the text, the parts it's made of
//! to edit one by one ([`parts`]), and a switch per preset ([`presets`]);
//! Save writes the text.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Game;

use super::{parts, presets};
use crate::errors;
use crate::i18n::tr;
use crate::window::{LauncherState, Window};

/// The game's launch options as text, and a switch per preset that edits
/// it; Apply saves it. None for a launcher ProtonHub can't read them from.
pub(super) fn group(
    win: &Rc<Window>,
    state: &LauncherState,
    game: &Game,
) -> Option<adw::PreferencesGroup> {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Launch Options", &[]))
        .build();
    match &state.launch {
        Ok(Some(_)) => {}
        Ok(None) => return None,
        Err(e) => {
            group.add(&errors::row(&tr("Can't read the launch options", &[]), e));
            return Some(group);
        }
    }
    group.set_description(Some(&tr(
        "The switches edit the text; save it to change the game. Not every Proton build reads every option.",
        &[],
    )));
    let running = state.running;
    let current = state.launch_options(&game.id);
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

    parts::add(&expander, &text, &save, &on_enter, running);
    group.add(&expander);
    presets::add(win, state, game, &group, &text, &on_enter);
    Some(group)
}
