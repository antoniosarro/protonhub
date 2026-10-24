//! The error log: every failure with its full message, and a way to copy
//! it with what a bug report needs.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::i18n::tr;
use crate::window::Window;

pub struct Entry {
    pub time: String,
    pub operation: String,
    pub message: String,
}

impl Entry {
    pub fn new(operation: String, message: String) -> Self {
        let time = glib::DateTime::now_local()
            .and_then(|t| t.format("%Y-%m-%d %H:%M:%S"))
            .map(Into::into)
            .unwrap_or_default();
        Self {
            time,
            operation,
            message,
        }
    }

    /// The entry with the version and platform, for a bug report.
    pub fn diagnostics(&self) -> String {
        format!(
            "protonhub {} on {} {}\n{} {} failed:\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            self.time,
            self.operation,
            self.message
        )
    }
}

/// A row showing an error's message, selectable, with a copy button.
pub fn row(title: &str, message: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(message))
        .subtitle_selectable(true)
        .build();
    row.add_css_class("error");
    let diagnostics = Entry::new(title.to_owned(), message.to_owned()).diagnostics();
    row.add_suffix(&copy_button(diagnostics));
    row
}

fn copy_button(text: String) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text(tr("Copy diagnostics", &[]))
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button.connect_clicked(move |b| b.clipboard().set_text(&text));
    button
}

pub fn show(win: &Rc<Window>) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .valign(gtk::Align::Start)
        .build();
    list.add_css_class("boxed-list");
    for e in win.errors.borrow().iter().rev() {
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&format!(
                "{} · {}",
                e.operation, e.time
            )))
            .subtitle(glib::markup_escape_text(&e.message))
            .subtitle_selectable(true)
            .build();
        row.add_suffix(&copy_button(e.diagnostics()));
        list.append(&row);
    }

    let all: String = win
        .errors
        .borrow()
        .iter()
        .map(Entry::diagnostics)
        .collect::<Vec<_>>()
        .join("\n");
    let copy_all = gtk::Button::builder().label(tr("Copy All", &[])).build();
    copy_all.connect_clicked(move |b| b.clipboard().set_text(&all));
    let clear = gtk::Button::builder().label(tr("Clear", &[])).build();

    let header = adw::HeaderBar::new();
    header.pack_start(&clear);
    header.pack_end(&copy_all);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    if win.errors.borrow().is_empty() {
        copy_all.set_sensitive(false);
        clear.set_sensitive(false);
        view.set_content(Some(
            &adw::StatusPage::builder()
                .icon_name("emblem-ok-symbolic")
                .title(tr("No errors", &[]))
                .build(),
        ));
    } else {
        view.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&list)
                .build(),
        ));
    }
    let dialog = adw::Dialog::builder()
        .title(tr("Errors", &[]))
        .content_width(560)
        .content_height(480)
        .child(&view)
        .build();
    clear.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[weak]
        dialog,
        move |_| {
            win.errors_cleared();
            dialog.close();
        }
    ));
    dialog.present(Some(&win.window));
}
