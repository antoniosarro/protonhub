//! The sidebar's rows: a launcher's, with what it has, and the app's own
//! places under them.

use adw::prelude::*;
use protonhub_core::manifest::LauncherKind;

use super::LauncherState;
use crate::i18n::{tr, trn};
use crate::ui;

/// A row: an icon beside a name and a line under it, named `name` for the
/// view it shows.
fn row(icon: &str, title: &str, subtitle: Option<&str>, name: &str) -> (gtk::ListBoxRow, gtk::Box) {
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    text.append(&ui::line(title, &[]));
    if let Some(subtitle) = subtitle {
        text.append(&ui::line(subtitle, &["caption", "dim-label"]));
    }
    let content = gtk::Box::builder()
        .spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&text);
    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .name(name)
        .tooltip_text(title)
        .build();
    (row, content)
}

/// A launcher's row: its icon and name, what it has, and a mark while it
/// runs.
pub fn launcher(state: &LauncherState) -> gtk::ListBoxRow {
    let l = &state.launcher;
    let icon = match l.kind {
        LauncherKind::Steam => "applications-games-symbolic",
        LauncherKind::Heroic => "starred-symbolic",
        LauncherKind::Lutris => "applications-system-symbolic",
        LauncherKind::Bottles => "package-x-generic-symbolic",
        LauncherKind::Winezgui => "application-x-executable-symbolic",
        LauncherKind::Faugus => "input-gaming-symbolic",
    };
    let mut parts = Vec::new();
    if let Some(lib) = state.library() {
        parts.push(trn("{n} game", "{n} games", lib.games.len(), &[]));
    }
    let tools = state.usage.as_ref().map_or(0, Vec::len);
    parts.push(trn("{n} tool", "{n} tools", tools, &[]));
    let (row, content) = row(icon, &l.title(), Some(&parts.join(" · ")), &l.to_string());
    if state.running {
        let dot = gtk::Image::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text(tr("Running", &[]))
            .build();
        dot.add_css_class("success");
        content.append(&dot);
    }
    row
}

/// A place of the app's own, such as its preferences.
pub fn place(icon: &str, title: &str, name: &str) -> gtk::ListBoxRow {
    row(icon, title, None, name).0
}

/// The error log's place, with how many errors there are beside it.
pub fn errors() -> (gtk::ListBoxRow, gtk::Label) {
    let (row, content) = row(
        "dialog-warning-symbolic",
        &tr("Errors", &[]),
        None,
        "errors",
    );
    let count = gtk::Label::builder().valign(gtk::Align::Center).build();
    count.add_css_class("count");
    content.append(&count);
    row.set_visible(false);
    (row, count)
}
