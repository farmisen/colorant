//! Terminal-specific output and detection.
//!
//! `osc` emits the xterm-flavored OSC escape sequences that repaint the
//! terminal, wrapping them in tmux DCS passthrough when needed. `utils`
//! identifies which terminal we're inside (`Terminal::Ghostty`,
//! `Terminal::ITerm2`, …) so callers can gate OSC emission on it.

pub mod osc;
pub mod style;
pub mod utils;
