//! A game's Wine prefix, and what runs in it with the game's own Wine or
//! Proton: winetricks verbs, Wine's tools, and Windows programs.
//!
//! A Steam game's prefix is `compatdata/<id>/pfx`, and its Proton the one
//! that last ran it, which Proton names in `compatdata/<id>/config_info`:
//! another version would upgrade the prefix behind Proton's back. A Heroic
//! game's are its settings' `winePrefix` (with `pfx/` in it for Proton) and
//! `wineVersion`.
//!
//! Proton's `wine` is a 32-bit program for a usual Linux system. Where
//! there's no 32-bit loader for it, as on NixOS, everything runs inside
//! `steam-run`, Steam's view of one.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use protonhub_manifest::LauncherKind;
use serde::Deserialize;

use crate::games::{Error, Game};
use crate::launcher::Launcher;

/// The loader 32-bit programs need, missing on NixOS.
const LOADER32: &str = "/lib/ld-linux.so.2";

/// Variables longer than this are left out of Wine's environment.
const LONG: usize = 8000;

/// How many of a failed program's last lines its error keeps.
const TAIL: usize = 30;

/// A game's prefix and the Wine that runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefix {
    /// `WINEPREFIX`: the directory with `drive_c` in it.
    pub dir: PathBuf,
    /// The `wine` binary.
    pub wine: PathBuf,
}

/// `game`'s prefix, for Steam and Heroic games.
pub fn find(launcher: &Launcher, game: &Game) -> Result<Prefix, Error> {
    match launcher.kind {
        LauncherKind::Steam => steam(&launcher.root, game),
        LauncherKind::Heroic => heroic(&launcher.root, game),
        LauncherKind::Faugus => faugus(launcher, game),
        kind => Err(Error::Unsupported {
            launcher: kind.title(),
        }),
    }
}

fn steam(root: &Path, game: &Game) -> Result<Prefix, Error> {
    let no_prefix = || Error::NoPrefix {
        game: game.name.clone(),
    };
    let data = crate::steam::libraries(root)?
        .into_iter()
        .map(|lib| lib.join("steamapps/compatdata").join(&game.id))
        .find(|d| d.join("pfx").is_dir())
        .ok_or_else(no_prefix)?;
    // Its second line is a path in the Proton that last ran it.
    let info = fs::read_to_string(data.join("config_info")).unwrap_or_default();
    let ran = info
        .lines()
        .nth(1)
        .and_then(|l| ["/files/", "/dist/"].iter().find_map(|d| l.split_once(d)))
        .map(|(dir, _)| PathBuf::from(dir));
    let wine = ran
        .iter()
        .chain(game.tool.as_ref().and_then(|t| t.dir.as_ref()))
        .find_map(|dir| proton_wine(dir))
        .ok_or_else(|| Error::NoWine {
            game: game.name.clone(),
        })?;
    Ok(Prefix {
        dir: data.join("pfx"),
        wine,
    })
}

/// A Faugus game's prefix is its `prefix`, Wine's own (umu links Proton's
/// `pfx` to it), run with its runner.
fn faugus(launcher: &Launcher, game: &Game) -> Result<Prefix, Error> {
    let dir = crate::faugus::prefix(launcher, &game.id)?
        .filter(|d| d.join("drive_c").is_dir())
        .ok_or_else(|| Error::NoPrefix {
            game: game.name.clone(),
        })?;
    let wine = game
        .tool
        .as_ref()
        .and_then(|t| t.dir.as_deref())
        .and_then(proton_wine)
        .ok_or_else(|| Error::NoWine {
            game: game.name.clone(),
        })?;
    Ok(Prefix { dir, wine })
}

fn heroic(root: &Path, game: &Game) -> Result<Prefix, Error> {
    let settings =
        crate::heroic::settings(root, &game.id)?.ok_or_else(|| Error::NotConfigured {
            game: game.id.clone(),
        })?;
    let prefix = settings["winePrefix"]
        .as_str()
        .filter(|p| !p.is_empty())
        .map(|p| match p.strip_prefix("~/") {
            Some(rest) => crate::paths::home().unwrap_or_default().join(rest),
            None => PathBuf::from(p),
        })
        .ok_or_else(|| Error::NoPrefix {
            game: game.name.clone(),
        })?;
    let bin = settings["wineVersion"]["bin"]
        .as_str()
        .filter(|b| !b.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Error::NoWine {
            game: game.name.clone(),
        })?;
    // Heroic runs Proton as Steam does: the prefix is in pfx/.
    let (dir, wine) = if settings["wineVersion"]["type"] == "proton" {
        let wine = bin.parent().and_then(proton_wine);
        (prefix.join("pfx"), wine)
    } else {
        (prefix, Some(bin))
    };
    if !dir.is_dir() {
        return Err(Error::NoPrefix {
            game: game.name.clone(),
        });
    }
    let wine = wine.ok_or_else(|| Error::NoWine {
        game: game.name.clone(),
    })?;
    Ok(Prefix { dir, wine })
}

/// Proton's `wine`: in `files/`, or `dist/` before Proton 5.13.
fn proton_wine(dir: &Path) -> Option<PathBuf> {
    ["files/bin/wine", "dist/bin/wine"]
        .iter()
        .map(|b| dir.join(b))
        .find(|b| b.is_file())
}

impl Prefix {
    /// The verbs winetricks installed, from its log in the prefix, in the
    /// order it installed them.
    pub fn installed(&self) -> Vec<String> {
        let log = fs::read_to_string(self.dir.join("winetricks.log")).unwrap_or_default();
        let mut verbs: Vec<String> = Vec::new();
        for verb in log.lines().map(str::trim).filter(|v| !v.is_empty()) {
            if !verbs.iter().any(|v| v == verb) {
                verbs.push(verb.to_owned());
            }
        }
        verbs
    }

    /// The variables that point Wine and winetricks at the prefix and at
    /// this Wine, with its libraries.
    pub fn env(&self) -> Vec<(&'static str, OsString)> {
        let bin = self.wine.parent().unwrap_or(Path::new("/"));
        let mut env = vec![
            ("WINEPREFIX", self.dir.clone().into_os_string()),
            ("WINE", self.wine.clone().into_os_string()),
            ("WINELOADER", self.wine.clone().into_os_string()),
            ("WINESERVER", bin.join("wineserver").into_os_string()),
        ];
        // Each directory once: Wine copies PATH into Windows' own, longer.
        let mut dirs = vec![bin.to_path_buf()];
        for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        env.push(("PATH", std::env::join_paths(dirs).unwrap_or_default()));
        // A build of its own carries its libraries; the system's has them
        // where the loader looks already.
        let root = bin.parent().unwrap_or(Path::new("/"));
        if !root.starts_with("/usr") {
            let join = |dirs: &[&str]| {
                let found: Vec<PathBuf> = dirs
                    .iter()
                    .map(|d| root.join(d))
                    .filter(|d| d.is_dir())
                    .collect();
                std::env::join_paths(found).unwrap_or_default()
            };
            let libs = join(&["lib/x86_64-linux-gnu", "lib/i386-linux-gnu", "lib64", "lib"]);
            let dlls = join(&["lib64/wine", "lib/wine"]);
            if !libs.is_empty() {
                env.push(("LD_LIBRARY_PATH", libs));
            }
            if !dlls.is_empty() {
                env.push(("WINEDLLPATH", dlls));
            }
        }
        env
    }

    /// `program` with `args` in the prefix: inside `steam-run` where Wine
    /// needs it, which gets the variables through `env` since it sets up
    /// its own.
    pub fn command<S: AsRef<OsStr>>(
        &self,
        program: impl AsRef<OsStr>,
        args: impl IntoIterator<Item = S>,
    ) -> Result<Command, Error> {
        let mut cmd = match fhs(&self.wine)? {
            Some(steam_run) => {
                let mut cmd = Command::new(steam_run);
                cmd.arg("env");
                for (key, value) in self.env() {
                    let mut pair = OsString::from(key);
                    pair.push("=");
                    pair.push(value);
                    cmd.arg(pair);
                }
                // It mounts a /tmp of its own, empty: a temporary
                // directory in the host's, as `nix develop` sets, isn't
                // there, and Wine takes TMP and TEMP for Windows' own.
                cmd.args(["TMPDIR=/tmp", "TMP=/tmp", "TEMP=/tmp", "TEMPDIR=/tmp"]);
                cmd.arg(program);
                cmd
            }
            None => {
                let mut cmd = Command::new(program);
                cmd.envs(self.env());
                cmd
            }
        };
        cmd.args(args);
        // Wine copies every variable into Windows' environment, where
        // cmd.exe fails on one past 8191 characters, and winetricks with
        // it; a development shell has some. No Windows program needs them.
        for (key, value) in std::env::vars_os() {
            if value.len() > LONG && key != "PATH" {
                cmd.env_remove(key);
            }
        }
        Ok(cmd)
    }

    /// winetricks installing `verbs` without asking, as far as their
    /// installers let it.
    pub fn winetricks(&self, verbs: &[String]) -> Result<Command, Error> {
        let winetricks = which("winetricks").ok_or(Error::Missing {
            program: "winetricks",
        })?;
        self.command(
            winetricks,
            std::iter::once("-q").chain(verbs.iter().map(String::as_str)),
        )
    }

    /// One of Wine's own programs: winecfg, regedit, control.
    pub fn wine_tool(&self, tool: &str) -> Result<Command, Error> {
        self.command(&self.wine, [tool])
    }

    /// A Windows program, from its directory; an installer package
    /// through msiexec.
    pub fn program(&self, exe: &Path) -> Result<Command, Error> {
        let msi = exe
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("msi"));
        let mut cmd = if msi {
            self.command(
                &self.wine,
                [OsStr::new("msiexec"), OsStr::new("/i"), exe.as_os_str()],
            )?
        } else {
            self.command(&self.wine, [exe])?
        };
        if let Some(dir) = exe.parent() {
            cmd.current_dir(dir);
        }
        Ok(cmd)
    }

    /// Ends every Wine program in the prefix.
    pub fn stop(&self) {
        let server = self.wine.with_file_name("wineserver");
        if let Ok(mut cmd) = self.command(server, ["-k"]) {
            let _ = cmd.stdout(Stdio::null()).stderr(Stdio::null()).status();
        }
    }
}

/// `steam-run`, where `wine` is a 32-bit program without the loader for
/// it; none where it runs as it is.
fn fhs(wine: &Path) -> Result<Option<PathBuf>, Error> {
    if Path::new(LOADER32).exists() || !is_32bit(wine) {
        return Ok(None);
    }
    which("steam-run").map(Some).ok_or(Error::NeedsFhs)
}

/// Whether `path` is a 32-bit ELF program.
fn is_32bit(path: &Path) -> bool {
    let mut head = [0u8; 5];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok_and(|()| head[..4] == *b"\x7fELF" && head[4] == 1)
}

/// `program`'s path, from `PATH`.
pub fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
}

/// Runs `cmd` to its end in a process group of its own, which `cancel`
/// ends, handing its id to `started` and each line it prints to `line`.
/// A failure's error has its last lines.
pub fn run(
    mut cmd: Command,
    started: impl FnOnce(u32),
    line: &mut dyn FnMut(&str),
) -> Result<(), Error> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let io_error = |source| Error::Io {
        path: PathBuf::from(&program),
        source,
    };
    let (reader, writer) = io::pipe().map_err(io_error)?;
    cmd.stdin(Stdio::null())
        .stdout(writer.try_clone().map_err(io_error)?)
        .stderr(writer)
        .process_group(0);
    let mut child = cmd.spawn().map_err(io_error)?;
    // The command holds the pipe's writing ends; the reader sees its end
    // only once they're closed.
    drop(cmd);
    started(child.id());
    let mut tail: Vec<String> = Vec::new();
    for text in BufReader::new(reader).split(b'\n').map_while(Result::ok) {
        // Progress bars redraw a line with carriage returns.
        let text = String::from_utf8_lossy(&text);
        let text = text
            .rsplit('\r')
            .find(|t| !t.trim().is_empty())
            .unwrap_or("");
        if text.trim().is_empty() {
            continue;
        }
        line(text);
        if tail.len() == TAIL {
            tail.remove(0);
        }
        tail.push(text.to_owned());
    }
    let status = child.wait().map_err(io_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Failed {
            program: Path::new(&program)
                .file_name()
                .map_or(program.clone(), |n| n.to_string_lossy().into_owned()),
            status: status.to_string(),
            output: tail.join("\n"),
        })
    }
}

/// Ends a `run` by its process group's id, and the Wine programs it
/// started in `prefix`.
pub fn cancel(prefix: &Prefix, pid: u32) {
    let _ = Command::new("kill")
        .args(["-TERM", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status();
    prefix.stop();
}

/// Starts `cmd` and leaves it running: a window of its own, like winecfg.
pub fn launch(mut cmd: Command) -> Result<(), Error> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| Error::Io {
            path: program.into(),
            source,
        })?;
    // Reaped when it ends, not left a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// A group of verbs on a game's page.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub title: String,
    pub verb: Vec<Verb>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Verb {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// The verbs offered, from verbs.toml, in its order.
pub fn verbs() -> Vec<Group> {
    #[derive(Deserialize)]
    struct File {
        group: Vec<Group>,
    }
    toml::from_str::<File>(include_str!("../verbs.toml"))
        .expect("verbs.toml is checked by the tests")
        .group
}

/// Whether `word` can be a verb: winetricks' are lowercase letters,
/// digits, `_`, `-`, `.` and, for settings, `=`.
pub fn is_verb(word: &str) -> bool {
    !word.is_empty()
        && !word.starts_with('-')
        && word
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-.=".contains(c))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::games::tests::{heroic, steam, write};
    use crate::testutil::TempDir;

    fn game(id: &str, tool_dir: Option<PathBuf>) -> Game {
        Game {
            id: id.into(),
            name: format!("Game {id}"),
            source: "steam",
            tool: tool_dir.map(|dir| crate::games::Choice {
                id: "x".into(),
                name: "x".into(),
                dir: Some(dir),
            }),
            explicit: false,
            epic_namespace: None,
            deck: None,
            art: None,
        }
    }

    #[test]
    fn verbs_load_and_are_verbs() {
        let all: Vec<Verb> = verbs().into_iter().flat_map(|g| g.verb).collect();
        assert!(all.len() > 20);
        for v in &all {
            assert!(is_verb(&v.id), "{}", v.id);
            assert_eq!(
                all.iter().filter(|w| w.id == v.id).count(),
                1,
                "{} twice",
                v.id
            );
        }
        assert!(!is_verb("-q") && !is_verb("a b") && !is_verb("") && is_verb("sound=pulse"));
    }

    #[test]
    fn steams_prefix_runs_with_the_proton_that_last_ran_it() {
        let home = TempDir::new("prefix-steam");
        let l = steam(&home);
        let last = home.join("protons/GE-Proton10-34");
        let set = home.join("protons/GE-Proton11-1");
        for p in [&last, &set] {
            write(&p.join("files/bin/wine"), "");
        }
        let data = l.root.join("steamapps/compatdata/10");
        write(&data.join("pfx/user.reg"), "");
        assert!(matches!(
            find(&l, &game("20", Some(set.clone()))),
            Err(Error::NoPrefix { .. })
        ));

        // No config_info: the game's tool.
        let p = find(&l, &game("10", Some(set.clone()))).unwrap();
        assert_eq!(p.dir, data.join("pfx"));
        assert_eq!(p.wine, set.join("files/bin/wine"));
        write(
            &data.join("config_info"),
            &format!("GE-Proton10-34\n{}/files/share/fonts/\n", last.display()),
        );
        let p = find(&l, &game("10", Some(set))).unwrap();
        assert_eq!(p.wine, last.join("files/bin/wine"), "the one that ran it");
        assert!(matches!(find(&l, &game("10", None)).map(|_| ()), Ok(())));
        fs::remove_dir_all(&last).unwrap();
        assert!(matches!(
            find(&l, &game("10", None)),
            Err(Error::NoWine { .. })
        ));
    }

    #[test]
    fn heroics_prefix_is_in_its_settings() {
        let home = TempDir::new("prefix-heroic");
        let l = heroic(&home);
        let proton = l.root.join("tools/proton/GE-Proton10-34");
        write(&proton.join("files/bin/wine"), "");
        let pfx = home.join("Games/Heroic/Prefixes/epic1");
        fs::create_dir_all(pfx.join("pfx")).unwrap();
        write(
            &l.root.join("GamesConfig/epic1.json"),
            &serde_json::json!({ "epic1": {
                "winePrefix": pfx,
                "wineVersion": { "bin": proton.join("proton"), "name": "GE-Proton10-34", "type": "proton" },
            } })
            .to_string(),
        );
        let p = find(&l, &game("epic1", None)).unwrap();
        assert_eq!(p.dir, pfx.join("pfx"));
        assert_eq!(p.wine, proton.join("files/bin/wine"));
        assert!(matches!(
            find(&l, &game("epic3", None)),
            Err(Error::NotConfigured { .. })
        ));
    }

    #[test]
    fn installed_verbs_and_the_environment() {
        let home = TempDir::new("prefix-env");
        let tool = home.join("GE-Proton10-34/files");
        write(&tool.join("bin/wine"), "");
        fs::create_dir_all(tool.join("lib/x86_64-linux-gnu")).unwrap();
        fs::create_dir_all(tool.join("lib/wine")).unwrap();
        let p = Prefix {
            dir: home.join("pfx"),
            wine: tool.join("bin/wine"),
        };
        assert!(p.installed().is_empty());
        write(
            &p.dir.join("winetricks.log"),
            "vcrun2022\n\nd3dx9\nvcrun2022\n",
        );
        assert_eq!(p.installed(), ["vcrun2022", "d3dx9"]);
        let env = p.env();
        let get = |k: &str| {
            env.iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("WINESERVER"), Some(tool.join("bin/wineserver").into()));
        assert_eq!(
            get("LD_LIBRARY_PATH"),
            Some(
                std::env::join_paths([tool.join("lib/x86_64-linux-gnu"), tool.join("lib")])
                    .unwrap()
            )
        );
        assert_eq!(get("WINEDLLPATH"), Some(tool.join("lib/wine").into()));
        assert!(
            get("PATH")
                .unwrap()
                .to_string_lossy()
                .starts_with(&*tool.join("bin").to_string_lossy())
        );
        // The system's Wine gets no library paths.
        let system = Prefix {
            dir: p.dir.clone(),
            wine: "/usr/bin/wine".into(),
        };
        assert!(system.env().iter().all(|(k, _)| *k != "LD_LIBRARY_PATH"));
    }

    #[test]
    fn runs_report_lines_and_fail_with_their_tail() {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "echo one; printf 'two\\r50%%\\r100%%\\n'; echo three >&2; exit 3",
        ]);
        let mut lines = Vec::new();
        let err = run(cmd, |_| {}, &mut |l| lines.push(l.to_owned())).unwrap_err();
        assert_eq!(lines, ["one", "100%", "three"]);
        match err {
            Error::Failed {
                program, output, ..
            } => {
                assert_eq!(program, "sh");
                assert_eq!(output, "one\n100%\nthree");
            }
            e => panic!("{e}"),
        }
    }

    #[test]
    fn a_cancelled_run_ends_with_its_children() {
        let mut cmd = Command::new("sh");
        // A child that would hold the pipe open, as an installer would.
        cmd.args(["-c", "sleep 30 & sleep 30"]);
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        let runner = std::thread::spawn(move || run(cmd, |pid| tx.send(pid).unwrap(), &mut |_| {}));
        let pid = rx.recv().unwrap();
        let prefix = Prefix {
            dir: "/nonexistent".into(),
            wine: "/nonexistent/bin/wine".into(),
        };
        cancel(&prefix, pid);
        assert!(runner.join().unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
