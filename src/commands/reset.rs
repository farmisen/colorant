//! `colorant reset` — emit OSC sequences resetting fg, bg, cursor, and the
//! 16-color palette to terminal defaults.

use crate::terminal::{osc, utils};
use anyhow::Result;
use std::io::stdout;

pub fn run() -> Result<()> {
    if utils::detect().is_none() {
        return Ok(());
    }

    let mut out = stdout().lock();
    osc::emit_reset(&mut out)?;
    Ok(())
}
