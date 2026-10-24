//! The tools tab: a cleanup of the unused ones, what's installed, Valve's
//! Protons, and what can be installed.

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::Usage;
use protonhub_core::install::human_size;
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::Tool;

use super::{dir_name, pill, scrolled};
use crate::i18n::{tr, trn};
use crate::window::{Env, LauncherState, Window, named};
use crate::{art, details, errors, tool};

/// The tools: a cleanup of the unused ones, what's installed, and what
/// can be.
pub(super) fn view(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    usage: &[Usage],
) -> gtk::Widget {
    let launcher = &state.launcher;
    let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
    if let Some(group) = unused_group(win, usage) {
        content.append(&group);
    }
    content.append(&installed_group(win, env, launcher, usage));
    if let Some(group) = officials_group(win, env, state) {
        content.append(&group);
    }
    content.append(&available_group(win, env, launcher, usage));
    scrolled(&content)
}

/// How much the tools nothing uses take, and a button to delete them.
fn unused_group(win: &Rc<Window>, usage: &[Usage]) -> Option<adw::PreferencesGroup> {
    let unused: Vec<_> = usage.iter().filter(|u| u.unused()).collect();
    if unused.is_empty() {
        return None;
    }
    let total: u64 = unused.iter().map(|u| u.size).sum();
    let names: Vec<String> = unused.iter().map(|u| dir_name(&u.entry.dir)).collect();
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
    let dirs: Vec<_> = unused.iter().map(|u| u.entry.dir.clone()).collect();
    clean.connect_clicked(glib::clone!(
        #[strong]
        win,
        move |_| confirm_cleanup(&win, &names, dirs.clone(), total)
    ));
    row.add_suffix(&clean);
    let group = adw::PreferencesGroup::new();
    group.add(&row);
    Some(group)
}

/// Every tool in the launcher's tool directories, ProtonHub's or not,
/// with its size and what uses it.
fn installed_group(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    usage: &[Usage],
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Installed", &[]))
        .build();
    if usage.is_empty() {
        let row = adw::ActionRow::builder()
            .title(tr("No tools installed yet", &[]))
            .subtitle(tr("Install one from the list below", &[]))
            .build();
        row.add_css_class("dim-label");
        group.add(&row);
        return group;
    }
    let total: u64 = usage.iter().map(|u| u.size).sum();
    group.set_description(Some(&trn(
        "{n} tool · {size}",
        "{n} tools · {size}",
        usage.len(),
        &[("size", &human_size(total))],
    )));
    let mut sorted: Vec<_> = usage.iter().collect();
    sorted.sort_by_key(|u| dir_name(&u.entry.dir).to_lowercase());
    for u in sorted {
        let record = u.entry.record.as_ref();
        let tool = match record {
            Some(r) => env.registry.tools.get(&r.tool),
            None => guess(env, launcher, &u.entry.dir),
        };
        let dir = dir_name(&u.entry.dir);
        let name = match (tool, record) {
            (Some(t), Some(r)) => named(&t.name, &r.version),
            (Some(t), None) if !simple(&dir).contains(&simple(&t.name)) => named(&t.name, &dir),
            _ => dir.clone(),
        };
        let family = tool.map_or(name.as_str(), |t| t.name.as_str());
        let avatar = art::tool_avatar(&env.client, tool, family, 36);
        group.add(&usage_row(
            win,
            u,
            &name,
            &avatar,
            tool,
            details::Removal::Folder,
        ));
    }
    group
}

/// An installed tool: its size and what uses it, marked Default or Unused,
/// and a button to its details.
fn usage_row(
    win: &Rc<Window>,
    u: &Usage,
    name: &str,
    avatar: &adw::Avatar,
    tool: Option<&Tool>,
    removal: details::Removal,
) -> adw::ActionRow {
    let mut subtitle = vec![human_size(u.size)];
    // The Unused mark says that already.
    subtitle.extend(users(u).filter(|_| !u.unused()));
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(name))
        .subtitle(glib::markup_escape_text(&subtitle.join(" · ")))
        .tooltip_text(u.entry.dir.display().to_string())
        .build();
    row.add_prefix(avatar);
    if u.users.as_ref().is_some_and(|u| u.default) {
        row.add_suffix(&pill(&tr("Default", &[]), "accent"));
    } else if u.unused() {
        row.add_suffix(&pill(&tr("Unused", &[]), "warning"));
    }
    let info = gtk::Button::builder()
        .icon_name("info-outline-symbolic")
        .tooltip_text(tr("Details", &[]))
        .valign(gtk::Align::Center)
        .build();
    info.add_css_class("flat");
    row.add_suffix(&info);
    row.set_activatable_widget(Some(&info));
    let (u, name, tool) = (u.clone(), name.to_owned(), tool.cloned());
    info.connect_clicked(glib::clone!(
        #[strong]
        win,
        move |_| {
            if let Ok(env) = &win.env {
                details::show(&win, env, &u, &name, tool.as_ref(), removal.clone());
            }
        }
    ));
    row
}

/// Valve's Protons, which Steam installs as apps: each with its size, what
/// uses it, and its details, where it can be uninstalled.
fn officials_group(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
) -> Option<adw::PreferencesGroup> {
    let group = adw::PreferencesGroup::builder()
        .title(tr("From Steam", &[]))
        .description(tr("Valve's Protons, which Steam installs and updates", &[]))
        .build();
    let found = match &state.officials {
        Ok(found) if found.is_empty() => return None,
        Ok(found) => found,
        Err(e) => {
            group.add(&errors::row(&tr("Can't read Steam's Protons", &[]), e));
            return Some(group);
        }
    };
    for o in found {
        let usage = Usage {
            entry: protonhub_core::install::Entry {
                dir: o.official.dir.clone(),
                record: None,
            },
            size: o.size,
            users: Some(o.users.clone()),
        };
        let avatar = art::avatar(&env.client, "Proton", Some(art::VALVE.into()), 36);
        let removal = details::Removal::Steam {
            official: o.official.clone(),
            launcher: state.launcher.clone(),
            running: state.running,
        };
        group.add(&usage_row(
            win,
            &usage,
            &o.official.name,
            &avatar,
            None,
            removal,
        ));
    }
    Some(group)
}

/// The registry's tools this launcher takes; each opens its releases.
fn available_group(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    usage: &[Usage],
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Available", &[]))
        .description(tr("Install a release, or keep a tool up to date", &[]))
        .build();
    let channels = win.channels();
    for tool in env.registry.tools.values() {
        let Some(dir) = launcher.tool_dir(tool) else {
            continue;
        };
        let here = installed(usage, &dir, tool);
        let mut subtitle = vec![tool.source.origin().to_owned()];
        if here > 0 {
            subtitle.push(trn("{n} installed", "{n} installed", here, &[]));
        }
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&tool.name))
            .subtitle(glib::markup_escape_text(&subtitle.join(" · ")))
            .activatable(true)
            .build();
        row.add_prefix(&art::tool_avatar(&env.client, Some(tool), &tool.name, 36));
        if channels
            .list
            .iter()
            .any(|c| c.tool == tool.id && launcher.matches(&c.launcher))
        {
            row.add_suffix(&pill(&tr("Auto-update", &[]), "accent"));
        }
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.connect_activated(glib::clone!(
            #[strong]
            win,
            #[strong]
            launcher,
            #[strong]
            tool,
            move |_| tool::show(&win, &tool, &launcher)
        ));
        group.add(&row);
    }
    group
}

/// The registry's tool a directory ProtonHub didn't install is a version
/// of: the only one that goes where it is, the one its name starts with,
/// or the one the others there are variants of (DXVK, for DXVK GPL-Async).
fn guess<'a>(env: &'a Env, launcher: &Launcher, dir: &Path) -> Option<&'a Tool> {
    let here: Vec<_> = env
        .registry
        .tools
        .values()
        .filter(|t| launcher.tool_dir(t).as_deref() == dir.parent())
        .collect();
    if let [only] = here[..] {
        return Some(only);
    }
    let name = simple(&dir_name(dir));
    let named = here
        .iter()
        .filter(|t| name.starts_with(&simple(&t.name)))
        .max_by_key(|t| t.name.len());
    named
        .or_else(|| {
            here.iter().find(|base| {
                here.iter()
                    .all(|t| simple(&t.name).starts_with(&simple(&base.name)))
            })
        })
        .copied()
}

/// A name with only its letters and digits, lowercased, to compare.
fn simple(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// How many versions of `tool` ProtonHub installed in `dir`.
fn installed(usage: &[Usage], dir: &Path, tool: &Tool) -> usize {
    usage
        .iter()
        .filter(|u| u.entry.dir.parent() == Some(dir))
        .filter(|u| u.entry.record.as_ref().is_some_and(|r| r.tool == tool.id))
        .count()
}

/// Who uses a tool, in a few words; none where nobody can tell.
fn users(u: &Usage) -> Option<String> {
    let users = u.users.as_ref()?;
    let games = match &users.games[..] {
        [] => None,
        [one] => Some(one.clone()),
        many => Some(trn("{n} game", "{n} games", many.len(), &[])),
    };
    Some(match (users.default, games) {
        (false, None) => tr("unused", &[]),
        (true, None) => tr("used by the default", &[]),
        (false, Some(g)) => tr("used by {games}", &[("games", &g)]),
        (true, Some(g)) => tr("used by the default and {games}", &[("games", &g)]),
    })
}

fn confirm_cleanup(win: &Rc<Window>, names: &[String], dirs: Vec<std::path::PathBuf>, total: u64) {
    let dialog = adw::AlertDialog::new(
        Some(&trn(
            "Delete {n} unused tool?",
            "Delete {n} unused tools?",
            names.len(),
            &[],
        )),
        Some(&tr(
            "This frees {size}:\n\n{tools}",
            &[("size", &human_size(total)), ("tools", &names.join("\n"))],
        )),
    );
    dialog.add_responses(&[
        ("cancel", &tr("Cancel", &[])),
        ("delete", &tr("Delete", &[])),
    ]);
    dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.connect_response(
        Some("delete"),
        glib::clone!(
            #[strong]
            win,
            move |_, _| win.clean_up(dirs.clone(), total)
        ),
    );
    dialog.present(Some(&win.window));
}
