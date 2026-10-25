//! Changes quick enough for the main thread: a game's tool or launch
//! options, a launcher's default, the channels file. Each reports how it
//! went and reads the launchers again.

use std::rc::Rc;

use protonhub_core::channels::Channels;
use protonhub_core::games::{self, Choice, Game};
use protonhub_core::launch;
use protonhub_core::launcher::Launcher;
use protonhub_core::steam;

use super::Window;
use crate::i18n::{tr, trn};

impl Window {
    /// Toasts what `result` did, or logs why it failed doing `operation`;
    /// then reads the launchers again, to show the change or, failed, what
    /// there was before.
    fn finish<E: ToString>(self: &Rc<Self>, operation: String, result: Result<String, E>) {
        match result {
            Ok(done) => self.toast(&done),
            Err(e) => self.report(operation, e.to_string()),
        }
        self.reload();
    }

    /// Sets the tool `game` runs with, or with none the default.
    pub fn assign(self: &Rc<Self>, launcher: &Launcher, game: &Game, tool: Option<&Choice>) {
        let result =
            games::assign(launcher, &game.id, tool.map(|c| c.id.as_str())).map(|()| match tool {
                Some(c) => tr(
                    "{game} now runs with {tool}",
                    &[("game", &game.name), ("tool", &c.name)],
                ),
                None => tr("{game} now runs with the default", &[("game", &game.name)]),
            });
        self.finish(
            tr("Setting the tool of {game}", &[("game", &game.name)]),
            result,
        );
    }

    /// Sets the launcher's default tool, or with none turns Steam Play off
    /// for games without one of their own.
    pub fn set_default(self: &Rc<Self>, launcher: &Launcher, tool: Option<&Choice>) {
        let result =
            games::set_default(launcher, tool.map(|c| c.id.as_str())).map(|()| match tool {
                Some(c) => tr(
                    "Games without a tool of their own now run with {tool}",
                    &[("tool", &c.name)],
                ),
                None => tr(
                    "Steam Play is off for games without a tool of their own",
                    &[],
                ),
            });
        self.finish(tr("Setting the default tool", &[]), result);
    }

    /// Moves every game set to the tool `from` to `to`.
    pub fn reassign(self: &Rc<Self>, launcher: &Launcher, from: &Choice, to: &Choice) {
        let exact = regex::Regex::new(&format!("^{}$", regex::escape(&from.id)))
            .expect("an escaped id is a regex");
        let result = games::reassign(launcher, &exact, &to.id).map(|moved| {
            trn(
                "Moved {n} game to {tool}",
                "Moved {n} games to {tool}",
                moved.len(),
                &[("tool", &to.name)],
            )
        });
        self.finish(tr("Moving games to {tool}", &[("tool", &to.name)]), result);
    }

    /// Sets `game`'s launch options to `text`; empty removes them.
    pub fn set_launch(self: &Rc<Self>, launcher: &Launcher, game: &Game, text: &str) {
        let result = launch::set(launcher, &game.id, text).map(|()| {
            if text.is_empty() {
                tr(
                    "{game} now launches without options",
                    &[("game", &game.name)],
                )
            } else {
                tr(
                    "Saved the launch options of {game}",
                    &[("game", &game.name)],
                )
            }
        });
        self.finish(
            tr(
                "Setting the launch options of {game}",
                &[("game", &game.name)],
            ),
            result,
        );
    }

    /// Sets several games' launch options at once; `skipped` games had
    /// options that couldn't be read, and were left alone.
    pub fn set_launch_many(
        self: &Rc<Self>,
        launcher: &Launcher,
        changes: &[(String, String)],
        skipped: usize,
    ) {
        let pairs: Vec<(&str, &str)> = changes
            .iter()
            .map(|(id, text)| (id.as_str(), text.as_str()))
            .collect();
        let result = launch::set_many(launcher, &pairs).map(|()| {
            let mut text = trn(
                "Changed the launch options of {n} game",
                "Changed the launch options of {n} games",
                changes.len(),
                &[],
            );
            if skipped > 0 {
                text.push_str("; ");
                text.push_str(&trn(
                    "left {n} alone: its options have an unclosed quote",
                    "left {n} alone: their options have an unclosed quote",
                    skipped,
                    &[],
                ));
            }
            text
        });
        self.finish(tr("Changing launch options", &[]), result);
    }

    /// Renames a Steam shortcut and sets what it runs and where.
    pub fn edit_shortcut(self: &Rc<Self>, launcher: &Launcher, game: &Game, to: &steam::Shortcut) {
        let result = games::edit_shortcut(launcher, &game.id, &to.name, &to.target, &to.start_dir)
            .map(|()| tr("Saved {game}", &[("game", &to.name)]));
        self.finish(tr("Changing {game}", &[("game", &game.name)]), result);
    }

    /// Starts `launcher`, a Steam ProtonHub quit.
    pub fn start_steam(self: &Rc<Self>, launcher: &Launcher) {
        self.quit.borrow_mut().retain(|l| l != launcher);
        let result = steam::start(launcher).map(|()| tr("Starting Steam", &[]));
        self.finish(tr("Starting Steam", &[]), result);
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
}
