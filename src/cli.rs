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

    /// Interactively pick themes and apply them to the current
    /// directory's `.colorantrc`. Browses installed and bundled palettes
    /// with a live preview; on apply, writes `extends` / `extends.dark` /
    /// `extends.light` based on the slots you assigned, auto-installs
    /// bundled themes that weren't yet on disk, and preserves any other
    /// keys in the existing rc.
    Set,
}

/// Sub-actions for the `themes` command group.
#[derive(Debug, Subcommand)]
pub enum ThemesAction {
    /// List themes from one or all sources, marking which are already
    /// installed locally.
    List {
        /// Restrict to a single source (e.g. `bundled`, `gogh`). Omit to
        /// list every source.
        #[arg(long)]
        source: Option<String>,
        /// Only show themes that are installed in the local themes dir.
        #[arg(long)]
        installed: bool,
    },

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

    /// Search for themes by name across known sources (bundled + remote).
    /// Remote sources must be `sync`'d first for their themes to show up.
    Search {
        /// Substring to match (case-insensitive).
        query: String,
        /// Restrict to a single source (e.g. `bundled`, `gogh`).
        #[arg(long)]
        source: Option<String>,
    },

    /// Refresh the cached catalog for remote sources (`gogh`). Network
    /// only happens during this command.
    Sync {
        /// Restrict to a single source. Omit to sync every remote.
        #[arg(long)]
        source: Option<String>,
    },

    /// Apply themes to the current directory's `.colorantrc`, writing
    /// `extends` / `extends.dark` / `extends.light` as needed. Themes
    /// that aren't yet installed (bundled or fetched from a remote) are
    /// installed automatically. Other keys in the rc are preserved.
    Apply {
        /// Theme to apply to both modes (`extends = <name>`). Optional;
        /// pass `--dark`/`--light` instead to set them separately.
        /// Source can be specified via `gogh:<name>` syntax.
        #[arg(conflicts_with_all = ["dark", "light"])]
        name: Option<String>,
        /// Theme for dark mode (`extends.dark = <name>`).
        #[arg(long)]
        dark: Option<String>,
        /// Theme for light mode (`extends.light = <name>`).
        #[arg(long)]
        light: Option<String>,
    },
}

/// The shell selector accepted by `colorant init <shell>`.
#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum Shell {
    /// Zsh — emits an `add-zsh-hook chpwd/precmd` snippet.
    Zsh,
}
