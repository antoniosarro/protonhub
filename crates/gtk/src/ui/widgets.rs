//! The small pieces views and pages are made of: labels, rows, buttons,
//! lists to choose from, pictures, and the words for paths.

use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::art;
use crate::i18n::tr;
use crate::window::{Env, Window};

/// A fact as an icon and a few words, colored by `class`.
pub fn chip(icon: &str, text: &str, class: &str, tooltip: Option<&str>) -> gtk::Widget {
    let chip = gtk::Box::builder().spacing(6).build();
    chip.add_css_class("chip");
    if !class.is_empty() {
        chip.add_css_class(class);
    }
    chip.append(&gtk::Image::from_icon_name(icon));
    chip.append(&gtk::Label::new(Some(text)));
    chip.set_valign(gtk::Align::Center);
    chip.set_tooltip_text(tooltip);
    chip.upcast()
}

/// A small rounded label, colored by `class`.
pub fn tag(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .build();
    label.add_css_class("tag");
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}

/// A line of text, left-aligned and ellipsized past its width.
pub fn line(text: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// A boxed list, with `rows` in it.
pub fn boxed(rows: &[gtk::Widget]) -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .valign(gtk::Align::Start)
        .build();
    list.add_css_class("boxed-list");
    for row in rows {
        list.append(row);
    }
    list
}

/// A dimmed row that says there's nothing to show.
pub fn dim_row(title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(title))
        .subtitle(glib::markup_escape_text(subtitle))
        .build();
    row.add_css_class("dim-label");
    row
}

pub fn banner(text: &str) -> adw::Banner {
    adw::Banner::builder().title(text).revealed(true).build()
}

pub fn status(icon: &str, title: &str, description: &str) -> gtk::Widget {
    adw::StatusPage::builder()
        .icon_name(icon)
        .title(title)
        .description(glib::markup_escape_text(description))
        .build()
        .upcast()
}

pub fn failed(what: &str, message: &str) -> gtk::Widget {
    status("dialog-error-symbolic", what, message)
}

/// A button with an icon and a label.
pub fn button(icon: &str, label: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder()
        .icon_name(icon)
        .label(label)
        .can_shrink(true)
        .build();
    // Its image, aligned to the label's baseline, reports one outside
    // itself where nothing beside the button has one.
    if let Some(image) = content.first_child() {
        image.set_valign(gtk::Align::Center);
    }
    gtk::Button::builder()
        .child(&content)
        .valign(gtk::Align::Center)
        .build()
}

/// A flat button with only an icon.
pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");
    button
}

/// A button that opens `dir` in the file manager.
pub fn open_button(win: &Rc<Window>, dir: &Path) -> gtk::Button {
    let open = icon_button("folder-open-symbolic", &tr("Open the folder", &[]));
    let dir = dir.to_owned();
    open.connect_clicked(glib::clone!(
        #[weak]
        win,
        move |_| {
            gtk::FileLauncher::new(Some(&gio::File::for_path(&dir))).launch(
                Some(&win.window),
                None::<&gio::Cancellable>,
                |_| {},
            );
        }
    ));
    open
}

/// Opens `uri` in the browser.
pub fn open_uri(win: &Window, uri: &str) {
    gtk::UriLauncher::new(uri).launch(Some(&win.window), None::<&gio::Cancellable>, |_| {});
}

/// Asks before deleting: `heading`, `body`, and a destructive `verb` that
/// does `then`.
pub fn confirm(win: &Rc<Window>, heading: &str, body: &str, verb: &str, then: impl Fn() + 'static) {
    let alert = adw::AlertDialog::new(Some(heading), Some(body));
    alert.add_responses(&[("cancel", &tr("Cancel", &[])), ("delete", verb)]);
    alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    alert.set_default_response(Some("cancel"));
    alert.set_close_response("cancel");
    alert.connect_response(Some("delete"), move |_, _| then());
    alert.present(Some(&win.window));
}

/// Marks `widget` as holding text that can't be used.
pub fn mark(widget: &impl IsA<gtk::Widget>, error: bool) {
    if error {
        widget.add_css_class("error");
    } else {
        widget.remove_css_class("error");
    }
}

/// A list of `labels` to choose from.
pub fn strings(labels: &[String]) -> gtk::StringList {
    gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())
}

/// Shows a list of strings as labels; with `chars`, as wide as that many
/// characters at most, ellipsized past it, so the buttons that show them
/// don't grow with the longest.
pub fn labels_factory(chars: Option<i32>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, item| {
        let label = gtk::Label::builder().xalign(0.0).build();
        if let Some(chars) = chars {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_width_chars(chars.min(8));
            label.set_max_width_chars(chars);
        }
        item.downcast_ref::<gtk::ListItem>()
            .expect("a list item")
            .set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a list item");
        if let (Some(label), Some(text)) = (
            item.child().and_downcast::<gtk::Label>(),
            item.item().and_downcast::<gtk::StringObject>(),
        ) {
            label.set_label(&text.string());
        }
    });
    factory
}

/// A drop-down of `labels`, `selected` chosen, its button no wider than
/// `chars` characters.
pub fn drop_down(labels: &[String], selected: Option<usize>, chars: i32) -> gtk::DropDown {
    let drop = gtk::DropDown::builder()
        .model(&strings(labels))
        .factory(&labels_factory(Some(chars)))
        .list_factory(&labels_factory(None))
        .valign(gtk::Align::Center)
        .build();
    if let Some(i) = selected {
        drop.set_selected(i as u32);
    }
    drop
}

/// A game's picture, from a file or a URL, decoded `width` pixels wide,
/// over an icon that shows where it has none or until it loads. `class`
/// sizes it.
pub fn art(env: &Env, source: Option<&str>, width: i32, class: &str) -> gtk::Overlay {
    let frame = gtk::Overlay::builder()
        .overflow(gtk::Overflow::Hidden)
        .build();
    frame.add_css_class(class);
    // Centered: filling the frame, it reports a baseline outside itself.
    frame.set_child(Some(
        &gtk::Image::builder()
            .icon_name("applications-games-symbolic")
            .valign(gtk::Align::Center)
            .build(),
    ));
    if let Some(source) = source {
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .build();
        frame.add_overlay(&picture);
        let weak = picture.downgrade();
        art::load(&env.client, source, width, move |t| {
            if let Some(p) = weak.upgrade() {
                p.set_paintable(Some(t));
            }
        });
    }
    frame
}

pub fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// `path` with the home directory shown as `~`.
pub fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}
