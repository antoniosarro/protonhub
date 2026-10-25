//! Preferences: the GitHub token ProtonHub sends.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::paths;

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
    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);
    dialog.present(Some(&win.window));
}
