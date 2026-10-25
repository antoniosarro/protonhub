//! A launcher's view: a tab for its games, as a grid of cards each with
//! the tool it runs with ([`games`]), one for its tools ([`tools`]), and
//! for a Steam one for its games' prefixes and shader caches
//! ([`storage`]).

mod game_card;
mod games;
mod storage;
mod tools;

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::manifest::LauncherKind;

use crate::i18n::tr;
use crate::pages::Route;
use crate::ui;
use crate::window::{Env, LauncherState, Window};

pub fn launcher(win: &Rc<Window>, env: &Env, state: &LauncherState) -> gtk::Widget {
    let view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    view.add_top_bar(&header);
    if let Some(banner) = banner(win, state) {
        view.add_top_bar(&banner);
    }

    let stack = adw::ViewStack::new();
    let tab = |child: &gtk::Widget, name: &str, title: &str, icon: &str| {
        stack.add_titled_with_icon(child, Some(name), title, icon);
    };
    match &state.library {
        Ok(Some(lib)) => tab(
            &games::view(win, env, state, lib),
            "games",
            &tr("Games", &[]),
            "applications-games-symbolic",
        ),
        Ok(None) => {}
        Err(e) => tab(
            &ui::failed(&tr("Can't read the games", &[]), e),
            "games",
            &tr("Games", &[]),
            "applications-games-symbolic",
        ),
    }
    let tools = match &state.usage {
        Ok(usage) => tools::view(win, env, state, usage),
        Err(e) => ui::failed(&tr("Can't read the tool directories", &[]), e),
    };
    tab(
        &tools,
        "tools",
        &tr("Tools", &[]),
        "applications-utilities-symbolic",
    );
    let storage = match &state.storage {
        Ok(Some(s)) => Some(storage::view(win, env, state, s)),
        Ok(None) => None,
        Err(e) => Some(ui::failed(
            &tr("Can't read the prefixes and shader caches", &[]),
            e,
        )),
    };
    if let Some(storage) = storage {
        tab(
            &storage,
            "storage",
            &tr("Storage", &[]),
            "drive-harddisk-symbolic",
        );
    }

    if stack.pages().n_items() > 1 {
        let switcher = adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        header.set_title_widget(Some(&switcher));
        // The tab last shown, if this launcher has it: only Steam has storage.
        if let Some(name) = win.tab.borrow().as_deref()
            && stack.child_by_name(name).is_some()
        {
            stack.set_visible_child_name(name);
        }
        stack.connect_visible_child_name_notify(glib::clone!(
            #[weak]
            win,
            move |s| {
                win.tab.replace(s.visible_child_name().map(Into::into));
            }
        ));
    } else {
        header.set_title_widget(Some(&adw::WindowTitle::new(&state.launcher.title(), "")));
    }
    view.set_content(Some(&stack));
    view.upcast()
}

/// Why the launcher's games can't be changed now, if they can't: it runs,
/// or from a Flatpak, nobody can tell.
fn banner(win: &Rc<Window>, state: &LauncherState) -> Option<adw::Banner> {
    let launcher = &state.launcher;
    state.library()?;
    let steam = launcher.kind == LauncherKind::Steam;
    if state.running {
        let banner = ui::banner(&tr(
            "{launcher} is running: quit it to change a game's tool, or it will undo the change",
            &[("launcher", &launcher.title())],
        ));
        // Steam can be asked to quit; Heroic and Faugus can't.
        if steam {
            banner.set_button_label(Some(&tr("Quit Steam", &[])));
            banner.connect_button_clicked(glib::clone!(
                #[weak]
                win,
                #[strong]
                launcher,
                move |_| win.quit_steam(&launcher)
            ));
        }
        Some(banner)
    } else if steam && win.quit.borrow().contains(launcher) {
        let banner = ui::banner(&tr(
            "ProtonHub quit Steam: start it again when you're done",
            &[],
        ));
        banner.set_button_label(Some(&tr("Start Steam", &[])));
        banner.connect_button_clicked(glib::clone!(
            #[weak]
            win,
            #[strong]
            launcher,
            move |_| win.start_steam(&launcher)
        ));
        Some(banner)
    } else if !protonhub_core::games::sees_launchers() {
        Some(ui::banner(&tr(
            "Quit {launcher} before changing a game's tool: from inside a Flatpak, ProtonHub can't tell whether it's running",
            &[("launcher", &launcher.title())],
        )))
    } else {
        None
    }
}

/// A card's action that opens the page `route` shows.
fn opens(win: &Rc<Window>, route: Route) -> Option<ui::Action> {
    Some(Rc::new(glib::clone!(
        #[weak]
        win,
        move || win.open(route.clone())
    )))
}

/// A key for what's kept of a launcher's tab across redraws.
fn key(state: &LauncherState, tab: &str) -> String {
    format!("{}/{tab}", state.launcher)
}
