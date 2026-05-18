//! `colorant reset` — emit OSC sequences resetting fg, bg, cursor, and the
//! 16-color palette to terminal defaults. Also resets the tab color on
//! terminals that support one (iTerm2 today).

use crate::terminal::{osc, utils};
use anyhow::Result;
use std::io::stdout;

pub fn run() -> Result<()> {
    let Some(terminal) = utils::detect() else {
        return Ok(());
    };

    let mut out = stdout().lock();
    osc::emit_reset(&mut out, terminal)?;
    Ok(())
}
