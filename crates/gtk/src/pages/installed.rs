//! An installed tool's page: where it is, what uses it, and deleting it,
//! in use or not. Valve's Protons are Steam's apps, uninstalled as Steam
//! does.

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::games::{OfficialUsage, Usage};
use protonhub_core::install::{Entry, human_size};
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::Tool;
use protonhub_core::steam::Official;

use crate::i18n::{tr, trn};
use crate::text::named;
use crate::window::{Env, LauncherState, Window};
use crate::{art, ui};

/// The tool in `dir` of `state`'s launcher, or none once it's gone.
pub fn page(
    win: &Rc<Window>,
    env: &Env,
    state: &LauncherState,
    dir: &Path,
) -> Option<adw::NavigationPage> {
    let installed = state
        .usage
        .as_ref()
        .ok()
        .and_then(|usage| usage.iter().find(|u| u.entry.dir == dir));
    let (usage, name, tool, official) = match installed {
        Some(u) => {
            let (name, tool) = identify(env, &state.launcher, &u.entry);
            (u.clone(), name, tool, None)
        }
        None => {
            let officials = state.officials.as_ref().ok()?;
            let o = officials.iter().find(|o| o.official.dir == dir)?;
            (
                official_usage(o),
                o.official.name.clone(),
                None,
                Some(&o.official),
            )
        }
    };

    let family = tool.map_or(name.as_str(), |t| t.name.as_str());
    let picture = match official {
        Some(_) => Some(art::VALVE.to_owned()),
        None => tool.and_then(art::tool_source),
    };
    let avatar = art::avatar(&env.client, family, picture, 96);
    let size = human_size(usage.size);
    let what = match (&usage.entry.record, official) {
        (_, Some(_)) => tr("{size} · installed by Steam", &[("size", &size)]),
        (Some(_), None) => tr("{size} · installed by ProtonHub", &[("size", &size)]),
        (None, None) => tr("{size} · added outside ProtonHub", &[("size", &size)]),
    };
    let (hero, extra) = ui::hero(&avatar, &name, &what);
    extra.append(&delete_button(win, state, &usage, &name, official));

    let columns = ui::columns(
        &[about(win, env, &usage, official).upcast()],
        &[users(&usage).upcast()],
    );
    let content = gtk::Box::new(gtk::Orientation::Vertical, 32);
    content.append(&hero);
    content.append(&columns);
    let key = format!("installed/{}", dir.display());
    let flips = [(&hero, ui::HERO), (&columns, ui::COLUMNS)];
    Some(ui::page(&name, &ui::scrolled(win, key, &content, 1200, &flips)).0)
}

/// Its folder, Steam's app id for one of Valve's Protons, and whether
/// its checksum was verified.
fn about(
    win: &Rc<Window>,
    env: &Env,
    usage: &Usage,
    official: Option<&Official>,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Details", &[]))
        .build();
    let dir = &usage.entry.dir;
    let place = adw::ActionRow::builder()
        .title(tr("Folder", &[]))
        .subtitle(glib::markup_escape_text(&ui::tilde(dir, &env.home)))
        .subtitle_selectable(true)
        .build();
    place.add_suffix(&ui::open_button(win, dir));
    group.add(&place);
    if let Some(official) = official {
        group.add(
            &adw::ActionRow::builder()
                .title(tr("Steam App", &[]))
                .subtitle(&official.appid)
                .subtitle_selectable(true)
                .build(),
        );
    }
    if let Some(record) = &usage.entry.record {
        group.add(
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
    group
}

/// The launcher's default, if it runs with the tool, and each game that
/// does.
fn users(usage: &Usage) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(tr("Used By", &[]))
        .build();
    match &usage.users {
        None => group.add(&ui::dim_row(
            &tr("ProtonHub can't tell what uses a tool like this one", &[]),
            "",
        )),
        Some(u) if u.games.is_empty() && !u.default => {
            group.add(&ui::dim_row(&tr("No game in any launcher", &[]), ""));
        }
        Some(u) => {
            if u.default {
                let row = adw::ActionRow::builder()
                    .title(tr("The launcher's default", &[]))
                    .subtitle(tr("Games without a tool of their own", &[]))
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name("emblem-default-symbolic"));
                group.add(&row);
            }
            for game in &u.games {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(game))
                    .build();
                row.add_prefix(&gtk::Image::from_icon_name("applications-games-symbolic"));
                group.add(&row);
            }
        }
    }
    group
}

/// Delete…, which asks first, saying what will break when something uses
/// the tool; one of Valve's Protons can't be while Steam runs.
fn delete_button(
    win: &Rc<Window>,
    state: &LauncherState,
    usage: &Usage,
    name: &str,
    official: Option<&Official>,
) -> gtk::Widget {
    let steam_runs = official.is_some() && state.running;
    let delete = gtk::Button::builder()
        .label(tr("Delete…", &[]))
        .halign(gtk::Align::Start)
        .sensitive(!win.busy() && !steam_runs)
        .build();
    delete.add_css_class("destructive-action");
    let (games, default) = usage
        .users
        .as_ref()
        .map_or((Vec::new(), false), |u| (u.games.clone(), u.default));
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
    if official.is_some() && (default || !games.is_empty()) {
        body.push(tr("Steam downloads it again when one of them starts.", &[]));
    }
    body.push(if body.is_empty() {
        tr("Nothing uses it. This can't be undone.", &[])
    } else {
        tr("This can't be undone.", &[])
    });
    let heading = tr("Delete {tool}?", &[("tool", name)]);
    let body = body.join("\n\n");
    let (name, dir, official, launcher) = (
        name.to_owned(),
        usage.entry.dir.clone(),
        official.cloned(),
        state.launcher.clone(),
    );
    delete.connect_clicked(glib::clone!(
        #[weak]
        win,
        move |_| {
            let (name, dir, official, launcher) = (
                name.clone(),
                dir.clone(),
                official.clone(),
                launcher.clone(),
            );
            ui::confirm(
                &win,
                &heading,
                &body,
                &tr("Delete", &[]),
                glib::clone!(
                    #[weak]
                    win,
                    move || remove(&win, &launcher, &dir, &name, official.clone())
                ),
            );
        }
    ));
    if !steam_runs {
        return delete.upcast();
    }
    let why = ui::line(
        &tr("Quit Steam to delete it: Steam would undo it", &[]),
        &["caption", "dim-label"],
    );
    let both = gtk::Box::builder().spacing(12).build();
    both.append(&delete);
    both.append(&why);
    both.upcast()
}

/// Deletes the tool in `dir`: its folder, or one of Valve's Protons as
/// Steam uninstalls it.
fn remove(
    win: &Rc<Window>,
    launcher: &Launcher,
    dir: &Path,
    name: &str,
    official: Option<Official>,
) {
    match official {
        None => win.delete(dir.to_owned(), name.to_owned()),
        Some(official) => win.uninstall(launcher, official, name.to_owned()),
    }
}

/// One of Valve's Protons as a tool installed, to show as one.
pub fn official_usage(o: &OfficialUsage) -> Usage {
    Usage {
        entry: Entry {
            dir: o.official.dir.clone(),
            record: None,
        },
        size: o.size,
        users: Some(o.users.clone()),
    }
}

/// What a tool installed is called, and the registry's tool it's a
/// version of, if it's known.
pub fn identify<'a>(
    env: &'a Env,
    launcher: &Launcher,
    entry: &Entry,
) -> (String, Option<&'a Tool>) {
    let record = entry.record.as_ref();
    let tool = match record {
        Some(r) => env.registry.tools.get(&r.tool),
        None => guess(env, launcher, &entry.dir),
    };
    let dir = ui::dir_name(&entry.dir);
    let name = match (tool, record) {
        (Some(t), Some(r)) => named(&t.name, &r.version),
        (Some(t), None) if !simple(&dir).contains(&simple(&t.name)) => named(&t.name, &dir),
        _ => dir,
    };
    (name, tool)
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
    let name = simple(&ui::dir_name(dir));
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
