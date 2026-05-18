//! OSC (Operating System Command) escape sequence emitter.
//!
//! These sequences are the standard xterm-flavored mechanism for setting and
//! resetting terminal colors at runtime. Every modern terminal supports them
//! to some degree — colorant currently drives Ghostty and iTerm2; Kitty,
//! WezTerm, and Alacritty are protocol-compatible and gated only on detection
//! (see [`super::utils::detect`]).
//!
//! Sequences used:
//! - `OSC 10 ; #rrggbb BEL` — set default foreground
//! - `OSC 11 ; #rrggbb BEL` — set default background
//! - `OSC 12 ; #rrggbb BEL` — set cursor color
//! - `OSC 4 ; N ; #rrggbb BEL` — set palette entry N (0..15)
//! - `OSC 110 BEL` / `OSC 111 BEL` / `OSC 112 BEL` — reset fg/bg/cursor
//! - `OSC 104 ; N BEL` — reset palette entry N
//!
//! Terminal-specific (iTerm2 only, via OSC 1337):
//! - `OSC 1337 ; SetColors=tab=rrggbb BEL` — set tab background color
//!   (6 hex chars, no `#`). iTerm2 derives tab text color from contrast,
//!   so there is no companion `tab_fg`.
//! - `OSC 1337 ; SetColors=tab=default BEL` — reset tab color to profile
//!   default.
//!
//! When running inside tmux (detected via `$TMUX`), each sequence is wrapped
//! in tmux's DCS passthrough so the outer terminal receives it verbatim —
//! tmux otherwise intercepts or filters most OSC sequences. For OSC 1337,
//! tmux additionally requires `set -g allow-passthrough on` in `tmux.conf`,
//! or the wrapped sequence is dropped before reaching iTerm2.

use super::utils::Terminal;
use crate::theme::model::ThemeLayer;
use std::io::Write;

/// Emit OSC sequences to set every populated field in `theme`. Unset fields
/// are left untouched. `terminal` gates terminal-specific emissions (today,
/// only iTerm2's tab-color escape).
pub fn emit<W: Write>(out: &mut W, terminal: Terminal, theme: &ThemeLayer) -> std::io::Result<()> {
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
    if let Some(c) = &theme.tab_bg
        && let Some(payload) = tab_payload(terminal, c.as_str())
    {
        write_osc(out, in_tmux, &payload)?;
    }
    out.flush()
}

/// Emit OSC sequences resetting fg, bg, cursor, and all 16 palette entries.
/// Also resets the tab color on terminals that support one.
pub fn emit_reset<W: Write>(out: &mut W, terminal: Terminal) -> std::io::Result<()> {
    let in_tmux = std::env::var_os("TMUX").is_some();
    write_osc(out, in_tmux, "110")?;
    write_osc(out, in_tmux, "111")?;
    write_osc(out, in_tmux, "112")?;
    for i in 0..16 {
        write_osc(out, in_tmux, &format!("104;{}", i))?;
    }
    if let Some(payload) = tab_reset_payload(terminal) {
        write_osc(out, in_tmux, payload)?;
    }
    out.flush()
}

/// Emit only the tab-reset OSC for terminals that expose a tab-color knob.
/// Used by `apply` when `tab_follows_window` is enabled but the resolved
/// theme has no `bg` to derive from — without this, the tab color would
/// keep whatever the previous directory's apply left it as.
pub fn emit_tab_reset<W: Write>(out: &mut W, terminal: Terminal) -> std::io::Result<()> {
    if let Some(payload) = tab_reset_payload(terminal) {
        let in_tmux = std::env::var_os("TMUX").is_some();
        write_osc(out, in_tmux, payload)?;
        out.flush()?;
    }
    Ok(())
}

/// Build the terminal-specific tab-set payload, or `None` if the terminal
/// has no runtime tab-color knob. `hex` is `#rrggbb`; iTerm2's wire format
/// drops the leading `#`.
fn tab_payload(terminal: Terminal, hex: &str) -> Option<String> {
    match terminal {
        Terminal::ITerm2 => Some(format!(
            "1337;SetColors=tab={}",
            hex.trim_start_matches('#')
        )),
        Terminal::Ghostty => None,
    }
}

/// Build the terminal-specific tab-reset payload, or `None` if the terminal
/// has no runtime tab-color knob. Returned as a `&'static str` so the reset
/// path never allocates.
fn tab_reset_payload(terminal: Terminal) -> Option<&'static str> {
    match terminal {
        Terminal::ITerm2 => Some("1337;SetColors=tab=default"),
        Terminal::Ghostty => None,
    }
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

    #[test]
    fn tab_payload_strips_hash_for_iterm2() {
        assert_eq!(
            tab_payload(Terminal::ITerm2, "#abcdef").as_deref(),
            Some("1337;SetColors=tab=abcdef")
        );
    }

    #[test]
    fn tab_payload_is_none_for_ghostty() {
        assert!(tab_payload(Terminal::Ghostty, "#abcdef").is_none());
    }

    #[test]
    fn tab_reset_payload_is_default_for_iterm2() {
        assert_eq!(
            tab_reset_payload(Terminal::ITerm2),
            Some("1337;SetColors=tab=default")
        );
    }

    #[test]
    fn tab_reset_payload_is_none_for_ghostty() {
        assert!(tab_reset_payload(Terminal::Ghostty).is_none());
    }
}
