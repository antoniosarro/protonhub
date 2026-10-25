//! Jobs: what runs on a worker thread, one at a time, with its progress
//! in a card at the bottom of the sidebar. Two installs into the same
//! directory would race for its staging area.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use adw::prelude::*;
use gtk::{gio, glib};
use protonhub_core::channels::{self, Channel};
use protonhub_core::games;
use protonhub_core::install::{self, Step};
use protonhub_core::launcher::{self, Launcher};
use protonhub_core::manifest::{LauncherKind, Tool};
use protonhub_core::prefix::{self, Prefix};
use protonhub_core::releases::Version;
use protonhub_core::steam;
use protonhub_core::storage;

use super::Window;
use crate::i18n::{tr, trn};
use crate::text::{capitalized, mib, named};

/// A flag for a download job's Cancel button to set, and the button's
/// action.
fn cancel_button() -> (Arc<AtomicBool>, Rc<dyn Fn()>) {
    let cancel = Arc::new(AtomicBool::new(false));
    let stop: Rc<dyn Fn()> = Rc::new({
        let cancel = cancel.clone();
        move || cancel.store(true, Ordering::SeqCst)
    });
    (cancel, stop)
}

/// The running job's card at the bottom of the sidebar. One job at a
/// time: two installs into the same directory would race for its staging
/// area.
pub(super) struct JobCard {
    pub(super) revealer: gtk::Revealer,
    title: gtk::Label,
    step: gtk::Label,
    bar: gtk::ProgressBar,
    /// Shown for a job that can be stopped.
    cancel: gtk::Button,
    stop: RefCell<Option<Rc<dyn Fn()>>>,
}

impl JobCard {
    pub(super) fn new() -> Self {
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
        let cancel = gtk::Button::builder()
            .label(tr("Cancel", &[]))
            .halign(gtk::Align::End)
            .visible(false)
            .build();
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
        card.append(&cancel);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .child(&card)
            .build();
        Self {
            revealer,
            title,
            step,
            bar,
            cancel,
            stop: RefCell::default(),
        }
    }

    fn start(&self, title: &str, stop: Option<Rc<dyn Fn()>>) {
        self.title.set_label(title);
        self.step.set_label("");
        self.bar.set_fraction(0.0);
        self.cancel.set_visible(stop.is_some());
        self.cancel.set_sensitive(true);
        *self.stop.borrow_mut() = stop;
        self.revealer.set_reveal_child(true);
    }

    fn show(&self, step: Step) {
        let (what, fraction) = match step {
            Step::Resolve => (tr("Finding the release", &[]), None),
            Step::Download { done, total } => (tr("Downloading", &[]), Some((done, total))),
            Step::Verify => (tr("Verifying the checksum", &[]), None),
            Step::Extract { done, total } => (tr("Extracting", &[]), Some((done, total))),
            Step::Output(line) => (line, None),
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

/// Makes the job card's Cancel button stop the job running.
pub(super) fn connect_cancel(win: &Rc<Window>) {
    let cancelling = tr("Cancelling…", &[]);
    // The window lives as long as the app, so its button can hold it.
    win.job.cancel.connect_clicked(glib::clone!(
        #[strong]
        win,
        move |button| {
            let Some(stop) = win.job.stop.borrow().clone() else {
                return;
            };
            button.set_sensitive(false);
            win.job.step.set_label(&cancelling);
            stop();
        }
    ));
}

impl Window {
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
        let (cancel, stop) = cancel_button();
        let client = env.client.cancellable(cancel.clone());
        let (downloads, arch) = (env.downloads.clone(), env.arch);
        let (owned, target) = (tool.clone(), launcher.clone());
        let restart = launcher.kind == LauncherKind::Steam;
        let stopped = tr("Stopped installing {tool}", &[("tool", &tool.name)]);
        self.run_stoppable(title, Some(stop), move |progress| {
            let entry = match install::install(
                &client, &owned, &version, &target, arch, &downloads, progress,
            ) {
                Ok(entry) => entry,
                Err(_) if cancel.load(Ordering::SeqCst) => return Ok(stopped),
                Err(e) => return Err(e.to_string()),
            };
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

    /// Quits `launcher`, a Steam, so its games can be changed; its page
    /// then offers to start it again.
    pub fn quit_steam(self: &Rc<Self>, launcher: &Launcher) {
        self.quit.borrow_mut().push(launcher.clone());
        let launcher = launcher.clone();
        self.run(tr("Quitting Steam", &[]), move |_| {
            steam::quit(&launcher).map_err(|e| e.to_string())?;
            Ok(tr("Steam quit: change its games, then start it again", &[]))
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
        let (cancel, stop) = cancel_button();
        let stopped = tr("Stopped updating {tool}", &[("tool", &tool.name)]);
        let (client, registry, home, downloads, arch) = (
            env.client.cancellable(cancel.clone()),
            env.registry.clone(),
            env.home.clone(),
            env.downloads.clone(),
            env.arch,
        );
        let name = tool.name.clone();
        self.run_stoppable(title, Some(stop), move |progress| {
            let launchers = launcher::detect(&home);
            let cx = channels::Context {
                client: &client,
                registry: &registry,
                launchers: &launchers,
                arch,
                downloads: &downloads,
            };
            let u = match channels::update(&cx, &channel, progress) {
                Ok(u) => u,
                Err(_) if cancel.load(Ordering::SeqCst) => return Ok(stopped),
                Err(e) => return Err(e.to_string()),
            };
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

    /// Installs winetricks `verbs` into `game`'s prefix, as a job that
    /// shows what winetricks prints and can be cancelled.
    pub fn tricks(self: &Rc<Self>, game: &games::Game, prefix: Prefix, verbs: Vec<String>) {
        let names = verbs.join(", ");
        let title = tr(
            "Installing {verbs} into {game}",
            &[("verbs", &names), ("game", &game.name)],
        );
        let done = tr(
            "Installed {verbs} into {game}",
            &[("verbs", &names), ("game", &game.name)],
        );
        let stopped = tr("Stopped installing into {game}", &[("game", &game.name)]);
        // The run's process group, once started, and whether it was stopped.
        let pid = Arc::new(AtomicU32::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop: Rc<dyn Fn()> = Rc::new({
            let (pid, cancelled, prefix) = (pid.clone(), cancelled.clone(), prefix.clone());
            move || {
                cancelled.store(true, Ordering::SeqCst);
                let (pid, prefix) = (pid.load(Ordering::SeqCst), prefix.clone());
                // wineserver can take a moment; not on the main thread.
                if pid != 0 {
                    std::thread::spawn(move || prefix::cancel(&prefix, pid));
                }
            }
        });
        self.run_stoppable(title, Some(stop), move |progress| {
            let cmd = prefix.winetricks(&verbs).map_err(|e| e.to_string())?;
            let ran = prefix::run(cmd, |id| pid.store(id, Ordering::SeqCst), &mut |line| {
                progress(Step::Output(line.to_owned()))
            });
            match ran {
                Ok(()) => Ok(done),
                Err(_) if cancelled.load(Ordering::SeqCst) => Ok(stopped),
                Err(e) => Err(e.to_string()),
            }
        });
    }

    /// Runs `work` on a worker thread with its progress in the sidebar,
    /// then toasts its result or logs its error, and reloads the launchers.
    fn run(
        self: &Rc<Self>,
        title: String,
        work: impl FnOnce(&mut dyn FnMut(Step)) -> Result<String, String> + Send + 'static,
    ) {
        self.run_stoppable(title, None, work);
    }

    /// `run`, with a Cancel button that calls `stop` while it runs.
    fn run_stoppable(
        self: &Rc<Self>,
        title: String,
        stop: Option<Rc<dyn Fn()>>,
        work: impl FnOnce(&mut dyn FnMut(Step)) -> Result<String, String> + Send + 'static,
    ) {
        if self.busy() {
            self.toast(&tr("Wait for the current job to finish", &[]));
            return;
        }
        self.busy.set(true);
        self.job.start(&title, stop);
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
                // Every line, as it comes.
                if let Step::Output(_) = step {
                    let _ = tx.send_blocking(Msg::Step(step));
                    return;
                }
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
                            win.job.stop.take();
                            win.job.revealer.set_reveal_child(false);
                            // Away from the window, its end is a notification
                            // too.
                            if !win.window.is_active()
                                && let Some(app) = win.window.application()
                            {
                                let body = match &result {
                                    Ok(done) => done.as_str(),
                                    Err(e) => e.lines().next().unwrap_or_default(),
                                };
                                let note = gio::Notification::new(&title);
                                note.set_body(Some(body));
                                app.send_notification(Some("job"), &note);
                            }
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
