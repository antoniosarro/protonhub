//! A game's Wine prefix on its page: the winetricks verbs in it and a
//! page to install more, Wine's tools, and running a Windows program in
//! it, all with the game's own Wine or Proton.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::games::Game;
use protonhub_core::manifest::LauncherKind;
use protonhub_core::prefix;

use crate::i18n::tr;
use crate::pages::tricks;
use crate::window::{LauncherState, Window};

/// The group, for Steam and Heroic games; why there's nothing to do where
/// the prefix can't be found.
pub(super) fn group(
    win: &Rc<Window>,
    state: &LauncherState,
    game: &Game,
) -> Option<adw::PreferencesGroup> {
    if !matches!(
        state.launcher.kind,
        LauncherKind::Steam | LauncherKind::Heroic | LauncherKind::Faugus
    ) {
        return None;
    }
    let group = adw::PreferencesGroup::builder()
        .title(tr("Wine Prefix", &[]))
        .build();
    let prefix = match prefix::find(&state.launcher, game) {
        Ok(p) => p,
        Err(e) => {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&e.to_string()))
                .build();
            row.add_css_class("dim-label");
            group.add(&row);
            return Some(group);
        }
    };
    group.set_description(Some(&glib::markup_escape_text(
        &prefix.dir.display().to_string(),
    )));

    let installed = prefix.installed();
    let components = adw::ActionRow::builder()
        .title(tr("Components", &[]))
        .subtitle(if installed.is_empty() {
            tr("None installed with winetricks", &[])
        } else {
            glib::markup_escape_text(&installed.join(", ")).into()
        })
        .subtitle_lines(2)
        .build();
    let install = gtk::Button::builder()
        .label(tr("Install…", &[]))
        .valign(gtk::Align::Center)
        .sensitive(!win.busy())
        .build();
    install.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        game,
        #[strong]
        prefix,
        move |_| win.push(&tricks::page(&win, &game, &prefix))
    ));
    components.add_suffix(&install);
    group.add(&components);

    // Wine's tools open windows of their own and run on their own.
    let tools = adw::ActionRow::builder()
        .title(tr("Wine's Tools", &[]))
        .build();
    let buttons = gtk::Box::builder()
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    let starting = tr(
        "Starting a program in the prefix of {game}",
        &[("game", &game.name)],
    );
    for (label, tool) in [
        (tr("Settings", &[]), "winecfg"),
        (tr("Registry", &[]), "regedit"),
        (tr("Control Panel", &[]), "control"),
    ] {
        let button = gtk::Button::with_label(&label);
        button.connect_clicked(glib::clone!(
            #[strong]
            win,
            #[strong]
            prefix,
            #[strong]
            starting,
            move |_| {
                if let Err(e) = prefix.wine_tool(tool).and_then(prefix::launch) {
                    win.report(starting.clone(), e.to_string());
                }
            }
        ));
        buttons.append(&button);
    }
    tools.add_suffix(&buttons);
    group.add(&tools);

    let program = adw::ActionRow::builder()
        .title(tr("Run a Windows Program", &[]))
        .subtitle(tr("An installer, a patch or a mod manager", &[]))
        .build();
    let pick = gtk::Button::builder()
        .label(tr("Choose…", &[]))
        .valign(gtk::Align::Center)
        .build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&tr("Windows Programs", &[])));
    filter.add_suffix("exe");
    filter.add_suffix("msi");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title(tr("Run a Windows Program", &[]))
        .filters(&filters)
        .build();
    pick.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        prefix,
        move |_| {
            dialog.open(
                Some(&win.window),
                None::<&gio::Cancellable>,
                glib::clone!(
                    #[strong]
                    win,
                    #[strong]
                    prefix,
                    #[strong]
                    starting,
                    move |file| {
                        let Some(path) = file.ok().and_then(|f| f.path()) else {
                            return;
                        };
                        if let Err(e) = prefix.program(&path).and_then(prefix::launch) {
                            win.report(starting.clone(), e.to_string());
                        }
                    }
                ),
            );
        }
    ));
    program.add_suffix(&pick);
    program.set_activatable_widget(Some(&pick));
    group.add(&program);
    Some(group)
}
