//! Steam's app cache (appcache/appinfo.vdf): what it knows of each app,
//! installed or not, such as its name and its Steam Deck rating.

use std::fs;
use std::path::Path;

use super::{app_manifests, libraries};
use crate::files::optional;
use crate::games::{Deck, DeckRating, Error};
use crate::vdf::{self, Obj, Value};

/// The id of every app installed in any library, games or not, from its
/// manifest's name: one that can't be read still counts.
pub(crate) fn app_ids(root: &Path) -> Result<Vec<String>, Error> {
    let mut ids = Vec::new();
    for lib in libraries(root)? {
        for path in app_manifests(&lib)? {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let id = &name["appmanifest_".len()..name.len() - ".acf".len()];
            ids.push(id.to_owned());
        }
    }
    Ok(ids)
}

/// The names Steam's app cache has for the apps `ids`, which it keeps for
/// apps no longer installed too.
pub(crate) fn app_names(root: &Path, ids: &[&str]) -> Result<Vec<(String, String)>, Error> {
    let mut names = Vec::new();
    app_info(root, ids, |id, info| {
        if let Some(name) = info
            .path(&["appinfo", "common"])
            .and_then(|c| c.str("name"))
        {
            names.push((id.to_string(), name.to_owned()));
        }
    })?;
    Ok(names)
}

/// Valve's Steam Deck ratings of the apps `ids` it rated, from Steam's app
/// cache.
pub(super) fn deck_ratings(root: &Path, ids: &[&str]) -> Result<Vec<(String, Deck)>, Error> {
    let mut decks = Vec::new();
    app_info(root, ids, |id, info| {
        let Some(compat) = info.path(&["appinfo", "common", "steam_deck_compatibility"]) else {
            return;
        };
        let category = match compat.get("category") {
            Some(Value::Int(n)) => *n,
            Some(Value::Str(s)) => s.parse().unwrap_or(0),
            _ => 0,
        };
        let rating = match category {
            1 => DeckRating::Unsupported,
            2 => DeckRating::Playable,
            3 => DeckRating::Verified,
            _ => return,
        };
        let runtime = compat
            .obj("configuration")
            .and_then(|c| c.str("recommended_runtime"))
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        decks.push((id.to_string(), Deck { rating, runtime }));
    })?;
    Ok(decks)
}

/// Each of the apps `ids` in Steam's app cache (appcache/appinfo.vdf),
/// with its info. Entries of other apps are skipped unread. Formats 28 and
/// 29 (2024 on, its keys in a table at the end); an older one is an error.
fn app_info(root: &Path, ids: &[&str], mut each: impl FnMut(u64, &Obj)) -> Result<(), Error> {
    let path = root.join("appcache/appinfo.vdf");
    let Some(bytes) = optional(&path, fs::read)? else {
        return Ok(());
    };
    let bad = |message: String| Error::Vdf {
        path: path.clone(),
        source: vdf::ParseError { line: 0, message },
    };
    let int = |at: usize, n: usize| -> Result<u64, Error> {
        let raw = bytes
            .get(at..at + n)
            .ok_or_else(|| bad(format!("byte {at}: truncated")))?;
        Ok(raw.iter().rev().fold(0, |v, &b| v << 8 | u64::from(b)))
    };
    let (mut pos, keys) = match int(0, 4)? {
        0x0756_4428 => (8, None),
        0x0756_4429 => {
            let table = int(8, 8)? as usize;
            let count = int(table, 4)? as usize;
            let keys: Vec<String> = bytes[table + 4..]
                .split(|&b| b == 0)
                .take(count)
                .map(|k| String::from_utf8_lossy(k).into_owned())
                .collect();
            if keys.len() < count {
                return Err(bad(format!("byte {table}: truncated key table")));
            }
            (16, Some(keys))
        }
        other => return Err(bad(format!("unknown format {other:#010x}"))),
    };
    let wanted: Vec<u64> = ids.iter().filter_map(|id| id.parse().ok()).collect();
    loop {
        let id = int(pos, 4)?;
        if id == 0 {
            break;
        }
        let size = int(pos + 4, 4)? as usize;
        let end = pos + 8 + size;
        if wanted.contains(&id) {
            // Before the KeyValues: its state, update time, access token,
            // two SHA-1s and change number, 60 bytes.
            let kv = bytes
                .get(pos + 68..end)
                .ok_or_else(|| bad(format!("byte {pos}: app {id} is truncated")))?;
            let obj = match &keys {
                Some(keys) => vdf::parse_binary_keyed(kv, keys),
                None => vdf::parse_binary(kv),
            }
            .map_err(|e| bad(format!("app {id}: {}", e.message)))?;
            each(id, &obj);
        }
        pos = end;
    }
    Ok(())
}
