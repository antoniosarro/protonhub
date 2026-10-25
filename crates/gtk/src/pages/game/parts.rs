//! The parts a game's launch options are made of, to edit one by one: a
//! row per variable, the programs the game runs in, and its arguments,
//! each kept in step with the text.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use protonhub_core::launch::{self, Options};

use crate::i18n::tr;
use crate::ui::mark;

/// Adds the parts and `text`, the launch options as text, to `expander`;
/// they follow the text, as do the expander's summary and `save`. Enter in
/// any field does `on_enter`.
pub(super) fn add(
    expander: &adw::ExpanderRow,
    text: &adw::EntryRow,
    save: &gtk::Button,
    on_enter: &Rc<dyn Fn()>,
    running: bool,
) {
    let (text, on_enter) = (text.clone(), on_enter.clone());
    let current = text.text().to_string();
    let current = current.as_str();
    // A row per variable, its name and value side by side; then the
    // programs it runs in and its arguments, each as words.
    let lines = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let add = gtk::Button::builder()
        .label(tr("Add Variable", &[]))
        .halign(gtk::Align::Start)
        .build();
    add.add_css_class("flat");
    let vars = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(9)
        .margin_bottom(6)
        .margin_start(12)
        .margin_end(12)
        .build();
    vars.append(&lines);
    vars.append(&add);
    let vars_row = gtk::ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .child(&vars)
        .build();
    let programs = adw::EntryRow::builder()
        .title(tr("Programs it runs in", &[]))
        .build();
    let args = adw::EntryRow::builder().title(tr("Arguments", &[])).build();
    for row in [&programs, &args, &text] {
        row.connect_entry_activated(glib::clone!(
            #[strong]
            on_enter,
            move |_| on_enter()
        ));
    }
    expander.add_row(&vars_row);
    expander.add_row(&programs);
    expander.add_row(&args);
    expander.add_row(&text);

    // Set while the parts are written to the text or read from it, so
    // neither writes back.
    let editing = Rc::new(Cell::new(false));
    let pairs: Rc<RefCell<Vec<(gtk::Box, gtk::Entry, gtk::Entry)>>> = Rc::default();
    let compose: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[strong]
        editing,
        #[strong]
        pairs,
        #[weak]
        text,
        #[weak]
        programs,
        #[weak]
        args,
        move || {
            if editing.get() {
                return;
            }
            let Some(mut options) = Options::parse(&text.text()) else {
                return;
            };
            // A pair without a name yet is left out, one with a name no
            // shell would set is marked.
            options.env = pairs
                .borrow()
                .iter()
                .filter_map(|(_, key, value)| {
                    let name = key.text();
                    let valid = name.is_empty() || launch::is_name(&name);
                    mark(key, !valid);
                    (valid && !name.is_empty()).then(|| (name.into(), value.text().into()))
                })
                .collect();
            for (row, words) in [
                (&programs, &mut options.wrappers),
                (&args, &mut options.args),
            ] {
                let split = launch::split(&row.text());
                mark(row, split.is_none());
                match split {
                    Some(split) => *words = split,
                    None => return,
                }
            }
            editing.set(true);
            text.set_text(&options.to_string());
            editing.set(false);
        }
    ));
    // Outside the macro below, where xgettext doesn't look.
    let hints = [tr("Name", &[]), tr("Value", &[]), tr("Remove", &[])];
    let add_pair: AddPair = Rc::new(glib::clone!(
        #[strong]
        pairs,
        #[strong]
        compose,
        #[strong]
        on_enter,
        #[weak]
        lines,
        #[upgrade_or_panic]
        move |name: &str, value: &str| {
            let [name_hint, value_hint, remove_hint] = &hints;
            let key = gtk::Entry::builder()
                .text(name)
                .placeholder_text(name_hint.as_str())
                .hexpand(true)
                .build();
            let val = gtk::Entry::builder()
                .text(value)
                .placeholder_text(value_hint.as_str())
                .hexpand(true)
                .build();
            let remove = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text(remove_hint.as_str())
                .build();
            remove.add_css_class("flat");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.append(&key);
            line.append(&gtk::Label::new(Some("=")));
            line.append(&val);
            line.append(&remove);
            for entry in [&key, &val] {
                entry.connect_changed(glib::clone!(
                    #[strong]
                    compose,
                    move |_| compose()
                ));
                entry.connect_activate(glib::clone!(
                    #[strong]
                    on_enter,
                    move |_| on_enter()
                ));
            }
            remove.connect_clicked(glib::clone!(
                #[strong]
                pairs,
                #[strong]
                compose,
                #[weak]
                lines,
                #[weak]
                line,
                move |_| {
                    lines.remove(&line);
                    pairs.borrow_mut().retain(|(l, _, _)| *l != line);
                    compose();
                }
            ));
            lines.append(&line);
            pairs.borrow_mut().push((line, key.clone(), val));
            key
        }
    ));
    add.connect_clicked(glib::clone!(
        #[strong]
        add_pair,
        move |_| {
            add_pair("", "").grab_focus();
        }
    ));
    for row in [&programs, &args] {
        row.connect_changed(glib::clone!(
            #[strong]
            compose,
            move |_| compose()
        ));
    }
    // The parts, the summary and Save follow the text, however it changed.
    let none = tr("None", &[]);
    let saved = current.to_owned();
    let refresh = glib::clone!(
        #[strong]
        editing,
        #[strong]
        pairs,
        #[weak]
        lines,
        #[weak]
        vars_row,
        #[weak]
        programs,
        #[weak]
        args,
        #[weak]
        expander,
        #[weak]
        save,
        move |t: &str| {
            expander.set_subtitle(&glib::markup_escape_text(if t.is_empty() {
                &none
            } else {
                t
            }));
            save.set_sensitive(!running && t != saved);
            if editing.get() {
                return;
            }
            let options = Options::parse(t);
            for row in [
                vars_row.upcast_ref::<gtk::Widget>(),
                programs.upcast_ref(),
                args.upcast_ref(),
            ] {
                row.set_sensitive(!running && options.is_some());
            }
            editing.set(true);
            for (line, _, _) in pairs.take() {
                lines.remove(&line);
            }
            let o = options.unwrap_or_default();
            for (name, value) in &o.env {
                add_pair(name, value);
            }
            programs.set_text(&launch::join(&o.wrappers));
            args.set_text(&launch::join(&o.args));
            mark(&programs, false);
            mark(&args, false);
            editing.set(false);
        }
    );
    refresh(current);
    text.connect_changed(move |t| {
        mark(t, Options::parse(&t.text()).is_none());
        refresh(&t.text());
    });
}

/// Adds a variable's row with its name and value, and gives its name's
/// entry.
type AddPair = Rc<dyn Fn(&str, &str) -> gtk::Entry>;
