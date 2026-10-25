//! A Steam game's prefix and shader cache on its page, each with its
//! size, its folder and a button to delete it; and how deleting either is
//! asked, which the storage tab's menus ask too.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Game;
use protonhub_core::install::human_size;
use protonhub_core::launcher::Launcher;
use protonhub_core::storage::{Item, Kind};

use crate::i18n::tr;
use crate::text::{app_name, kind_name};
use crate::ui;
use crate::window::{Env, LauncherState, Window};

pub(super) fn group(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    game: &Game,
) -> Option<adw::PreferencesGroup> {
    let Ok(Some(storage)) = &state.storage else {
        return None;
    };
    let group = adw::PreferencesGroup::builder()
        .title(tr("Storage", &[]))
        .build();
    let items: Vec<_> = storage.items.iter().filter(|i| i.app == game.id).collect();
    if items.is_empty() {
        group.add(&ui::dim_row(
            &tr("No prefixes or shader caches", &[]),
            &tr("Steam makes them when a game first runs", &[]),
        ));
    }
    for item in items {
        let row = adw::ActionRow::builder()
            .title(kind_name(item.kind))
            .subtitle(glib::markup_escape_text(&format!(
                "{} · {}",
                human_size(item.size),
                ui::tilde(&item.dir, &env.home)
            )))
            .build();
        row.add_suffix(&ui::open_button(win, &item.dir));
        let ask = Deletion::of(&state.launcher, item);
        let delete = ui::icon_button("user-trash-symbolic", ask.label.trim_end_matches('…'));
        delete.set_sensitive(!win.busy() && !state.running);
        delete.connect_clicked(glib::clone!(
            #[weak]
            win,
            move |_| ask.confirm(&win)
        ));
        row.add_suffix(&delete);
        group.add(&row);
    }
    Some(group)
}

/// Deleting folders of a Steam's storage: what the button says, what the
/// confirmation asks, and what it deletes.
pub struct Deletion {
    pub label: String,
    heading: String,
    body: String,
    launcher: Launcher,
    dirs: Vec<PathBuf>,
    size: u64,
}

impl Deletion {
    /// Deleting one of an app's prefix and shader cache.
    pub fn of(launcher: &Launcher, item: &Item) -> Self {
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
        Self {
            label,
            heading,
            body,
            launcher: launcher.clone(),
            dirs: vec![item.dir.clone()],
            size: item.size,
        }
    }

    /// Asks, then deletes.
    pub fn confirm(&self, win: &Rc<Window>) {
        let body = format!(
            "{}\n\n{}",
            self.body,
            tr("This frees {size}.", &[("size", &human_size(self.size))])
        );
        let (launcher, dirs, size) = (self.launcher.clone(), self.dirs.clone(), self.size);
        ui::confirm(
            win,
            &self.heading,
            &body,
            &tr("Delete", &[]),
            glib::clone!(
                #[weak]
                win,
                move || win.delete_storage(&launcher, dirs.clone(), size)
            ),
        );
    }
}
