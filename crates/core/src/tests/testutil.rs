//! Test helpers: temporary directories, and an HTTP server on localhost that
//! answers from a closure and logs what it was asked.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::launcher::{Launcher, Packaging};
use crate::manifest::LauncherKind;

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

pub fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn compat_tool(root: &Path, name: &str) {
    write(
        &root.join(format!("compatibilitytools.d/{name}/compatibilitytool.vdf")),
        &format!(
            "\"compatibilitytools\" {{ \"compat_tools\" {{ \"{name}\" // internal\n {{ \"install_path\" \".\" \"display_name\" \"{name} (GE)\" }} }} }}"
        ),
    );
}

/// A Steam with two games (one mapped to GE-Proton11-7, one on the
/// default proton_9), a shortcut, a Proton 9.0 install, and an unused
/// GE-Proton10-1.
pub fn steam(home: &Path) -> Launcher {
    let root = home.join("Steam");
    let lib2 = home.join("Games");
    write(
        &root.join("steamapps/libraryfolders.vdf"),
        &format!(
            "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
            root.display(),
            lib2.display()
        ),
    );
    for (lib, id, name) in [
        (&root, "10", "Zeta"),
        (&lib2, "20", "Alpha"),
        (&root, "1493710", "Proton Experimental"),
    ] {
        write(
            &lib.join(format!("steamapps/appmanifest_{id}.acf")),
            &format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n}}\n"),
        );
    }
    fs::create_dir_all(lib2.join("steamapps/common/Proton 9.0")).unwrap();
    compat_tool(&root, "GE-Proton11-7");
    compat_tool(&root, "GE-Proton10-1");
    write(
        &root.join("config/config.vdf"),
        "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"0\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton_9\"\n\t\t\t\t\t}\n\t\t\t\t\t\"10\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"GE-Proton11-7\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n\t\"Other\"\t\t\"kept\"\n}\n",
    );
    let mut shortcut = vec![0x00];
    shortcut.extend(b"shortcuts\0\x000\0\x02appid\0");
    shortcut.extend((-5i32).to_le_bytes());
    shortcut.extend(b"\x01AppName\0My Shortcut\0\x08\x08\x08");
    write(&root.join("userdata/1/config/shortcuts.vdf"), "");
    fs::write(root.join("userdata/1/config/shortcuts.vdf"), shortcut).unwrap();
    Launcher {
        kind: LauncherKind::Steam,
        packaging: Packaging::Native,
        root,
    }
}

/// A Heroic with an Epic game set to GE-Proton10-34, a sideloaded one on
/// the default Wine-GE, a native Linux game, and GE-Proton9-1 unused.
pub fn heroic(home: &Path) -> Launcher {
    let root = home.join("heroic");
    for bin in [
        "tools/proton/GE-Proton10-34/proton",
        "tools/proton/GE-Proton9-1/proton",
        "tools/wine/lutris-GE-Proton8-26-x86_64/bin/wine",
        "tools/dxvk/dxvk-2.0/x64/d3d11.dll",
    ] {
        write(&root.join(bin), "");
    }
    let wine = root.join("tools/wine/lutris-GE-Proton8-26-x86_64/bin/wine");
    let proton = root.join("tools/proton/GE-Proton10-34/proton");
    write(
        &root.join("config.json"),
        &serde_json::json!({ "defaultSettings": { "wineVersion": { "bin": wine, "name": "Wine-GE", "type": "wine" } } }).to_string(),
    );
    write(
        &root.join("store_cache/legendary_library.json"),
        &serde_json::json!({ "library": [
            { "app_name": "epic1", "title": "Epic Game", "is_installed": true, "install": { "platform": "Windows" } },
            { "app_name": "epic2", "title": "Native", "is_installed": true, "install": { "platform": "linux" } },
            { "app_name": "epic3", "title": "Not Here", "is_installed": false, "install": {} },
        ] })
        .to_string(),
    );
    write(
        &root.join("sideload_apps/library.json"),
        &serde_json::json!({ "games": [
            { "app_name": "side1", "title": "Sideloaded", "is_installed": true, "install": { "platform": "Windows" } },
        ] })
        .to_string(),
    );
    write(
        &root.join("GamesConfig/epic1.json"),
        &serde_json::to_string_pretty(&serde_json::json!({
            "epic1": { "zfirst": 1, "wineVersion": { "bin": proton, "name": "GE-Proton10-34", "type": "proton" }, "afterwards": true },
            "version": "v0",
        }))
        .unwrap(),
    );
    write(
        &root.join("GamesConfig/side1.json"),
        &serde_json::json!({ "side1": { "language": "" } }).to_string(),
    );
    Launcher {
        kind: LauncherKind::Heroic,
        packaging: Packaging::Native,
        root,
    }
}
