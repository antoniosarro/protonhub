//! The games tab: each game, its ratings, and the tool it runs with.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::{Choice, Library};
use protonhub_core::launcher::Launcher;

use super::game_row::{game_row, tier_name};
use super::{boxed, scrolled};
use crate::errors;
use crate::i18n::{tr, trn};
use crate::window::{Env, LauncherState, Window};

/// Each game with its picture, ratings, and a list of the tools it can
/// run with, the current one chosen; choosing another sets it.
pub(super) fn view(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    lib: &Library,
    state: &LauncherState,
) -> gtk::Widget {
    let running = state.running;
    if lib.games.is_empty() && lib.problems.is_empty() {
        return adw::StatusPage::builder()
            .icon_name("applications-games-symbolic")
            .title(tr("No games installed", &[]))
            .description(tr(
                "Games installed with {launcher} show up here",
                &[("launcher", &launcher.title())],
            ))
            .build()
            .upcast();
    }
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);

    let search = gtk::SearchEntry::builder()
        .placeholder_text(tr("Search games", &[]))
        .hexpand(true)
        .build();
    let move_games = gtk::Button::builder()
        .child(
            &adw::ButtonContent::builder()
                .icon_name("media-playlist-shuffle-symbolic")
                .label(tr("Move Games…", &[]))
                .can_shrink(true)
                .build(),
        )
        .tooltip_text(tr("Move every game set to one tool to another", &[]))
        .sensitive(!running && lib.games.iter().any(|g| g.explicit))
        .build();
    move_games.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        #[strong]
        lib,
        move |_| move_dialog(&win, &launcher, &lib)
    ));
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.append(&search);
    bar.append(&move_games);
    content.append(&bar);
    if state.info.iter().any(|(_, i)| i.protondb.is_some()) {
        content.append(&legend());
    }

    if let Some(e) = &state.info_error {
        // Not an error of ProtonHub's: a source was down or broken.
        let row = adw::ActionRow::builder()
            .title(tr("Some ratings or anti-cheat statuses are missing", &[]))
            .subtitle(glib::markup_escape_text(e))
            .subtitle_lines(1)
            .tooltip_text(e)
            .build();
        let icon = gtk::Image::from_icon_name("dialog-warning-symbolic");
        icon.add_css_class("warning");
        row.add_prefix(&icon);
        content.append(&boxed(&row));
    }
    for problem in &lib.problems {
        content.append(&boxed(&errors::row(
            &tr("Left out: a file couldn't be read", &[]),
            problem,
        )));
    }

    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    let none = adw::ActionRow::builder()
        .title(tr("No games match", &[]))
        .build();
    none.add_css_class("dim-label");
    list.set_placeholder(Some(&none));
    for game in &lib.games {
        let info = state
            .info
            .iter()
            .find(|(id, _)| *id == game.id)
            .map(|(_, i)| i);
        list.append(&game_row(win, env, launcher, lib, game, info, running));
    }
    list.set_filter_func(glib::clone!(
        #[weak]
        search,
        #[upgrade_or]
        true,
        move |row| {
            let query = search.text().to_lowercase();
            query.is_empty() || row.widget_name().contains(query.trim())
        }
    ));
    search.connect_search_changed(glib::clone!(
        #[weak]
        list,
        move |_| list.invalidate_filter()
    ));
    content.append(&list);
    scrolled(&content)
}

/// What the colors of the bands at the start of the game rows mean.
fn legend() -> gtk::Box {
    let legend = gtk::Box::builder()
        .spacing(14)
        .halign(gtk::Align::End)
        .build();
    legend.add_css_class("caption");
    let title = gtk::Label::new(Some(&tr("ProtonDB rating:", &[])));
    title.add_css_class("dim-label");
    legend.append(&title);
    for tier in ["platinum", "gold", "silver", "bronze", "borked"] {
        let dot = gtk::Box::builder()
            .width_request(10)
            .height_request(10)
            .valign(gtk::Align::Center)
            .build();
        dot.add_css_class("dot");
        dot.add_css_class(&format!("tier-{tier}"));
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        item.append(&dot);
        item.append(&gtk::Label::new(Some(&tier_name(tier))));
        legend.append(&item);
    }
    legend
}

/// Moves every game set to one tool to another: the tools games are set
/// to on the left, every tool on the right.
fn move_dialog(win: &Rc<Window>, launcher: &Launcher, lib: &Library) {
    let mut froms: Vec<(Choice, usize)> = Vec::new();
    for tool in lib
        .games
        .iter()
        .filter(|g| g.explicit)
        .filter_map(|g| g.tool.as_ref())
    {
        match froms.iter_mut().find(|(c, _)| c.id == tool.id) {
            Some((_, n)) => *n += 1,
            None => froms.push((tool.clone(), 1)),
        }
    }
    let labels: Vec<String> = froms
        .iter()
        .map(|(c, n)| {
            trn(
                "{tool} ({n} game)",
                "{tool} ({n} games)",
                *n,
                &[("tool", &c.name)],
            )
        })
        .collect();
    let from = adw::ComboRow::builder()
        .title(tr("From", &[]))
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    let to_labels: Vec<&str> = lib.choices.iter().map(|c| c.name.as_str()).collect();
    let to = adw::ComboRow::builder()
        .title(tr("To", &[]))
        .model(&gtk::StringList::new(&to_labels))
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(tr("Games on the launcher's default stay on it", &[]))
        .build();
    group.add(&from);
    group.add(&to);
    let page = adw::PreferencesPage::new();
    page.add(&group);

    let go = gtk::Button::builder().label(tr("Move", &[])).build();
    go.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&go);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(tr("Move Games", &[]))
        .content_width(460)
        .child(&view)
        .build();
    let choices = lib.choices.clone();
    go.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        #[weak]
        dialog,
        move |_| {
            let (Some((f, _)), Some(t)) = (
                froms.get(from.selected() as usize),
                choices.get(to.selected() as usize),
            ) else {
                return;
            };
            dialog.close();
            win.reassign(&launcher, f, t);
        }
    ));
    dialog.present(Some(&win.window));
}
