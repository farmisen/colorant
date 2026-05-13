//! `colorant init <shell>` — emit a shell integration snippet to stdout.
//!
//! Usage in shell rc files (zsh):
//!
//! ```sh
//! eval "$(colorant init zsh)"
//! ```

use crate::cli::Shell;
use crate::shell;
use anyhow::Result;

pub fn run(shell_kind: Shell) -> Result<()> {
    let binary = current_exe_path();
    let snippet = match shell_kind {
        Shell::Zsh => shell::zsh::hook(&binary),
    };
    print!("{}", snippet);
    Ok(())
}

/// Best effort: ask the OS for the absolute path to our own binary. Falls back
/// to the plain name `colorant` so the snippet still works if the binary is
/// on PATH.
fn current_exe_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "colorant".to_string())
}
