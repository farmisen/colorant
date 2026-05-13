//! Command-line argument types.

use clap::{Parser, Subcommand, ValueEnum};

/// Top-level CLI argument structure parsed by clap.
#[derive(Debug, Parser)]
#[command(
    name = "colorant",
    version,
    about = "Per-directory terminal theme switcher with system dark/light mode support"
)]
pub struct Cli {
    /// The subcommand the user invoked.
    #[command(subcommand)]
    pub command: Command,
}

/// One of colorant's subcommands. Each variant maps to a `run` function in
/// `crate::commands`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Find the nearest .colorantrc and apply its theme. No-op on unsupported
    /// terminals. Falls back to the global default theme (if configured) or
    /// resets the terminal when no .colorantrc is found.
    Apply,

    /// Reset terminal colors to defaults.
    Reset,

    /// Print the .colorantrc path that would be applied for the current dir.
    Current,

    /// Print a shell-specific integration snippet.
    ///
    /// Typical use: `eval "$(colorant init zsh)"` in ~/.zshrc.
    Init {
        #[arg(value_enum)]
        shell: Shell,
    },
}

/// The shell selector accepted by `colorant init <shell>`.
#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum Shell {
    /// Zsh — emits an `add-zsh-hook chpwd/precmd` snippet.
    Zsh,
}
