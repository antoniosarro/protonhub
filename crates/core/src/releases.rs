//! Releases, where every tool's versions come from: GitHub, Forgejo (such
//! as Codeberg), and GitLab release lists, or one archive at a fixed URL.
//!
//! GitHub allows 60 unauthenticated API calls an hour, so every response
//! is cached with its ETag and revalidated: a 304 doesn't count against the
//! limit. A token (`GITHUB_TOKEN`) raises it to 5000, and is only ever sent
//! to GitHub.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use protonhub_manifest::{ReleaseAsset, Source, Tool};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const API: &str = "https://api.github.com";

/// Which release of a tool to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Version {
    /// The newest release that isn't a draft or a prerelease and whose tag
    /// the manifest accepts.
    Latest,
    Tag(String),
}

/// How many releases `Version::Latest` looks through, newest first.
const LATEST_WINDOW: usize = 50;

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub tag: String,
    pub name: Option<String>,
    pub published_at: Option<String>,
    pub prerelease: bool,
    /// The release notes, in Markdown.
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    name: Option<String>,
    published_at: Option<String>,
    prerelease: bool,
    draft: bool,
    body: Option<String>,
    assets: Vec<RawAsset>,
}

/// A GitLab release; its asset links have no size.
#[derive(Deserialize)]
struct RawGitlab {
    tag_name: String,
    name: Option<String>,
    released_at: Option<String>,
    #[serde(default)]
    upcoming_release: bool,
    description: Option<String>,
    assets: RawGitlabAssets,
}

#[derive(Deserialize)]
struct RawGitlabAssets {
    links: Vec<RawGitlabLink>,
}

#[derive(Deserialize)]
struct RawGitlabLink {
    name: String,
    url: String,
    direct_asset_url: Option<String>,
}

impl From<RawGitlab> for RawRelease {
    fn from(r: RawGitlab) -> Self {
        Self {
            tag_name: r.tag_name,
            name: r.name,
            published_at: r.released_at,
            prerelease: r.upcoming_release,
            draft: false,
            body: r.description,
            assets: r
                .assets
                .links
                .into_iter()
                .map(|l| RawAsset {
                    name: l.name,
                    browser_download_url: l.direct_asset_url.unwrap_or(l.url),
                    size: 0,
                })
                .collect(),
        }
    }
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

impl From<RawRelease> for Release {
    fn from(r: RawRelease) -> Self {
        Self {
            tag: r.tag_name,
            name: r.name,
            published_at: r.published_at,
            prerelease: r.prerelease,
            body: r.body,
            assets: r
                .assets
                .into_iter()
                .map(|a| ReleaseAsset {
                    name: a.name,
                    url: a.browser_download_url,
                    size: a.size,
                })
                .collect(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cancelled")]
    Cancelled,
    #[error("{url}: {source}")]
    Http { url: String, source: ureq::Error },
    #[error("{url}: HTTP {status}{}", if message.is_empty() { String::new() } else { format!(": {message}") })]
    Status {
        url: String,
        status: u16,
        message: String,
    },
    #[error(
        "the GitHub API rate limit is used up until {reset} (unix time); set GITHUB_TOKEN to raise it from 60 to 5000 requests an hour"
    )]
    RateLimited { reset: u64 },
    #[error("{url}: unexpected response: {source}")]
    Decode {
        url: String,
        source: serde_json::Error,
    },
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("{url}: downloaded {got} bytes of {want}; run again to resume")]
    Incomplete { url: String, got: u64, want: u64 },
    #[error("{repo} has no release tagged {tag}")]
    NoSuchTag { repo: String, tag: String },
    #[error("{tag} is not a release of {tool} (its manifest's tag pattern excludes it)")]
    NotAVersion { tool: String, tag: String },
    #[error("none of the latest {LATEST_WINDOW} releases of {repo} is a stable release of {tool}")]
    NoStableRelease { tool: String, repo: String },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_owned(),
        source,
    }
}

fn http_err(url: &str) -> impl FnOnce(ureq::Error) -> Error + '_ {
    move |source| Error::Http {
        url: url.to_owned(),
        source,
    }
}

#[derive(Serialize, Deserialize)]
struct Cached {
    etag: String,
    body: String,
}

/// Parses JSON, or, failing that, JSON missing the commas between values
/// on separate lines, a slip hand-edited files like Are We Anti-Cheat
/// Yet's make; the first error if that fails too.
fn decode<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(body).or_else(|e| {
        // A value's end, then a value's start on a later line. Never valid
        // without a comma: strings can't hold a newline.
        let gap = regex::Regex::new(r#"(["\]}])(\s*\n\s*)(["\[{])"#).expect("a valid regex");
        serde_json::from_str(&gap.replace_all(body, "$1,$2$3")).map_err(|_| e)
    })
}

#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    api: String,
    token: Option<String>,
    cache: PathBuf,
    /// Set to stop a download, as a Cancel button does.
    cancel: Option<Arc<AtomicBool>>,
}

impl Client {
    /// A client for the API at `api` (normally [`API`]) that caches responses
    /// in `cache`.
    pub fn new(api: impl Into<String>, cache: PathBuf, token: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .user_agent(concat!("protonhub/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Self {
            agent,
            api: api.into(),
            token: token.filter(|t| !t.is_empty()),
            cache,
            cancel: None,
        }
    }

    /// This client, with its downloads stopped once `cancel` is set; what
    /// was downloaded stays to resume.
    pub fn cancellable(&self, cancel: Arc<AtomicBool>) -> Self {
        Self {
            cancel: Some(cancel),
            ..self.clone()
        }
    }

    /// The newest `n` releases of `tool`, prereleases included, without
    /// those its manifest's tag pattern excludes.
    pub fn releases(&self, tool: &Tool, n: usize) -> Result<Vec<Release>, Error> {
        let all: Vec<RawRelease> = match &tool.source {
            Source::Github(repo) => {
                let url = format!(
                    "{}/repos/{repo}/releases?per_page={LATEST_WINDOW}",
                    self.api
                );
                self.get_json(&url, true)?.unwrap_or_default()
            }
            Source::Forgejo(base) => {
                let url = format!("{}/releases?limit={LATEST_WINDOW}", forgejo_api(base));
                self.get_json(&url, false)?.unwrap_or_default()
            }
            Source::Gitlab(base) => {
                let url = format!("{}/releases?per_page={LATEST_WINDOW}", gitlab_api(base));
                let list: Vec<RawGitlab> = self.get_json(&url, false)?.unwrap_or_default();
                list.into_iter().map(RawRelease::from).collect()
            }
            Source::Url { url, version } => return Ok(vec![fixed(url, version)]),
        };
        Ok(all
            .into_iter()
            .filter(|r| !r.draft && tool.is_version(&r.tag_name))
            .take(n)
            .map(Release::from)
            .collect())
    }

    pub fn release(&self, tool: &Tool, version: &Version) -> Result<Release, Error> {
        let repo = tool.source.origin().to_owned();
        let tag = match version {
            Version::Latest => {
                return self
                    .releases(tool, LATEST_WINDOW)?
                    .into_iter()
                    .find(|r| !r.prerelease)
                    .ok_or_else(|| Error::NoStableRelease {
                        tool: tool.id.clone(),
                        repo,
                    });
            }
            Version::Tag(tag) => tag,
        };
        if !tool.is_version(tag) {
            return Err(Error::NotAVersion {
                tool: tool.id.clone(),
                tag: tag.clone(),
            });
        }
        let raw: Option<RawRelease> = match &tool.source {
            Source::Github(r) => {
                self.get_json(&format!("{}/repos/{r}/releases/tags/{tag}", self.api), true)?
            }
            Source::Forgejo(base) => {
                self.get_json(&format!("{}/releases/tags/{tag}", forgejo_api(base)), false)?
            }
            Source::Gitlab(base) => self
                .get_json::<RawGitlab>(&format!("{}/releases/{tag}", gitlab_api(base)), false)?
                .map(RawRelease::from),
            Source::Url { url, version } if version == tag => return Ok(fixed(url, version)),
            Source::Url { .. } => None,
        };
        raw.map(Release::from).ok_or_else(|| Error::NoSuchTag {
            repo,
            tag: tag.clone(),
        })
    }

    /// GETs an API URL, revalidating the cached copy; `None` on 404.
    /// `github` sends GitHub's headers and the token.
    fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        github: bool,
    ) -> Result<Option<T>, Error> {
        self.get_json_aged(url, github, None)
    }

    /// GETs JSON from a URL outside GitHub's API, as `get_json` does, but
    /// a cached copy younger than `max_age` is used without asking.
    pub fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        max_age: Duration,
    ) -> Result<Option<T>, Error> {
        self.get_json_aged(url, false, Some(max_age))
    }

    fn get_json_aged<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        github: bool,
        max_age: Option<Duration>,
    ) -> Result<Option<T>, Error> {
        let url = url.to_owned();
        let cache = self
            .cache
            .join("api")
            .join(format!("{}.json", hex(&Sha256::digest(url.as_bytes()))));
        // An unreadable or corrupt entry is only a cache miss.
        let cached: Option<Cached> = fs::read(&cache)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let fresh = max_age.is_some_and(|max| {
            fs::metadata(&cache)
                .and_then(|m| m.modified())
                .is_ok_and(|t| t.elapsed().is_ok_and(|age| age < max))
        });
        if let (true, Some(c)) = (fresh, &cached) {
            return decode(&c.body)
                .map(Some)
                .map_err(|source| Error::Decode { url, source });
        }

        let mut req = self.agent.get(&url).header("Accept", "application/json");
        if github {
            req = req
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
            if let Some(token) = &self.token {
                req = req.header("Authorization", format!("Bearer {token}"));
            }
        }
        if let Some(c) = &cached {
            req = req.header("If-None-Match", &c.etag);
        }
        let mut resp = req.call().map_err(http_err(&url))?;
        let header = |name| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let status = resp.status().as_u16();
        let body = match (status, cached) {
            (304, Some(c)) => c.body,
            (200, _) => {
                let etag = header("etag");
                let body = resp.body_mut().read_to_string().map_err(http_err(&url))?;
                if let Some(etag) = etag {
                    let entry = serde_json::to_vec(&Cached {
                        etag,
                        body: body.clone(),
                    })
                    .expect("strings serialize");
                    write_atomic(&cache, &entry)?;
                }
                body
            }
            (404, _) => return Ok(None),
            (403 | 429, _) if header("x-ratelimit-remaining").as_deref() == Some("0") => {
                let reset = header("x-ratelimit-reset").and_then(|r| r.parse().ok());
                return Err(Error::RateLimited {
                    reset: reset.unwrap_or(0),
                });
            }
            (status, _) => {
                let body = resp.body_mut().read_to_string().unwrap_or_default();
                return Err(Error::Status {
                    url,
                    status,
                    message: api_message(&body),
                });
            }
        };
        decode(&body)
            .map(Some)
            .map_err(|source| Error::Decode { url, source })
    }

    /// A file at `url`, downloaded once into the cache, such as a picture;
    /// returns where it is.
    pub fn fetch_file(&self, url: &str) -> Result<PathBuf, Error> {
        let path = self
            .cache
            .join("files")
            .join(hex(&Sha256::digest(url.as_bytes())));
        if !path.is_file() {
            let body = self
                .get_ok(url)?
                .with_config()
                .limit(16 << 20)
                .read_to_vec()
                .map_err(http_err(url))?;
            write_atomic(&path, &body)?;
        }
        Ok(path)
    }

    /// A small text file, such as a checksum list.
    pub fn fetch_text(&self, url: &str) -> Result<String, Error> {
        self.get_ok(url)?.read_to_string().map_err(http_err(url))
    }

    /// GETs `url`; any status but 200 is an error.
    fn get_ok(&self, url: &str) -> Result<ureq::Body, Error> {
        let resp = self.agent.get(url).call().map_err(http_err(url))?;
        match resp.status().as_u16() {
            200 => Ok(resp.into_body()),
            status => Err(Error::Status {
                url: url.to_owned(),
                status,
                message: String::new(),
            }),
        }
    }

    /// Downloads `asset` into `dir`, resuming a partial download left there,
    /// and returns the file's path. `progress` gets (bytes so far, total).
    /// An asset of unknown size (0, as GitLab's) can't be resumed or
    /// checked for length, so it is always downloaded whole.
    pub fn download(
        &self,
        asset: &ReleaseAsset,
        dir: &Path,
        progress: &mut dyn FnMut(u64, u64),
    ) -> Result<PathBuf, Error> {
        fs::create_dir_all(dir).map_err(io_err(dir))?;
        let part = dir.join(format!("{}.part", asset.name));
        let known = asset.size > 0;
        let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have > 0 && (!known || have > asset.size) {
            fs::remove_file(&part).map_err(io_err(&part))?;
            have = 0;
        }
        if !known || have < asset.size {
            let mut req = self.agent.get(&asset.url);
            if have > 0 {
                req = req.header("Range", format!("bytes={have}-"));
            }
            let resp = req.call().map_err(http_err(&asset.url))?;
            let mut file = match resp.status().as_u16() {
                206 => OpenOptions::new().append(true).open(&part),
                // The server ignored the range: start over.
                200 => {
                    have = 0;
                    File::create(&part)
                }
                status => {
                    return Err(Error::Status {
                        url: asset.url.clone(),
                        status,
                        message: String::new(),
                    });
                }
            }
            .map_err(io_err(&part))?;
            let mut body = resp.into_body().into_reader();
            let mut buf = vec![0; 1 << 16];
            loop {
                if self
                    .cancel
                    .as_ref()
                    .is_some_and(|c| c.load(Ordering::SeqCst))
                {
                    return Err(Error::Cancelled);
                }
                let n = match body.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(source) => {
                        return Err(Error::Io {
                            path: part.clone(),
                            source,
                        });
                    }
                };
                file.write_all(&buf[..n]).map_err(io_err(&part))?;
                have += n as u64;
                progress(have, asset.size);
            }
        }
        if known && have != asset.size {
            return Err(Error::Incomplete {
                url: asset.url.clone(),
                got: have,
                want: asset.size,
            });
        }
        let done = dir.join(&asset.name);
        fs::rename(&part, &done).map_err(io_err(&part))?;
        Ok(done)
    }
}

/// A Forgejo repository's API, from its URL:
/// https://codeberg.org/o/r → https://codeberg.org/api/v1/repos/o/r.
fn forgejo_api(base: &str) -> String {
    let (host, path) = split_url(base);
    format!("{host}/api/v1/repos/{path}")
}

/// A GitLab project's API, from its URL:
/// https://gitlab.com/g/p → https://gitlab.com/api/v4/projects/g%2Fp.
fn gitlab_api(base: &str) -> String {
    let (host, path) = split_url(base);
    format!("{host}/api/v4/projects/{}", path.replace('/', "%2F"))
}

/// (scheme and host, path) of a URL.
fn split_url(url: &str) -> (&str, &str) {
    let start = url.find("://").map_or(0, |i| i + 3);
    match url[start..].find('/') {
        Some(i) => (&url[..start + i], &url[start + i + 1..]),
        None => (url, ""),
    }
}

/// The single release of a fixed-URL source; its size is unknown.
fn fixed(url: &str, version: &str) -> Release {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    Release {
        tag: version.to_owned(),
        name: None,
        published_at: None,
        prerelease: false,
        body: None,
        assets: vec![ReleaseAsset {
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            url: url.to_owned(),
            size: 0,
        }],
    }
}

/// GitHub's error bodies are `{"message": "..."}`.
fn api_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct Message {
        message: String,
    }
    serde_json::from_str::<Message>(body)
        .map(|m| m.message)
        .unwrap_or_default()
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let dir = path.parent().expect("cache files are in a directory");
    fs::create_dir_all(dir).map_err(io_err(dir))?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data).map_err(io_err(&tmp))?;
    fs::rename(&tmp, path).map_err(io_err(path))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use protonhub_manifest::Registry;

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
            "/api/v1/repos/o/r/releases?limit=50" => Response::ok(
                serde_json::json!([release("v2", true), release("v1", false)]).to_string(),
            ),
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
}
