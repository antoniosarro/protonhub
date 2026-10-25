//! The MangoHud editor: the settings in mangohud.toml for one config
//! file, the global one (a place in the sidebar) or a game's (a page
//! over it), a tab per group, and a preview of them as they change.
//! MangoHud doesn't draw over GTK, so the preview is the real one over
//! vkcube, reading a copy of the settings that it reloads when the copy
//! changes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use protonhub_core::mangohud::{self, Config, Kind, Setting};
use protonhub_core::paths;

use crate::i18n::tr;
use crate::ui;
use crate::window::Window;

/// The global settings' editor, for the sidebar's place.
pub fn global(win: &Rc<Window>) -> gtk::Widget {
    match mangohud::global_path() {
        Some(path) => editor(win, path, None).upcast(),
        None => {
            let message = "no home directory: HOME isn't set";
            win.report(tr("Opening the MangoHud settings", &[]), message.into());
            let view = adw::ToolbarView::new();
            view.add_top_bar(&adw::HeaderBar::new());
            view.set_content(Some(&ui::failed(
                &tr("Opening the MangoHud settings", &[]),
                message,
            )));
            view.upcast()
        }
    }
}

/// A page that edits the settings in `path`, starting from `from`'s while
/// it doesn't exist yet.
pub fn page(
    win: &Rc<Window>,
    title: &str,
    path: PathBuf,
    from: Option<PathBuf>,
) -> adw::NavigationPage {
    adw::NavigationPage::builder()
        .title(title)
        .child(&editor(win, path, from))
        .build()
}

/// Edits the settings in `path`, starting from `from`'s while it doesn't
/// exist yet: a header bar with the tabs, Preview and Save, over the tab
/// shown.
fn editor(win: &Rc<Window>, path: PathBuf, from: Option<PathBuf>) -> adw::ToolbarView {
    let view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    view.add_top_bar(&header);
    let start = if path.exists() {
        &path
    } else {
        from.as_ref().unwrap_or(&path)
    };
    let config = match mangohud::read(start) {
        Ok(c) => c,
        Err(e) => {
            let reading = tr("Reading {file}", &[("file", &start.display().to_string())]);
            view.set_content(Some(&ui::failed(&reading, &e.to_string())));
            win.report(reading, e.to_string());
            return view;
        }
    };
    // What's in the file: a new one from `from` differs from it until saved.
    let saved = Rc::new(RefCell::new(if start == &path {
        config.to_string()
    } else {
        String::new()
    }));
    let config = Rc::new(RefCell::new(config));
    let preview_file = paths::cache_dir().map(|d| d.join("mangohud-preview.conf"));
    let preview: Rc<RefCell<Option<gio::Subprocess>>> = Rc::default();

    let save = gtk::Button::builder()
        .label(tr("Save", &[]))
        .sensitive(*saved.borrow() != config.borrow().to_string())
        .build();
    save.add_css_class("suggested-action");
    let show_preview = gtk::ToggleButton::builder()
        .label(tr("Preview", &[]))
        .sensitive(mangohud::can_preview() && preview_file.is_some())
        .tooltip_text(if mangohud::can_preview() {
            tr("MangoHud as it will look, over a spinning cube", &[])
        } else {
            tr("Needs MangoHud and vkcube (from vulkan-tools)", &[])
        })
        .build();
    header.pack_start(&show_preview);
    header.pack_end(&save);

    // After each change: Save, and the preview's copy.
    let changed: Rc<dyn Fn()> = Rc::new(glib::clone!(
        #[strong]
        config,
        #[strong]
        saved,
        #[strong]
        preview,
        #[strong]
        preview_file,
        #[weak]
        save,
        move || {
            let text = config.borrow().to_string();
            save.set_sensitive(text != *saved.borrow());
            if let (Some(_), Some(file)) = (&*preview.borrow(), &preview_file) {
                let _ = mangohud::write(file, &config.borrow());
            }
        }
    ));

    // A tab per group, a click apart: there are dozens of metrics, in
    // two columns.
    let stack = gtk::Stack::new();
    for group in mangohud::groups() {
        let rows: Vec<gtk::Widget> = group
            .setting
            .into_iter()
            .map(|s| row(Rc::new(s), &config, &changed))
            .collect();
        let half = rows.len().div_ceil(2);
        let columns: Vec<gtk::Widget> = rows
            .chunks(half.max(1))
            .map(|rows| {
                let g = adw::PreferencesGroup::new();
                for r in rows {
                    g.add(r);
                }
                g.upcast()
            })
            .collect();
        let (first, second) = columns.split_at(columns.len().min(1));
        let key = format!("mangohud/{}/{}", path.display(), group.title);
        let columns = ui::columns(first, second);
        let content = ui::scrolled(win, key, &columns, 1200, &[(&columns, ui::COLUMNS)]);
        stack.add_titled(&content, None::<&str>, &group.title);
    }
    header.set_title_widget(Some(&gtk::StackSwitcher::builder().stack(&stack).build()));
    view.set_content(Some(&stack));

    // Outside the macros below, where xgettext doesn't look.
    let done = tr("Saved the MangoHud settings", &[]);
    let writing = tr("Writing {file}", &[("file", &path.display().to_string())]);
    let starting = tr("Starting the preview", &[]);
    save.connect_clicked(glib::clone!(
        #[strong]
        win,
        #[strong]
        config,
        #[strong]
        saved,
        move |save| match mangohud::write(&path, &config.borrow()) {
            Ok(()) => {
                *saved.borrow_mut() = config.borrow().to_string();
                save.set_sensitive(false);
                win.toast(&done);
            }
            Err(e) => win.report(writing.clone(), e.to_string()),
        }
    ));
    show_preview.connect_toggled(glib::clone!(
        #[strong]
        win,
        #[strong]
        config,
        #[strong]
        preview,
        move |toggle| {
            if !toggle.is_active() {
                if let Some(p) = preview.take() {
                    p.force_exit();
                }
                return;
            }
            let Some(file) = &preview_file else { return };
            match start_preview(file, &config.borrow()) {
                Ok(p) => {
                    // Closing vkcube unpresses the button.
                    p.wait_async(
                        None::<&gio::Cancellable>,
                        glib::clone!(
                            #[weak]
                            toggle,
                            move |_| toggle.set_active(false)
                        ),
                    );
                    *preview.borrow_mut() = Some(p);
                }
                Err(e) => {
                    toggle.set_active(false);
                    win.report(starting.clone(), e.to_string());
                }
            }
        }
    ));
    // Gone with the editor: popped, or another place chosen.
    view.connect_unmap(glib::clone!(
        #[weak]
        show_preview,
        move |_| {
            show_preview.set_active(false);
            if let Some(p) = preview.take() {
                p.force_exit();
            }
        }
    ));
    view
}

/// Runs `mangohud vkcube` reading `file`, written from `config` first.
fn start_preview(file: &std::path::Path, config: &Config) -> Result<gio::Subprocess, String> {
    mangohud::write(file, config).map_err(|e| format!("{}: {e}", file.display()))?;
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
    launcher.setenv(mangohud::CONFIG_VAR, file, true);
    // It would win over the file.
    launcher.unsetenv("MANGOHUD_CONFIG");
    let mut argv = vec!["mangohud", "vkcube"];
    // vkcube draws through X11 unless told.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        argv.extend(["--wsi", "wayland"]);
    }
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(std::ffi::OsStr::new).collect();
    launcher.spawn(&argv).map_err(|e| e.to_string())
}

/// The row that shows and changes `setting`.
fn row(setting: Rc<Setting>, config: &Rc<RefCell<Config>>, changed: &Rc<dyn Fn()>) -> gtk::Widget {
    let title = glib::markup_escape_text(&setting.name);
    let subtitle = setting
        .description
        .as_deref()
        .map(glib::markup_escape_text)
        .unwrap_or_default();
    let value = setting.value(&config.borrow()).to_owned();
    // Sets the setting to what the row shows.
    let set = glib::clone!(
        #[strong]
        config,
        #[strong]
        changed,
        #[strong]
        setting,
        move |value: &str| {
            setting.set(&mut config.borrow_mut(), value);
            changed();
        }
    );
    match setting.kind {
        Kind::Switch => {
            let row = adw::SwitchRow::builder()
                .title(title)
                .subtitle(subtitle)
                .active(mangohud::is_on(&value))
                .build();
            row.connect_active_notify(move |r| set(if r.is_active() { "1" } else { "0" }));
            row.upcast()
        }
        Kind::Number => {
            let digits = if setting.step < 1.0 { 2 } else { 0 };
            let row = adw::SpinRow::with_range(setting.min, setting.max, setting.step);
            row.set_title(&title);
            row.set_subtitle(&subtitle);
            row.set_digits(digits);
            row.set_value(value.parse().unwrap_or_default());
            row.connect_value_notify(move |r| {
                set(&format!("{:.*}", digits as usize, r.value()));
            });
            row.upcast()
        }
        Kind::Choice => {
            let names: Vec<&str> = setting.choices.iter().map(String::as_str).collect();
            let row = adw::ComboRow::builder()
                .title(title)
                .subtitle(subtitle)
                .model(&gtk::StringList::new(&names))
                .build();
            let at = |v: &str| setting.choices.iter().position(|c| c == v);
            if let Some(i) = at(&value).or_else(|| at(&setting.default)) {
                row.set_selected(i as u32);
            }
            row.connect_selected_notify(glib::clone!(
                #[strong]
                setting,
                move |r| {
                    if let Some(choice) = setting.choices.get(r.selected() as usize) {
                        set(choice);
                    }
                }
            ));
            row.upcast()
        }
        Kind::Color => {
            let button = gtk::ColorDialogButton::builder()
                .dialog(&gtk::ColorDialog::builder().with_alpha(false).build())
                .valign(gtk::Align::Center)
                .build();
            if let Ok(rgba) = gdk::RGBA::parse(format!("#{}", value.trim_start_matches('#'))) {
                button.set_rgba(&rgba);
            }
            button.connect_rgba_notify(move |b| {
                let c = b.rgba();
                let byte = |v: f32| (v * 255.0).round() as u8;
                set(&format!(
                    "{:02X}{:02X}{:02X}",
                    byte(c.red()),
                    byte(c.green()),
                    byte(c.blue())
                ));
            });
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .activatable_widget(&button)
                .build();
            row.add_suffix(&button);
            row.upcast()
        }
        Kind::Text => {
            // Empty is the default, shown as the hint.
            let entry = gtk::Entry::builder()
                .text(config.borrow().get(&setting.key).unwrap_or_default())
                .placeholder_text(setting.default.as_str())
                .valign(gtk::Align::Center)
                .build();
            entry.connect_changed(move |e| set(&e.text()));
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(subtitle)
                .build();
            row.add_suffix(&entry);
            row.upcast()
        }
    }
}
