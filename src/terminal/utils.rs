//! Terminal detection.
//!
//! The protocol layer in [`super::osc`] is plain xterm OSC and works in any
//! conforming terminal, but we gate emission on detection so we don't repaint
//! sessions we haven't verified end-to-end — some terminals quietly ignore
//! subsets of OSC 4 / OSC 10-12, or render the escape bytes as literal
//! output.

/// A terminal colorant knows how to drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminal {
    Ghostty,
    ITerm2,
}

/// Detect the surrounding terminal from environment variables. Returns
/// `None` if we're not inside one we recognize.
///
/// Signals checked, in priority order:
/// 1. `TERM_PROGRAM` — set by the terminal itself in local sessions.
/// 2. `LC_TERMINAL` — forwarded over SSH by iTerm2 when the user enables
///    "Send LC_TERMINAL" in its preferences.
/// 3. `TERM` — fallback for Ghostty, which exports `xterm-ghostty` and
///    survives some multiplexer setups that strip `TERM_PROGRAM`.
pub fn detect() -> Option<Terminal> {
    match std::env::var("TERM_PROGRAM").ok().as_deref() {
        Some("ghostty") => return Some(Terminal::Ghostty),
        Some("iTerm.app") => return Some(Terminal::ITerm2),
        _ => {}
    }
    if matches!(std::env::var("LC_TERMINAL").ok().as_deref(), Some("iTerm2")) {
        return Some(Terminal::ITerm2);
    }
    if std::env::var("TERM")
        .ok()
        .is_some_and(|t| t.split('-').any(|s| s == "ghostty"))
    {
        return Some(Terminal::Ghostty);
    }
    None
}
