# Using the GUI

`just gui` builds and runs it from a checkout. It finds the tool
manifests as the CLI does ([usage.md](usage.md)).

## Layout

The sidebar lists the launchers found: Steam, Heroic, Lutris, Bottles and
WineZGUI, with "(Flatpak)" or "(Snap)" when that's how they're installed,
how many games and tools each has, and a mark on one that's running. A
launcher's page has the views **Games** and **Tools**, and on Steam's
**Storage**; launchers without per-game tools (Lutris, Bottles) have only
Tools. In a narrow window the
sidebar and the page take turns, with a back button.

## Games

Every installed game, with its picture and where it's from. A band at
the start of the row is its ProtonDB rating (Steam games), in the colors
of the legend above the list; hovering it shows the tier and the number
of reports. Under the name, a shield shows its anti-cheat status from Are
We Anti-Cheat Yet (Supported, Running, Planned, Broken or Denied), when
the game has anti-cheat. Both are cached for a day; if a source is down,
the page says so and shows the rest. Pictures come from Steam's library
cache, else Steam's CDN, and from Heroic's library; downloaded ones are
cached in `~/.cache/protonhub/files`.

The list at the end of each row is the tool the game runs with: choose
another to change it. On Steam, "Default (…)" leaves the game to Steam's
default. Heroic's list is the one Heroic shows: its own tools, Steam's
Protons in compatibilitytools.d (Valve's Protons too, when Heroic's "show
Valve Proton" setting is on) and Lutris's Wine builds, marked "(Steam)"
or "(Lutris)" where Heroic has the same version. **Search games** filters
the list.
**Move Games…** moves every game set to one tool to another at once. Each
change keeps the previous settings file as `<name>.protonhub-backup`.

A game's row, or its info button, opens its page over the list, with a
back button at the top left (or Escape, Alt+←, or B on a gamepad): its picture, its
ProtonDB rating and anti-cheat status, the tool it runs with (to change,
as in its row), its launch options (Steam), its prefix and shader cache
on Steam (each with its size, a button to open its folder and one to
delete it, as in [Storage](#storage)), its ProtonDB, SteamDB and Steam
store pages, and its app id.

**Launch Options** shows the game's launch options as text, and a switch
for each preset ([usage.md](usage.md#launch-options)): MangoHud, GameMode,
Native Wayland, HDR, NTSync, FSR 4 and DLSS upgrades, and Proton's log.
The switches show what the text has on and edit it; **Save** (or Enter)
writes it to Steam. A switch for a program that isn't installed says so,
since the game wouldn't start with it. Non-Steam shortcuts keep theirs in
Steam.

While Steam or Heroic runs, its games can't be changed (it would overwrite
the change when it exits), and a banner says so. A Heroic game needs its
settings saved in Heroic once before ProtonHub can change it.

## Tools

- When some tools are unused, a row at the top says how much space they
  take, and **Clean Up…** deletes them after listing them once more. It
  deletes tools installed by other apps too.
- **Installed** lists everything in the launcher's tool directories,
  installed by ProtonHub, by hand or by another app. Each shows its size
  and what uses it ("used by Warframe", "used by the default and 3
  games"), and is marked **Default** or **Unused**. Games of other
  launchers count (Heroic can run Steam's tools).
- The info button on an installed tool (or the row itself) opens its
  details: its folder (with a button to open it), whether its checksum
  was verified, every game that runs with it, and **Delete…**. Deleting
  works whether or not something uses the tool; when something does, the
  confirmation lists the games that won't start until they're given
  another tool.
- **From Steam** (on Steam's page) lists Valve's Protons (Proton 10.0,
  Hotfix, Experimental…), which Steam installs as apps of their own, with
  their size and what uses them. Their details can delete one as Steam's
  uninstall does: its app manifest, then its files. Only while Steam
  isn't running, which would undo it; Steam downloads it again when a
  game or the default that uses it starts. Clean Up never deletes these.
- **Available** lists the tools the launcher can use, with the owner's
  avatar where the tool is on GitHub, and marks those kept up to date.

Choosing an available tool opens its dialog: its repository, where it
installs, **Keep up to date**, and its newest releases. "Latest" marks the
newest stable one, installed versions are ticked, the notes button shows a
release's notes, and focus starts on the newest one not installed, so
Enter installs it.

**Keep up to date** makes the tool a channel
([usage.md](usage.md#channels)): turn it on, choose how many versions to
keep and whether games move to new releases, and **Update** installs the
latest release now. The daily timer (`protonhub timer enable`) does the
same unattended.

## Storage

Steam's page has the games' Wine prefixes and shader caches
([usage.md](usage.md#storage)):

- When uninstalled games left some behind, a row at the top says how much
  space they take; it opens to list them, and **Clean Up…** deletes them
  after asking. Saves of games without Steam Cloud are lost with their
  prefixes.
- **Games** lists each game's prefix and shader cache with their sizes,
  the largest first. Its menu resets the prefix (the game makes a new one
  when it next runs, without the saves and settings kept in the old one)
  or deletes the shader cache (Steam builds it again).

Both are off while Steam runs.

## Jobs

An install, removal or cleanup runs in the background, one at a time. A
card at the bottom of the sidebar shows the step (finding the release,
downloading, verifying the checksum, extracting) with a progress bar and
the MiB done; buttons that would start another job are disabled until it
ends. A toast reports the result. After installing into Steam, restart
Steam to see the new tool.

## Errors

A failure shows a toast with **Details**, and a warning button appears in
the header bar. The error log lists every failure with its time and full
message. The copy button on each, or **Copy All**, copies it with
ProtonHub's version and platform, ready for a bug report.

If ProtonHub can't start at all (the registry doesn't load, or `HOME` isn't
set), the window says why instead of showing launchers.

## Gamepad and Steam Deck

A gamepad drives the window: the D-pad or left stick moves the focus, A
activates, B closes a dialog, and the shoulder buttons switch launchers. In
the Steam Deck's Game Mode (gamescope), the window starts fullscreen.

## Language

The window follows the system language where there's a translation
([translating.md](translating.md)), Italian so far. `LANGUAGE=it`
chooses one for a single run.

## Flatpak

Inside its Flatpak, ProtonHub can't see the processes outside the sandbox,
so it can't tell whether Steam or Heroic is running; a banner on the
Games view says so. Quit the launcher before changing games, or it overwrites the change
when it exits.

## Keys

| Key | |
|---|---|
| Ctrl+R, F5 | Look for launchers and tools again |
| Alt+1 … Alt+9 | Show the first … ninth launcher |
| Ctrl+E | The error log |
| Ctrl+Q, Ctrl+W | Close the window |

## Screenshots

`just screenshot OUT.png [STEP...]` runs the GUI in a private headless
sway and saves a screenshot, after sending each STEP to `wtype`
(e.g. `'-k Return'`, `'-M alt -k 2 -m alt'`, `'-s 2000'` to wait). There
is no pointer, so only what keys reach can be shown. Set
`SHOT_HOME` to a fixture home directory with launcher directories in it;
without one there are no launchers. See `scripts/screenshot.sh`.
