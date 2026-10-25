//! The error log: every failure with its full message, and a way to copy
//! it with what a bug report needs.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::i18n::tr;
use crate::ui;
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

/// The error log, a place in the sidebar: every failure, newest first.
pub fn view(win: &Rc<Window>) -> gtk::Widget {
    let copy_all = gtk::Button::builder().label(tr("Copy All", &[])).build();
    let clear = gtk::Button::builder().label(tr("Clear", &[])).build();
    let header = adw::HeaderBar::new();
    header.pack_start(&clear);
    header.pack_end(&copy_all);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    let errors = win.errors.borrow();
    if errors.is_empty() {
        copy_all.set_sensitive(false);
        clear.set_sensitive(false);
        view.set_content(Some(&ui::status(
            "emblem-ok-symbolic",
            &tr("No errors", &[]),
            "",
        )));
        return view.upcast();
    }
    let rows: Vec<gtk::Widget> = errors
        .iter()
        .rev()
        .map(|e| {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&format!(
                    "{} · {}",
                    e.operation, e.time
                )))
                .subtitle(glib::markup_escape_text(&e.message))
                .subtitle_selectable(true)
                .build();
            row.add_suffix(&copy_button(e.diagnostics()));
            row.upcast()
        })
        .collect();
    let all: String = errors
        .iter()
        .map(Entry::diagnostics)
        .collect::<Vec<_>>()
        .join("\n");
    copy_all.connect_clicked(move |b| b.clipboard().set_text(&all));
    clear.connect_clicked(glib::clone!(
        #[weak]
        win,
        move |_| win.errors_cleared()
    ));
    view.set_content(Some(&ui::scrolled(
        win,
        "errors".into(),
        &ui::boxed(&rows),
        1200,
        &[],
    )));
    view.upcast()
}
