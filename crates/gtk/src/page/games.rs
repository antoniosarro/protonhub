//! The games tab: each game, its ratings, and the tool it runs with.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::{Choice, Library};
use protonhub_core::launch::{self, Options};
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::LauncherKind;

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
    if let Ok(Some(current)) = &state.launch {
        let edit = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .icon_name("document-edit-symbolic")
                    .label(tr("Launch Options…", &[]))
                    .can_shrink(true)
                    .build(),
            )
            .tooltip_text(tr(
                "Switch launch options on or off for several games at once",
                &[],
            ))
            .sensitive(!running)
            .build();
        edit.connect_clicked(glib::clone!(
            #[strong]
            win,
            #[strong]
            launcher,
            #[strong]
            lib,
            #[strong]
            current,
            move |_| launch_dialog(&win, &launcher, &lib, &current)
        ));
        bar.append(&edit);
    }
    content.append(&bar);
    if matches!(launcher.kind, LauncherKind::Steam | LauncherKind::Faugus) {
        content.append(&boxed(&default_row(win, launcher, lib, running)));
    }
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
    // A row opens the game's page; its tool list and button stay its own.
    let ids: Vec<String> = lib.games.iter().map(|g| g.id.clone()).collect();
    list.connect_row_activated(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        move |_, row| {
            if let Some(id) = usize::try_from(row.index()).ok().and_then(|i| ids.get(i)) {
                super::game::open(&win, row, &launcher, id);
            }
        }
    ));
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

/// The launcher's default tool, chosen from its tools, or for Steam none;
/// choosing sets it.
fn default_row(
    win: &Rc<Window>,
    launcher: &Launcher,
    lib: &Library,
    running: bool,
) -> adw::ComboRow {
    // A default that isn't installed any more is shown too, as it is.
    let mut options: Vec<Option<Choice>> = lib.choices.iter().cloned().map(Some).collect();
    if let Some(d) = &lib.default
        && !lib.choices.iter().any(|c| c.id == d.id)
    {
        options.push(Some(d.clone()));
    }
    if launcher.kind == LauncherKind::Steam {
        options.insert(0, None);
    }
    let labels: Vec<String> = options
        .iter()
        .map(|o| match o {
            Some(c) => c.name.clone(),
            None => tr("None: Steam Play off", &[]),
        })
        .collect();
    let current = options
        .iter()
        .position(|o| o.as_ref().map(|c| &c.id) == lib.default.as_ref().map(|d| &d.id))
        .unwrap_or(0);
    let row = adw::ComboRow::builder()
        .title(tr("Default Tool", &[]))
        .subtitle(tr("What games without a tool of their own run with", &[]))
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .selected(current as u32)
        .sensitive(!running)
        .build();
    row.connect_selected_notify(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        move |row| {
            let at = row.selected() as usize;
            if at != current {
                win.set_default(&launcher, options[at].as_ref());
            }
        }
    ));
    row
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

/// Switches launch option presets on or off for the games chosen, leaving
/// the rest of each game's options as they are.
fn launch_dialog(
    win: &Rc<Window>,
    launcher: &Launcher,
    lib: &Library,
    current: &[(String, String)],
) {
    let apply = gtk::Button::builder()
        .label(tr("Apply", &[]))
        .sensitive(false)
        .build();
    apply.add_css_class("suggested-action");
    let header = adw::HeaderBar::new();
    header.pack_end(&apply);
    let page = adw::PreferencesPage::new();

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
            row.add_prefix(&check);
            games.add(&row);
            (g.id.clone(), check)
        })
        .collect();
    page.add(&games);

    let states = [tr("Unchanged", &[]), tr("On", &[]), tr("Off", &[])];
    let model = gtk::StringList::new(&states.each_ref().map(String::as_str));
    let switches = tr("Switches", &[]);
    let mut groups: Vec<(String, adw::PreferencesGroup)> = Vec::new();
    let mut rows = Vec::new();
    for preset in launch::presets() {
        let title = preset.group.clone().unwrap_or_else(|| switches.clone());
        if !groups.iter().any(|(t, _)| *t == title) {
            let group = adw::PreferencesGroup::builder()
                .title(glib::markup_escape_text(&title))
                .build();
            page.add(&group);
            groups.push((title.clone(), group));
        }
        let row = adw::ComboRow::builder()
            .use_markup(false)
            .title(preset.name.as_str())
            .subtitle(preset.description.as_str())
            .subtitle_lines(2)
            .model(&model)
            .build();
        let (_, group) = groups
            .iter()
            .find(|(t, _)| *t == title)
            .expect("added above");
        group.add(&row);
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

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(tr("Launch Options", &[]))
        .content_width(560)
        .content_height(680)
        .child(&view)
        .build();
    let current = current.to_vec();
    apply.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        launcher,
        #[weak]
        dialog,
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
            dialog.close();
            win.set_launch_many(&launcher, &changes, skipped);
        }
    ));
    dialog.present(Some(&win.window));
}
