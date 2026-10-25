//! The page that picks winetricks verbs to install into a game's prefix:
//! the common ones by group, side by side, those installed ticked
//! already, and any other by name.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Game;
use protonhub_core::prefix::{self, Prefix};

use crate::i18n::tr;
use crate::ui;
use crate::window::Window;

pub fn page(win: &Rc<Window>, game: &Game, prefix: &Prefix) -> adw::NavigationPage {
    let installed = prefix.installed();
    let chosen: Rc<RefCell<Vec<String>>> = Rc::default();
    let install = gtk::Button::builder()
        .label(tr("Install", &[]))
        .sensitive(false)
        .build();
    install.add_css_class("suggested-action");

    let other = adw::EntryRow::builder()
        .title(tr("Other verbs, separated by spaces", &[]))
        .build();
    // Install is for a choice that winetricks would take.
    let update = Rc::new(glib::clone!(
        #[strong]
        chosen,
        #[weak]
        install,
        #[weak]
        other,
        move || {
            let typed = other.text();
            let valid = typed.split_whitespace().all(prefix::is_verb);
            ui::mark(&other, !valid);
            install
                .set_sensitive(valid && (!chosen.borrow().is_empty() || !typed.trim().is_empty()));
        }
    ));
    other.connect_changed(glib::clone!(
        #[strong]
        update,
        move |_| update()
    ));
    let first = adw::PreferencesGroup::builder()
        .description(tr(
            "winetricks installs them with the game's own Wine or Proton. Some installers take minutes, and some show windows of their own.",
            &[],
        ))
        .build();
    first.add(&other);

    let mut columns: [Vec<gtk::Widget>; 2] = Default::default();
    for (i, group) in prefix::verbs().into_iter().enumerate() {
        let g = adw::PreferencesGroup::builder()
            .title(glib::markup_escape_text(&group.title))
            .build();
        for verb in group.verb {
            let has = installed.contains(&verb.id);
            let check = gtk::CheckButton::builder()
                .active(has)
                .sensitive(!has)
                .valign(gtk::Align::Center)
                .build();
            let subtitle = match (has, &verb.description) {
                (true, _) => tr("Installed", &[]),
                (false, Some(d)) => d.clone(),
                (false, None) => String::new(),
            };
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&verb.name))
                .subtitle(glib::markup_escape_text(&subtitle))
                .activatable_widget(&check)
                .sensitive(!has)
                .build();
            row.add_prefix(&check);
            check.connect_toggled(glib::clone!(
                #[strong]
                chosen,
                #[strong]
                update,
                move |c| {
                    let mut chosen = chosen.borrow_mut();
                    chosen.retain(|v| *v != verb.id);
                    if c.is_active() {
                        chosen.push(verb.id.clone());
                    }
                    drop(chosen);
                    update();
                }
            ));
            g.add(&row);
        }
        columns[i % 2].push(g.upcast());
    }

    let columns = ui::columns(&columns[0], &columns[1]);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    content.append(&first);
    content.append(&columns);
    let key = format!("tricks/{}", game.id);
    let (page, header) = ui::page(
        &tr("Install Components", &[]),
        &ui::scrolled(win, key, &content, 1200, &[(&columns, ui::COLUMNS)]),
    );
    header.pack_end(&install);
    // Enter in the field installs, as Enter saves elsewhere.
    other.connect_entry_activated(glib::clone!(
        #[weak]
        install,
        move |_| {
            if install.is_sensitive() {
                install.emit_clicked();
            }
        }
    ));
    install.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[strong]
        game,
        #[strong]
        prefix,
        #[weak]
        other,
        move |_| {
            let mut verbs = chosen.borrow().clone();
            verbs.extend(other.text().split_whitespace().map(str::to_owned));
            win.back();
            win.tricks(&game, prefix.clone(), verbs);
        }
    ));
    page
}
