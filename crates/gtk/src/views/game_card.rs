//! A game's card: its picture with its ratings over it, its name, where
//! it's from, and the list of tools it can run with.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::{Game, Library};
use protonhub_core::info::Info;

use crate::i18n::{tr, trn};
use crate::text::{anticheat_class, deck_rating, deck_text, source, tier_name};
use crate::window::{Env, LauncherState, Window};
use crate::{choices, ui};

pub(super) fn card(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    lib: &Library,
    game: &Game,
) -> gtk::Box {
    let art = ui::art(env, game.art.as_deref(), 460, "art");
    if let Some(badge) = state.info(&game.id).and_then(protondb_badge) {
        art.add_overlay(&badge);
    }
    if let Some(badge) = marks(state.info(&game.id), game) {
        art.add_overlay(&badge);
    }

    let name = ui::line(&game.name, &["heading"]);
    name.set_max_width_chars(16);
    name.set_tooltip_text(Some(&game.name));
    let from = ui::line(&source(game.source), &["caption", "dim-label"]);

    let options = choices::for_game(state.launcher.kind, lib, game);
    let tools = ui::drop_down(&options.labels, options.selected, 12);
    tools.set_sensitive(!state.running);
    tools.set_tooltip_text(Some(&tr("The tool it runs with", &[])));
    let current = options.selected;
    tools.connect_selected_notify(glib::clone!(
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

    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(10)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    body.append(&name);
    body.append(&from);
    tools.set_margin_top(6);
    body.append(&tools);

    let card = ui::card(gtk::Orientation::Vertical, 220);
    card.add_css_class("game-card");
    card.append(&art);
    card.append(&body);
    card
}

/// ProtonDB's rating, as a dot in its tier's color and its name.
fn protondb_badge(info: &Info) -> Option<gtk::Box> {
    let r = info.protondb.as_ref()?;
    let badge = gtk::Box::builder()
        .spacing(5)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_top(8)
        .margin_start(8)
        .build();
    badge.add_css_class("badge");
    let dot = gtk::Box::builder().valign(gtk::Align::Center).build();
    dot.add_css_class("dot");
    dot.add_css_class(&format!("tier-{}", r.tier));
    badge.append(&dot);
    badge.append(&gtk::Label::new(Some(&tier_name(&r.tier))));
    badge.set_tooltip_text(Some(&trn(
        "ProtonDB: {tier}, from {n} report",
        "ProtonDB: {tier}, from {n} reports",
        r.total as usize,
        &[("tier", &tier_name(&r.tier))],
    )));
    Some(badge)
}

/// Its anti-cheat's status and its Steam Deck rating, as icons colored by
/// how they fare.
fn marks(info: Option<&Info>, game: &Game) -> Option<gtk::Box> {
    let anticheat = info.and_then(|i| i.anticheat.as_ref());
    if anticheat.is_none() && game.deck.is_none() {
        return None;
    }
    let badge = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::End)
        .valign(gtk::Align::Start)
        .margin_top(8)
        .margin_end(8)
        .build();
    badge.add_css_class("badge");
    if let Some(a) = anticheat {
        let icon = gtk::Image::from_icon_name("security-high-symbolic");
        icon.add_css_class(anticheat_class(&a.status));
        icon.set_tooltip_text(Some(&format!(
            "{}: {}",
            tr(
                "Anti-cheat: {names}",
                &[("names", &a.anticheats.join(", "))]
            ),
            a.status
        )));
        badge.append(&icon);
    }
    if let Some(d) = &game.deck {
        let icon = gtk::Image::from_icon_name("input-gaming-symbolic");
        icon.add_css_class(deck_rating(d.rating).0);
        icon.set_tooltip_text(Some(&tr(
            "Steam Deck: {rating}",
            &[("rating", &deck_text(d))],
        )));
        badge.append(&icon);
    }
    Some(badge)
}
