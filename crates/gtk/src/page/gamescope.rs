//! gamescope in a game's launch options: whether the game runs in it, and
//! the options of its that matter most, as rows that edit the text.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::launch::{self, Options};

use super::game::mark;
use crate::i18n::tr;

const FILTER: &[&str] = &["-F", "--filter"];
/// gamescope's upscaling filters; none for its own default.
const FILTERS: [&str; 6] = ["", "fsr", "nis", "linear", "nearest", "pixel"];

/// Changes gamescope's arguments in the text.
type Edit = Rc<dyn Fn(&dyn Fn(&mut Vec<String>))>;

/// The row, kept in step with `text`, the launch options as text; Enter
/// in its fields does `on_enter`, as in the text's.
pub fn row(text: &adw::EntryRow, running: bool, on_enter: &Rc<dyn Fn()>) -> adw::ExpanderRow {
    let missing = protonhub_core::games::sees_launchers()
        && !protonhub_core::install::on_path(launch::GAMESCOPE);
    let expander = adw::ExpanderRow::builder()
        .title(tr("Gamescope", &[]))
        .subtitle(if missing {
            tr(
                "{program} isn't installed: the game won't start with it",
                &[("program", launch::GAMESCOPE)],
            )
        } else {
            tr(
                "Runs the game in a compositor of its own, at the resolution and frame rate set here",
                &[],
            )
        })
        .show_enable_switch(true)
        .build();
    let numbers: Vec<(adw::EntryRow, &[&str])> = [
        (tr("Output Width", &[]), &["-W", "--output-width"][..]),
        (tr("Output Height", &[]), &["-H", "--output-height"]),
        (tr("Game Width", &[]), &["-w", "--nested-width"]),
        (tr("Game Height", &[]), &["-h", "--nested-height"]),
        (tr("Frame Rate", &[]), &["-r", "--nested-refresh"]),
    ]
    .into_iter()
    .map(|(title, names)| {
        let row = adw::EntryRow::builder()
            .title(title)
            .input_purpose(gtk::InputPurpose::Digits)
            .build();
        row.connect_entry_activated(glib::clone!(
            #[strong]
            on_enter,
            move |_| on_enter()
        ));
        expander.add_row(&row);
        (row, names)
    })
    .collect();
    let labels = [
        tr("Default", &[]),
        "AMD FSR".to_owned(),
        "NVIDIA NIS".to_owned(),
        tr("Linear", &[]),
        tr("Nearest Neighbor", &[]),
        tr("Pixel Art", &[]),
    ];
    let filter = adw::ComboRow::builder()
        .title(tr("Upscaling", &[]))
        .model(&gtk::StringList::new(
            &labels.each_ref().map(String::as_str),
        ))
        .build();
    expander.add_row(&filter);
    let switches: Vec<(adw::SwitchRow, &[&str])> = [
        (tr("Fullscreen", &[]), None, &["-f", "--fullscreen"][..]),
        (tr("HDR", &[]), None, &["--hdr-enabled"]),
        (
            tr("MangoHud", &[]),
            Some(tr(
                "Drawn by gamescope, as it recommends over the MangoHud switch",
                &[],
            )),
            &["--mangoapp"],
        ),
    ]
    .into_iter()
    .map(|(title, subtitle, names)| {
        let row = adw::SwitchRow::builder().title(title).build();
        row.set_subtitle(subtitle.as_deref().unwrap_or_default());
        expander.add_row(&row);
        (row, names)
    })
    .collect();

    // Set while the rows are set to match the text, so they don't write
    // it back.
    let syncing = Rc::new(Cell::new(false));
    let edit: Edit = Rc::new(glib::clone!(
        #[strong]
        syncing,
        #[weak]
        text,
        move |change| {
            if syncing.get() {
                return;
            }
            let Some(mut options) = Options::parse(&text.text()) else {
                return;
            };
            let mut args = options.gamescope().unwrap_or_default().to_vec();
            change(&mut args);
            options.set_gamescope(Some(args));
            let new = options.to_string();
            if new != text.text() {
                text.set_text(&new);
            }
        }
    ));
    expander.connect_enable_expansion_notify(glib::clone!(
        #[strong]
        syncing,
        #[weak]
        text,
        move |expander| {
            if syncing.get() {
                return;
            }
            let Some(mut options) = Options::parse(&text.text()) else {
                return;
            };
            let on = expander.enables_expansion();
            if on != options.gamescope().is_some() {
                options.set_gamescope(on.then(Vec::new));
                text.set_text(&options.to_string());
            }
        }
    ));
    for (row, names) in &numbers {
        let names = *names;
        row.connect_changed(glib::clone!(
            #[strong]
            edit,
            move |row| {
                let value = row.text();
                let valid = value.is_empty() || value.parse::<u32>().is_ok();
                mark(row, !valid);
                if valid {
                    edit(&|args| {
                        launch::set_option(
                            args,
                            names,
                            Some(value.as_str()).filter(|v| !v.is_empty()),
                        )
                    });
                }
            }
        ));
    }
    filter.connect_selected_notify(glib::clone!(
        #[strong]
        edit,
        move |row| {
            let value = FILTERS[row.selected() as usize];
            edit(&|args| launch::set_option(args, FILTER, Some(value).filter(|v| !v.is_empty())));
        }
    ));
    for (row, names) in &switches {
        let names = *names;
        row.connect_active_notify(glib::clone!(
            #[strong]
            edit,
            move |row| {
                let on = row.is_active();
                edit(&|args| launch::set_flag(args, names, on));
            }
        ));
    }

    let sync = Rc::new(glib::clone!(
        #[weak]
        expander,
        move |text: &str| {
            let options = Options::parse(text);
            let args = options.as_ref().and_then(|o| o.gamescope());
            syncing.set(true);
            expander.set_sensitive(!running && options.is_some());
            expander.set_enable_expansion(args.is_some());
            let args = args.unwrap_or_default();
            for (row, names) in &numbers {
                let value = launch::option(args, names).unwrap_or_default();
                // Unchanged, so the cursor stays where it is while typing.
                if row.text() != value {
                    row.set_text(&value);
                }
            }
            let value = launch::option(args, FILTER).unwrap_or_default();
            let at = FILTERS.iter().position(|f| *f == value).unwrap_or(0);
            filter.set_selected(at as u32);
            for (row, names) in &switches {
                row.set_active(launch::flag(args, names));
            }
            syncing.set(false);
        }
    ));
    sync(&text.text());
    // Setting the text empties it first: synced once it's set, the rows
    // don't empty, losing the cursor, or close.
    let pending = Rc::new(Cell::new(false));
    text.connect_changed(move |e| {
        if !pending.replace(true) {
            let (e, sync, pending) = (e.clone(), sync.clone(), pending.clone());
            glib::idle_add_local_once(move || {
                pending.set(false);
                sync(&e.text());
            });
        }
    });
    expander
}
