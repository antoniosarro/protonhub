//! A tool's page: where it comes from and installs into, its releases to
//! install one, each with its notes a page away, and keeping it up to
//! date beside them.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::channels::Channel;
use protonhub_core::install;
use protonhub_core::launcher::Launcher;
use protonhub_core::manifest::{Source, Tool};
use protonhub_core::releases::{Release, Version};

use crate::i18n::tr;
use crate::window::{Env, Window};
use crate::{art, errors, ui};

pub(super) fn page(
    win: &Rc<Window>,
    env: &Env,
    launcher: &Launcher,
    tool: &Tool,
) -> Option<adw::NavigationPage> {
    let dir = launcher.tool_dir(tool)?;
    let avatar = art::tool_avatar(&env.client, Some(tool), &tool.name, 96);
    let (hero, extra) = ui::hero(
        &avatar,
        &tool.name,
        &tr(
            "Installs into {dir}",
            &[("dir", &ui::tilde(&dir, &env.home))],
        ),
    );
    let uri = match &tool.source {
        Source::Github(repo) => format!("https://github.com/{repo}"),
        Source::Forgejo(url) | Source::Gitlab(url) | Source::Url { url, .. } => url.clone(),
    };
    let repo = ui::button("web-browser-symbolic", tool.source.origin());
    repo.set_tooltip_text(Some(&uri));
    repo.set_halign(gtk::Align::Start);
    repo.connect_clicked(glib::clone!(
        #[weak]
        win,
        move |_| ui::open_uri(&win, &uri)
    ));
    extra.append(&repo);

    let releases = adw::PreferencesGroup::builder()
        .title(tr("Releases", &[]))
        .build();
    // Versions already here can't be installed again.
    let installed: Vec<String> = install::list(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| e.record)
        .filter(|r| r.tool == tool.id)
        .map(|r| r.version)
        .collect();
    let listed = win.releases.borrow().get(&tool.id).cloned();
    match listed {
        Some(listed) => fill(
            win,
            tool,
            launcher,
            &releases,
            Ok(listed),
            &installed,
            false,
        ),
        None => fetch(win, env, tool, launcher, &releases, installed),
    }

    let changed = Rc::new(Cell::new(false));
    let columns = ui::columns(
        &[releases.upcast()],
        &[updates(win, launcher, tool, &changed).upcast()],
    );
    let content = gtk::Box::new(gtk::Orientation::Vertical, 32);
    content.append(&hero);
    content.append(&columns);
    let key = format!("tool/{launcher}/{}", tool.id);
    let flips = [(&hero, ui::HERO), (&columns, ui::COLUMNS)];
    let (page, _) = ui::page(&tool.name, &ui::scrolled(win, key, &content, 1200, &flips));
    // The tools view marks the tools kept up to date: drawn again once
    // this page goes, if that changed.
    page.connect_hidden(glib::clone!(
        #[weak]
        win,
        move |_| {
            if changed.replace(false) {
                // Not while the window builds this page again.
                glib::idle_add_local_once(move || win.redraw());
            }
        }
    ));
    Some(page)
}

/// Lists the releases on a worker thread, then in `group`.
fn fetch(
    win: &Rc<Window>,
    env: &Env,
    tool: &Tool,
    launcher: &Launcher,
    group: &adw::PreferencesGroup,
    installed: Vec<String>,
) {
    let loading = adw::ActionRow::builder()
        .title(tr("Looking for releases…", &[]))
        .build();
    loading.add_prefix(&adw::Spinner::new());
    group.add(&loading);
    let client = env.client.clone();
    let fetched = tool.clone();
    let listed = gio::spawn_blocking(move || client.releases(&fetched, 30));
    glib::spawn_future_local(glib::clone!(
        #[weak]
        win,
        #[strong]
        tool,
        #[strong]
        launcher,
        #[weak]
        group,
        async move {
            let result = match listed.await {
                Ok(r) => r.map_err(|e| e.to_string()),
                Err(_) => Err("the release lookup crashed".to_owned()),
            };
            if let Ok(r) = &result {
                win.releases.borrow_mut().insert(tool.id.clone(), r.clone());
            }
            group.remove(&loading);
            fill(&win, &tool, &launcher, &group, result, &installed, true);
        }
    ));
}

/// Keeping the tool up to date here: whether it's a channel, how many
/// versions to keep, whether games move to new releases, and Update Now.
/// Every change is saved to the channels file at once, and sets `changed`.
fn updates(
    win: &Rc<Window>,
    launcher: &Launcher,
    tool: &Tool,
    changed: &Rc<Cell<bool>>,
) -> adw::PreferencesGroup {
    let current = win
        .channels()
        .list
        .into_iter()
        .find(|c| c.tool == tool.id && launcher.matches(&c.launcher));
    let group = adw::PreferencesGroup::builder()
        .title(tr("Updates", &[]))
        .build();
    let on = adw::SwitchRow::builder()
        .title(tr("Keep up to date", &[]))
        .subtitle(tr(
            "Install new releases as they come out, with the daily timer (protonhub timer enable)",
            &[],
        ))
        .active(current.is_some())
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
    group.add(&on);
    for row in [
        keep.upcast_ref::<gtk::Widget>(),
        moves.upcast_ref(),
        now_row.upcast_ref(),
    ] {
        group.add(row);
        on.bind_property("active", row, "sensitive")
            .sync_create()
            .build();
    }

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
        let (win, on, channel, changed) =
            (win.clone(), on.clone(), channel.clone(), changed.clone());
        move || {
            let on = on.is_active();
            let ch = channel();
            win.edit_channels(|c| {
                c.list
                    .retain(|o| !(o.tool == ch.tool && o.launcher == ch.launcher));
                if on {
                    c.list.push(ch);
                }
            });
            changed.set(true);
        }
    };
    on.connect_active_notify({
        let save = save.clone();
        move |_| save()
    });
    keep.connect_value_notify({
        let save = save.clone();
        move |_| save()
    });
    moves.connect_active_notify(move |_| save());
    now.connect_clicked(glib::clone!(
        #[weak]
        win,
        #[strong]
        tool,
        move |_| win.update_channel(&tool, channel())
    ));
    group
}

/// A page of a release's notes, as the plain Markdown they're written in.
fn notes(tag: &str, body: &str) -> adw::NavigationPage {
    let label = gtk::Label::builder()
        .label(body)
        .wrap(true)
        .selectable(true)
        .xalign(0.0)
        .yalign(0.0)
        .margin_top(18)
        .margin_bottom(32)
        .margin_start(24)
        .margin_end(24)
        .build();
    label.add_css_class("notes");
    let clamp = adw::Clamp::builder()
        .maximum_size(900)
        .child(&label)
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .vexpand(true)
        .build();
    ui::page(tag, &scrolled).0
}

/// Lists the releases in `group`, or why there are none; with `focus`,
/// the newest not installed takes the focus, so Enter installs it. Kept
/// out of the `clone!` above, where xgettext doesn't look for strings.
fn fill(
    win: &Rc<Window>,
    tool: &Tool,
    launcher: &Launcher,
    group: &adw::PreferencesGroup,
    result: Result<Vec<Release>, String>,
    installed: &[String],
    focus: bool,
) {
    let releases = match result {
        Ok(r) => r,
        Err(message) => {
            group.add(&errors::row(&tr("Can't list the releases", &[]), &message));
            win.report(
                tr("Listing {tool} releases", &[("tool", &tool.name)]),
                message,
            );
            return;
        }
    };
    if releases.is_empty() {
        group.add(&ui::dim_row(
            &tr("No releases", &[]),
            &tr("The repository has published none for this tool", &[]),
        ));
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
            row.add_suffix(&ui::tag(&tr("Latest", &[]), "accent"));
        } else if r.prerelease {
            row.add_suffix(&ui::tag(&tr("Prerelease", &[]), "warning"));
        }
        if let Some(body) = r.body.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
            let button = ui::icon_button("text-x-generic-symbolic", &tr("Release notes", &[]));
            let (tag, body) = (r.tag.clone(), body.to_owned());
            button.connect_clicked(glib::clone!(
                #[weak]
                win,
                move |_| win.push(&notes(&tag, &body))
            ));
            row.add_suffix(&button);
        }
        if installed.contains(&r.tag) {
            let done = gtk::Image::builder()
                .icon_name("object-select-symbolic")
                .tooltip_text(tr("Installed", &[]))
                .valign(gtk::Align::Center)
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
            #[weak]
            win,
            #[strong]
            tool,
            #[strong]
            launcher,
            move |_| win.install(&tool, Version::Tag(tag.clone()), launcher.clone())
        ));
        row.add_suffix(&button);
        // Enter on a focused row installs, like clicking.
        row.set_activatable_widget(Some(&button));
        group.add(&row);
    }
    if let (true, Some(button)) = (focus, first_installable) {
        button.grab_focus();
    }
}
