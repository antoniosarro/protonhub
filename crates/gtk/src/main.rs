//! The GUI: a thin layer over protonhub-core. A sidebar of the launchers
//! found; each one's view shows its games as cards, with the tool each
//! runs with, and its tools, to install and delete; details open as pages
//! over it. Anything that fails lands in the error log.

mod art;
mod choices;
mod errors;
mod gamepad;
mod i18n;
mod pages;
mod text;
mod ui;
mod views;
mod window;

use adw::prelude::*;

const APP_ID: &str = "dev.antoniosarro.ProtonHub";

fn main() -> gtk::glib::ExitCode {
    i18n::init();
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("style.css"));
        gtk::style_context_add_provider_for_display(
            &gtk::gdk::Display::default().expect("a display to show the window on"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(|app| {
        let win = window::Window::new(app);
        gamepad::attach(&win);
        win.present();
    });
    app.run()
}
