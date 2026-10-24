//! A tool's dialog: where it comes from, keeping it up to date, and its
//! releases to install one.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::channels::Channel;
use protonhub_core::install;
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::{Source, Tool};
use protonhub_core::releases::{Release, Version};

use crate::art;
use crate::i18n::tr;
use crate::page::{pill, tilde};
use crate::window::Window;

pub fn show(win: &Rc<Window>, tool: &Tool, launcher: &Launcher) {
    let Ok(env) = &win.env else { return };
    let Some(dir) = launcher.tool_dir(tool) else {
        return;
    };
    let page = adw::PreferencesPage::new();

    let hero = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    let avatar = art::tool_avatar(&env.client, Some(tool), &tool.name, 80);
    avatar.set_margin_bottom(6);
    hero.append(&avatar);
    let name = gtk::Label::new(Some(&tool.name));
    name.add_css_class("title-1");
    hero.append(&name);
    let from = gtk::LinkButton::builder()
        .label(tool.source.origin())
        .uri(match &tool.source {
            Source::Github(repo) => format!("https://github.com/{repo}"),
            Source::Forgejo(url) | Source::Gitlab(url) | Source::Url { url, .. } => url.clone(),
        })
        .halign(gtk::Align::Center)
        .build();
    hero.append(&from);
    let into = gtk::Label::builder()
        .label(tr(
            "Installs into {dir}",
            &[("dir", &tilde(&dir, &env.home))],
        ))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    into.add_css_class("caption");
    into.add_css_class("dim-label");
    hero.append(&into);
    let top = adw::PreferencesGroup::new();
    top.add(&hero);
    page.add(&top);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&page));
    let dialog = adw::Dialog::builder()
        .title(&tool.name)
        .content_width(540)
        .content_height(680)
        .child(&view)
        .build();

    let current = win
        .channels()
        .list
        .into_iter()
        .find(|c| c.tool == tool.id && launcher.matches(&c.launcher));
    let updates = adw::PreferencesGroup::builder()
        .title(tr("Updates", &[]))
        .build();
    updates.add(&channel_row(win, &dialog, launcher, tool, current));
    page.add(&updates);

    let releases = adw::PreferencesGroup::builder()
        .title(tr("Releases", &[]))
        .build();
    let loading = adw::ActionRow::builder()
        .title(tr("Looking for releases…", &[]))
        .build();
    loading.add_prefix(&adw::Spinner::new());
    releases.add(&loading);
    page.add(&releases);
    dialog.present(Some(&win.window));

    // Versions already here can't be installed again.
    let installed: Vec<String> = install::list(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.record)
        .filter(|r| r.tool == tool.id)
        .map(|r| r.version)
        .collect();

    let client = env.client.clone();
    let fetched = tool.clone();
    let listed = gio::spawn_blocking(move || client.releases(&fetched, 30));
    glib::spawn_future_local(glib::clone!(
        #[strong]
        win,
        #[strong]
        tool,
        #[strong]
        launcher,
        #[weak]
        dialog,
        async move {
            let result = match listed.await {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => Err("the release lookup crashed".to_owned()),
            };
            releases.remove(&loading);
            fill(
                &win, &tool, &launcher, &dialog, &releases, result, &installed,
            );
        }
    ));
}

/// The tool's channel: whether it's kept up to date here, how many
/// versions to keep, whether games move to new releases, and Update Now.
/// Every change is saved to the channels file at once.
fn channel_row(
    win: &Rc<Window>,
    dialog: &adw::Dialog,
    launcher: &Launcher,
    tool: &Tool,
    current: Option<Channel>,
) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::builder()
        .title(tr("Keep up to date", &[]))
        .subtitle(tr(
            "Install new releases as they come out, with the daily timer (protonhub timer enable)",
            &[],
        ))
        .show_enable_switch(true)
        .enable_expansion(current.is_some())
        .expanded(current.is_some())
        .build();
    let keep = adw::SpinRow::with_range(1.0, 10.0, 1.0);
    keep.set_title(&tr("Versions to keep", &[]));
    keep.set_subtitle(&tr("Older ones are deleted once no game uses them", &[]));
    keep.set_value(current.as_ref().map_or(3, |c| c.keep) as f64);
    let moves = adw::SwitchRow::builder()
        .title(tr("Move games to new releases", &[]))
        .subtitle(tr(
            "Games set to an older version switch to the new one",
            &[],
        ))
        .active(current.as_ref().is_some_and(|c| c.move_games))
        .build();
    let now_row = adw::ActionRow::builder()
        .title(tr("Update now", &[]))
        .build();
    let now = gtk::Button::builder()
        .label(tr("Update", &[]))
        .valign(gtk::Align::Center)
        .sensitive(!win.busy())
        .build();
    now_row.add_suffix(&now);
    now_row.set_activatable_widget(Some(&now));
    row.add_row(&keep);
    row.add_row(&moves);
    row.add_row(&now_row);

    let spec = current.map_or_else(|| win.channel_spec(launcher), |c| c.launcher);
    let channel = {
        let (keep, moves, tool, spec) =
            (keep.clone(), moves.clone(), tool.id.clone(), spec.clone());
        move || Channel {
            tool: tool.clone(),
            launcher: spec.clone(),
            keep: keep.value() as usize,
            move_games: moves.is_active(),
        }
    };
    let save = {
        let (win, row, channel) = (win.clone(), row.clone(), channel.clone());
        move || {
            let on = row.enables_expansion();
            let ch = channel();
            win.edit_channels(|c| {
                c.list
                    .retain(|o| !(o.tool == ch.tool && o.launcher == ch.launcher));
                if on {
                    c.list.push(ch);
                }
            });
        }
    };
    row.connect_enable_expansion_notify({
        let save = save.clone();
        move |_| save()
    });
    keep.connect_value_notify({
        let save = save.clone();
        move |_| save()
    });
    moves.connect_active_notify(move |_| save());
    // The tools view marks auto-updated tools: redraw it once closed.
    dialog.connect_closed(glib::clone!(
        #[strong]
        win,
        move |_| win.redraw()
    ));
    now.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        tool,
        #[weak]
        dialog,
        move |_| {
            dialog.close();
            win.update_channel(&tool, channel());
        }
    ));
    row
}

/// A release's notes, as the plain Markdown they're written in.
fn show_notes(parent: &adw::Dialog, tag: &str, body: &str) {
    let label = gtk::Label::builder()
        .label(body)
        .wrap(true)
        .selectable(true)
        .xalign(0.0)
        .yalign(0.0)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&label)
            .vexpand(true)
            .build(),
    ));
    adw::Dialog::builder()
        .title(tag)
        .content_width(560)
        .content_height(520)
        .child(&view)
        .build()
        .present(Some(parent));
}

/// Lists the releases in `group`, or why there are none. Kept out of the
/// `clone!` above, where xgettext doesn't look for strings.
fn fill(
    win: &Rc<Window>,
    tool: &Tool,
    launcher: &Launcher,
    dialog: &adw::Dialog,
    group: &adw::PreferencesGroup,
    result: Result<Vec<Release>, String>,
    installed: &[String],
) {
    let releases = match result {
        Ok(r) => r,
        Err(message) => {
            group.add(&crate::errors::row(
                &tr("Can't list the releases", &[]),
                &message,
            ));
            win.report(
                tr("Listing {tool} releases", &[("tool", &tool.name)]),
                message,
            );
            return;
        }
    };
    if releases.is_empty() {
        let row = adw::ActionRow::builder()
            .title(tr("No releases", &[]))
            .subtitle(tr("The repository has published none for this tool", &[]))
            .build();
        row.add_css_class("dim-label");
        group.add(&row);
        return;
    }
    let latest = releases.iter().position(|r| !r.prerelease);
    let mut first_installable: Option<gtk::Button> = None;
    for (i, r) in releases.iter().enumerate() {
        // The day of its ISO 8601 time.
        let date = r.published_at.as_deref().and_then(|t| t.split('T').next());
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&r.tag))
            .subtitle(date.unwrap_or_default())
            .build();
        if Some(i) == latest {
            row.add_suffix(&pill(&tr("Latest", &[]), "accent"));
        } else if r.prerelease {
            row.add_suffix(&pill(&tr("Prerelease", &[]), "warning"));
        }
        if let Some(body) = r.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            let notes = gtk::Button::builder()
                .icon_name("text-x-generic-symbolic")
                .tooltip_text(tr("Release notes", &[]))
                .valign(gtk::Align::Center)
                .build();
            notes.add_css_class("flat");
            let (tag, body) = (r.tag.clone(), body.to_owned());
            notes.connect_clicked(glib::clone!(
                #[weak]
                dialog,
                move |_| show_notes(&dialog, &tag, &body)
            ));
            row.add_suffix(&notes);
        }
        let here = installed.contains(&r.tag);
        if here {
            let done = gtk::Image::builder()
                .icon_name("object-select-symbolic")
                .tooltip_text(tr("Installed", &[]))
                .build();
            done.add_css_class("success");
            row.add_suffix(&done);
            group.add(&row);
            continue;
        }
        let button = gtk::Button::builder()
            .label(tr("Install", &[]))
            .sensitive(!win.busy())
            .valign(gtk::Align::Center)
            .build();
        if Some(i) == latest {
            button.add_css_class("suggested-action");
        }
        if button.is_sensitive() && first_installable.is_none() {
            first_installable = Some(button.clone());
        }
        let tag = r.tag.clone();
        button.connect_clicked(glib::clone!(
            #[strong]
            win,
            #[strong]
            tool,
            #[strong]
            launcher,
            #[weak]
            dialog,
            move |_| {
                dialog.close();
                win.install(&tool, Version::Tag(tag.clone()), launcher.clone());
            }
        ));
        row.add_suffix(&button);
        // Enter on a focused row installs, like clicking.
        row.set_activatable_widget(Some(&button));
        group.add(&row);
    }
    // Enter installs the newest release not installed yet.
    if let Some(button) = first_installable {
        button.grab_focus();
    }
}
