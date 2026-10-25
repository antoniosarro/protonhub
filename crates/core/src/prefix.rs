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
#[path = "tests/prefix.rs"]
mod tests;
