//! The release lists as GitHub, Forgejo and GitLab's APIs give them, and
//! the [`Release`] each makes.

use protonhub_manifest::ReleaseAsset;
use serde::Deserialize;

use super::Release;

#[derive(Deserialize)]
pub(super) struct RawRelease {
    pub(super) tag_name: String,
    name: Option<String>,
    published_at: Option<String>,
    prerelease: bool,
    pub(super) draft: bool,
    body: Option<String>,
    assets: Vec<RawAsset>,
}

/// A GitLab release; its asset links have no size.
#[derive(Deserialize)]
pub(super) struct RawGitlab {
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
