//! Terminal-specific output and detection.
//!
//! `osc` emits the xterm-flavored OSC escape sequences that repaint the
//! terminal, wrapping them in tmux DCS passthrough when needed. `utils` holds
//! shared terminal detection helpers (which terminal we're inside, whether
//! it's one we know how to theme).

pub mod osc;
pub mod utils;
