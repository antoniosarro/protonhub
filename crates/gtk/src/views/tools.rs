//! The tools tab: how much the unused ones take, then a card per tool
//! installed, per Valve's Proton, and per tool that can be installed.

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Usage;
use protonhub_core::install::human_size;
use protonhub_core::manifest::Tool;

use super::{key, opens};
use crate::i18n::{tr, trn};
use crate::pages::{Route, installed};
use crate::text::users;
use crate::window::{Env, LauncherState, Window};
use crate::{art, errors, ui};

pub(super) fn view(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    usage: &[Usage],
) -> gtk::Widget {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 32);
    if let Some(strip) = unused(win, state, usage) {
        content.append(&strip);
    }
    content.append(&installed_section(win, env, state, usage));
    if let Some(section) = officials(win, env, state) {
        content.append(&section);
    }
    content.append(&available(win, env, state, usage));
    ui::scrolled(win, key(state, "tools"), &content, 1600, &[])
}

/// A tool's card: its picture beside its name and a line under it, and a
/// mark at its end.
fn tile(avatar: &adw::Avatar, name: &str, under: &str, mark: Option<gtk::Label>) -> gtk::Box {
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(3)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    let title = ui::line(name, &["heading"]);
    title.set_max_width_chars(20);
    title.set_tooltip_text(Some(name));
    text.append(&title);
    let under = ui::line(under, &["caption", "dim-label"]);
    under.set_max_width_chars(26);
    text.append(&under);
    let tile = ui::card(gtk::Orientation::Horizontal, 280);
    tile.add_css_class("tile");
    tile.set_spacing(12);
    tile.append(avatar);
    tile.append(&text);
    if let Some(mark) = mark {
        tile.append(&mark);
    }
    tile
}

/// How much the tools nothing uses take, and a button to the page that
/// deletes them.
fn unused(win: &Rc<Window>, state: &LauncherState, usage: &[Usage]) -> Option<gtk::Widget> {
    let unused: Vec<_> = usage.iter().filter(|u| u.unused()).collect();
    if unused.is_empty() {
        return None;
    }
    let total: u64 = unused.iter().map(|u| u.size).sum();
    let row = adw::ActionRow::builder()
        .title(trn(
            "{n} unused tool takes {size}",
            "{n} unused tools take {size}",
            unused.len(),
            &[("size", &human_size(total))],
        ))
        .subtitle(tr("No game in any launcher runs it", &[]))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("user-trash-symbolic"));
    let clean = gtk::Button::builder()
        .label(tr("Clean Up…", &[]))
        .valign(gtk::Align::Center)
        .sensitive(!win.busy())
        .build();
    clean.add_css_class("destructive-action");
    clean.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[strong(rename_to = launcher)]
        state.launcher,
        move |_| win.open(Route::Cleanup {
            launcher: launcher.clone()
        })
    ));
    row.add_suffix(&clean);
    Some(ui::boxed(&[row.upcast()]).upcast())
}

/// Every tool in the launcher's tool directories, ProtonHub's or not.
fn installed_section(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    usage: &[Usage],
) -> gtk::Box {
    let launcher = &state.launcher;
    if usage.is_empty() {
        let none = ui::dim_row(
            &tr("No tools installed yet", &[]),
            &tr("Install one from the list below", &[]),
        );
        return ui::section(
            &tr("Installed", &[]),
            None,
            None,
            &ui::boxed(&[none.upcast()]),
        );
    }
    let total: u64 = usage.iter().map(|u| u.size).sum();
    let detail = trn(
        "{n} tool · {size}",
        "{n} tools · {size}",
        usage.len(),
        &[("size", &human_size(total))],
    );
    let grid = ui::Grid::new();
    let mut sorted: Vec<_> = usage.iter().collect();
    sorted.sort_by_key(|u| ui::dir_name(&u.entry.dir).to_lowercase());
    for u in sorted {
        let (name, tool) = installed::identify(env, launcher, &u.entry);
        let family = tool.map_or(name.as_str(), |t| t.name.as_str());
        let avatar = art::tool_avatar(&env.client, tool, family, 40);
        let card = tile(&avatar, &name, &usage_line(u), usage_mark(u));
        card.set_tooltip_text(Some(&u.entry.dir.display().to_string()));
        grid.add(
            &card,
            opens(
                win,
                Route::Installed {
                    launcher: launcher.clone(),
                    dir: u.entry.dir.clone(),
                },
            ),
        );
    }
    ui::section(&tr("Installed", &[]), Some(&detail), None, &grid.flow)
}

/// An installed tool's size and what uses it.
fn usage_line(u: &Usage) -> String {
    let mut line = vec![human_size(u.size)];
    // The Unused mark says that already.
    line.extend(users(u).filter(|_| !u.unused()));
    line.join(" · ")
}

/// Default or Unused, where a tool is either.
fn usage_mark(u: &Usage) -> Option<gtk::Label> {
    if u.users.as_ref().is_some_and(|u| u.default) {
        Some(ui::tag(&tr("Default", &[]), "accent"))
    } else if u.unused() {
        Some(ui::tag(&tr("Unused", &[]), "warning"))
    } else {
        None
    }
}

/// Valve's Protons, which Steam installs as apps.
fn officials(win: &Rc<Window>, env: &Env, state: &LauncherState) -> Option<gtk::Box> {
    let title = tr("From Steam", &[]);
    let detail = tr("Valve's Protons, which Steam installs and updates", &[]);
    let found = match &state.officials {
        Ok(found) if found.is_empty() => return None,
        Ok(found) => found,
        Err(e) => {
            let row = errors::row(&tr("Can't read Steam's Protons", &[]), e);
            return Some(ui::section(
                &title,
                Some(&detail),
                None,
                &ui::boxed(&[row.upcast()]),
            ));
        }
    };
    let grid = ui::Grid::new();
    for o in found {
        let usage = installed::official_usage(o);
        let avatar = art::avatar(&env.client, "Proton", Some(art::VALVE.into()), 40);
        let card = tile(
            &avatar,
            &o.official.name,
            &usage_line(&usage),
            usage_mark(&usage),
        );
        grid.add(
            &card,
            opens(
                win,
                Route::Installed {
                    launcher: state.launcher.clone(),
                    dir: o.official.dir.clone(),
                },
            ),
        );
    }
    Some(ui::section(&title, Some(&detail), None, &grid.flow))
}

/// The registry's tools this launcher takes; each opens its releases.
fn available(win: &Rc<Window>, env: &Env, state: &LauncherState, usage: &[Usage]) -> gtk::Box {
    let launcher = &state.launcher;
    let channels = win.channels();
    let grid = ui::Grid::new();
    for tool in env.registry.tools.values() {
        let Some(dir) = launcher.tool_dir(tool) else {
            continue;
        };
        let here = installed_here(usage, &dir, tool);
        let mut line = vec![tool.source.origin().to_owned()];
        if here > 0 {
            line.push(trn("{n} installed", "{n} installed", here, &[]));
        }
        let kept = channels
            .list
            .iter()
            .any(|c| c.tool == tool.id && launcher.matches(&c.launcher));
        let avatar = art::tool_avatar(&env.client, Some(tool), &tool.name, 40);
        let card = tile(
            &avatar,
            &tool.name,
            &line.join(" · "),
            kept.then(|| ui::tag(&tr("Auto-update", &[]), "accent")),
        );
        grid.add(
            &card,
            opens(
                win,
                Route::Tool {
                    launcher: launcher.clone(),
                    tool: tool.id.clone(),
                },
            ),
        );
    }
    ui::section(
        &tr("Available", &[]),
        Some(&tr("Install a release, or keep a tool up to date", &[])),
        None,
        &grid.flow,
    )
}

/// How many versions of `tool` ProtonHub installed in `dir`.
fn installed_here(usage: &[Usage], dir: &Path, tool: &Tool) -> usize {
    usage
        .iter()
        .filter(|u| u.entry.dir.parent() == Some(dir))
        .filter(|u| u.entry.record.as_ref().is_some_and(|r| r.tool == tool.id))
        .count()
}
