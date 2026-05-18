//! `colorant apply` — the main orchestration command.
//!
//! 1. Bail silently if we're not in a terminal we know how to theme. The
//!    shell hook fires this on every `chpwd`/`precmd`, so a silent no-op is
//!    the right behavior elsewhere.
//! 2. Walk up from `cwd` looking for `.colorantrc`. If found, resolve it for
//!    the current mode, optionally auto-derive `tab_bg` from `bg`, and emit
//!    OSC sequences for the resulting palette.
//! 3. If not found, apply the configured global default palette (also
//!    subject to `tab_follows_window`); otherwise emit the OSC reset
//!    sequence.

use crate::config::{Config, THEME_FILE_NAME};
use crate::mode;
use crate::terminal::utils::Terminal;
use crate::terminal::{osc, utils};
use crate::theme::model::ThemeLayer;
use crate::theme::parse;
use crate::theme::resolve::{PALETTE_EXTENSION, Resolver};
use crate::walk;
use anyhow::Result;
use std::io::{Write, stdout};

pub fn run(config: &Config) -> Result<()> {
    let Some(terminal) = utils::detect() else {
        return Ok(());
    };

    let cwd = std::env::current_dir()?;
    let resolver = Resolver::new(config.base_theme_dir.clone());
    let current_mode = mode::detect();

    if let Some(rc_path) = walk::find_nearest(&cwd, THEME_FILE_NAME) {
        let theme = resolver.resolve(&rc_path, current_mode)?;
        let mut out = stdout().lock();
        if theme.is_empty() {
            osc::emit_reset(&mut out, terminal)?;
        } else {
            emit_themed(&mut out, terminal, theme, config)?;
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
                emit_themed(&mut out, terminal, palette.layer, config)?;
                return Ok(());
            }
        }
    }

    let mut out = stdout().lock();
    osc::emit_reset(&mut out, terminal)?;
    Ok(())
}

/// Apply the `tab_follows_window` policy to `layer` and emit it. When the
/// policy is on but `tab_bg` can't be derived (no explicit value AND no `bg`
/// to fall back to), emit a separate tab-reset so the terminal returns to
/// its profile default rather than holding the previous directory's tab
/// color.
fn emit_themed<W: Write>(
    out: &mut W,
    terminal: Terminal,
    mut layer: ThemeLayer,
    config: &Config,
) -> Result<()> {
    if config.tab_follows_window {
        layer.fill_tab_from_bg();
    }
    osc::emit(out, terminal, &layer)?;
    if config.tab_follows_window && layer.tab_bg.is_none() {
        osc::emit_tab_reset(out, terminal)?;
    }
    Ok(())
}
