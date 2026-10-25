//! `protonhub`, the command line: each command is a function in
//! [`commands`], over protonhub-core.

mod args;
mod commands;
mod common;
mod progress;
mod timer;

use std::process::ExitCode;

use clap::Parser;

use args::{Cli, Command, RegistryCommand, StorageCommand, TimerCommand};
use commands::{channels, games, launch, lock, registry, storage, tools};
use common::{Cx, Result};

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("protonhub: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let cx = Cx {
        json: cli.json,
        registry: cli.registry,
    };
    let cx = &cx;
    match cli.command {
        Command::Tools => tools::tools(cx),
        Command::Launchers => tools::launchers(cx),
        Command::Releases { tool, n } => tools::releases(cx, tool, n),
        Command::Install { spec, launcher } => tools::install(cx, spec, launcher),
        Command::List => tools::list(cx),
        Command::Remove { spec, launcher } => tools::remove(cx, spec, launcher),
        Command::Usage { launcher } => tools::usage(cx, launcher),
        Command::Cleanup { launcher, yes } => tools::cleanup(cx, launcher, yes),
        Command::Changelog { spec } => tools::changelog(cx, spec),
        Command::Games { launcher, info } => games::games(cx, launcher, info),
        Command::Game {
            game,
            launcher,
            info,
        } => games::game(cx, game, launcher, info),
        Command::Assign {
            game,
            tool,
            launcher,
        } => games::assign(cx, game, tool, launcher),
        Command::Default { tool, launcher } => games::default(cx, tool, launcher),
        Command::Reassign { from, to, launcher } => games::reassign(cx, from, to, launcher),
        Command::Shortcut {
            game,
            name,
            target,
            start_in,
            launcher,
        } => games::shortcut(cx, game, name, target, start_in, launcher),
        Command::SteamUser { user, launcher } => games::steam_user(cx, user, launcher),
        Command::AddToSteam { launcher } => games::add_to_steam(cx, launcher),
        Command::Launch {
            game,
            set,
            on,
            off,
            launcher,
        } => launch::launch(cx, game, set, on, off, launcher),
        Command::Presets => launch::presets(cx),
        Command::Tricks {
            game,
            verbs,
            run,
            launcher,
        } => launch::tricks(cx, game, verbs, run, launcher),
        Command::Verbs => launch::verbs(cx),
        Command::Storage(StorageCommand::List { launcher }) => storage::list(cx, launcher),
        Command::Storage(StorageCommand::Cleanup { launcher, yes }) => {
            storage::cleanup(cx, launcher, yes)
        }
        Command::Storage(StorageCommand::Remove {
            game,
            prefix,
            shader_cache: _,
            launcher,
        }) => storage::remove(cx, game, prefix, launcher),
        Command::Channel(cmd) => channels::channel(cx, cmd),
        Command::Update { check, notify } => channels::update(cx, check, notify),
        Command::Export { output } => lock::export(cx, output),
        Command::Apply { file, dry_run } => lock::apply(cx, file, dry_run),
        Command::Timer(TimerCommand::Enable) => registry::timer_enable(cx),
        Command::Timer(TimerCommand::Disable) => registry::timer_disable(cx),
        Command::Registry(RegistryCommand::Verify { dir }) => registry::verify(cx, dir),
        Command::Registry(RegistryCommand::Update) => registry::update(cx),
        Command::Registry(RegistryCommand::Check { dir }) => registry::check(cx, dir),
    }
}
