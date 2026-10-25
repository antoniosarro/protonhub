//! Releases, where every tool's versions come from: GitHub, Forgejo (such
//! as Codeberg), and GitLab release lists, or one archive at a fixed URL.
//!
//! GitHub allows 60 unauthenticated API calls an hour, so every response
//! is cached with its ETag and revalidated: a 304 doesn't count against the
//! limit. A token (`GITHUB_TOKEN`, or one saved: [`paths::github_token`])
//! raises it to 5000, and is only ever sent
//! to GitHub.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protonhub_manifest::{ReleaseAsset, Source, Tool};

mod http;
mod raw;

use raw::{RawGitlab, RawRelease};
use serde::Serialize;

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
        "the GitHub API rate limit is used up until {reset} (unix time); set a GitHub token (GITHUB_TOKEN, or ProtonHub's preferences) to raise it from 60 to 5000 requests an hour"
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

#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    api: String,
    /// Shared by its clones, so a token set applies to all.
    token: Arc<Mutex<Option<String>>>,
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
            token: Arc::new(Mutex::new(token.filter(|t| !t.is_empty()))),
            cache,
            cancel: None,
        }
    }

    /// Sends `token` to GitHub from now on, or none.
    pub fn set_token(&self, token: Option<String>) {
        *self.token.lock().expect("never poisoned") = token.filter(|t| !t.is_empty());
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

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
#[path = "../tests/releases.rs"]
mod tests;
