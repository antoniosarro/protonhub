//! The main window: a sidebar of launchers with the job running in the
//! background, each launcher's page, and the error log.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib::prelude::StaticVariantType;
use gtk::{gio, glib};
use protonhub_core::channels::{self, Channel, Channels};
use protonhub_core::games;
use protonhub_core::info;
use protonhub_core::install::{self, Step};
use protonhub_core::launch;
use protonhub_core::launcher::{self, Launcher};
use protonhub_core::manifest::{Arch, LauncherKind, Registry, Tool};
use protonhub_core::paths;
use protonhub_core::releases::{self, Client, Version};
use protonhub_core::steam;
use protonhub_core::storage;

use crate::i18n::{tr, trn};
use crate::{errors, page};

/// What the window works with; loading it can fail, and then the window
/// shows why instead of tabs.
pub struct Env {
    pub registry: Arc<Registry>,
    pub client: Arc<Client>,
    pub home: PathBuf,
    pub downloads: PathBuf,
    pub arch: Arch,
}

impl Env {
    fn load() -> Result<Self, String> {
        let registry = Registry::load_dir(&paths::registry_dir()).map_err(|e| e.to_string())?;
        let home = paths::home().ok_or("HOME is not set")?;
        let cache = paths::cache_dir().ok_or("neither XDG_CACHE_HOME nor HOME is set")?;
        let arch = Arch::host().ok_or("this CPU architecture has no builds")?;
        let token = std::env::var("GITHUB_TOKEN").ok();
        Ok(Self {
            registry: Arc::new(registry),
            client: Arc::new(Client::new(releases::API, cache.clone(), token)),
            home,
            downloads: cache.join("downloads"),
            arch,
        })
    }
}

/// What the tabs show, read on a worker thread: finding sizes walks
/// gigabytes of files.
pub struct Snapshot {
    pub launchers: Vec<LauncherState>,
}

pub struct LauncherState {
    pub launcher: Launcher,
    pub usage: Result<Vec<games::Usage>, String>,
    /// None for a launcher without per-game tools.
    pub library: Result<Option<games::Library>, String>,
    pub running: bool,
    /// ProtonDB's rating and the anti-cheat status of each game, by id.
    pub info: Vec<(String, info::Info)>,
    /// Why some of that couldn't be read; the rest is still there.
    pub info_error: Option<String>,
    /// Valve's Protons, in a Steam, with what uses them.
    pub officials: Result<Vec<games::OfficialUsage>, String>,
    /// A Steam's prefixes and shader caches; none for another launcher.
    pub storage: Result<Option<storage::Storage>, String>,
    /// Each game's launch options, by id; none where they can't be read.
    pub launch: Result<Option<Vec<(String, String)>>, String>,
}

impl Snapshot {
    fn read(registry: &Registry, home: &Path, client: &Client) -> Self {
        let found = launcher::detect(home);
        // One list for every launcher; it's cached for a day.
        let anticheat = info::AntiCheatList::fetch(client, info::AWACY).map_err(|e| e.to_string());
        let officials = games::officials(&found).map_err(|e| e.to_string());
        let mut usage = match games::usage(registry, &found) {
            Ok(u) => u.into_iter().map(|(_, u)| Ok(u)).collect(),
            Err(e) => vec![Err(e.to_string()); found.len()],
        };
        let launchers = found
            .into_iter()
            .map(|l| {
                let library = games::library(&l).map_err(|e| e.to_string());
                let mut info_error = None;
                let mut fail = |e: String| {
                    info_error.get_or_insert(e);
                };
                let ac = anticheat.as_ref().map_err(|e| fail(e.clone())).ok();
                let info = match &library {
                    Ok(Some(lib)) => lib
                        .games
                        .iter()
                        .map(|g| {
                            let protondb = info::protondb(client, info::PROTONDB, g)
                                .map_err(|e| fail(e.to_string()))
                                .ok()
                                .flatten();
                            let anticheat = ac.and_then(|ac| ac.find(g));
                            (
                                g.id.clone(),
                                info::Info {
                                    protondb,
                                    anticheat,
                                },
                            )
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                LauncherState {
                    usage: usage.remove(0),
                    library,
                    running: games::is_running(&l),
                    storage: storage::scan(&l).map_err(|e| e.to_string()),
                    launch: launch::all(&l).map_err(|e| e.to_string()),
                    info,
                    info_error,
                    officials: match &officials {
                        Ok(all) => Ok(all
                            .iter()
                            .find(|(o, _)| *o == l)
                            .map(|(_, found)| found.clone())
                            .unwrap_or_default()),
                        Err(e) => Err(e.clone()),
                    },
                    launcher: l,
                }
            })
            .collect();
        Self { launchers }
    }
}

/// The running job's card at the bottom of the sidebar. One job at a
/// time: two installs into the same directory would race for its staging
/// area.
struct JobCard {
    revealer: gtk::Revealer,
    title: gtk::Label,
    step: gtk::Label,
    bar: gtk::ProgressBar,
}

impl JobCard {
    fn new() -> Self {
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        title.add_css_class("heading");
        let step = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        step.add_css_class("caption");
        step.add_css_class("dim-label");
        let bar = gtk::ProgressBar::new();
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        card.add_css_class("job-card");
        card.append(&title);
        card.append(&step);
        card.append(&bar);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .child(&card)
            .build();
        Self {
            revealer,
            title,
            step,
            bar,
        }
    }

    fn start(&self, title: &str) {
        self.title.set_label(title);
        self.step.set_label("");
        self.bar.set_fraction(0.0);
        self.revealer.set_reveal_child(true);
    }

    fn show(&self, step: Step) {
        let (what, fraction) = match step {
            Step::Resolve => (tr("Finding the release", &[]), None),
            Step::Download { done, total } => (tr("Downloading", &[]), Some((done, total))),
            Step::Verify => (tr("Verifying the checksum", &[]), None),
            Step::Extract { done, total } => (tr("Extracting", &[]), Some((done, total))),
        };
        match fraction {
            Some((done, 0)) => {
                // GitLab doesn't say how big an asset is.
                self.bar.pulse();
                self.step.set_label(&format!("{what} · {}", mib(done)));
            }
            Some((done, total)) => {
                self.bar.set_fraction(done as f64 / total as f64);
                self.step.set_label(&tr(
                    "{step} · {done} of {total}",
                    &[
                        ("step", &what),
                        ("done", &mib(done)),
                        ("total", &mib(total)),
                    ],
                ));
            }
            None => {
                self.bar.pulse();
                self.step.set_label(&what);
            }
        }
    }
}

pub struct Window {
    pub window: adw::ApplicationWindow,
    split: adw::NavigationSplitView,
    content: adw::NavigationPage,
    sidebar: gtk::ListBox,
    /// One page per launcher, by its name, or why there are none.
    pages: gtk::Stack,
    toasts: adw::ToastOverlay,
    errors_button: gtk::Button,
    job: JobCard,
    busy: Cell<bool>,
    /// The view (games or tools) launcher pages open on, kept across redraws.
    pub view: RefCell<Option<String>>,
    /// The game whose page is shown over its launcher's, kept across
    /// redraws.
    pub game: RefCell<Option<(Launcher, String)>>,
    pub errors: RefCell<Vec<errors::Entry>>,
    pub env: Result<Env, String>,
    snapshot: RefCell<Option<Rc<Snapshot>>>,
    /// Counts reloads, so an older one finishing late is dropped.
    generation: Cell<u64>,
}

impl Window {
    pub fn new(app: &adw::Application) -> Rc<Self> {
        let refresh = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text(tr("Look for launchers and tools again (Ctrl+R)", &[]))
            .build();
        let errors_button = gtk::Button::builder()
            .icon_name("dialog-warning-symbolic")
            .visible(false)
            .build();
        errors_button.add_css_class("error");
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.pack_start(&refresh);
        sidebar_header.pack_end(&errors_button);

        let sidebar = gtk::ListBox::new();
        sidebar.add_css_class("navigation-sidebar");
        let job = JobCard::new();
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        sidebar_view.set_content(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&sidebar)
                .vexpand(true)
                .build(),
        ));
        sidebar_view.add_bottom_bar(&job.revealer);

        let pages = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        let content = adw::NavigationPage::builder()
            .title("ProtonHub")
            .child(&pages)
            .build();
        let split = adw::NavigationSplitView::builder()
            .sidebar(&adw::NavigationPage::new(&sidebar_view, "ProtonHub"))
            .content(&content)
            .min_sidebar_width(220.0)
            .max_sidebar_width(280.0)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("ProtonHub")
            .default_width(1040)
            .default_height(700)
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
            content,
            sidebar,
            pages,
            toasts,
            errors_button: errors_button.clone(),
            job,
            busy: Cell::new(false),
            view: RefCell::default(),
            game: RefCell::default(),
            errors: RefCell::default(),
            env: Env::load(),
            snapshot: RefCell::default(),
            generation: Cell::new(0),
        });
        win.sidebar.connect_row_selected(glib::clone!(
            #[strong]
            win,
            move |_, row| {
                let Some(row) = row else { return };
                win.pages.set_visible_child_name(&row.widget_name());
                if let Some(title) = row.tooltip_text() {
                    win.content.set_title(&title);
                }
            }
        ));
        win.sidebar.connect_row_activated(glib::clone!(
            #[strong]
            win,
            move |_, _| win.split.set_show_content(true)
        ));
        // The window lives as long as the app, so its actions can hold it.
        let refresh_action = gio::ActionEntry::builder("refresh")
            .activate(glib::clone!(
                #[strong]
                win,
                move |_: &adw::ApplicationWindow, _, _| win.reload()
            ))
            .build();
        let errors_action = gio::ActionEntry::builder("errors")
            .activate(glib::clone!(
                #[strong]
                win,
                move |_: &adw::ApplicationWindow, _, _| errors::show(&win)
            ))
            .build();
        // Alt+1…9 shows the nth launcher.
        let tab_action = gio::ActionEntry::builder("tab")
            .parameter_type(Some(&i32::static_variant_type()))
            .activate(glib::clone!(
                #[strong]
                win,
                move |_: &adw::ApplicationWindow, _, n| {
                    let n = n.and_then(|v| v.get::<i32>()).unwrap_or(0);
                    if let Some(row) = win.sidebar.row_at_index(n) {
                        win.sidebar.select_row(Some(&row));
                    }
                }
            ))
            .build();
        win.window
            .add_action_entries([refresh_action, errors_action, tab_action]);
        for n in 0..9 {
            app.set_accels_for_action(&format!("win.tab({n})"), &[&format!("<Alt>{}", n + 1)]);
        }
        refresh.set_action_name(Some("win.refresh"));
        errors_button.set_action_name(Some("win.errors"));
        app.set_accels_for_action("win.refresh", &["<Control>r", "F5"]);
        app.set_accels_for_action("win.errors", &["<Control>e"]);
        app.set_accels_for_action("window.close", &["<Control>q", "<Control>w"]);
        win.reload();
        win
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

    /// Shows the launcher `delta` away from the current one, wrapping around.
    pub fn switch_tab(&self, delta: i32) {
        let mut n = 0;
        while self.sidebar.row_at_index(n).is_some() {
            n += 1;
        }
        if n == 0 {
            return;
        }
        let current = self.sidebar.selected_row().map_or(0, |r| r.index());
        let next = (current + delta).rem_euclid(n);
        if let Some(row) = self.sidebar.row_at_index(next) {
            self.sidebar.select_row(Some(&row));
        }
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
                    if first && let Some(row) = win.sidebar.selected_row() {
                        row.grab_focus();
                    }
                }
            }
        ));
    }

    /// Shows a page in place of the launchers: loading, or why there are none.
    fn status(&self, child: &impl IsA<gtk::Widget>) {
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(child));
        self.pages.add_named(&view, Some("status"));
        self.content.set_title("ProtonHub");
    }

    /// Rebuilds the sidebar and every launcher's page from the last
    /// snapshot, keeping the launcher shown.
    pub fn redraw(self: &Rc<Self>) {
        let shown = self.sidebar.selected_row().map(|r| r.widget_name());
        self.sidebar.remove_all();
        while let Some(child) = self.pages.first_child() {
            self.pages.remove(&child);
        }
        let env = match &self.env {
            Ok(env) => env,
            Err(e) => {
                self.status(
                    &adw::StatusPage::builder()
                        .icon_name("dialog-error-symbolic")
                        .title(tr("ProtonHub can't start", &[]))
                        .description(glib::markup_escape_text(e))
                        .build(),
                );
                return;
            }
        };
        let Some(snapshot) = self.snapshot.borrow().clone() else {
            self.status(
                &adw::Spinner::builder()
                    .width_request(32)
                    .height_request(32)
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .build(),
            );
            return;
        };
        if snapshot.launchers.is_empty() {
            self.status(
                &adw::StatusPage::builder()
                    .icon_name("applications-games-symbolic")
                    .title(tr("No launchers found", &[]))
                    .description(tr(
                        "ProtonHub looks for Steam, Heroic, Lutris, Bottles and WineZGUI, native, Flatpak or Snap.",
                        &[],
                    ))
                    .build(),
            );
            return;
        }
        for state in &snapshot.launchers {
            let name = state.launcher.to_string();
            self.pages
                .add_named(&page::build(self, env, state), Some(&name));
            self.sidebar.append(&sidebar_row(state));
        }
        let row = shown
            .and_then(|name| {
                let mut i = 0;
                while let Some(row) = self.sidebar.row_at_index(i) {
                    if row.widget_name() == name {
                        return Some(row);
                    }
                    i += 1;
                }
                None
            })
            .or_else(|| self.sidebar.row_at_index(0));
        self.sidebar.select_row(row.as_ref());
    }

    /// Goes back from a game's page to its launcher's.
    pub fn back(&self) {
        if let Some(nav) = self
            .pages
            .visible_child()
            .and_downcast::<adw::NavigationView>()
        {
            nav.pop();
        }
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
        self.errors_button.set_visible(true);
        self.errors_button
            .set_tooltip_text(Some(&trn("{n} error", "{n} errors", n, &[])));
        let toast = adw::Toast::builder()
            .title(tr("{operation} failed", &[("operation", &operation)]))
            .button_label(tr("Details", &[]))
            .timeout(8)
            .build();
        toast.connect_button_clicked(glib::clone!(
            #[strong(rename_to = win)]
            self,
            move |_| errors::show(&win)
        ));
        self.toasts.add_toast(toast);
    }

    pub fn errors_cleared(&self) {
        self.errors.borrow_mut().clear();
        self.errors_button.set_visible(false);
    }

    pub fn busy(&self) -> bool {
        self.busy.get()
    }

    pub fn install(self: &Rc<Self>, tool: &Tool, version: Version, launcher: Launcher) {
        let Ok(env) = &self.env else { return };
        let title = match &version {
            Version::Latest => tr("Installing the latest {tool}", &[("tool", &tool.name)]),
            Version::Tag(t) => tr(
                "Installing {version}",
                &[("version", &named(&tool.name, t))],
            ),
        };
        let (client, downloads, arch) = (env.client.clone(), env.downloads.clone(), env.arch);
        let (owned, target) = (tool.clone(), launcher.clone());
        let restart = launcher.kind == LauncherKind::Steam;
        self.run(title, move |progress| {
            let entry = install::install(
                &client, &owned, &version, &target, arch, &downloads, progress,
            )
            .map_err(|e| e.to_string())?;
            let version = entry.record.map(|r| r.version).unwrap_or_default();
            let version = named(&owned.name, &version);
            let missing = install::missing(&owned);
            Ok(if !missing.is_empty() {
                tr(
                    "Installed {version}; it needs {programs}, which isn't installed",
                    &[("version", &version), ("programs", &missing.join(", "))],
                )
            } else if restart {
                tr(
                    "Installed {version}; restart Steam to use it",
                    &[("version", &version)],
                )
            } else {
                tr("Installed {version}", &[("version", &version)])
            })
        });
    }

    /// Deletes the tool in `dir`, whoever installed it and whatever uses it.
    pub fn delete(self: &Rc<Self>, dir: PathBuf, name: String) {
        let title = tr("Deleting {tool}", &[("tool", &name)]);
        self.run(title, move |_| {
            install::delete(&dir).map_err(|e| e.to_string())?;
            Ok(tr("Deleted {tool}", &[("tool", &name)]))
        });
    }

    /// Uninstalls one of Valve's Protons from `launcher`, a Steam, as Steam
    /// does; refused if Steam started meanwhile.
    pub fn uninstall(
        self: &Rc<Self>,
        launcher: &Launcher,
        official: steam::Official,
        name: String,
    ) {
        let title = tr("Deleting {tool}", &[("tool", &name)]);
        let launcher = launcher.clone();
        self.run(title, move |_| {
            if games::is_running(&launcher) {
                return Err(tr("Steam is running: quit it first", &[]));
            }
            steam::uninstall(&official).map_err(|e| e.to_string())?;
            Ok(tr("Deleted {tool}", &[("tool", &name)]))
        });
    }

    /// Deletes the tools in `dirs`, which nothing uses.
    pub fn clean_up(self: &Rc<Self>, dirs: Vec<PathBuf>, freed: u64) {
        let title = trn(
            "Deleting {n} unused tool",
            "Deleting {n} unused tools",
            dirs.len(),
            &[],
        );
        self.run(title, move |_| {
            for dir in &dirs {
                install::delete(dir).map_err(|e| e.to_string())?;
            }
            Ok(tr("Freed {size}", &[("size", &install::human_size(freed))]))
        });
    }

    /// Deletes prefixes and shader caches of `launcher`, a Steam; refused
    /// if Steam started meanwhile.
    pub fn delete_storage(self: &Rc<Self>, launcher: &Launcher, dirs: Vec<PathBuf>, freed: u64) {
        let title = trn(
            "Deleting {n} folder",
            "Deleting {n} folders",
            dirs.len(),
            &[],
        );
        let launcher = launcher.clone();
        self.run(title, move |_| {
            let dirs: Vec<_> = dirs.iter().map(PathBuf::as_path).collect();
            storage::delete(&launcher, &dirs).map_err(|e| e.to_string())?;
            Ok(tr("Freed {size}", &[("size", &install::human_size(freed))]))
        });
    }

    /// Updates one channel now, as the timer would.
    pub fn update_channel(self: &Rc<Self>, tool: &Tool, channel: Channel) {
        let Ok(env) = &self.env else { return };
        let title = tr("Updating {tool}", &[("tool", &tool.name)]);
        let (client, registry, home, downloads, arch) = (
            env.client.clone(),
            env.registry.clone(),
            env.home.clone(),
            env.downloads.clone(),
            env.arch,
        );
        let name = tool.name.clone();
        self.run(title, move |progress| {
            let launchers = launcher::detect(&home);
            let cx = channels::Context {
                client: &client,
                registry: &registry,
                launchers: &launchers,
                arch,
                downloads: &downloads,
            };
            let u = channels::update(&cx, &channel, progress).map_err(|e| e.to_string())?;
            let mut did = Vec::new();
            let latest = named(&name, &u.latest);
            if u.installed.is_some() {
                did.push(tr("installed {version}", &[("version", &latest)]));
            }
            if !u.moved.is_empty() {
                did.push(trn(
                    "moved {n} game onto it",
                    "moved {n} games onto it",
                    u.moved.len(),
                    &[],
                ));
            }
            if !u.removed.is_empty() {
                did.push(trn(
                    "deleted {n} old version",
                    "deleted {n} old versions",
                    u.removed.len(),
                    &[],
                ));
            }
            did.extend(u.notes);
            Ok(if did.is_empty() {
                tr("{version} is up to date", &[("version", &latest)])
            } else {
                capitalized(&did.join("; "))
            })
        });
    }

    /// How a channel names `launcher` among the launchers found.
    pub fn channel_spec(&self, launcher: &Launcher) -> String {
        let snapshot = self.snapshot.borrow();
        let found = snapshot.iter().flat_map(|s| &s.launchers);
        launcher.spec(found.map(|s| &s.launcher))
    }

    /// The channels file, read fresh: the CLI and the timer change it too.
    pub fn channels(self: &Rc<Self>) -> Channels {
        let Some(path) = Channels::path() else {
            return Channels::default();
        };
        Channels::load(&path).unwrap_or_else(|e| {
            self.report(tr("Reading the channels", &[]), e.to_string());
            Channels::default()
        })
    }

    /// Changes the channels file with `change`, and reports a failure.
    pub fn edit_channels(self: &Rc<Self>, change: impl FnOnce(&mut Channels)) {
        let Some(path) = Channels::path() else {
            self.report(
                tr("Saving the channels", &[]),
                "neither XDG_CONFIG_HOME nor HOME is set".into(),
            );
            return;
        };
        let mut channels = self.channels();
        change(&mut channels);
        if let Err(e) = channels.save(&path) {
            self.report(tr("Saving the channels", &[]), e.to_string());
        }
    }

    /// Moves every game set to the tool `from` to `to`.
    pub fn reassign(
        self: &Rc<Self>,
        launcher: &Launcher,
        from: &games::Choice,
        to: &games::Choice,
    ) {
        let exact = regex::Regex::new(&format!("^{}$", regex::escape(&from.id)))
            .expect("an escaped id is a regex");
        match games::reassign(launcher, &exact, &to.id) {
            Ok(moved) => self.toast(&trn(
                "Moved {n} game to {tool}",
                "Moved {n} games to {tool}",
                moved.len(),
                &[("tool", &to.name)],
            )),
            Err(e) => self.report(
                tr("Moving games to {tool}", &[("tool", &to.name)]),
                e.to_string(),
            ),
        }
        self.reload();
    }

    /// Sets `game`'s launch options to `text`; empty removes them.
    pub fn set_launch(self: &Rc<Self>, launcher: &Launcher, game: &games::Game, text: &str) {
        match launch::set(launcher, &game.id, text) {
            Ok(()) => self.toast(&if text.is_empty() {
                tr(
                    "{game} now launches without options",
                    &[("game", &game.name)],
                )
            } else {
                tr(
                    "Saved the launch options of {game}",
                    &[("game", &game.name)],
                )
            }),
            Err(e) => self.report(
                tr(
                    "Setting the launch options of {game}",
                    &[("game", &game.name)],
                ),
                e.to_string(),
            ),
        }
        self.reload();
    }

    /// Sets the tool `game` runs with; quick enough for the main thread.
    pub fn assign(
        self: &Rc<Self>,
        launcher: &Launcher,
        game: &games::Game,
        tool: Option<&games::Choice>,
    ) {
        match games::assign(launcher, &game.id, tool.map(|c| c.id.as_str())) {
            Ok(()) => {
                self.toast(&match tool {
                    Some(c) => tr(
                        "{game} now runs with {tool}",
                        &[("game", &game.name), ("tool", &c.name)],
                    ),
                    None => tr("{game} now runs with the default", &[("game", &game.name)]),
                });
            }
            Err(e) => self.report(
                tr("Setting the tool of {game}", &[("game", &game.name)]),
                e.to_string(),
            ),
        }
        // Usage changed, or the row must show the old choice again.
        self.reload();
    }

    /// Runs `work` on a worker thread with its progress in the sidebar,
    /// then toasts its result or logs its error, and reloads the launchers.
    fn run(
        self: &Rc<Self>,
        title: String,
        work: impl FnOnce(&mut dyn FnMut(Step)) -> Result<String, String> + Send + 'static,
    ) {
        if self.busy() {
            self.toast(&tr("Wait for the current job to finish", &[]));
            return;
        }
        self.busy.set(true);
        self.job.start(&title);
        // Buttons that would start another job are disabled until it ends.
        self.redraw();

        enum Msg {
            Step(Step),
            Done(Result<String, String>),
        }
        let (tx, rx) = async_channel::unbounded();
        std::thread::spawn(move || {
            // Only steps the window can show: a new step or percentage.
            let mut last = None;
            let mut progress = |step: Step| {
                let key = match step {
                    Step::Download { done, total } | Step::Extract { done, total } => {
                        // Percent, or MiB of an unknown total.
                        let at = (done * 100).checked_div(total).unwrap_or(done >> 20);
                        (std::mem::discriminant(&step), at)
                    }
                    _ => (std::mem::discriminant(&step), 0),
                };
                if last != Some(key) {
                    last = Some(key);
                    let _ = tx.send_blocking(Msg::Step(step));
                }
            };
            let result = work(&mut progress);
            let _ = tx.send_blocking(Msg::Done(result));
        });
        glib::spawn_future_local(glib::clone!(
            #[strong(rename_to = win)]
            self,
            async move {
                while let Ok(msg) = rx.recv().await {
                    match msg {
                        Msg::Step(step) => win.job.show(step),
                        Msg::Done(result) => {
                            win.busy.set(false);
                            win.job.revealer.set_reveal_child(false);
                            match result {
                                Ok(done) => win.toast(&done),
                                Err(e) => win.report(title.clone(), e),
                            }
                            win.reload();
                        }
                    }
                }
            }
        ));
    }
}

/// A sidebar row: the launcher's icon and name, and what it has.
fn sidebar_row(state: &LauncherState) -> gtk::ListBoxRow {
    let l = &state.launcher;
    let icon = gtk::Image::from_icon_name(match l.kind {
        LauncherKind::Steam => "applications-games-symbolic",
        LauncherKind::Heroic => "starred-symbolic",
        LauncherKind::Lutris => "applications-system-symbolic",
        LauncherKind::Bottles => "package-x-generic-symbolic",
        LauncherKind::Winezgui => "application-x-executable-symbolic",
    });
    let name = gtk::Label::builder()
        .label(l.title())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let tools = state.usage.as_ref().map_or(0, Vec::len);
    let mut parts = Vec::new();
    if let Ok(Some(lib)) = &state.library {
        parts.push(trn("{n} game", "{n} games", lib.games.len(), &[]));
    }
    parts.push(trn("{n} tool", "{n} tools", tools, &[]));
    let what = gtk::Label::builder()
        .label(parts.join(" · "))
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    what.add_css_class("caption");
    what.add_css_class("dim-label");
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.append(&name);
    text.append(&what);
    let row_box = gtk::Box::builder()
        .spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    row_box.append(&icon);
    row_box.append(&text);
    let row = gtk::ListBoxRow::builder()
        .child(&row_box)
        .name(l.to_string())
        .tooltip_text(l.title())
        .build();
    if state.running {
        let dot = gtk::Image::builder()
            .icon_name("media-playback-start-symbolic")
            .tooltip_text(tr("Running", &[]))
            .hexpand(true)
            .halign(gtk::Align::End)
            .build();
        dot.add_css_class("success");
        row_box.append(&dot);
    }
    row
}

/// `text` with its first letter capitalized.
pub fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A version as people read it: the tag alone when it names the tool
/// (GE-Proton11-7), else after the tool's name (DXVK v3.1.1).
pub fn named(tool: &str, version: &str) -> String {
    if version.contains(tool) {
        version.to_owned()
    } else {
        format!("{tool} {version}")
    }
}

fn mib(bytes: u64) -> String {
    format!("{:.0} MiB", bytes as f64 / (1024.0 * 1024.0))
}
