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

    /// Manage bundled palettes (list, install, locate the themes dir).
    Themes {
        #[command(subcommand)]
        action: ThemesAction,
    },

    /// Diagnose silent failures in a .colorantrc: unknown keys, invalid
    /// colors, missing extends palettes, and so on. Without `path`, walks
    /// up from the current directory like `colorant current`.
    Doctor {
        /// Path to a specific `.colorantrc` to check.
        path: Option<std::path::PathBuf>,
    },

    /// Print the resolved colors that would apply for the current
    /// directory, with hex codes and 24-bit swatches. Defaults to the
    /// current OS dark/light mode; pass `--all` to print both modes.
    Show {
        /// Print both dark and light resolutions instead of just the
        /// current mode.
        #[arg(long)]
        all: bool,
    },
}

/// Sub-actions for the `themes` command group.
#[derive(Debug, Subcommand)]
pub enum ThemesAction {
    /// List bundled palettes, marking which are already installed.
    List,

    /// Copy bundled palettes into the user's themes directory.
    ///
    /// Use a specific name to install just that palette, or `--all` to
    /// install every bundled palette. By default refuses to overwrite
    /// existing files — pass `--force` to overwrite.
    Install {
        /// Name of a single bundled palette to install.
        ///
        /// Mutually exclusive with `--all`. Without either, the command
        /// errors with a hint pointing at both options — the validation
        /// lives in `commands::themes::run_install` so the message can be
        /// helpful, since clap's `required_unless_present` only reports a
        /// generic "argument required" error.
        name: Option<String>,

        /// Install every bundled palette.
        #[arg(long, conflicts_with = "name")]
        all: bool,

        /// Overwrite existing palette files.
        #[arg(long, short)]
        force: bool,
    },

    /// Print the configured themes directory.
    Path,
}

/// The shell selector accepted by `colorant init <shell>`.
#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum Shell {
    /// Zsh — emits an `add-zsh-hook chpwd/precmd` snippet.
    Zsh,
}
