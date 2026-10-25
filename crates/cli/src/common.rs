//! What the commands share: how they print, the directories and client
//! they use, and finding the tool, launcher or game a command names.

use std::path::PathBuf;

use protonhub_core::channels::Channels;
use protonhub_core::games;
use protonhub_core::launcher::{self, Launcher};
use protonhub_core::manifest::{Arch, LauncherKind, Registry, Tool};
use protonhub_core::manifests;
use protonhub_core::paths;
use protonhub_core::releases::{self, Client, Version};
use serde::Serialize;

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

/// What every command is given: the options before it.
pub struct Cx {
    pub json: bool,
    /// The directory of tool manifests named with --registry.
    pub registry: Option<PathBuf>,
}

impl Cx {
    /// The tool manifests: --registry's, else the default ones.
    pub fn registry(&self) -> Result<Registry> {
        Ok(manifests::load_default(self.registry.as_deref())?)
    }

    /// Where the tool manifests are: --registry, else the default.
    pub fn registry_dir(&self) -> PathBuf {
        self.registry.clone().unwrap_or_else(paths::registry_dir)
    }

    /// Prints `text`, or `value` as JSON with --json.
    pub fn print<T: Serialize + ?Sized>(&self, text: String, value: &T) -> Result<()> {
        if self.json {
            println!("{}", serde_json::to_string_pretty(value)?);
        } else if !text.is_empty() {
            println!("{text}");
        }
        Ok(())
    }
}

pub fn arch() -> Result<Arch> {
    Arch::host().ok_or_else(|| "this CPU architecture has no builds".into())
}

pub fn channels_path() -> Result<PathBuf> {
    Channels::path().ok_or_else(|| "neither XDG_CONFIG_HOME nor HOME is set".into())
}

pub fn home() -> Result<PathBuf> {
    paths::home().ok_or_else(|| "HOME is not set".into())
}

pub fn cache_dir() -> Result<PathBuf> {
    paths::cache_dir().ok_or_else(|| "neither XDG_CACHE_HOME nor HOME is set".into())
}

pub fn client() -> Result<Client> {
    Ok(Client::new(
        releases::API,
        cache_dir()?,
        paths::github_token(),
    ))
}

/// `tool@version` → ("tool", Some("version")); `tool` and `tool@latest`
/// have no version.
pub fn parse_spec(spec: &str) -> (&str, Option<&str>) {
    match spec.split_once('@') {
        Some((tool, "latest")) | Some((tool, "")) => (tool, None),
        Some((tool, version)) => (tool, Some(version)),
        None => (spec, None),
    }
}

pub fn version(tag: Option<&str>) -> Version {
    tag.map_or(Version::Latest, |t| Version::Tag(t.into()))
}

/// The day of an ISO 8601 time: 2026-10-01T12:00:00Z is 2026-10-01.
pub fn date(time: Option<&str>) -> &str {
    time.and_then(|t| t.split('T').next()).unwrap_or("")
}

/// "1 game", "3 games".
pub fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

pub fn find_tool<'a>(registry: &'a Registry, id: &str) -> Result<&'a Tool> {
    registry.tools.get(id).ok_or_else(|| {
        let known: Vec<_> = registry.tools.keys().map(String::as_str).collect();
        format!("no tool {id} in the registry; it has: {}", known.join(", ")).into()
    })
}

/// The one detected launcher that `spec` (KIND or KIND:PACKAGING, or none
/// at all) names and `tool` supports.
pub fn pick_launcher(tool: &Tool, spec: Option<&str>) -> Result<Launcher> {
    let kinds: Vec<_> = tool.launchers.keys().map(|k| k.name()).collect();
    pick_one(
        spec,
        |l| l.tool_dir(tool).is_some(),
        &format!("can take {} ({})", tool.id, kinds.join(", ")),
    )
}

/// The one detected launcher that `spec` names among those `keep` accepts;
/// `what` says what they have in common, for the errors.
pub fn pick_one(
    spec: Option<&str>,
    keep: impl Fn(&Launcher) -> bool,
    what: &str,
) -> Result<Launcher> {
    let (supported, mut matching) = detected(spec, keep)?;
    let names = |ls: &[Launcher]| {
        ls.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match matching.len() {
        1 => Ok(matching.remove(0)),
        0 if supported.is_empty() => Err(format!("no launcher was found that {what}").into()),
        0 => Err(format!(
            "no launcher matches {}; found: {}",
            spec.unwrap_or_default(),
            names(&supported)
        )
        .into()),
        _ => Err(format!(
            "several launchers {what}: {}; choose one with --launcher",
            names(&matching)
        )
        .into()),
    }
}

/// The detected launchers `keep` accepts, and of those the ones `spec`
/// (KIND or KIND:PACKAGING, or none at all) names.
pub fn detected(
    spec: Option<&str>,
    keep: impl Fn(&Launcher) -> bool,
) -> Result<(Vec<Launcher>, Vec<Launcher>)> {
    let supported: Vec<_> = launcher::detect(&home()?)
        .into_iter()
        .filter(keep)
        .collect();
    let matching = supported
        .iter()
        .filter(|l| spec.is_none_or(|s| l.matches(s)))
        .cloned()
        .collect();
    Ok((supported, matching))
}

pub fn is_steam(l: &Launcher) -> bool {
    l.kind == LauncherKind::Steam
}

/// Launchers that keep a compatibility tool per game.
pub fn has_games(l: &Launcher) -> bool {
    matches!(
        l.kind,
        LauncherKind::Steam | LauncherKind::Heroic | LauncherKind::Faugus
    )
}

/// The launcher with an installed game whose id or name is `game`, and
/// the game: every launcher `spec` names is searched, and the game must be
/// in exactly one.
pub fn locate(spec: Option<&str>, game: &str) -> Result<(Launcher, games::Game)> {
    let mut found = Vec::new();
    for l in detected(spec, has_games)?.1 {
        let library = games::library(&l)?.expect("steam and heroic have games");
        if let Some(g) = library
            .games
            .iter()
            .find(|g| g.id == game || g.name.eq_ignore_ascii_case(game))
        {
            found.push((l, g.clone()));
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!("no installed game {game}; `protonhub games` lists them").into()),
        _ => {
            let names: Vec<_> = found.iter().map(|(l, _)| l.to_string()).collect();
            Err(format!(
                "{game} is in {}; choose one with --launcher",
                names.join(", ")
            )
            .into())
        }
    }
}

pub fn find_game<'a>(library: &'a games::Library, game: &str) -> Result<&'a games::Game> {
    let found: Vec<_> = library
        .games
        .iter()
        .filter(|g| g.id == game || g.name.eq_ignore_ascii_case(game))
        .collect();
    match found[..] {
        [one] => Ok(one),
        [] => Err(format!("no installed game {game}; `protonhub games` lists them").into()),
        _ => Err(format!("several games are called {game}; use its id").into()),
    }
}

pub fn find_choice<'a>(library: &'a games::Library, tool: &str) -> Result<&'a games::Choice> {
    library
        .choices
        .iter()
        .find(|c| c.id == tool || c.name.eq_ignore_ascii_case(tool))
        .ok_or_else(|| {
            let names: Vec<_> = library.choices.iter().map(|c| c.name.as_str()).collect();
            format!("no tool {tool}; the launcher has: {}", names.join(", ")).into()
        })
}

/// Warns that from inside the Flatpak, nobody can tell whether `target`
/// runs, and would undo a change when it exits.
pub fn warn_unseen(target: &Launcher) {
    if !games::sees_launchers() {
        eprintln!(
            "protonhub: warning: inside the Flatpak, ProtonHub can't tell whether {target} is running; \
             if it is, it overwrites this change when it exits"
        );
    }
}

/// The tool a game runs with, marked when it's the launcher's default.
pub fn tool_label(game: &games::Game) -> String {
    match (&game.tool, game.explicit) {
        (Some(t), true) => t.name.clone(),
        (Some(t), false) => format!("{} (default)", t.name),
        (None, _) => "-".into(),
    }
}

/// The last line of a cleanup of `empty` or not: `nothing` to do, what it
/// freed, or with the size `pending`, what --yes would.
pub fn freed(empty: bool, total: u64, yes: bool, nothing: &str, pending: &str) -> String {
    let size = protonhub_core::install::human_size(total);
    match (empty, yes) {
        (true, _) => nothing.into(),
        (false, true) => format!("freed {size}"),
        (false, false) => format!("{size} {pending}"),
    }
}
