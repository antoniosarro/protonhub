//! Preferences: the GitHub token ProtonHub sends, and which of Steam's
//! users launch options go to.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::manifest::LauncherKind;
use protonhub_core::{launcher, paths, steam};

use crate::i18n::tr;
use crate::window::Window;

pub fn show(win: &Rc<Window>) {
    let page = adw::PreferencesPage::new();
    let from_env = std::env::var_os("GITHUB_TOKEN").is_some();
    let github = adw::PreferencesGroup::builder()
        .title(tr("GitHub", &[]))
        .description(if from_env {
            tr("GITHUB_TOKEN is set, and used instead of a token saved here", &[])
        } else {
            tr(
                "A token raises GitHub's limit from 60 to 5000 requests an hour. It needs no scopes, and goes to GitHub alone.",
                &[],
            )
        })
        .build();
    let token = adw::PasswordEntryRow::builder()
        .title(tr("Access Token", &[]))
        .text(paths::saved_token().unwrap_or_default())
        .show_apply_button(true)
        .sensitive(!from_env)
        .build();
    let (saved, removed, saving) = (
        tr("Token saved", &[]),
        tr("Token removed", &[]),
        tr("Saving the GitHub token", &[]),
    );
    token.connect_apply(glib::clone!(
        #[strong]
        win,
        move |row| {
            let text = row.text();
            match paths::save_token(Some(&text)) {
                Ok(()) => {
                    if let Ok(env) = &win.env {
                        env.client.set_token(paths::github_token());
                    }
                    win.toast(if text.trim().is_empty() {
                        &removed
                    } else {
                        &saved
                    });
                }
                Err(e) => win.report(saving.clone(), e.to_string()),
            }
        }
    ));
    github.add(&token);
    page.add(&github);
    if let Some(group) = steam_users(win) {
        page.add(&group);
    }
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);
    dialog.present(Some(&win.window));
}

/// Which of Steam's users launch options go to, where a Steam has more
/// than one.
fn steam_users(win: &Rc<Window>) -> Option<adw::PreferencesGroup> {
    let home = win.env.as_ref().ok()?.home.clone();
    let steams: Vec<_> = launcher::detect(&home)
        .into_iter()
        .filter(|l| l.kind == LauncherKind::Steam)
        .filter_map(|l| Some((steam::users(&l.root).ok()?, l)))
        .filter(|(users, _)| users.len() > 1)
        .collect();
    if steams.is_empty() {
        return None;
    }
    let group = adw::PreferencesGroup::builder()
        .title(tr("Steam Profile", &[]))
        .description(tr(
            "The user whose launch options ProtonHub shows and changes",
            &[],
        ))
        .build();
    let chosen = paths::steam_user();
    let (last, saving) = (
        tr("The one signed in last", &[]),
        tr("Choosing the Steam profile", &[]),
    );
    for (users, steam) in steams {
        let mut labels = vec![last.clone()];
        labels.extend(users.iter().map(|u| match &u.name {
            Some(name) => format!("{name} ({})", u.account),
            None => u.account.clone(),
        }));
        let at = users
            .iter()
            .position(|u| Some(&u.account) == chosen.as_ref())
            .map_or(0, |i| i + 1);
        let row = adw::ComboRow::builder()
            .title(steam.title())
            .model(&gtk::StringList::new(
                &labels.iter().map(String::as_str).collect::<Vec<_>>(),
            ))
            .selected(at as u32)
            .build();
        row.connect_selected_notify(glib::clone!(
            #[strong]
            win,
            #[strong]
            saving,
            move |row| {
                let account = (row.selected() as usize)
                    .checked_sub(1)
                    .and_then(|i| users.get(i))
                    .map(|u| u.account.as_str());
                match paths::save_steam_user(account) {
                    Ok(()) => win.reload(),
                    Err(e) => win.report(saving.clone(), e.to_string()),
                }
            }
        ));
        group.add(&row);
    }
    Some(group)
}
