//! What the views say about things, in words: ratings, sources, sizes,
//! who uses a tool.

use protonhub_core::games::{Deck, DeckRating, Usage};
use protonhub_core::storage::{Item, Kind};

use crate::i18n::{tr, trn};

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

pub fn mib(bytes: u64) -> String {
    format!("{:.0} MiB", bytes as f64 / (1024.0 * 1024.0))
}

/// A ProtonDB tier as people read it.
pub fn tier_name(tier: &str) -> String {
    match tier {
        "platinum" => tr("Platinum", &[]),
        "gold" => tr("Gold", &[]),
        "silver" => tr("Silver", &[]),
        "bronze" => tr("Bronze", &[]),
        "borked" => tr("Borked", &[]),
        "pending" => tr("Pending", &[]),
        other => capitalized(other),
    }
}

/// An anti-cheat status's style class: how it fares on Linux.
pub fn anticheat_class(status: &str) -> &'static str {
    match status {
        "Supported" | "Running" => "success",
        "Planned" => "warning",
        _ => "error",
    }
}

/// A Deck rating's style class and name.
pub fn deck_rating(rating: DeckRating) -> (&'static str, String) {
    match rating {
        DeckRating::Verified => ("success", tr("Verified", &[])),
        DeckRating::Playable => ("warning", tr("Playable", &[])),
        DeckRating::Unsupported => ("error", tr("Unsupported", &[])),
    }
}

/// A Deck rating in words, with what Valve rated it with.
pub fn deck_text(deck: &Deck) -> String {
    let (_, rating) = deck_rating(deck.rating);
    match deck.runtime.as_deref() {
        Some("native") => tr("{rating}, natively", &[("rating", &rating)]),
        Some("proton-stable") => tr(
            "{rating}, with Steam's default Proton",
            &[("rating", &rating)],
        ),
        Some(runtime) => tr(
            "{rating}, with Proton {version}",
            &[
                ("rating", &rating),
                (
                    "version",
                    runtime.strip_prefix("proton-").unwrap_or(runtime),
                ),
            ],
        ),
        None => rating,
    }
}

/// Where a game is from.
pub fn source(source: &str) -> String {
    match source {
        "steam" => "Steam".into(),
        "shortcut" => tr("Non-Steam game", &[]),
        "epic" => "Epic Games".into(),
        "gog" => "GOG".into(),
        "amazon" => "Amazon".into(),
        "sideload" => tr("Sideloaded", &[]),
        "faugus" => "Faugus Launcher".into(),
        _ => String::new(),
    }
}

/// Who uses a tool, in a few words; none where nobody can tell.
pub fn users(u: &Usage) -> Option<String> {
    let users = u.users.as_ref()?;
    let games = match &users.games[..] {
        [] => None,
        [one] => Some(one.clone()),
        many => Some(trn("{n} game", "{n} games", many.len(), &[])),
    };
    Some(match (users.default, games) {
        (false, None) => tr("unused", &[]),
        (true, None) => tr("used by the default", &[]),
        (false, Some(g)) => tr("used by {games}", &[("games", &g)]),
        (true, Some(g)) => tr("used by the default and {games}", &[("games", &g)]),
    })
}

pub fn kind_name(kind: Kind) -> String {
    match kind {
        Kind::Prefix => tr("Prefix", &[]),
        Kind::ShaderCache => tr("Shader cache", &[]),
    }
}

/// The app a prefix or shader cache is for, or where Steam doesn't know
/// it, what it is.
pub fn app_name(item: &Item) -> String {
    match &item.name {
        Some(name) => name.clone(),
        None if item.shortcut() => tr("Non-Steam game", &[]),
        None => tr("Steam app {id}", &[("id", &item.app)]),
    }
}
