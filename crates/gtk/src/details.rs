//! An installed tool's dialog: where it is, what uses it, and deleting it,
//! in use or not. Valve's Protons are Steam's apps, uninstalled as Steam
//! does.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::games::Usage;
use protonhub_core::install::human_size;
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::Tool;
use protonhub_core::steam::Official;

use crate::art;
use crate::i18n::{tr, trn};
use crate::page::tilde;
use crate::window::{Env, Window};

/// How a tool is deleted.
#[derive(Clone)]
pub enum Removal {
    /// Its folder, whoever put it there.
    Folder,
    /// Uninstalled as Steam's app; not while Steam runs.
    Steam {
        official: Official,
        launcher: Launcher,
        running: bool,
    },
}

pub fn show(
    win: &Rc<Window>,
    env: &Env,
    usage: &Usage,
    name: &str,
    tool: Option<&Tool>,
    removal: Removal,
) {
    let page = adw::PreferencesPage::new();

    let hero = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    let family = tool.map_or(name, |t| t.name.as_str());
    let picture = match &removal {
        Removal::Steam { .. } => Some(art::VALVE.to_owned()),
        Removal::Folder => tool.and_then(art::tool_source),
    };
    let avatar = art::avatar(&env.client, family, picture, 80);
    avatar.set_margin_bottom(6);
    hero.append(&avatar);
    let title = gtk::Label::builder()
        .label(name)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    title.add_css_class("title-2");
    hero.append(&title);
    let what = gtk::Label::new(Some(&match &usage.entry.record {
        _ if matches!(removal, Removal::Steam { .. }) => tr(
            "{size} · installed by Steam",
            &[("size", &human_size(usage.size))],
        ),
        Some(_) => tr(
            "{size} · installed by ProtonHub",
            &[("size", &human_size(usage.size))],
        ),
        None => tr(
            "{size} · added outside ProtonHub",
            &[("size", &human_size(usage.size))],
        ),
    }));
    what.add_css_class("dim-label");
    hero.append(&what);
    let top = adw::PreferencesGroup::new();
    top.add(&hero);
    page.add(&top);

    let about = adw::PreferencesGroup::new();
    let dir = usage.entry.dir.clone();
    let place = adw::ActionRow::builder()
        .title(tr("Folder", &[]))
        .subtitle(glib::markup_escape_text(&tilde(&dir, &env.home)))
        .subtitle_selectable(true)
        .build();
    place.add_suffix(&open_button(win, &dir));
    about.add(&place);
    if let Removal::Steam { official, .. } = &removal {
        about.add(
            &adw::ActionRow::builder()
                .title(tr("Steam App", &[]))
                .subtitle(&official.appid)
                .subtitle_selectable(true)
                .build(),
        );
    }
    if let Some(record) = &usage.entry.record {
        about.add(
            &adw::ActionRow::builder()
                .title(tr("Checksum", &[]))
                .subtitle(if record.checksum.is_some() {
                    tr("Verified when installed", &[])
                } else {
                    tr("Not verified: the release publishes none", &[])
                })
                .build(),
        );
    }
    page.add(&about);

    let users = adw::PreferencesGroup::builder()
        .title(tr("Used By", &[]))
        .build();
    match &usage.users {
        None => users.add(&dim_row(&tr(
            "ProtonHub can't tell what uses a tool like this one",
            &[],
        ))),
        Some(u) if u.games.is_empty() && !u.default => {
            users.add(&dim_row(&tr("No game in any launcher", &[])));
        }
        Some(u) => {
            if u.default {
                let row = adw::ActionRow::builder()
                    .title(tr("The launcher's default", &[]))
                    .subtitle(tr("Games without a tool of their own", &[]))
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name("emblem-default-symbolic"));
                users.add(&row);
            }
            for game in &u.games {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(game))
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name("applications-games-symbolic"));
                users.add(&row);
            }
        }
    }
    page.add(&users);

    let steam_runs = matches!(removal, Removal::Steam { running: true, .. });
    let delete = gtk::Button::builder()
        .label(tr("Delete…", &[]))
        .halign(gtk::Align::Center)
        .width_request(200)
        .height_request(40)
        .sensitive(!win.busy() && !steam_runs)
        .build();
    delete.add_css_class("destructive-action");
    let danger = adw::PreferencesGroup::new();
    danger.add(&delete);
    if steam_runs {
        let why = gtk::Label::builder()
            .label(tr("Quit Steam to delete it: Steam would undo it", &[]))
            .margin_top(12)
            .build();
        why.add_css_class("caption");
        why.add_css_class("dim-label");
        danger.add(&why);
    }
    page.add(&danger);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(name)
        .content_width(480)
        .content_height(620)
        .child(&view)
        .build();
    let (name, games, default) = (
        name.to_owned(),
        usage
            .users
            .as_ref()
            .map(|u| u.games.clone())
            .unwrap_or_default(),
        usage.users.as_ref().is_some_and(|u| u.default),
    );
    delete.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[weak]
        dialog,
        move |_| confirm(&win, &dialog, &name, &dir, &games, default, &removal)
    ));
    dialog.present(Some(&win.window));
}

/// A button that opens `dir` in the file manager.
pub fn open_button(win: &Rc<Window>, dir: &std::path::Path) -> gtk::Button {
    let open = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text(tr("Open the folder", &[]))
        .valign(gtk::Align::Center)
        .build();
    open.add_css_class("flat");
    let dir = dir.to_owned();
    open.connect_clicked(glib::clone!(
        #[strong]
        win,
        move |_| {
            gtk::FileLauncher::new(Some(&gio::File::for_path(&dir))).launch(
                Some(&win.window),
                None::<&gio::Cancellable>,
                |_| {},
            );
        }
    ));
    open
}

fn dim_row(title: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    row.add_css_class("dim-label");
    row
}

/// Asks before deleting, saying what will break when something uses it.
fn confirm(
    win: &Rc<Window>,
    parent: &adw::Dialog,
    name: &str,
    dir: &std::path::Path,
    games: &[String],
    default: bool,
    removal: &Removal,
) {
    let mut body = Vec::new();
    if !games.is_empty() {
        body.push(trn(
            "{n} game runs with it and won't start until you choose another tool for it:",
            "{n} games run with it and won't start until you choose another tool for them:",
            games.len(),
            &[],
        ));
        body.push(games.join("\n"));
    }
    if default {
        body.push(tr(
            "It's the launcher's default: games without a tool of their own will need a new default.",
            &[],
        ));
    }
    if matches!(removal, Removal::Steam { .. }) && (default || !games.is_empty()) {
        body.push(tr("Steam downloads it again when one of them starts.", &[]));
    }
    if body.is_empty() {
        body.push(tr("Nothing uses it. This can't be undone.", &[]));
    } else {
        body.push(tr("This can't be undone.", &[]));
    }
    let alert = adw::AlertDialog::new(
        Some(&tr("Delete {tool}?", &[("tool", name)])),
        Some(&body.join("\n\n")),
    );
    alert.add_responses(&[
        ("cancel", &tr("Cancel", &[])),
        ("delete", &tr("Delete", &[])),
    ]);
    alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    alert.set_close_response("cancel");
    let (name, dir, removal) = (name.to_owned(), dir.to_owned(), removal.clone());
    alert.connect_response(
        Some("delete"),
        glib::clone!(
            #[strong]
            win,
            #[weak]
            parent,
            move |_, _| {
                parent.close();
                match &removal {
                    Removal::Folder => win.delete(dir.clone(), name.clone()),
                    Removal::Steam {
                        official, launcher, ..
                    } => win.uninstall(launcher, official.clone(), name.clone()),
                }
            }
        ),
    );
    alert.present(Some(parent));
}
