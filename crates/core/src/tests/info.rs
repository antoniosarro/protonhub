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
