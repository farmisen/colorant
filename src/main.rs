mod cli;
mod commands;
mod config;
mod mode;
mod shell;
mod terminal;
mod theme;
mod walk;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let config = config::Config::load()?;
    match cli.command {
        cli::Command::Apply => commands::apply::run(&config),
        cli::Command::Reset => commands::reset::run(),
        cli::Command::Current => commands::current::run(),
        cli::Command::Init { shell } => commands::init::run(shell),
    }
}
