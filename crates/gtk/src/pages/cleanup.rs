//! Pages that list what nothing needs any more, and delete it all: a
//! launcher's unused tools, and what a Steam's uninstalled games left.

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::install::human_size;

use crate::i18n::{tr, trn};
use crate::text::{app_name, kind_name};
use crate::ui;
use crate::window::{Env, LauncherState, Window};

/// The launcher's tools nothing uses; none once there are none.
pub(super) fn tools(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
) -> Option<adw::NavigationPage> {
    let unused: Vec<_> = state
        .usage
        .as_ref()
        .ok()?
        .iter()
        .filter(|u| u.unused())
        .collect();
    if unused.is_empty() {
        return None;
    }
    let total: u64 = unused.iter().map(|u| u.size).sum();
    let rows = unused
        .iter()
        .map(|u| {
            row(
                &ui::dir_name(&u.entry.dir),
                &human_size(u.size),
                &ui::tilde(&u.entry.dir, &env.home),
            )
        })
        .collect::<Vec<_>>();
    let dirs: Vec<PathBuf> = unused.iter().map(|u| u.entry.dir.clone()).collect();
    let page = page(
        win,
        &format!("cleanup/{}", state.launcher),
        &tr("Unused Tools", &[]),
        &trn(
            "Delete {n} unused tool?",
            "Delete {n} unused tools?",
            unused.len(),
            &[],
        ),
        &tr(
            "No game in any launcher runs them. Deleting them frees {size}, and deletes tools installed by other apps too.",
            &[("size", &human_size(total))],
        ),
        &rows,
        (!win.busy()).then(|| {
            Box::new(glib::clone!(
                #[weak]
                win,
                move || win.clean_up(dirs.clone(), total)
            )) as Box<dyn Fn()>
        }),
    );
    Some(page)
}

/// What a Steam's uninstalled games left behind; none once there's nothing.
pub(super) fn leftovers(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
) -> Option<adw::NavigationPage> {
    let Ok(Some(storage)) = &state.storage else {
        return None;
    };
    let orphaned: Vec<_> = storage.orphaned().collect();
    if orphaned.is_empty() {
        return None;
    }
    let total: u64 = orphaned.iter().map(|i| i.size).sum();
    let rows = orphaned
        .iter()
        .map(|item| {
            // Why it can go: what it was for isn't there any more.
            let why = if item.shortcut() {
                tr("Removed from Steam", &[])
            } else {
                tr("No longer installed", &[])
            };
            row(
                &app_name(item),
                &format!(
                    "{why} · {} · {}",
                    kind_name(item.kind),
                    human_size(item.size)
                ),
                &ui::tilde(&item.dir, &env.home),
            )
        })
        .collect::<Vec<_>>();
    let dirs: Vec<PathBuf> = orphaned.iter().map(|i| i.dir.clone()).collect();
    let launcher = state.launcher.clone();
    let page = page(
        win,
        &format!("leftovers/{}", state.launcher),
        &tr("Leftovers", &[]),
        &trn(
            "Delete {n} leftover folder?",
            "Delete {n} leftover folders?",
            orphaned.len(),
            &[],
        ),
        &format!(
            "{} {}",
            tr(
                "Their games are no longer installed. Saves not in Steam Cloud are lost with their prefixes.",
                &[],
            ),
            tr("This frees {size}.", &[("size", &human_size(total))])
        ),
        &rows,
        (!win.busy() && !state.running).then(|| {
            Box::new(glib::clone!(
                #[weak]
                win,
                move || win.delete_storage(&launcher, dirs.clone(), total)
            )) as Box<dyn Fn()>
        }),
    );
    Some(page)
}

/// A folder to delete: its name, a line about it, and where it is.
fn row(title: &str, subtitle: &str, place: &str) -> gtk::Widget {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(subtitle))
        .tooltip_text(place)
        .build()
        .upcast()
}

/// A page that asks `heading`, says `body`, lists `rows`, and has a
/// Delete button in its header bar that does `delete`; without it, the
/// button can't be used now.
fn page(
    win: &Rc<Window>,
    key: &str,
    title: &str,
    heading: &str,
    body: &str,
    rows: &[gtk::Widget],
    delete: Option<Box<dyn Fn()>>,
) -> adw::NavigationPage {
    let heading = gtk::Label::builder()
        .label(heading)
        .wrap(true)
        .xalign(0.0)
        .build();
    heading.add_css_class("title-2");
    let body = gtk::Label::builder()
        .label(body)
        .wrap(true)
        .xalign(0.0)
        .build();
    body.add_css_class("dim-label");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(&heading);
    content.append(&body);
    let list = ui::boxed(rows);
    list.set_margin_top(12);
    content.append(&list);
    let button = gtk::Button::builder()
        .label(tr("Delete", &[]))
        .sensitive(delete.is_some())
        .build();
    button.add_css_class("destructive-action");
    if let Some(delete) = delete {
        button.connect_clicked(move |_| delete());
    }
    let (page, header) = ui::page(
        title,
        &ui::scrolled(win, key.to_owned(), &content, 900, &[]),
    );
    header.pack_end(&button);
    page
}
