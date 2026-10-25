//! Asking over HTTP: API responses cached with their ETags and
//! revalidated, files downloaded once, and downloads that resume.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use protonhub_manifest::ReleaseAsset;

use super::{Client, Error, hex, http_err, io_err};

#[derive(Serialize, Deserialize)]
struct Cached {
    etag: String,
    body: String,
}

/// Parses JSON, or, failing that, JSON missing the commas between values
/// on separate lines, a slip hand-edited files like Are We Anti-Cheat
/// Yet's make; the first error if that fails too.
pub(super) fn decode<T: serde::de::DeserializeOwned>(body: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(body).or_else(|e| {
        // A value's end, then a value's start on a later line. Never valid
        // without a comma: strings can't hold a newline.
        let gap = regex::Regex::new(r#"(["\]}])(\s*\n\s*)(["\[{])"#).expect("a valid regex");
        serde_json::from_str(&gap.replace_all(body, "$1,$2$3")).map_err(|_| e)
    })
}

impl Client {
    /// GETs an API URL, revalidating the cached copy; `None` on 404.
    /// `github` sends GitHub's headers and the token.
    pub(super) fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        github: bool,
    ) -> Result<Option<T>, Error> {
        self.get_json_aged(url, github, None)
    }

    /// GETs `path` from GitHub's API, as the releases are: revalidated,
    /// with the token; `None` on 404.
    pub fn github<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<Option<T>, Error> {
        self.get_json(&format!("{}{path}", self.api), true)
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
            if let Some(token) = self.token.lock().expect("never poisoned").as_ref() {
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
