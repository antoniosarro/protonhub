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
