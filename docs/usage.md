# Using the CLI

It reads the tool manifests from `$PROTONHUB_REGISTRY`, the ones installed
with it (`share/protonhub/registry`), or `./registry` in a checkout;
`--registry DIR` overrides all three. In the dev shell, `just run <args>`
builds and runs it.

| Command | |
|---|---|
| `protonhub tools` | The tools in the registry and the launchers each supports |
| `protonhub launchers` | The launchers found for you, as `kind:packaging` and their data directory |
| `protonhub releases TOOL [-n N]` | The tool's newest releases, prereleases marked |
| `protonhub install TOOL[@VERSION] [--launcher L]` | Install a release; the version is a tag, or `latest` (the default) |
| `protonhub list` | What is in each launcher's tool directories, and which of it protonhub installed |
| `protonhub remove TOOL@VERSION [--launcher L]` | Remove a version protonhub installed |
| `protonhub games [--launcher L] [--info]` | Steam's and Heroic's installed games and the tool each runs with ("(default)" when it's the launcher's default); `--info` adds ProtonDB's rating and the anti-cheat status (network, cached for a day) |
| `protonhub game GAME [--launcher L] [--info]` | One game: its id, source and tool, its prefix and shader cache (Steam), and with `--info` its ProtonDB rating and anti-cheat status. Every launcher is searched; `--launcher` is needed only when several have a game of that name |
| `protonhub launch GAME [--set TEXT \| --on ID… --off ID…] [--launcher L]` | A Steam or Heroic game's launch options and the presets they have on; `--set` replaces them (`""` removes them), `--on` and `--off` switch presets ([launch options](#launch-options)) |
| `protonhub presets` | The launch option presets: their id, name, what each sets, and what it's for |
| `protonhub assign GAME TOOL [--launcher L]` | Set the tool a game runs with. GAME is its id or name, TOOL a tool's id or name from `games`, or `default` (Steam) |
| `protonhub usage [--launcher L]` | Each installed tool's size and what uses it: games, the default, `unused`, or `-` where ProtonHub can't tell |
| `protonhub cleanup [--launcher L] [--yes]` | List the tools nothing uses and the space they take; `--yes` deletes them |
| `protonhub storage list [--launcher L]` | Each Wine prefix and shader cache in Steam's libraries, largest first, with its app's name, and `(uninstalled)` or `(removed from Steam)` for a leftover |
| `protonhub storage cleanup [--launcher L] [--yes]` | List the prefixes and shader caches of games no longer installed; `--yes` deletes them ([storage](#storage)) |
| `protonhub storage remove GAME --prefix\|--shader-cache [--launcher L]` | Delete one game's prefix, which resets it, or its shader cache, which Steam builds again |
| `protonhub channel add TOOL [--launcher L] [--keep N] [--move-games]` | Keep TOOL at its latest release ([channels](#channels)) |
| `protonhub channel list`, `channel remove TOOL [--launcher L]` | List or stop them |
| `protonhub update [--check] [--notify]` | Update every channel, or with `--check` only say which have a new release |
| `protonhub timer enable`, `timer disable` | Run `update --notify` daily with a systemd user timer |
| `protonhub reassign FROM TO [--launcher L]` | Move every game set to a tool matching FROM (a regex on its id or name) to TO |
| `protonhub changelog TOOL[@VERSION]` | A release's notes |
| `protonhub export [-o FILE]`, `apply FILE [--dry-run]` | Copy what's installed to another machine ([lockfiles](#lockfiles)) |
| `protonhub registry check DIR` | Validate every manifest in DIR |
| `protonhub registry verify DIR` | Resolve every tool's latest release, for every arch, over the network; fails if any can't pick exactly one asset and its checksum |

```sh
protonhub install ge-proton --launcher steam
protonhub install wine-ge@GE-Proton8-26 --launcher lutris
protonhub remove ge-proton@GE-Proton11-7 --launcher steam
```

```sh
protonhub assign "Hollow Knight" GE-Proton11-7 --launcher steam
protonhub assign 367520 default --launcher steam
protonhub cleanup            # what would go
protonhub cleanup --yes      # delete it
```

## Channels

A channel keeps a tool at its latest release in one launcher. Each
`update`:

1. installs the newest stable release, if it isn't there;
2. with `--move-games`, moves the games set to the tool's older versions
   onto it (not while the launcher runs: they're listed as not moved);
3. deletes versions past the newest `--keep` (default 3) that no game in
   any launcher uses. Versions games still use stay, and so does
   everything where ProtonHub can't tell what uses it (Lutris, Bottles,
   WineZGUI, DXVK).

Channels are in `$XDG_CONFIG_HOME/protonhub/channels.toml`; the GUI's
"Keep up to date" edits the same file.

```sh
protonhub channel add ge-proton --launcher steam --keep 2 --move-games
protonhub update --check
protonhub timer enable     # daily, with a notification when something changed
```

Inside the Flatpak, `timer enable` can't reach the host's systemd; it
says so, and a host timer can run
`flatpak run --command=protonhub dev.antoniosarro.ProtonHub update --notify`
instead. home-manager users can declare channels and the timer
([packaging/README.md](../packaging/README.md#nix)).

`timer enable` writes `protonhub-update.service` and `.timer` to
`~/.config/systemd/user` and enables the timer: daily, catching up after
the machine was off, at a random time within the hour. Nothing runs in
between. The service runs the `protonhub` on your PATH, so it survives
upgrades; with a checkout's registry, it names it with
`PROTONHUB_REGISTRY`. Notifications go through `notify-send`; without it,
to the journal (`journalctl --user -u protonhub-update`).

## Lockfiles

`export` writes what ProtonHub installed (not what other apps did) as TOML:
tool, version, launcher and checksum. On another machine, `apply`
installs what's missing, into the launcher of that kind, and leaves the
rest alone. A launcher kind the other machine has twice (native and
Flatpak) is written as `steam:flatpak`; a missing one is skipped with the
reason. If a version's checksum differs from the lockfile's, the release
changed since the export, and `apply` removes it again and fails.

```sh
protonhub export -o protonhub.lock     # desktop
protonhub apply protonhub.lock         # laptop
```

## Launch options

Steam keeps a game's launch options per user, in
`userdata/<account>/config/localconfig.vdf`; ProtonHub changes those of
the user signed in last. They read `VAR=value… program… %command%
arguments…`: variables, the programs the game runs in, and after
`%command%` the game's own arguments. A preset sets variables, adds a
program (MangoHud's `mangohud`, GameMode's `gamemoderun`), or both;
switching it off takes them out again and leaves everything else as it
was. Presets are data, in [`presets.toml`](../crates/core/presets.toml).

Heroic has no such text: a game's settings file
(`GamesConfig/<app>.json`) keeps its variables, the programs it runs in
with their arguments, and its arguments apart, and switches for MangoHud
and GameMode of its own. ProtonHub shows them in the same form, with
Heroic's switches as `mangohud` and `gamemoderun`, and writes them back
apart, setting Heroic's switches for those two programs. A program's
arguments are the words after it up to the next program: one the
settings already had, or a preset's. Like the game's tool, they can be
changed once the game's settings were saved in Heroic.

Like `assign`, changing them is refused while the launcher runs, and the
previous file is kept beside it with `.protonhub-backup` added to its
name.

```sh
protonhub launch "Elden Ring"                      # what it has
protonhub launch "Elden Ring" --on mangohud --on wayland
protonhub launch "Elden Ring" --set "PROTON_LOG=1 %command% -windowed"
```

## Games and tools

`assign` is refused while the launcher runs: Steam and Heroic both write
their settings back when they exit, which would undo it. Before writing,
ProtonHub keeps the previous file next to it as
`<name>.protonhub-backup` (`config/config.vdf` for Steam,
`GamesConfig/<game>.json` for Heroic).

Heroic only has a settings file for a game once its settings were saved
in Heroic; until then `assign` asks you to open them once.

A tool counts as used when any game in any launcher runs it: Heroic games
can run Steam's tools, and Lutris games can name any tool by its path.
Only directories of tools that games are set to are judged (Steam's
`compatibilitytools.d`, Heroic's `tools/proton` and `tools/wine`), so
DXVK and anything in Lutris or Bottles is never called unused. `cleanup`
deletes tools ProtonHub didn't install too, such as ones added by hand or
by another app.

## Storage

Steam keeps a Wine prefix (`steamapps/compatdata/<app id>`) and a shader
cache (`steamapps/shadercache/<app id>`) for each game, in the library
the game is in, and leaves both behind when the game is uninstalled. A
folder counts as left over when no installed app or non-Steam shortcut
has its app id. If a file listing them can't be read, nothing counts as
left over, so a game's prefix is never taken for a leftover.
Leftovers are named from Steam's app cache (`appcache/appinfo.vdf`),
which keeps the names of games no longer installed; a non-Steam shortcut
removed from Steam has no name left anywhere and shows as one.

A prefix holds the saves and settings of games that don't keep them in
Steam Cloud: deleting a leftover prefix loses them for good. Deleting is
refused while Steam runs, since a game or Steam's shader processing could
be using the folder.

```sh
protonhub storage cleanup          # what would go
protonhub storage cleanup --yes    # delete it
protonhub storage remove "Elden Ring" --shader-cache
```

## Choosing a launcher

`--launcher` takes a kind (`steam`, `heroic`, `lutris`, `bottles`, `winezgui`) or a kind
and packaging (`steam:flatpak`, `heroic:native`). Without it, install,
remove and assign use the one launcher found that fits, and ask you to
choose when there are several; games, usage and cleanup cover every
launcher.

## Missing programs

A few tools need a program they don't bring (Roberta needs ScummVM).
They install without it, with a warning naming what's missing.

## Output

Progress goes to stderr: redrawn in place on a terminal, one line per step
otherwise. With `--json`, the result goes to stdout as JSON (install prints
the installed entry and its record). Errors go to stderr, and the exit
status is 1.

## GitHub token

Without a token, GitHub allows 60 API requests an hour. protonhub caches
responses and revalidates them, which doesn't count, so this rarely
matters; when it does, set `GITHUB_TOKEN` (any token, no scopes needed) to
raise the limit to 5000.

## Files

| Path | |
|---|---|
| `$XDG_CACHE_HOME/protonhub/api/` | Cached API responses and their ETags |
| `$XDG_CONFIG_HOME/protonhub/channels.toml` | The channels |
| `~/.config/systemd/user/protonhub-update.{service,timer}` | The timer, once enabled |
| `$XDG_CACHE_HOME/protonhub/downloads/` | Downloads in progress; an interrupted one resumes on the next install |
| `<tool dir>/<version>/.protonhub.json` | The record of a version protonhub installed |
