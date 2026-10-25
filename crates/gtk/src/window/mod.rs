//! The main window: a sidebar of the launchers found and the app's own
//! places, with the job running at its bottom; beside it, a navigation
//! view whose root shows what the sidebar has chosen, and over which
//! pages are pushed, each with a back button.

mod actions;
mod jobs;
mod nav;
mod sidebar;
mod snapshot;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib::prelude::StaticVariantType;
use gtk::{gio, glib};
use protonhub_core::launcher::Launcher;
use protonhub_core::manifests;
use protonhub_core::paths;
use protonhub_core::releases::Release;

pub use snapshot::{Env, LauncherState, Snapshot};

use crate::errors;
use crate::i18n::{tr, trn};
use crate::pages::Route;

/// The app's own places in the sidebar, by the name of the view each shows.
const MANGOHUD: &str = "mangohud";
const PREFERENCES: &str = "preferences";
const ERRORS: &str = "errors";
/// The view shown while there are no launchers to: loading, or why.
const STATUS: &str = "status";

pub struct Window {
    pub window: adw::ApplicationWindow,
    split: adw::NavigationSplitView,
    launchers: gtk::ListBox,
    places: gtk::ListBox,
    errors_row: gtk::ListBoxRow,
    errors_count: gtk::Label,
    /// The pages pushed over `root`, which shows `views`.
    nav: adw::NavigationView,
    root: adw::NavigationPage,
    /// A view per launcher, by its name, and the places'.
    views: gtk::Stack,
    /// The name of the view chosen in the sidebar.
    shown: RefCell<String>,
    /// The pages pushed, each with what it shows where a redraw builds it
    /// again with what was just read.
    pushed: RefCell<Vec<(Option<Route>, adw::NavigationPage)>>,
    /// Set while the launchers are drawn again, so choosing the one shown
    /// again keeps the pages over it.
    redrawing: Cell<bool>,
    toasts: adw::ToastOverlay,
    job: jobs::JobCard,
    busy: Cell<bool>,
    /// The tab (games, tools or storage) launcher views open on.
    pub tab: RefCell<Option<String>>,
    pub errors: RefCell<Vec<errors::Entry>>,
    /// The Steams ProtonHub quit, to offer starting again.
    pub quit: RefCell<Vec<Launcher>>,
    pub env: Result<Env, String>,
    snapshot: RefCell<Option<Rc<Snapshot>>>,
    /// Counts reloads, so an older one finishing late is dropped.
    generation: Cell<u64>,
    /// Where views were scrolled to, and what was searched in them, by
    /// key, to show again after a redraw.
    pub scrolls: RefCell<HashMap<String, f64>>,
    pub searches: RefCell<HashMap<String, String>>,
    /// Each tool's releases, by its id, as listed this session.
    pub releases: RefCell<HashMap<String, Vec<Release>>>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(tr("Look for launchers and tools again (Ctrl+R)", &[]))
            .action_name("win.refresh")
            .build();
        let sidebar_header = adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new("ProtonHub", ""))
            .build();
        sidebar_header.pack_end(&refresh);

        let launchers = gtk::ListBox::new();
        launchers.add_css_class("navigation-sidebar");
        // Chosen only when activated: a row of a list that selects takes
        // the focus at the start, and chooses itself with it.
        let places = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        places.add_css_class("navigation-sidebar");
        places.append(&sidebar::place(
            "video-display-symbolic",
            &tr("MangoHud", &[]),
            MANGOHUD,
        ));
        places.append(&sidebar::place(
            "emblem-system-symbolic",
            &tr("Preferences", &[]),
            PREFERENCES,
        ));
        let (errors_row, errors_count) = sidebar::errors();
        places.append(&errors_row);
        let lists = gtk::Box::new(gtk::Orientation::Vertical, 0);
        lists.append(&launchers);
        lists.append(&gtk::Box::builder().vexpand(true).build());
        lists.append(&places);
        let job = jobs::JobCard::new();
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        sidebar_view.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&lists)
                .vexpand(true)
                .build(),
        ));
        sidebar_view.add_bottom_bar(&job.revealer);

        let views = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let root = adw::NavigationPage::builder()
            .title("ProtonHub")
            .child(&views)
            .build();
        let nav = adw::NavigationView::new();
        nav.add(&root);
        let split = adw::NavigationSplitView::builder()
            .sidebar(&adw::NavigationPage::new(&sidebar_view, "ProtonHub"))
            .content(&adw::NavigationPage::new(&nav, "ProtonHub"))
            .min_sidebar_width(220.0)
            .max_sidebar_width(260.0)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("ProtonHub")
            .default_width(1240)
            .default_height(780)
            .width_request(360)
            .height_request(294)
            .content(&toasts)
            .build();
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse("max-width: 640sp").expect("a valid condition"),
        );
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        // For style.css, which has no media queries.
        narrow.connect_apply(glib::clone!(
            #[weak]
            window,
            move |_| window.add_css_class("narrow")
        ));
        narrow.connect_unapply(glib::clone!(
            #[weak]
            window,
            move |_| window.remove_css_class("narrow")
        ));
        window.add_breakpoint(narrow);

        let win = Rc::new(Self {
            window,
            split,
            launchers,
            places,
            errors_row,
            errors_count,
            nav,
            root,
            views,
            shown: RefCell::default(),
            pushed: RefCell::default(),
            redrawing: Cell::new(false),
            toasts,
            job,
            busy: Cell::new(false),
            tab: RefCell::default(),
            errors: RefCell::default(),
            quit: RefCell::default(),
            env: Env::load(),
            snapshot: RefCell::default(),
            generation: Cell::new(0),
            scrolls: RefCell::default(),
            searches: RefCell::default(),
            releases: RefCell::default(),
        });
        jobs::connect_cancel(&win);
        win.connect_navigation();
        win.add_actions(app);
        win.reload();
        // The repository's tool manifests, for the next start; offline or
        // failing, the ones there are stay.
        if let (Ok(env), Some(cache), true) = (&win.env, paths::cache_dir(), manifests::used()) {
            let client = env.client.clone();
            std::thread::spawn(move || manifests::update(&client, &cache, false));
        }
        win
    }

    /// The window's actions, and their keys.
    fn add_actions(self: &Rc<Self>, app: &adw::Application) {
        let action = |name: &str, run: fn(&Rc<Self>)| {
            gio::ActionEntry::builder(name)
                .activate(glib::clone!(
                    #[strong(rename_to = win)]
                    self,
                    move |_: &adw::ApplicationWindow, _, _| run(&win)
                ))
                .build()
        };
        // Alt+1…9 shows the nth launcher.
        let tab = gio::ActionEntry::builder("tab")
            .parameter_type(Some(&i32::static_variant_type()))
            .activate(glib::clone!(
                #[strong(rename_to = win)]
                self,
                move |_: &adw::ApplicationWindow, _, n| {
                    let n = n.and_then(|v| v.get::<i32>()).unwrap_or(0);
                    if let Some(row) = win.launchers.row_at_index(n) {
                        win.launchers.select_row(Some(&row));
                    }
                }
            ))
            .build();
        self.window.add_action_entries([
            action("refresh", |win| win.reload()),
            action("errors", |win| win.choose_place(ERRORS)),
            action("mangohud", |win| win.choose_place(MANGOHUD)),
            action("preferences", |win| win.choose_place(PREFERENCES)),
            tab,
        ]);
        for n in 0..9 {
            app.set_accels_for_action(&format!("win.tab({n})"), &[&format!("<Alt>{}", n + 1)]);
        }
        app.set_accels_for_action("win.refresh", &["<Control>r", "F5"]);
        app.set_accels_for_action("win.errors", &["<Control>e"]);
        app.set_accels_for_action("win.mangohud", &["<Control>m"]);
        app.set_accels_for_action("win.preferences", &["<Control>comma"]);
        app.set_accels_for_action("window.close", &["<Control>q", "<Control>w"]);
    }

    /// Shows the window: fullscreen in the Steam Deck's Game Mode
    /// (gamescope), where there's nothing else on screen.
    pub fn present(&self) {
        let gamescope = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d == "gamescope")
            || std::env::var_os("SteamDeck").is_some_and(|v| v == "1");
        if gamescope {
            self.window.fullscreen();
        }
        self.window.present();
    }

    /// Reads the launchers again on a worker thread, then redraws.
    pub fn reload(self: &Rc<Self>) {
        let Ok(env) = &self.env else {
            self.redraw();
            return;
        };
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if self.snapshot.borrow().is_none() {
            self.redraw();
        }
        let (registry, home, client) = (env.registry.clone(), env.home.clone(), env.client.clone());
        let read = gio::spawn_blocking(move || Snapshot::read(&registry, &home, &client));
        glib::spawn_future_local(glib::clone!(
            #[strong(rename_to = win)]
            self,
            async move {
                let Ok(snapshot) = read.await else {
                    win.report("Reading the launchers".into(), "the reader crashed".into());
                    return;
                };
                if win.generation.get() == generation {
                    let first = win.snapshot.replace(Some(Rc::new(snapshot))).is_none();
                    win.redraw();
                    // Somewhere for the keyboard and gamepad to start from.
                    if first && let Some(row) = win.launchers.selected_row() {
                        row.grab_focus();
                    }
                }
            }
        ));
    }

    /// What was last read, if anything has been.
    pub fn snapshot(&self) -> Option<Rc<Snapshot>> {
        self.snapshot.borrow().clone()
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Logs a failure and says so in a toast that leads to the log.
    pub fn report(self: &Rc<Self>, operation: String, message: String) {
        self.errors
            .borrow_mut()
            .push(errors::Entry::new(operation.clone(), message));
        let n = self.errors.borrow().len();
        self.errors_row.set_visible(true);
        self.errors_count.set_label(&n.to_string());
        self.errors_row
            .set_tooltip_text(Some(&trn("{n} error", "{n} errors", n, &[])));
        if *self.shown.borrow() == ERRORS {
            self.show(ERRORS, &tr("Errors", &[]));
        }
        let toast = adw::Toast::builder()
            .title(tr("{operation} failed", &[("operation", &operation)]))
            .button_label(tr("Details", &[]))
            .timeout(8)
            .build();
        toast.connect_button_clicked(glib::clone!(
            #[strong(rename_to = win)]
            self,
            move |_| win.choose_place(ERRORS)
        ));
        self.toasts.add_toast(toast);
    }

    /// Empties the error log, and goes back to the launchers from it.
    pub fn errors_cleared(self: &Rc<Self>) {
        self.errors.borrow_mut().clear();
        self.errors_row.set_visible(false);
        match self.launchers.row_at_index(0) {
            Some(row) => self.launchers.select_row(Some(&row)),
            None => {
                self.mark_place(None);
                self.show(STATUS, "ProtonHub");
            }
        }
    }
}
