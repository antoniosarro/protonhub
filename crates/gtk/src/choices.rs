//! The tools a game, or a launcher's default, can be set to: what a list
//! of them offers and which one it shows chosen.

use protonhub_core::games::{Choice, Game, Library};
use protonhub_core::manifest::LauncherKind;

use crate::i18n::{tr, trn};

/// What a list of tools offers: each choice (none for the default), its
/// label, and the one chosen now.
pub struct Options {
    pub choices: Vec<Option<Choice>>,
    pub labels: Vec<String>,
    pub selected: Option<usize>,
}

/// The tools a game's list offers, the game's current one chosen.
pub fn for_game(kind: LauncherKind, lib: &Library, game: &Game) -> Options {
    // Steam's and Faugus's games can go back to the default; Heroic's
    // always name one.
    let mut choices: Vec<Option<Choice>> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    if matches!(kind, LauncherKind::Steam | LauncherKind::Faugus) {
        choices.push(None);
        labels.push(match &lib.default {
            Some(d) => tr("Default ({tool})", &[("tool", &d.name)]),
            None => tr("Default (none)", &[]),
        });
    }
    for c in &lib.choices {
        choices.push(Some(c.clone()));
        labels.push(c.name.clone());
    }
    let current = game.tool.as_ref().map(|t| t.id.as_str());
    let mut selected = choices
        .iter()
        .position(|o| match o {
            None => !game.explicit,
            Some(c) => game.explicit && Some(c.id.as_str()) == current,
        })
        .or_else(|| {
            // Heroic games on the default name it like any other tool.
            choices
                .iter()
                .position(|o| o.as_ref().map(|c| c.id.as_str()) == current)
        });
    if selected.is_none()
        && let Some(tool) = &game.tool
    {
        // A tool the launcher doesn't list, like a system Wine.
        choices.push(Some(tool.clone()));
        labels.push(tool.name.clone());
        selected = Some(choices.len() - 1);
    }
    Options {
        choices,
        labels,
        selected,
    }
}

/// The launcher's default tool among its tools, or for Steam none, which
/// turns Steam Play off.
pub fn for_default(kind: LauncherKind, lib: &Library) -> Options {
    // A default that isn't installed any more is offered too, as it is.
    let mut choices: Vec<Option<Choice>> = lib.choices.iter().cloned().map(Some).collect();
    if let Some(d) = &lib.default
        && !lib.choices.iter().any(|c| c.id == d.id)
    {
        choices.push(Some(d.clone()));
    }
    if kind == LauncherKind::Steam {
        choices.insert(0, None);
    }
    let labels = choices
        .iter()
        .map(|o| match o {
            Some(c) => c.name.clone(),
            None => tr("None: Steam Play off", &[]),
        })
        .collect();
    let selected = choices
        .iter()
        .position(|o| o.as_ref().map(|c| &c.id) == lib.default.as_ref().map(|d| &d.id));
    Options {
        choices,
        labels,
        selected,
    }
}

/// The tools games are set to, each with how many are, and its label;
/// games on the default aren't counted.
pub fn in_use(lib: &Library) -> Vec<(Choice, String)> {
    let mut counts: Vec<(Choice, usize)> = Vec::new();
    for tool in lib
        .games
        .iter()
        .filter(|g| g.explicit)
        .filter_map(|g| g.tool.as_ref())
    {
        match counts.iter_mut().find(|(c, _)| c.id == tool.id) {
            Some((_, n)) => *n += 1,
            None => counts.push((tool.clone(), 1)),
        }
    }
    counts
        .into_iter()
        .map(|(c, n)| {
            let label = trn(
                "{tool} ({n} game)",
                "{tool} ({n} games)",
                n,
                &[("tool", &c.name)],
            );
            (c, label)
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/choices.rs"]
mod tests;
