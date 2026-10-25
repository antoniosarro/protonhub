//! One launcher's page: a tab for its games, each with the tool it runs
//! with ([`games`]), one for its tools ([`tools`]), and for a Steam one for
//! its games' prefixes and shader caches ([`storage`]). The helpers here
//! are what the tabs share.

mod game;
mod game_row;
mod games;
mod gamescope;
mod prefix;
mod storage;
mod tools;

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::i18n::tr;
use crate::window::{Env, LauncherState, Window};

pub fn build(win: &Rc<Window>, env: &Env, state: &LauncherState) -> gtk::Widget {
    let launcher = &state.launcher;
    let view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    view.add_top_bar(&header);

    let library = state.library.as_ref().map(Option::as_ref);
    if library.is_ok_and(|l| l.is_some()) {
        if state.running {
            view.add_top_bar(&banner(&tr(
                "{launcher} is running: quit it to change a game's tool, or it will undo the change",
                &[("launcher", &launcher.title())],
            )));
        } else if !protonhub_core::games::sees_launchers() {
            view.add_top_bar(&banner(&tr(
                "Quit {launcher} before changing a game's tool: from inside a Flatpak, ProtonHub can't tell whether it's running",
                &[("launcher", &launcher.title())],
            )));
        }
    }

    let stack = adw::ViewStack::new();
    match library {
        Ok(Some(lib)) => {
            stack.add_titled_with_icon(
                &games::view(win, env, launcher, lib, state),
                Some("games"),
                &tr("Games", &[]),
                "applications-games-symbolic",
            );
        }
        Ok(None) => {}
        Err(e) => {
            stack.add_titled_with_icon(
                &failed(&tr("Can't read the games", &[]), e),
                Some("games"),
                &tr("Games", &[]),
                "applications-games-symbolic",
            );
        }
    }
    let tools = match &state.usage {
        Ok(usage) => tools::view(win, env, state, usage),
        Err(e) => failed(&tr("Can't read the tool directories", &[]), e),
    };
    stack.add_titled_with_icon(
        &tools,
        Some("tools"),
        &tr("Tools", &[]),
        "applications-utilities-symbolic",
    );
    let storage = match &state.storage {
        Ok(Some(s)) => Some(storage::view(win, env, state, s)),
        Ok(None) => None,
        Err(e) => Some(failed(
            &tr("Can't read the prefixes and shader caches", &[]),
            e,
        )),
    };
    if let Some(storage) = storage {
        stack.add_titled_with_icon(
            &storage,
            Some("storage"),
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
        if let Some(name) = win.view.borrow().as_deref()
            && stack.child_by_name(name).is_some()
        {
            stack.set_visible_child_name(name);
        }
        stack.connect_visible_child_name_notify(glib::clone!(
            #[strong]
            win,
            move |s| {
                win.view.replace(s.visible_child_name().map(Into::into));
            }
        ));
    } else {
        header.set_title_widget(Some(&adw::WindowTitle::new(&launcher.title(), "")));
    }
    view.set_content(Some(&stack));

    // A game's page goes over this one. The one shown before a redraw
    // (after a job) is shown again, with what was just read.
    let nav = adw::NavigationView::new();
    let root = adw::NavigationPage::new(&view, &launcher.title());
    let game = win
        .game
        .borrow()
        .clone()
        .filter(|(l, _)| l == launcher)
        .and_then(|(_, id)| game::page(win, launcher, &id));
    match game {
        Some(page) => nav.replace(&[root, page]),
        None => nav.add(&root),
    }
    nav.connect_popped(glib::clone!(
        #[strong]
        win,
        move |_, _| {
            win.game.replace(None);
        }
    ));
    nav.upcast()
}

fn banner(text: &str) -> adw::Banner {
    adw::Banner::builder().title(text).revealed(true).build()
}

fn failed(what: &str, message: &str) -> gtk::Widget {
    adw::StatusPage::builder()
        .icon_name("dialog-error-symbolic")
        .title(what)
        .description(glib::markup_escape_text(message))
        .build()
        .upcast()
}

/// A view's content: scrolled, and no wider than reads well.
fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let clamp = adw::Clamp::builder()
        .maximum_size(880)
        .tightening_threshold(600)
        .child(child)
        .margin_top(18)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .vexpand(true)
        .build()
        .upcast()
}

/// A small rounded label, colored by `class`.
pub fn pill(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .build();
    label.add_css_class("pill");
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}

/// A boxed list of one row.
fn boxed(row: &impl IsA<gtk::Widget>) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .build();
    list.add_css_class("boxed-list");
    list.append(row);
    list
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// `path` with the home directory shown as `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
