//! The page that moves every game set to one tool to another: the tools
//! games are set to, and every tool.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Library;
use protonhub_core::launcher::Launcher;

use crate::i18n::tr;
use crate::window::Window;
use crate::{choices, ui};

pub fn page(win: &Rc<Window>, launcher: &Launcher, lib: &Library) -> adw::NavigationPage {
    let froms = choices::in_use(lib);
    let labels: Vec<String> = froms.iter().map(|(_, label)| label.clone()).collect();
    let from = adw::ComboRow::builder()
        .title(tr("From", &[]))
        .model(&ui::strings(&labels))
        .build();
    let to_labels: Vec<String> = lib.choices.iter().map(|c| c.name.clone()).collect();
    let to = adw::ComboRow::builder()
        .title(tr("To", &[]))
        .model(&ui::strings(&to_labels))
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(tr("Games on the launcher's default stay on it", &[]))
        .build();
    group.add(&from);
    group.add(&to);

    let go = gtk::Button::builder().label(tr("Move", &[])).build();
    go.add_css_class("suggested-action");
    let choices = lib.choices.clone();
    go.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[strong]
        launcher,
        move |_| {
            let (Some((f, _)), Some(t)) = (
                froms.get(from.selected() as usize),
                choices.get(to.selected() as usize),
            ) else {
                return;
            };
            win.back();
            win.reassign(&launcher, f, t);
        }
    ));
    let key = format!("move/{launcher}");
    let (page, header) = ui::page(
        &tr("Move Games", &[]),
        &ui::scrolled(win, key, &group, 640, &[]),
    );
    header.pack_end(&go);
    page
}
