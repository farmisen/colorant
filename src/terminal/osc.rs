//! OSC (Operating System Command) escape sequence emitter.
//!
//! These sequences are the standard xterm-flavored mechanism for setting and
//! resetting terminal colors at runtime. Every modern terminal we care about
//! supports them: Ghostty, Kitty, iTerm2, WezTerm, and recent Alacritty.
//!
//! Sequences used:
//! - `OSC 10 ; #rrggbb BEL` — set default foreground
//! - `OSC 11 ; #rrggbb BEL` — set default background
//! - `OSC 12 ; #rrggbb BEL` — set cursor color
//! - `OSC 4 ; N ; #rrggbb BEL` — set palette entry N (0..15)
//! - `OSC 110 BEL` / `OSC 111 BEL` / `OSC 112 BEL` — reset fg/bg/cursor
//! - `OSC 104 ; N BEL` — reset palette entry N
//!
//! When running inside tmux (detected via `$TMUX`), each sequence is wrapped
//! in tmux's DCS passthrough so it actually reaches the outer terminal.

use crate::theme::model::ThemeLayer;
use std::io::Write;

/// Emit OSC sequences to set every populated field in `theme`. Unset fields
/// are left untouched.
pub fn emit<W: Write>(out: &mut W, theme: &ThemeLayer) -> std::io::Result<()> {
    let in_tmux = std::env::var_os("TMUX").is_some();

    if let Some(c) = &theme.fg {
        write_osc(out, in_tmux, &format!("10;{}", c.as_str()))?;
    }
    if let Some(c) = &theme.bg {
        write_osc(out, in_tmux, &format!("11;{}", c.as_str()))?;
    }
    if let Some(c) = &theme.cursor {
        write_osc(out, in_tmux, &format!("12;{}", c.as_str()))?;
    }
    for (i, slot) in theme.palette.iter().enumerate() {
        if let Some(c) = slot {
            write_osc(out, in_tmux, &format!("4;{};{}", i, c.as_str()))?;
        }
    }
    out.flush()
}

/// Emit OSC sequences resetting fg, bg, cursor, and all 16 palette entries.
pub fn emit_reset<W: Write>(out: &mut W) -> std::io::Result<()> {
    let in_tmux = std::env::var_os("TMUX").is_some();
    write_osc(out, in_tmux, "110")?;
    write_osc(out, in_tmux, "111")?;
    write_osc(out, in_tmux, "112")?;
    for i in 0..16 {
        write_osc(out, in_tmux, &format!("104;{}", i))?;
    }
    out.flush()
}

/// Write a single OSC payload, wrapping in tmux DCS passthrough if needed.
fn write_osc<W: Write>(out: &mut W, in_tmux: bool, payload: &str) -> std::io::Result<()> {
    if in_tmux {
        // tmux DCS passthrough: ESC P tmux ; ESC <doubled-ESC-payload> ESC \
        // Each ESC inside the inner payload must be doubled. Our inner payload
        // is `ESC ] <payload> BEL`, so we write the leading ESC as ESC ESC.
        write!(out, "\x1bPtmux;\x1b\x1b]{}\x07\x1b\\", payload)
    } else {
        write!(out, "\x1b]{}\x07", payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::model::{HexColor, ThemeLayer};

    fn capture(theme: &ThemeLayer) -> String {
        let mut buf = Vec::new();
        // Test the no-tmux path directly to avoid env interactions.
        if let Some(c) = &theme.fg {
            write_osc(&mut buf, false, &format!("10;{}", c.as_str())).unwrap();
        }
        if let Some(c) = &theme.bg {
            write_osc(&mut buf, false, &format!("11;{}", c.as_str())).unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn emits_fg_and_bg() {
        let t = ThemeLayer {
            fg: Some(HexColor::parse("#cdd6f4").unwrap()),
            bg: Some(HexColor::parse("#1e1e2e").unwrap()),
            ..Default::default()
        };
        let s = capture(&t);
        assert!(s.contains("\x1b]10;#cdd6f4\x07"));
        assert!(s.contains("\x1b]11;#1e1e2e\x07"));
    }

    #[test]
    fn tmux_wraps_with_dcs() {
        let mut buf = Vec::new();
        write_osc(&mut buf, true, "10;#abcdef").unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("\x1bPtmux;\x1b\x1b]"));
        assert!(s.ends_with("\x07\x1b\\"));
    }
}
