//! Test helpers: temporary directories, and an HTTP server on localhost that
//! answers from a closure and logs what it was asked.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A directory removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("protonhub-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub path: String,
    /// Lowercased names.
    pub headers: BTreeMap<String, String>,
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

pub struct Server {
    pub url: String,
    pub log: Arc<Mutex<Vec<Request>>>,
}

impl Server {
    pub fn requests(&self) -> Vec<Request> {
        self.log.lock().unwrap().clone()
    }
}

/// Serves until the test process exits, one connection at a time.
pub fn serve(handler: impl Fn(&Request) -> Response + Send + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let log = Arc::new(Mutex::new(Vec::new()));
    let thread_log = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split(' ').nth(1).unwrap_or("").to_owned();
            let mut headers = BTreeMap::new();
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                let Some((name, value)) = line.trim_end().split_once(": ") else {
                    break;
                };
                headers.insert(name.to_ascii_lowercase(), value.to_owned());
            }
            let req = Request { path, headers };
            let resp = handler(&req);
            thread_log.lock().unwrap().push(req);
            let mut head = format!(
                "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                resp.status,
                resp.body.len()
            );
            for (name, value) in resp.headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&resp.body);
        }
    });
    Server { url, log }
}

/// A plain .tar of `files` (path, contents).
pub fn tar(files: &[(&str, &str)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for (path, contents) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
        header.set_cksum();
        tar.append(&header, contents.as_bytes()).unwrap();
    }
    tar.into_inner().unwrap()
}

pub type Published = Arc<Mutex<(String, Vec<(String, Vec<u8>)>)>>;

/// A GitHub whose repo o/r has one release, the one in the returned
/// handle (tag, files), which a test can replace.
pub fn github(release: (&str, Vec<(&str, Vec<u8>)>)) -> (Server, Published) {
    let published: Published = Arc::new(Mutex::new((
        release.0.to_owned(),
        release
            .1
            .into_iter()
            .map(|(n, b)| (n.to_owned(), b))
            .collect(),
    )));
    let state = published.clone();
    let server = serve(move |req| {
        let (tag, files) = &*state.lock().unwrap();
        let host = &req.headers["host"];
        match req.path.as_str() {
            "/repos/o/r/releases?per_page=50" => Response::ok(
                serde_json::json!([{
                    "tag_name": tag, "name": null, "published_at": null, "body": null,
                    "prerelease": false, "draft": false,
                    "assets": files.iter().map(|(name, bytes)| serde_json::json!({
                        "name": name, "size": bytes.len(),
                        "browser_download_url": format!("http://{host}/f/{name}"),
                    })).collect::<Vec<_>>(),
                }])
                .to_string(),
            ),
            path => match files.iter().find(|(n, _)| path == format!("/f/{n}")) {
                Some((_, bytes)) => Response::ok(bytes.clone()),
                None => Response::status(404),
            },
        }
    });
    (server, published)
}

pub fn gz(tar: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(tar).unwrap();
    gz.finish().unwrap()
}
