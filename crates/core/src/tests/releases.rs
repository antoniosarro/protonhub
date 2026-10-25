use std::path::Path;
use std::sync::atomic::Ordering;

use protonhub_manifest::Registry;

use super::http::decode;
use super::*;
use crate::testutil::{Response, TempDir, serve};

#[test]
fn decode_repairs_missing_commas() {
    let broken = "{\"notes\": [\n  [\n    \"a\"\n    \"b\"\n  ]\n  [\"c\"]\n]}";
    let v: serde_json::Value = decode(broken).unwrap();
    assert_eq!(v["notes"], serde_json::json!([["a", "b"], ["c"]]));
    assert!(
        decode::<serde_json::Value>("{\"a\": 1 2}").is_err(),
        "not every error"
    );
}

fn tool(tag: Option<&str>) -> Tool {
    let tag = tag.map(|t| format!("tag = '{t}'")).unwrap_or_default();
    let text = format!(
        r#"
        [t]
        name = "T"
        source = {{ github = "o/r" }}
        {tag}
        asset = {{ x86_64 = '\.tar\.gz$' }}
        launchers = {{ steam = "compatibilitytools.d" }}
        "#
    );
    let mut r = Registry::default();
    r.add(Path::new("t.toml"), &text).unwrap();
    r.tools.remove("t").unwrap()
}

fn release(tag: &str, prerelease: bool) -> serde_json::Value {
    serde_json::json!({
        "tag_name": tag, "name": tag, "published_at": "2026-10-01T00:00:00Z",
        "prerelease": prerelease, "draft": false, "body": "notes",
        "assets": [{ "name": format!("{tag}.tar.gz"), "browser_download_url": "http://x/a", "size": 3 }],
    })
}

#[test]
fn latest_skips_prereleases_and_other_tools() {
    let list = serde_json::json!([
        release("GE-Proton8-27-LoL", false),
        release("GE-Proton8-27-rc1", true),
        release("GE-Proton8-26", false),
    ])
    .to_string();
    let server = serve(move |_| Response::ok(list.clone()));
    let cache = TempDir::new("gh-latest");
    let client = Client::new(&server.url, cache.to_owned(), None);

    let t = tool(Some(r"^GE-Proton\d+-\d+(-rc\d+)?$"));
    let r = client.release(&t, &Version::Latest).unwrap();
    assert_eq!(r.tag, "GE-Proton8-26");
    assert_eq!(r.assets[0].name, "GE-Proton8-26.tar.gz");
    let all: Vec<_> = client
        .releases(&t, 10)
        .unwrap()
        .into_iter()
        .map(|r| r.tag)
        .collect();
    assert_eq!(all, ["GE-Proton8-27-rc1", "GE-Proton8-26"]);
    assert_eq!(server.requests()[0].path, "/repos/o/r/releases?per_page=50");
}

#[test]
fn revalidates_with_the_etag() {
    let list = serde_json::json!([release("v1", false)]).to_string();
    let server = serve(move |req| {
        if req.headers.get("if-none-match").map(String::as_str) == Some("\"abc\"") {
            Response::status(304)
        } else {
            Response::ok(list.clone()).header("ETag", "\"abc\"")
        }
    });
    let cache = TempDir::new("gh-etag");
    let client = Client::new(&server.url, cache.to_owned(), Some("tok".into()));
    let t = tool(None);
    assert_eq!(client.release(&t, &Version::Latest).unwrap().tag, "v1");
    assert_eq!(client.release(&t, &Version::Latest).unwrap().tag, "v1");
    let reqs = server.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].headers["authorization"], "Bearer tok");
    assert!(!reqs[0].headers.contains_key("if-none-match"));
    assert_eq!(reqs[1].headers["if-none-match"], "\"abc\"");
}

#[test]
fn reports_the_rate_limit() {
    let server = serve(|_| {
        Response::status(403)
            .header("x-ratelimit-remaining", "0")
            .header("x-ratelimit-reset", "1791658501")
    });
    let cache = TempDir::new("gh-rate");
    let client = Client::new(&server.url, cache.to_owned(), None);
    let err = client.release(&tool(None), &Version::Latest).unwrap_err();
    assert!(
        matches!(err, Error::RateLimited { reset: 1791658501 }),
        "{err}"
    );
}

#[test]
fn tags_must_exist_and_belong_to_the_tool() {
    let server = serve(|_| Response::status(404));
    let cache = TempDir::new("gh-tag");
    let client = Client::new(&server.url, cache.to_owned(), None);
    let t = tool(Some(r"^v\d+$"));
    let missing = client.release(&t, &Version::Tag("v9".into())).unwrap_err();
    assert!(matches!(missing, Error::NoSuchTag { .. }), "{missing}");
    let foreign = client
        .release(&t, &Version::Tag("v9-LoL".into()))
        .unwrap_err();
    assert!(matches!(foreign, Error::NotAVersion { .. }), "{foreign}");
    assert_eq!(
        server.requests().len(),
        1,
        "the foreign tag is refused without asking"
    );
}

#[test]
fn resumes_a_partial_download() {
    let data: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    let served = data.clone();
    let server = serve(move |req| match req.headers.get("range") {
        Some(range) => {
            let from: usize = range["bytes=".len()..range.len() - 1].parse().unwrap();
            Response {
                status: 206,
                headers: Vec::new(),
                body: served[from..].to_vec(),
            }
        }
        None => Response::ok(served.clone()),
    });
    let dir = TempDir::new("gh-resume");
    std::fs::write(dir.join("a.tar.gz.part"), &data[..1000]).unwrap();
    let client = Client::new(&server.url, dir.join("cache"), None);
    let asset = ReleaseAsset {
        name: "a.tar.gz".into(),
        url: format!("{}/a.tar.gz", server.url),
        size: data.len() as u64,
    };
    let mut last = (0, 0);
    let path = client
        .download(&asset, &dir, &mut |d, t| last = (d, t))
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), data);
    assert_eq!(last, (data.len() as u64, data.len() as u64));
    assert_eq!(server.requests()[0].headers["range"], "bytes=1000-");
    assert!(!dir.join("a.tar.gz.part").exists());
}

#[test]
fn a_short_download_is_an_error_and_kept_to_resume() {
    let server = serve(|_| Response::ok(vec![1; 10]));
    let dir = TempDir::new("gh-short");
    let client = Client::new(&server.url, dir.join("cache"), None);
    let asset = ReleaseAsset {
        name: "a.tar.gz".into(),
        url: format!("{}/a.tar.gz", server.url),
        size: 20,
    };
    let err = client.download(&asset, &dir, &mut |_, _| {}).unwrap_err();
    assert!(
        matches!(
            err,
            Error::Incomplete {
                got: 10,
                want: 20,
                ..
            }
        ),
        "{err}"
    );
    assert_eq!(
        std::fs::metadata(dir.join("a.tar.gz.part")).unwrap().len(),
        10
    );
}

#[test]
fn a_cancelled_download_stops_and_is_kept_to_resume() {
    let server = serve(|_| Response::ok(vec![1; 1 << 20]));
    let dir = TempDir::new("gh-cancel");
    let cancel = Arc::new(AtomicBool::new(false));
    let client = Client::new(&server.url, dir.join("cache"), None).cancellable(cancel.clone());
    let asset = ReleaseAsset {
        name: "a.tar.gz".into(),
        url: format!("{}/a.tar.gz", server.url),
        size: 1 << 20,
    };
    let err = client
        .download(&asset, &dir, &mut |_, _| {
            cancel.store(true, Ordering::SeqCst)
        })
        .unwrap_err();
    assert!(matches!(err, Error::Cancelled), "{err}");
    let kept = std::fs::metadata(dir.join("a.tar.gz.part")).unwrap().len();
    assert!(kept > 0 && kept < 1 << 20, "{kept}");
}

fn tool_from(source: &str) -> Tool {
    let mut r = Registry::default();
    r.add(
        Path::new("t.toml"),
        &format!("[t]\nname = \"T\"\nsource = {source}\nasset = {{ any = 'x' }}\nlaunchers = {{ steam = \"c\" }}"),
    )
    .unwrap();
    r.tools.remove("t").unwrap()
}

#[test]
fn reads_forgejo_releases_without_the_github_token() {
    let server = serve(|req| match req.path.as_str() {
        "/api/v1/repos/o/r/releases?limit=50" => {
            Response::ok(serde_json::json!([release("v2", true), release("v1", false)]).to_string())
        }
        "/api/v1/repos/o/r/releases/tags/v1" => Response::ok(release("v1", false).to_string()),
        _ => Response::status(404),
    });
    let cache = TempDir::new("forgejo");
    let client = Client::new("http://unused", cache.to_owned(), Some("secret".into()));
    let t = tool_from(&format!("{{ forgejo = \"{}/o/r\" }}", server.url));
    assert_eq!(client.release(&t, &Version::Latest).unwrap().tag, "v1");
    assert_eq!(
        client.release(&t, &Version::Tag("v1".into())).unwrap().tag,
        "v1"
    );
    let missing = client.release(&t, &Version::Tag("v9".into())).unwrap_err();
    assert!(matches!(missing, Error::NoSuchTag { .. }), "{missing}");
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.headers.contains_key("authorization")),
        "the GitHub token stays with GitHub"
    );
}

#[test]
fn reads_gitlab_releases_and_downloads_assets_of_unknown_size() {
    let server = serve(|req| match req.path.as_str() {
        "/api/v4/projects/g%2Fp/releases?per_page=50" => {
            let host = &req.headers["host"];
            Response::ok(
                serde_json::json!([{
                    "tag_name": "v3.1-1", "name": "v3.1-1", "released_at": "2026-09-19T07:30:22.000Z",
                    "upcoming_release": false, "description": "notes",
                    "assets": { "links": [
                        { "name": "a.tar.gz", "url": "http://elsewhere/a", "direct_asset_url": format!("http://{host}/raw/a.tar.gz") },
                    ] },
                }])
                .to_string(),
            )
        }
        "/raw/a.tar.gz" => Response::ok(vec![7; 1000]),
        _ => Response::status(404),
    });
    let dir = TempDir::new("gitlab");
    let client = Client::new("http://unused", dir.join("cache"), None);
    let t = tool_from(&format!("{{ gitlab = \"{}/g/p\" }}", server.url));
    let r = client.release(&t, &Version::Latest).unwrap();
    assert_eq!(
        (r.tag.as_str(), r.published_at.as_deref()),
        ("v3.1-1", Some("2026-09-19T07:30:22.000Z"))
    );
    assert_eq!(r.assets[0].size, 0, "GitLab doesn't say");
    // A stale partial file is no use for an asset of unknown size.
    std::fs::write(dir.join("a.tar.gz.part"), [1, 2, 3]).unwrap();
    let path = client.download(&r.assets[0], &dir, &mut |_, _| {}).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), vec![7; 1000]);
    assert!(
        server
            .requests()
            .iter()
            .all(|r| !r.headers.contains_key("range"))
    );
}

#[test]
fn a_fixed_url_is_one_version() {
    let cache = TempDir::new("fixed");
    let client = Client::new("http://unused", cache.to_owned(), None);
    let t = tool_from(
        "{ url = \"https://example.org/x/archive/main.tar.gz?raw=1\", version = \"main\" }",
    );
    let r = client.release(&t, &Version::Latest).unwrap();
    assert_eq!(
        (r.tag.as_str(), r.assets[0].name.as_str()),
        ("main", "main.tar.gz")
    );
    assert!(client.release(&t, &Version::Tag("main".into())).is_ok());
    assert!(matches!(
        client.release(&t, &Version::Tag("v1".into())),
        Err(Error::NoSuchTag { .. })
    ));
}

#[test]
fn api_urls() {
    assert_eq!(
        forgejo_api("https://codeberg.org/luxtorpeda/luxtorpeda"),
        "https://codeberg.org/api/v1/repos/luxtorpeda/luxtorpeda"
    );
    assert_eq!(
        gitlab_api("https://gitlab.com/Ph42oN/dxvk-gplasync"),
        "https://gitlab.com/api/v4/projects/Ph42oN%2Fdxvk-gplasync"
    );
}
