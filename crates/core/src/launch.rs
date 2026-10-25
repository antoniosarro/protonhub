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
}

/// Launch options as text Steam runs through a shell. Values are quoted
/// alone, since a quoted `'VAR=value'` is no assignment, and %command% is
/// left as it is for Steam to find.
impl std::fmt::Display for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // Quoting fails only on a NUL byte, which shlex never splits out.
        let quote = |w: &str| shlex::try_quote(w).map_or_else(|_| w.to_owned(), |q| q.into_owned());
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

/// `NAME=value` as a shell reads an assignment.
fn assignment(word: &str) -> Option<(String, String)> {
    let (key, value) = word.split_once('=')?;
    let mut chars = key.chars();
    let first = chars.next()?;
    let valid = (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then(|| (key.to_owned(), value.to_owned()))
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

        assert!(matches!(set(&l, "4294967291", "x"), Err(Error::Shortcut)));
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
