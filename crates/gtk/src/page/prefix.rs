//! A game's Wine prefix on its page: the winetricks verbs in it and a
//! dialog to install more, Wine's tools, and running a Windows program in
//! it, all with the game's own Wine or Proton.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::games::Game;
use protonhub_core::manifest::LauncherKind;
use protonhub_core::prefix::{self, Prefix};

use crate::i18n::tr;
use crate::window::{LauncherState, Window};

/// The group, for Steam and Heroic games; why there's nothing to do where
/// the prefix can't be found.
pub fn group(
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
        move |_| choose(&win, &game, &prefix)
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

/// The dialog that picks verbs to install: the common ones by group, the
/// installed ones ticked already, and any other by name.
fn choose(win: &Rc<Window>, game: &Game, prefix: &Prefix) {
    let installed = prefix.installed();
    let chosen: Rc<RefCell<Vec<String>>> = Rc::default();
    let install = gtk::Button::builder()
        .label(tr("Install", &[]))
        .sensitive(false)
        .build();
    install.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&install);

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
            if valid {
                other.remove_css_class("error");
            } else {
                other.add_css_class("error");
            }
            install
                .set_sensitive(valid && (!chosen.borrow().is_empty() || !typed.trim().is_empty()));
        }
    ));
    other.connect_changed(glib::clone!(
        #[strong]
        update,
        move |_| update()
    ));

    let page = adw::PreferencesPage::new();
    let first = adw::PreferencesGroup::builder()
        .description(tr(
            "winetricks installs them with the game's own Wine or Proton. Some installers take minutes, and some show windows of their own.",
            &[],
        ))
        .build();
    first.add(&other);
    page.add(&first);
    for group in prefix::verbs() {
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
        page.add(&g);
    }

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(tr("Install Components", &[]))
        .content_width(520)
        .content_height(640)
        .child(&view)
        .build();
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
        #[strong]
        win,
        #[strong]
        game,
        #[strong]
        prefix,
        #[weak]
        dialog,
        #[weak]
        other,
        move |_| {
            let mut verbs = chosen.borrow().clone();
            verbs.extend(other.text().split_whitespace().map(str::to_owned));
            dialog.close();
            win.tricks(&game, prefix.clone(), verbs);
        }
    ));
    dialog.present(Some(&win.window));
}
