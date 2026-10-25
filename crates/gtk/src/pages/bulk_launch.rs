//! The page that switches launch option presets on or off for the games
//! chosen, leaving the rest of each game's options as they are: the games
//! beside the switches.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Library;
use protonhub_core::launch::{self, Options};
use protonhub_core::launcher::Launcher;

use crate::i18n::tr;
use crate::ui;
use crate::window::Window;

pub fn page(
    win: &Rc<Window>,
    launcher: &Launcher,
    lib: &Library,
    current: &[(String, String)],
) -> adw::NavigationPage {
    let apply = gtk::Button::builder()
        .label(tr("Apply", &[]))
        .sensitive(false)
        .build();
    apply.add_css_class("suggested-action");

    let games = adw::PreferencesGroup::builder()
        .title(tr("Games", &[]))
        .build();
    let all = gtk::CheckButton::new();
    let all_row = adw::ActionRow::builder()
        .title(tr("All Games", &[]))
        .activatable_widget(&all)
        .build();
    all_row.add_prefix(&all);
    games.add(&all_row);
    let none = tr("None", &[]);
    let checks: Vec<(String, gtk::CheckButton)> = lib
        .games
        .iter()
        .map(|g| {
            let text = current
                .iter()
                .find(|(id, _)| *id == g.id)
                .map(|(_, t)| t.as_str());
            let check = gtk::CheckButton::new();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&g.name))
                .subtitle(glib::markup_escape_text(text.unwrap_or(&none)))
                .subtitle_lines(1)
                .activatable_widget(&check)
                .build();
            row.add_css_class("launch-options");
            row.add_prefix(&check);
            games.add(&row);
            (g.id.clone(), check)
        })
        .collect();

    let states = [tr("Unchanged", &[]), tr("On", &[]), tr("Off", &[])];
    let model = gtk::StringList::new(&states.each_ref().map(String::as_str));
    let switches = tr("Switches", &[]);
    let mut groups: Vec<(String, adw::PreferencesGroup)> = Vec::new();
    let mut rows = Vec::new();
    for preset in launch::presets() {
        let title = preset.group.clone().unwrap_or_else(|| switches.clone());
        let at = match groups.iter().position(|(t, _)| *t == title) {
            Some(at) => at,
            None => {
                let group = adw::PreferencesGroup::builder()
                    .title(glib::markup_escape_text(&title))
                    .build();
                groups.push((title, group));
                groups.len() - 1
            }
        };
        // Plain text, set once it's known to be: built with it, the row
        // may read it as markup first.
        let row = adw::ComboRow::builder()
            .use_markup(false)
            .subtitle_lines(2)
            .model(&model)
            .build();
        row.set_title(&preset.name);
        row.set_subtitle(&preset.description);
        groups[at].1.add(&row);
        rows.push((preset, row));
    }

    let checks = Rc::new(checks);
    let rows = Rc::new(rows);
    // Apply is for games chosen and a switch to change.
    let update = Rc::new(glib::clone!(
        #[weak]
        apply,
        #[strong]
        checks,
        #[strong]
        rows,
        move || apply.set_sensitive(
            checks.iter().any(|(_, c)| c.is_active())
                && rows.iter().any(|(_, r)| r.selected() != 0)
        )
    ));
    for (_, check) in checks.iter() {
        check.connect_toggled(glib::clone!(
            #[strong]
            update,
            move |_| update()
        ));
    }
    for (_, row) in rows.iter() {
        row.connect_selected_notify(glib::clone!(
            #[strong]
            update,
            move |_| update()
        ));
    }
    all.connect_toggled(glib::clone!(
        #[strong]
        checks,
        move |all| {
            for (_, check) in checks.iter() {
                check.set_active(all.is_active());
            }
        }
    ));

    let current = current.to_vec();
    apply.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[strong]
        launcher,
        move |_| {
            let mut changes = Vec::new();
            let mut skipped = 0;
            for (id, _) in checks.iter().filter(|(_, c)| c.is_active()) {
                let text = current
                    .iter()
                    .find(|(g, _)| g == id)
                    .map_or("", |(_, t)| t.as_str());
                let Some(mut options) = Options::parse(text) else {
                    skipped += 1;
                    continue;
                };
                for (preset, row) in rows.iter() {
                    match row.selected() {
                        1 => options.set(preset, true),
                        2 => options.set(preset, false),
                        _ => {}
                    }
                }
                let new = options.to_string();
                if new != text {
                    changes.push((id.clone(), new));
                }
            }
            win.back();
            win.set_launch_many(&launcher, &changes, skipped);
        }
    ));

    let presets: Vec<gtk::Widget> = groups.into_iter().map(|(_, g)| g.upcast()).collect();
    let content = ui::columns(&[games.upcast()], &presets);
    let key = format!("launch/{launcher}");
    let (page, header) = ui::page(
        &tr("Launch Options", &[]),
        &ui::scrolled(win, key, &content, 1400, &[(&content, ui::COLUMNS)]),
    );
    header.pack_end(&apply);
    page
}
