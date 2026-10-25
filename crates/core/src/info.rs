//! What players report about games: ProtonDB's rating of how a Steam game
//! runs with Proton, and Are We Anti-Cheat Yet's status of a game's
//! anti-cheat on Linux. Both are cached for a day.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::games::Game;
use crate::releases::{Client, Error};

pub const PROTONDB: &str = "https://www.protondb.com/api/v1/reports/summaries";
pub const AWACY: &str =
    "https://raw.githubusercontent.com/AreWeAntiCheatYet/AreWeAntiCheatYet/HEAD/games.json";
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// ProtonDB's summary of the reports for a game.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rating {
    /// platinum, gold, silver, bronze, borked, or pending.
    pub tier: String,
    /// How sure ProtonDB is: strong, good, moderate, low, inadequate.
    pub confidence: String,
    /// How many reports there are.
    pub total: u64,
}

/// A game's anti-cheat, as Are We Anti-Cheat Yet lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AntiCheat {
    /// Supported, Running, Planned, Broken, or Denied.
    pub status: String,
    pub anticheats: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Info {
    pub protondb: Option<Rating>,
    pub anticheat: Option<AntiCheat>,
}

#[derive(Deserialize)]
struct Listed {
    name: String,
    status: String,
    #[serde(default)]
    anticheats: Vec<String>,
    #[serde(default, rename = "storeIds")]
    store_ids: StoreIds,
}

#[derive(Default, Deserialize)]
struct StoreIds {
    steam: Option<String>,
    epic: Option<Epic>,
}

#[derive(Deserialize)]
struct Epic {
    namespace: Option<String>,
}

/// Are We Anti-Cheat Yet's list: the games with anti-cheat, and its status.
pub struct AntiCheatList(Vec<Listed>);

impl AntiCheatList {
    pub fn fetch(client: &Client, url: &str) -> Result<Self, Error> {
        Ok(Self(client.fetch_json(url, DAY)?.unwrap_or_default()))
    }

    /// `game`'s entry: by its Steam app id or Epic namespace, else, for a
    /// game from another store, by name.
    pub fn find(&self, game: &Game) -> Option<AntiCheat> {
        let found = match (game.source, &game.epic_namespace) {
            ("steam", _) => self
                .0
                .iter()
                .find(|l| l.store_ids.steam.as_deref() == Some(&game.id)),
            (_, Some(ns)) => self.0.iter().find(|l| {
                l.store_ids
                    .epic
                    .as_ref()
                    .and_then(|e| e.namespace.as_deref())
                    == Some(ns)
            }),
            _ => {
                let name = simple(&game.name);
                self.0.iter().find(|l| simple(&l.name) == name)
            }
        }?;
        Some(AntiCheat {
            status: found.status.clone(),
            anticheats: found.anticheats.clone(),
        })
    }
}

/// ProtonDB's rating of a Steam game; none for one without reports, or a
/// game not from Steam.
pub fn protondb(client: &Client, url: &str, game: &Game) -> Result<Option<Rating>, Error> {
    if game.source != "steam" {
        return Ok(None);
    }
    client.fetch_json(&format!("{url}/{}.json", game.id), DAY)
}

/// A name with only its letters and digits, lowercased: "Elden Ring™" and
/// "ELDEN RING" match.
fn simple(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() && c.is_ascii())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
#[path = "tests/info.rs"]
mod tests;
