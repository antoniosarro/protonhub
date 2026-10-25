//! A game's launch options: what Steam runs it with, as
//! `VAR=value… wrapper… %command% arguments…`, and presets that each switch
//! part of that on or off.

use std::collections::BTreeMap;

use protonhub_manifest::LauncherKind;
use serde::{Deserialize, Serialize};

use crate::games::{self, Error};
use crate::launcher::Launcher;
use crate::{heroic, steam};

/// What Steam replaces with the game's own command line.
const COMMAND: &str = "%command%";

/// The compositor a game can run in, at a resolution and refresh rate of
/// its own: `gamescope ARGS… -- %command%`.
pub const GAMESCOPE: &str = "gamescope";

/// Launch options split into their parts. Words ProtonHub doesn't know are
/// kept where they were.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Options {
    /// Variables set before the game, in order.
    pub env: Vec<(String, String)>,
    /// The words between them and %command%: the programs the game runs
    /// in, with their arguments.
    pub wrappers: Vec<String>,
    /// Whether there is a %command%. Without one, Steam adds the options
    /// to the game's command line, so they're all arguments.
    pub command: bool,
    /// The game's own arguments.
    pub args: Vec<String>,
}

impl Options {
    /// None for text a shell couldn't split, like an unclosed quote.
    pub fn parse(text: &str) -> Option<Self> {
        let words = shlex::split(text)?;
        let Some(at) = words.iter().position(|w| w == COMMAND) else {
            return Some(Self {
                args: words,
                ..Self::default()
            });
        };
        let mut options = Self {
            command: true,
            args: words[at + 1..].to_vec(),
            ..Self::default()
        };
        for word in &words[..at] {
            // An assignment after a program is that program's argument.
            match assignment(word) {
                Some(pair) if options.wrappers.is_empty() => options.env.push(pair),
                _ => options.wrappers.push(word.clone()),
            }
        }
        Some(options)
    }

    /// Whether everything `preset` sets is set.
    pub fn has(&self, preset: &Preset) -> bool {
        preset
            .env
            .iter()
            .all(|(k, v)| self.env.iter().any(|(ek, ev)| ek == k && ev == v))
            && preset
                .wrapper
                .as_ref()
                .is_none_or(|w| self.wrappers.contains(w))
    }

    /// Switches `preset` on, or off: its variables and its program.
    pub fn set(&mut self, preset: &Preset, on: bool) {
        for (key, value) in &preset.env {
            match (self.env.iter().position(|(k, _)| k == key), on) {
                (Some(i), true) => self.env[i].1 = value.clone(),
                (None, true) => self.env.push((key.clone(), value.clone())),
                (_, false) => self.env.retain(|(k, _)| k != key),
            }
        }
        if let Some(wrapper) = &preset.wrapper {
            let at = self.wrappers.iter().position(|w| w == wrapper);
            match at {
                // Outermost: it runs whatever was there.
                None if on => self.wrappers.insert(0, wrapper.clone()),
                Some(i) if !on => {
                    self.wrappers.remove(i);
                }
                _ => {}
            }
        }
    }

    /// gamescope's arguments, when the game runs in it.
    pub fn gamescope(&self) -> Option<&[String]> {
        let (at, end) = self.gamescope_span()?;
        Some(&self.wrappers[at + 1..end])
    }

    /// Runs the game in gamescope with `args`, innermost, or with none not
    /// in it.
    pub fn set_gamescope(&mut self, args: Option<Vec<String>>) {
        let words: Vec<String> = args
            .map(|args| {
                let mut words = vec![GAMESCOPE.to_owned()];
                words.extend(args);
                words.push("--".to_owned());
                words
            })
            .unwrap_or_default();
        match self.gamescope_span() {
            Some((at, end)) => {
                let end = (end + 1).min(self.wrappers.len());
                self.wrappers.splice(at..end, words);
            }
            None => self.wrappers.extend(words),
        }
    }

    /// Where gamescope is among the programs, and where its `--` is, or the
    /// end.
    fn gamescope_span(&self) -> Option<(usize, usize)> {
        let at = self.wrappers.iter().position(|w| w == GAMESCOPE)?;
        let end = self.wrappers[at..]
            .iter()
            .position(|w| w == "--")
            .map_or(self.wrappers.len(), |i| at + i);
        Some((at, end))
    }
}

/// The value of a program's option among its `args`, written `-W 1920`,
/// `--output-width 1920` or `--output-width=1920`.
pub fn option(args: &[String], names: &[&str]) -> Option<String> {
    args.iter().enumerate().find_map(|(i, arg)| {
        if names.contains(&arg.as_str()) {
            return args.get(i + 1).cloned();
        }
        names
            .iter()
            .find_map(|n| arg.strip_prefix(n)?.strip_prefix('='))
            .map(str::to_owned)
    })
}

/// Sets a program's option to `value` where it was, written the first of
/// `names`' way, or with none takes it out, however it was written.
pub fn set_option(args: &mut Vec<String>, names: &[&str], value: Option<&str>) {
    let mut at = None;
    let mut i = 0;
    while i < args.len() {
        if names.contains(&args[i].as_str()) {
            at.get_or_insert(i);
            args.drain(i..(i + 2).min(args.len()));
        } else if names
            .iter()
            .any(|n| args[i].strip_prefix(n).is_some_and(|r| r.starts_with('=')))
        {
            at.get_or_insert(i);
            args.remove(i);
        } else {
            i += 1;
        }
    }
    if let Some(value) = value {
        let at = at.unwrap_or(args.len());
        args.splice(at..at, [names[0].to_owned(), value.to_owned()]);
    }
}

/// Whether a program's switch, an option without a value, is among its
/// `args`.
pub fn flag(args: &[String], names: &[&str]) -> bool {
    args.iter().any(|a| names.contains(&a.as_str()))
}

/// Switches a program's switch on, or off however it was written.
pub fn set_flag(args: &mut Vec<String>, names: &[&str], on: bool) {
    if !on {
        args.retain(|a| !names.contains(&a.as_str()));
    } else if !flag(args, names) {
        args.push(names[0].to_owned());
    }
}

/// Launch options as text Steam runs through a shell. Values are quoted
/// alone, since a quoted `'VAR=value'` is no assignment, and %command% is
/// left as it is for Steam to find.
impl std::fmt::Display for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let mut words: Vec<String> = self
            .env
            .iter()
            .map(|(k, v)| format!("{k}={}", quote(v)))
            .collect();
        words.extend(self.wrappers.iter().map(|w| quote(w)));
        if self.command || !words.is_empty() {
            words.push(COMMAND.to_owned());
        }
        words.extend(self.args.iter().map(|w| quote(w)));
        f.write_str(&words.join(" "))
    }
}

/// Words as a shell splits them; none for text it couldn't, like an
/// unclosed quote.
pub fn split(text: &str) -> Option<Vec<String>> {
    shlex::split(text)
}

/// Words quoted where needed, as a shell splits them back.
pub fn join(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

fn quote(word: &str) -> String {
    // Quoting fails only on a NUL byte, which shlex never splits out.
    shlex::try_quote(word).map_or_else(|_| word.to_owned(), |q| q.into_owned())
}

/// Whether `key` can be a variable's name: a shell reads `key=value` as
/// setting it.
pub fn is_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `NAME=value` as a shell reads an assignment.
fn assignment(word: &str) -> Option<(String, String)> {
    let (key, value) = word.split_once('=')?;
    is_name(key).then(|| (key.to_owned(), value.to_owned()))
}

/// A switch on a game's page: variables to set, a program to run it in, or
/// both.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub wrapper: Option<String>,
    /// The heading it's under on the page; none for the first switches.
    pub group: Option<String>,
}

/// The presets, from presets.toml, in its order.
pub fn presets() -> Vec<Preset> {
    #[derive(Deserialize)]
    struct File {
        preset: Vec<Preset>,
    }
    toml::from_str::<File>(include_str!("../presets.toml"))
        .expect("presets.toml is checked by the tests")
        .preset
}

/// The launch options of each game that has some, by id, or none for a
/// launcher ProtonHub can't read them from.
pub fn all(launcher: &Launcher) -> Result<Option<Vec<(String, String)>>, Error> {
    match launcher.kind {
        LauncherKind::Steam => steam::launch_options(&launcher.root).map(Some),
        LauncherKind::Heroic => heroic::launch_options(&launcher.root).map(Some),
        _ => Ok(None),
    }
}

/// Sets `game`'s launch options to `text`, as it is; empty removes them.
/// Refused while the launcher runs, which would write its own back.
pub fn set(launcher: &Launcher, game: &str, text: &str) -> Result<(), Error> {
    if games::is_running(launcher) {
        return Err(Error::Running {
            launcher: launcher.kind.title(),
        });
    }
    match launcher.kind {
        LauncherKind::Steam => steam::set_launch_options(&launcher.root, game, text),
        LauncherKind::Heroic => heroic::set_launch_options(&launcher.root, game, text),
        kind => Err(Error::Unsupported {
            launcher: kind.title(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::games::tests::{heroic, steam, write};
    use crate::testutil::TempDir;

    fn preset(id: &str) -> Preset {
        presets().into_iter().find(|p| p.id == id).unwrap()
    }

    #[test]
    fn presets_load_and_each_does_something() {
        let all = presets();
        assert!(all.len() >= 2);
        for p in &all {
            assert!(
                !p.env.is_empty() || p.wrapper.is_some(),
                "{} sets nothing",
                p.id
            );
            assert_eq!(
                all.iter().filter(|q| q.id == p.id).count(),
                1,
                "{} twice",
                p.id
            );
        }
    }

    #[test]
    fn splits_and_keeps_unknown_words_in_place() {
        let o = Options::parse("PROTON_LOG=1 gamemoderun protonhax init %command% -dx11").unwrap();
        assert_eq!(o.env, [("PROTON_LOG".into(), "1".into())]);
        assert_eq!(o.wrappers, ["gamemoderun", "protonhax", "init"]);
        assert_eq!(o.args, ["-dx11"]);
        assert_eq!(
            o.to_string(),
            "PROTON_LOG=1 gamemoderun protonhax init %command% -dx11"
        );
        // An assignment after a program is the program's.
        let o = Options::parse("env FOO=1 %command%").unwrap();
        assert!(o.env.is_empty());
        assert_eq!(o.wrappers, ["env", "FOO=1"]);
    }

    #[test]
    fn without_command_everything_is_an_argument() {
        let mut o = Options::parse("-novid -high").unwrap();
        assert_eq!(o.args, ["-novid", "-high"]);
        assert_eq!(o.to_string(), "-novid -high");
        o.set(&preset("log"), true);
        assert_eq!(o.to_string(), "PROTON_LOG=1 %command% -novid -high");
    }

    #[test]
    fn switches_presets_on_and_off() {
        let mut o = Options::parse("gamemoderun protonhax init %command%").unwrap();
        let (mangohud, gamemode, wayland) =
            (preset("mangohud"), preset("gamemode"), preset("wayland"));
        assert!(o.has(&gamemode) && !o.has(&mangohud) && !o.has(&wayland));
        o.set(&mangohud, true);
        o.set(&wayland, true);
        assert_eq!(
            o.to_string(),
            "PROTON_ENABLE_WAYLAND=1 mangohud gamemoderun protonhax init %command%"
        );
        o.set(&mangohud, true);
        assert_eq!(o.wrappers.len(), 4, "on twice is on once");
        o.set(&gamemode, false);
        o.set(&wayland, false);
        o.set(&mangohud, false);
        assert_eq!(o.to_string(), "protonhax init %command%");
        o.set(&gamemode, false);
        assert_eq!(
            o.to_string(),
            "protonhax init %command%",
            "off twice is off"
        );
    }

    #[test]
    fn a_preset_with_another_value_is_off_and_takes_its_value() {
        let mut o = Options::parse("PROTON_LOG=0 %command%").unwrap();
        assert!(!o.has(&preset("log")));
        o.set(&preset("log"), true);
        assert_eq!(o.to_string(), "PROTON_LOG=1 %command%");
    }

    #[test]
    fn quotes_values_alone_and_rejects_unclosed_quotes() {
        let o = Options::parse(
            "DXVK_HUD='fps,frametimes' WINEDLLOVERRIDES=\"a b\" %command% --name 'x y'",
        )
        .unwrap();
        assert_eq!(o.env[1], ("WINEDLLOVERRIDES".into(), "a b".into()));
        let text = o.to_string();
        assert!(
            text.starts_with("DXVK_HUD='fps,frametimes' WINEDLLOVERRIDES='a b' %command%"),
            "{text}"
        );
        assert_eq!(Options::parse(&text), Some(o), "it reads back");
        assert_eq!(Options::parse("FOO='open %command%"), None);
        assert_eq!(Options::parse("").unwrap().to_string(), "");
    }

    #[test]
    fn runs_the_game_in_gamescope_and_edits_its_options() {
        let mut o = Options::parse("mangohud %command% -novid").unwrap();
        assert_eq!(o.gamescope(), None);
        o.set_gamescope(Some(vec!["-f".into()]));
        assert_eq!(o.to_string(), "mangohud gamescope -f -- %command% -novid");

        let mut o =
            Options::parse("gamescope --output-width=1280 -r 60 -- protonhax init %command%")
                .unwrap();
        let mut args = o.gamescope().unwrap().to_vec();
        assert_eq!(
            option(&args, &["-W", "--output-width"]).as_deref(),
            Some("1280")
        );
        assert_eq!(
            option(&args, &["-r", "--nested-refresh"]).as_deref(),
            Some("60")
        );
        assert_eq!(option(&args, &["-H", "--output-height"]), None);
        set_option(&mut args, &["-W", "--output-width"], Some("2560"));
        set_option(&mut args, &["-r", "--nested-refresh"], None);
        set_flag(&mut args, &["-f", "--fullscreen"], true);
        set_flag(&mut args, &["-f", "--fullscreen"], true);
        assert_eq!(args, ["-W", "2560", "-f"]);
        assert!(flag(&args, &["-f", "--fullscreen"]));
        o.set_gamescope(Some(args));
        assert_eq!(
            o.to_string(),
            "gamescope -W 2560 -f -- protonhax init %command%",
            "the programs after it are kept"
        );
        o.set_gamescope(None);
        assert_eq!(o.to_string(), "protonhax init %command%");

        // Without its --, the rest are gamescope's.
        let mut o = Options::parse("gamescope -f %command%").unwrap();
        assert_eq!(o.gamescope().unwrap(), ["-f"]);
        o.set_gamescope(None);
        assert_eq!(o.to_string(), "%command%");
    }

    fn local_config(root: &Path, account: &str, apps: &str) -> std::path::PathBuf {
        let path = root.join(format!("userdata/{account}/config/localconfig.vdf"));
        write(
            &path,
            &format!(
                "\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n{apps}\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n\t\"friends\"\t\t\"kept\"\n}}\n"
            ),
        );
        path
    }

    #[test]
    fn reads_and_writes_steams_launch_options() {
        let home = TempDir::new("launch-steam");
        let l = steam(&home);
        let path = local_config(
            &l.root,
            "1",
            "\t\t\t\t\t\"10\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"5\"\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"gamemoderun %command%\"\n\t\t\t\t\t}\n\t\t\t\t\t\"30\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"\"\n\t\t\t\t\t}\n",
        );
        assert_eq!(
            all(&l).unwrap().unwrap(),
            [("10".to_owned(), "gamemoderun %command%".to_owned())],
            "empty ones left out"
        );

        set(&l, "20", "PROTON_LOG=1 %command%").unwrap();
        set(&l, "10", "").unwrap();
        let options = all(&l).unwrap().unwrap();
        assert_eq!(
            options,
            [("20".to_owned(), "PROTON_LOG=1 %command%".to_owned())]
        );
        let after = fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("\"LastPlayed\"\t\t\"5\"") && after.contains("\"friends\"\t\t\"kept\"")
        );
        let backup = path.with_file_name("localconfig.vdf.protonhub-backup");
        // The file before the last change: 20's set, 10's not yet removed.
        let backup = fs::read_to_string(backup).unwrap();
        assert!(backup.contains("gamemoderun %command%") && backup.contains("PROTON_LOG=1"));

        // A shortcut's are in its entry in shortcuts.vdf.
        set(&l, "4294967291", "-novid").unwrap();
        let options = all(&l).unwrap().unwrap();
        assert!(options.contains(&("4294967291".to_owned(), "-novid".to_owned())));
        let shortcuts = crate::steam::shortcuts(&l.root).unwrap();
        assert_eq!(shortcuts[0].1.name, "My Shortcut", "the rest kept");
        assert!(matches!(set(&l, "99", "x"), Err(Error::UnknownGame { .. })));
    }

    #[test]
    fn steams_user_is_the_one_signed_in_last_or_the_only_one() {
        let home = TempDir::new("launch-users");
        let l = steam(&home);
        local_config(
            &l.root,
            "1",
            "\t\t\t\t\t\"10\" { \"LaunchOptions\" \"one\" }\n",
        );
        local_config(
            &l.root,
            "2",
            "\t\t\t\t\t\"10\" { \"LaunchOptions\" \"two\" }\n",
        );
        assert!(
            matches!(all(&l), Err(Error::Format { .. })),
            "two users, neither last"
        );
        write(
            &l.root.join("config/loginusers.vdf"),
            "\"users\"\n{\n\t\"76561197960265729\"\n\t{\n\t\t\"MostRecent\"\t\t\"0\"\n\t}\n\t\"76561197960265730\"\n\t{\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n",
        );
        assert_eq!(all(&l).unwrap().unwrap()[0].1, "two");
        fs::remove_file(l.root.join("config/loginusers.vdf")).unwrap();
        fs::remove_dir_all(l.root.join("userdata/2")).unwrap();
        assert_eq!(all(&l).unwrap().unwrap()[0].1, "one", "the only one");
    }

    #[test]
    fn reads_and_writes_heroics_launch_options() {
        let home = TempDir::new("launch-heroic");
        let l = heroic(&home);
        let path = l.root.join("GamesConfig/side1.json");
        write(
            &path,
            &serde_json::json!({ "side1": {
                "language": "kept",
                "showMangohud": false,
                "useGameMode": true,
                "enviromentOptions": [{ "key": "DXVK_HUD", "value": "fps,gpu" }],
                "wrapperOptions": [{ "exe": "gamescope", "args": "-W 1920 --" }, { "exe": "protonhax", "args": "init" }],
                "launcherArgs": "-novid 'a b'",
            } })
            .to_string(),
        );
        let text = "DXVK_HUD='fps,gpu' gamemoderun gamescope -W 1920 -- protonhax init %command% -novid 'a b'";
        assert_eq!(
            all(&l).unwrap().unwrap(),
            [("side1".to_owned(), text.to_owned())],
            "epic1 has none"
        );

        // MangoHud on, GameMode off: Heroic's own switches. A new word
        // after a program is that program's.
        let mut o = Options::parse(text).unwrap();
        o.set(&preset("mangohud"), true);
        o.set(&preset("gamemode"), false);
        o.set(&preset("wayland"), true);
        o.wrappers.push("--fullscreen".into());
        set(&l, "side1", &o.to_string()).unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let s = &after["side1"];
        assert_eq!(s["language"], "kept");
        assert_eq!(
            (&s["showMangohud"], &s["useGameMode"]),
            (&true.into(), &false.into())
        );
        assert_eq!(
            s["enviromentOptions"],
            serde_json::json!([
                { "key": "DXVK_HUD", "value": "fps,gpu" },
                { "key": "PROTON_ENABLE_WAYLAND", "value": "1" },
            ])
        );
        assert_eq!(
            s["wrapperOptions"],
            serde_json::json!([
                { "exe": "gamescope", "args": "-W 1920 --" },
                { "exe": "protonhax", "args": "init --fullscreen" },
            ])
        );
        assert_eq!(s["launcherArgs"], "-novid 'a b'");
        assert!(path.with_file_name("side1.json.protonhub-backup").is_file());

        set(&l, "side1", "").unwrap();
        assert!(all(&l).unwrap().unwrap().is_empty(), "all removed");
        assert!(matches!(
            set(&l, "epic3", "x"),
            Err(Error::NotConfigured { .. })
        ));
        assert!(matches!(set(&l, "side1", "'x"), Err(Error::Unclosed)));
    }
}
