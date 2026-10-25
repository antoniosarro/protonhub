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
mod tests {
    use super::*;
    use crate::testutil::{Response, TempDir, serve};

    fn game(id: &str, name: &str, source: &'static str, ns: Option<&str>) -> Game {
        Game {
            id: id.into(),
            name: name.into(),
            source,
            tool: None,
            explicit: false,
            epic_namespace: ns.map(str::to_owned),
            deck: None,
            art: None,
        }
    }

    #[test]
    fn finds_anti_cheat_by_store_id_or_name() {
        let server = serve(|_| {
            Response::ok(
                serde_json::json!([
                    { "name": "Warframe", "status": "Supported", "anticheats": ["Unknown (Custom)"],
                      "storeIds": { "steam": "230410", "epic": { "namespace": "244aaaa0", "slug": "warframe" } } },
                    { "name": "ELDEN RING™", "status": "Running", "anticheats": ["Easy Anti-Cheat"], "storeIds": {} },
                ])
                .to_string(),
            )
            .header("ETag", "\"a\"")
        });
        let cache = TempDir::new("awacy");
        let client = Client::new("http://unused", cache.to_owned(), None);
        let list = AntiCheatList::fetch(&client, &format!("{}/games.json", server.url)).unwrap();
        let status = |g: &Game| list.find(g).map(|a| a.status);
        assert_eq!(
            status(&game("230410", "x", "steam", None)).as_deref(),
            Some("Supported")
        );
        assert_eq!(
            status(&game("app", "x", "epic", Some("244aaaa0"))).as_deref(),
            Some("Supported")
        );
        assert_eq!(
            status(&game("g1", "Elden Ring", "gog", None)).as_deref(),
            Some("Running")
        );
        // A Steam game is matched by its id only.
        assert_eq!(status(&game("1", "Elden Ring", "steam", None)), None);

        // Within a day, the cached copy is used without asking again.
        AntiCheatList::fetch(&client, &format!("{}/games.json", server.url)).unwrap();
        assert_eq!(server.requests().len(), 1);
    }

    #[test]
    fn rates_steam_games_only() {
        let server = serve(|req| {
            match req.path.as_str() {
            "/summaries/1030300.json" => Response::ok(
                r#"{"bestReportedTier":"platinum","confidence":"strong","score":0.98,"tier":"platinum","total":356,"trendingTier":"platinum"}"#,
            )
            .header("ETag", "\"r\""),
            _ => Response::status(404),
        }
        });
        let cache = TempDir::new("protondb");
        let client = Client::new("http://unused", cache.to_owned(), None);
        let url = format!("{}/summaries", server.url);
        let rating = protondb(&client, &url, &game("1030300", "x", "steam", None))
            .unwrap()
            .unwrap();
        assert_eq!((rating.tier.as_str(), rating.total), ("platinum", 356));
        assert_eq!(
            protondb(&client, &url, &game("7", "x", "steam", None)).unwrap(),
            None
        );
        assert_eq!(
            protondb(&client, &url, &game("1030300", "x", "shortcut", None)).unwrap(),
            None
        );
        assert_eq!(server.requests().len(), 2, "the shortcut isn't asked about");
    }
}
