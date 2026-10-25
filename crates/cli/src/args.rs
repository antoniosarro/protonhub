//! The command line: what `protonhub` takes, as clap reads it.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Install and manage Proton, Wine and DXVK builds across game launchers.
#[derive(Parser)]
#[command(version)]
pub struct Cli {
    /// The directory of tool manifests [default: $PROTONHUB_REGISTRY, the
    /// installed ones, or ./registry]
    #[arg(long, global = true)]
    pub registry: Option<PathBuf>,
    /// Print JSON instead of text
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// List the tools the registry defines
    Tools,
    /// List the launchers found for this user
    Launchers,
    /// List a tool's newest releases
    Releases {
        tool: String,
        /// How many
        #[arg(short, default_value_t = 10)]
        n: usize,
    },
    /// Install TOOL[@VERSION]; the version is a release tag, or latest
    Install {
        spec: String,
        /// The launcher, as KIND or KIND:PACKAGING (steam, heroic:flatpak);
        /// needed when more than one is found
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List what is in each launcher's tool directories
    List,
    /// Remove TOOL@VERSION, installed by protonhub
    Remove {
        spec: String,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List the games of Steam, Heroic and Faugus Launcher and the tool each
    /// runs with
    Games {
        #[arg(long)]
        launcher: Option<String>,
        /// Add ProtonDB's rating and Are We Anti-Cheat Yet's status (network,
        /// cached for a day)
        #[arg(long)]
        info: bool,
    },
    /// Show GAME (its id or name): its tool, and its prefix and shader
    /// cache (Steam)
    Game {
        game: String,
        #[arg(long)]
        launcher: Option<String>,
        /// Add ProtonDB's rating and Are We Anti-Cheat Yet's status (network,
        /// cached for a day)
        #[arg(long)]
        info: bool,
    },
    /// Show GAME's launch options (Steam), or change them: --set replaces
    /// them, --on and --off switch presets
    Launch {
        game: String,
        /// The whole launch options; "" removes them
        #[arg(long, conflicts_with_all = ["on", "off"])]
        set: Option<String>,
        /// Switch a preset on (`protonhub presets` lists them); repeatable
        #[arg(long)]
        on: Vec<String>,
        /// Switch a preset off; repeatable
        #[arg(long)]
        off: Vec<String>,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List the launch option presets and what each sets
    Presets,
    /// Show GAME's Wine prefix and the winetricks verbs in it, install
    /// VERBs with winetricks, or run a program in it, all with the game's
    /// own Wine or Proton (Steam, Heroic)
    Tricks {
        game: String,
        /// Verbs to install: `protonhub verbs` lists common ones, and any
        /// winetricks verb works
        verbs: Vec<String>,
        /// Run winecfg, regedit, control, or a Windows program (.exe, .msi)
        /// in the prefix instead
        #[arg(long, conflicts_with = "verbs")]
        run: Option<String>,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List the winetricks verbs ProtonHub offers, by group
    Verbs,
    /// Set the tool GAME (its id or name) runs with: a tool's id or name,
    /// or "default" for the launcher's default
    Assign {
        game: String,
        tool: String,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Show the default tool of Steam or Faugus Launcher, what games without
    /// one of their own run with, or set it: a tool's id or name, or for
    /// Steam "none" to turn Steam Play off for them
    Default {
        tool: Option<String>,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Show a non-Steam shortcut's name, target and start folder, or change
    /// them; its launch options are `launch`'s
    Shortcut {
        game: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        start_in: Option<String>,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Show Steam's users and the one launch options go to, or choose one:
    /// an account id or name, or "last" for the one signed in last
    SteamUser {
        user: Option<String>,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Add ProtonHub to Steam's library as a non-Steam game, to open it
    /// from Game Mode on a Steam Deck (Steam not running)
    AddToSteam {
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Show each installed tool's size and the games that use it
    Usage {
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List the tools no game uses, and with --yes delete them
    Cleanup {
        #[arg(long)]
        launcher: Option<String>,
        /// Delete them
        #[arg(long)]
        yes: bool,
    },
    /// Steam games' Wine prefixes and shader caches, and those uninstalled
    /// games left behind
    #[command(subcommand)]
    Storage(StorageCommand),
    /// Keep a tool at its latest release in a launcher
    #[command(subcommand)]
    Channel(ChannelCommand),
    /// Update every channel: install new releases, move games if asked,
    /// delete old versions no game uses
    Update {
        /// Only say which channels have a new release
        #[arg(long)]
        check: bool,
        /// Send a desktop notification when something changed or failed
        #[arg(long)]
        notify: bool,
    },
    /// Move every game set to a tool matching FROM (a regex on its id or
    /// name) to the tool TO
    Reassign {
        from: String,
        to: String,
        #[arg(long)]
        launcher: Option<String>,
    },
    /// Print the notes of TOOL[@VERSION]'s release
    Changelog { spec: String },
    /// Write what ProtonHub installed here as a lockfile, to stdout or FILE
    Export {
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Install what a lockfile lists and isn't here
    Apply {
        file: PathBuf,
        /// Only say what would be installed
        #[arg(long)]
        dry_run: bool,
    },
    /// Run `update --notify` daily, with a systemd user timer
    #[command(subcommand)]
    Timer(TimerCommand),
    /// Work with tool manifests
    #[command(subcommand)]
    Registry(RegistryCommand),
}

#[derive(Subcommand)]
pub enum ChannelCommand {
    /// Keep TOOL updated in a launcher
    Add {
        tool: String,
        #[arg(long)]
        launcher: Option<String>,
        /// How many versions to keep, newest first
        #[arg(long, default_value_t = 3)]
        keep: usize,
        /// Move games set to an older version to the new one
        #[arg(long)]
        move_games: bool,
    },
    /// List the channels
    List,
    /// Stop keeping TOOL updated
    Remove {
        tool: String,
        #[arg(long)]
        launcher: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum StorageCommand {
    /// List each prefix and shader cache with its size and game, largest
    /// first
    List {
        #[arg(long)]
        launcher: Option<String>,
    },
    /// List the ones of games no longer installed, and with --yes delete
    /// them; saves not in Steam Cloud are in the prefix
    Cleanup {
        #[arg(long)]
        launcher: Option<String>,
        /// Delete them
        #[arg(long)]
        yes: bool,
    },
    /// Delete GAME's (its app id or name) prefix, which resets it, or its
    /// shader cache, which Steam builds again
    Remove {
        game: String,
        /// The prefix
        #[arg(
            long,
            required_unless_present = "shader_cache",
            conflicts_with = "shader_cache"
        )]
        prefix: bool,
        /// The shader cache
        #[arg(long)]
        shader_cache: bool,
        #[arg(long)]
        launcher: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum TimerCommand {
    /// Install and start the timer
    Enable,
    /// Stop and remove the timer
    Disable,
}

#[derive(Subcommand)]
pub enum RegistryCommand {
    /// Load every manifest in DIR and report the first error
    Check { dir: PathBuf },
    /// Resolve every tool's latest release in DIR, for every arch, and
    /// report the asset and checksum each picks or why it can't (network)
    Verify { dir: PathBuf },
    /// Fetch the tool manifests from ProtonHub's repository now; they're
    /// used over the built-in ones where they load
    Update,
}
