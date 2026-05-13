//! `colorant apply` — the main orchestration command.
//!
//! 1. Bail silently if we're not in a terminal we know how to theme. The
//!    shell hook fires this on every `chpwd`/`precmd`, so a silent no-op is
//!    the right behavior elsewhere.
//! 2. Walk up from `cwd` looking for `.colorantrc`. If found, resolve it for
//!    the current mode and emit OSC sequences for the resulting palette.
//! 3. If not found, apply the configured global default palette (if any),
//!    otherwise emit the OSC reset sequence.

use crate::config::{Config, THEME_FILE_NAME};
use crate::mode;
use crate::terminal::{osc, utils};
use crate::theme::parse;
use crate::theme::resolve::{PALETTE_EXTENSION, Resolver};
use crate::walk;
use anyhow::Result;
use std::io::stdout;

pub fn run(config: &Config) -> Result<()> {
    if !utils::supported_terminal() {
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let resolver = Resolver::new(config.base_theme_dir.clone());
    let current_mode = mode::detect();

    if let Some(rc_path) = walk::find_nearest(&cwd, THEME_FILE_NAME) {
        let theme = resolver.resolve(&rc_path, current_mode)?;
        let mut out = stdout().lock();
        if theme.is_empty() {
            osc::emit_reset(&mut out)?;
        } else {
            osc::emit(&mut out, &theme)?;
        }
        return Ok(());
    }

    // No `.colorantrc` in the parent chain — try the global default palette.
    if let Some(default_name) = &config.default_theme {
        let palette_path =
            config
                .base_theme_dir
                .join(format!("{}.{}", default_name.as_str(), PALETTE_EXTENSION));
        if palette_path.exists() {
            let palette = parse::parse_palette_file(&palette_path)?;
            if !palette.layer.is_empty() {
                let mut out = stdout().lock();
                osc::emit(&mut out, &palette.layer)?;
                return Ok(());
            }
        }
    }

    let mut out = stdout().lock();
    osc::emit_reset(&mut out)?;
    Ok(())
}
