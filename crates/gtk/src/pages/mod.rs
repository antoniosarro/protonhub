//! Pages pushed over the launchers' views, each with a back button: what
//! was a dialog is a page here. A page that shows what was read has a
//! [`Route`], from which the window builds it again after reading the
//! launchers again; the others stay as they are.

pub mod bulk_launch;
mod cleanup;
pub mod game;
pub mod installed;
pub mod mangohud;
pub mod move_games;
pub mod preferences;
mod tool;
pub mod tricks;

use std::path::PathBuf;
use std::rc::Rc;

use protonhub_core::launcher::Launcher;

use crate::window::Window;

/// What a page shows, to build it again with what was just read.
#[derive(Clone, PartialEq)]
pub enum Route {
    /// A game of the launcher's, by its id.
    Game { launcher: Launcher, id: String },
    /// A tool installed in the launcher's tool directories, or one of
    /// Valve's Protons, by its folder.
    Installed { launcher: Launcher, dir: PathBuf },
    /// A tool of the registry's the launcher takes, by its id: its
    /// releases and its updates.
    Tool { launcher: Launcher, tool: String },
    /// The launcher's tools that nothing uses, to delete.
    Cleanup { launcher: Launcher },
    /// What a Steam's uninstalled games left behind, to delete.
    Leftovers { launcher: Launcher },
}

/// The page `route` shows, with what was read last; none once what it
/// shows is gone.
pub fn build(win: &Rc<Window>, route: &Route) -> Option<adw::NavigationPage> {
    let (snapshot, Ok(env)) = (win.snapshot()?, &win.env) else {
        return None;
    };
    match route {
        Route::Game { launcher, id } => {
            let state = snapshot.state(launcher)?;
            let lib = state.library()?;
            let game = lib.games.iter().find(|g| g.id == *id)?;
            Some(game::page(win, env, state, lib, game))
        }
        Route::Installed { launcher, dir } => {
            installed::page(win, env, snapshot.state(launcher)?, dir)
        }
        Route::Tool { launcher, tool } => {
            let tool = env.registry.tools.values().find(|t| t.id == *tool)?;
            tool::page(win, env, launcher, tool)
        }
        Route::Cleanup { launcher } => cleanup::tools(win, env, snapshot.state(launcher)?),
        Route::Leftovers { launcher } => cleanup::leftovers(win, env, snapshot.state(launcher)?),
    }
}
