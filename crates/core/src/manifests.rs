//! The tool manifests in ProtonHub's repository, fetched so that a fixed
//! or new tool reaches users without a release. They override the
//! manifests installed with ProtonHub tool by tool; a fetch whose
//! manifests don't load (they need a newer ProtonHub) is dropped, and the
//! installed ones stay. A build run from a checkout uses the checkout's,
//! which may be newer than the repository's.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use protonhub_manifest::{LoadError, Registry};
use serde::Deserialize;

use crate::releases::{self, Client};

/// The repository, and the branch whose `registry/` is fetched.
const REPO: &str = "antoniosarro/protonhub";
const BRANCH: &str = "main";

/// How long fetched manifests are used before they're fetched again.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Fetch(#[from] releases::Error),
    #[error("the fetched manifests don't load, so the built-in ones stay: {0}")]
    Load(#[from] LoadError),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Where fetched manifests are kept, in ProtonHub's cache directory.
pub fn dir(cache: &Path) -> PathBuf {
    cache.join("registry")
}

/// The built-in manifests in `built_in`, with those fetched into `cache`
/// over them where they load.
pub fn load(cache: &Path, built_in: &Path) -> Result<Registry, LoadError> {
    let mut registry = Registry::load_dir(built_in)?;
    if let Ok(fetched) = Registry::load_dir(&dir(cache)) {
        registry.tools.extend(fetched.tools);
    }
    Ok(registry)
}

/// Fetches the repository's manifests into `cache`, unless they were in
/// the last day and not `force`d; whether it fetched them. They replace
/// the ones fetched before only once all are fetched and load.
pub fn update(client: &Client, cache: &Path, force: bool) -> Result<bool, Error> {
    let dir = dir(cache);
    let fresh = fs::metadata(&dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < MAX_AGE);
    if fresh && !force {
        return Ok(false);
    }
    #[derive(Deserialize)]
    struct Entry {
        name: String,
        download_url: Option<String>,
    }
    let listing: Vec<Entry> = client
        .github(&format!("/repos/{REPO}/contents/registry?ref={BRANCH}"))?
        .unwrap_or_default();
    let files: Vec<(String, String)> = listing
        .into_iter()
        .filter(|e| e.name.ends_with(".toml"))
        .filter_map(|e| Some((e.name, e.download_url?)))
        .collect();
    if files.is_empty() {
        return Ok(false);
    }
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| Error::Io { path, source }
    };
    let staging = cache.join("registry.new");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(io(&staging))?;
    let fetched = (|| {
        for (name, url) in &files {
            let path = staging.join(name);
            fs::write(&path, client.fetch_text(url)?).map_err(io(&path))?;
        }
        Registry::load_dir(&staging)?;
        Ok(())
    })();
    if let Err(e) = fetched {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&staging, &dir).map_err(io(&dir))?;
    Ok(true)
}

/// Whether fetched manifests are used: by an installed ProtonHub, not
/// named others to use.
pub fn used() -> bool {
    std::env::var_os("PROTONHUB_REGISTRY").is_none() && crate::paths::installed_registry().is_some()
}

/// The manifests ProtonHub uses: `explicit`'s (`--registry`) or
/// `$PROTONHUB_REGISTRY`'s alone; for an installed ProtonHub, its own with
/// the fetched over them; else a checkout's `registry`.
pub fn load_default(explicit: Option<&Path>) -> Result<Registry, LoadError> {
    match (explicit, crate::paths::cache_dir()) {
        (Some(dir), _) => Registry::load_dir(dir),
        (None, Some(cache)) if used() => load(&cache, &crate::paths::registry_dir()),
        (None, _) => Registry::load_dir(&crate::paths::registry_dir()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Response, TempDir, serve};

    const GE: &str = "[ge]\nname = \"GE\"\nsource = { github = \"o/r\" }\nasset = { x86_64 = 'x' }\nlaunchers = { steam = \"compatibilitytools.d\" }\n";

    /// A repository whose registry/ has a.toml, with `text`, and a README.
    fn repo(text: &'static str) -> crate::testutil::Server {
        serve(move |req| {
            let host = &req.headers["host"];
            if req
                .path
                .starts_with("/repos/antoniosarro/protonhub/contents/registry")
            {
                Response::ok(format!(
                    r#"[{{"name":"a.toml","download_url":"http://{host}/raw/a.toml"}},{{"name":"README.md","download_url":"http://{host}/raw/README.md"}}]"#
                ))
            } else if req.path == "/raw/a.toml" {
                Response::ok(text)
            } else {
                Response::status(404)
            }
        })
    }

    #[test]
    fn fetches_once_a_day_and_keeps_the_last_good_ones() {
        let tmp = TempDir::new("manifests-fetch");
        let cache = tmp.join("cache");
        let server = repo(GE);
        let client = Client::new(&server.url, cache.join("http"), None);
        assert!(update(&client, &cache, false).unwrap());
        let names: Vec<_> = fs::read_dir(dir(&cache))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["a.toml"]);
        assert!(!update(&client, &cache, false).unwrap(), "fetched today");

        let broken = repo("[ge]\nname = ");
        let client = Client::new(&broken.url, cache.join("http2"), None);
        let err = update(&client, &cache, true).unwrap_err();
        assert!(matches!(err, Error::Load(_)), "{err}");
        assert_eq!(fs::read_to_string(dir(&cache).join("a.toml")).unwrap(), GE);
    }

    #[test]
    fn fetched_manifests_override_the_built_in_ones_where_they_load() {
        let tmp = TempDir::new("manifests");
        let (built_in, cache) = (tmp.join("built-in"), tmp.join("cache"));
        fs::create_dir_all(&built_in).unwrap();
        fs::write(built_in.join("a.toml"), GE).unwrap();
        fs::write(
            built_in.join("b.toml"),
            GE.replace("[ge]", "[other]").replace("\"GE\"", "\"Other\""),
        )
        .unwrap();
        assert_eq!(load(&cache, &built_in).unwrap().tools["ge"].name, "GE");

        fs::create_dir_all(dir(&cache)).unwrap();
        fs::write(
            dir(&cache).join("a.toml"),
            GE.replace("\"GE\"", "\"GE, fixed\""),
        )
        .unwrap();
        let r = load(&cache, &built_in).unwrap();
        assert_eq!(r.tools["ge"].name, "GE, fixed");
        assert_eq!(r.tools["other"].name, "Other", "built-in ones stay");

        // A field this version doesn't know: the fetched ones are ignored.
        fs::write(
            dir(&cache).join("a.toml"),
            GE.replace("name ", "newfield = 1\nname "),
        )
        .unwrap();
        assert_eq!(load(&cache, &built_in).unwrap().tools["ge"].name, "GE");
    }
}
