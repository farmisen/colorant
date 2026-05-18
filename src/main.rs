mod cli;
mod commands;
mod config;
mod fs_util;
mod mode;
mod shell;
mod terminal;
mod theme;
mod walk;

use anyhow::Result;
use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Error: {e:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = cli::Cli::parse();
    let config = config::Config::load()?;
    Ok(match cli.command {
        cli::Command::Apply => {
            commands::apply::run(&config)?;
            ExitCode::SUCCESS
        }
        cli::Command::Reset => {
            commands::reset::run()?;
            ExitCode::SUCCESS
        }
        cli::Command::Current => {
            commands::current::run()?;
            ExitCode::SUCCESS
        }
        cli::Command::Init { shell } => {
            commands::init::run(shell)?;
            ExitCode::SUCCESS
        }
        cli::Command::Themes { action } => {
            commands::themes::run(&config, action)?;
            ExitCode::SUCCESS
        }
        cli::Command::Doctor { path } => commands::doctor::run(&config, path)?,
        cli::Command::Show { all } => {
            commands::show::run(&config, all)?;
            ExitCode::SUCCESS
        }
    })
}
