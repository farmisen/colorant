//! ANSI styling helpers for command output.
//!
//! Used by `colorant doctor` (red/green error/clean markers) and
//! `colorant show` (24-bit background swatches for each resolved color).
//!
//! Color emission is gated once at construction by `Style::detect`, which
//! checks `std::io::stdout().is_terminal()` and the `NO_COLOR` env var.
//! When colors are disabled, every `Painted` falls back to plain text — so
//! piping a command's output anywhere (or setting `NO_COLOR=1`) produces a
//! clean log.

use std::io::IsTerminal;

/// Minimal ANSI-coloring helper, captured once and threaded by value so
/// the decision (color or not) doesn't drift mid-report.
#[derive(Clone, Copy)]
pub struct Style {
    color: bool,
}

impl Style {
    /// Enable colors only when stdout is a TTY and `NO_COLOR` is unset.
    /// This matches `colorant doctor | tee log` producing plain text — and
    /// keeps integration tests (which run colorant via `Command` with
    /// captured stdout) free of escape codes.
    pub fn detect() -> Self {
        Self::for_env(
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").is_some(),
        )
    }

    /// Pure decision split out from `detect` for unit-testability.
    pub fn for_env(stdout_is_tty: bool, no_color_set: bool) -> Self {
        Self {
            color: stdout_is_tty && !no_color_set,
        }
    }

    /// Wrap `text` in the SGR red foreground escape.
    pub fn red<'a>(self, text: &'a str) -> Painted<'a> {
        Painted::new(text, RED, self.color)
    }

    /// Wrap `text` in the SGR green foreground escape.
    pub fn green<'a>(self, text: &'a str) -> Painted<'a> {
        Painted::new(text, GREEN, self.color)
    }

    /// Wrap `text` in a 24-bit RGB foreground escape. The text itself is
    /// usually a block character (`█`) so the glyph fills its cell with the
    /// requested color — that's what `colorant show` uses to draw swatches.
    pub fn fg_rgb<'a>(self, text: &'a str, r: u8, g: u8, b: u8) -> RgbPainted<'a> {
        RgbPainted {
            text,
            rgb: if self.color { Some((r, g, b)) } else { None },
        }
    }
}

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const RESET: &str = "\x1b[0m";

/// `Display` wrapper that emits an SGR escape pair around `text` when the
/// originating `Style` had colors enabled.
pub struct Painted<'a> {
    text: &'a str,
    prefix: &'static str,
    suffix: &'static str,
}

impl<'a> Painted<'a> {
    fn new(text: &'a str, prefix: &'static str, enabled: bool) -> Self {
        Self {
            text,
            prefix: if enabled { prefix } else { "" },
            suffix: if enabled { RESET } else { "" },
        }
    }
}

impl std::fmt::Display for Painted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}{}", self.prefix, self.text, self.suffix)
    }
}

/// `Display` wrapper for a 24-bit foreground-colored span. Skips the
/// escape entirely when colors are disabled, leaving just the inner text
/// (typically a block character that renders as plain text in pipes).
pub struct RgbPainted<'a> {
    text: &'a str,
    rgb: Option<(u8, u8, u8)>,
}

impl std::fmt::Display for RgbPainted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.rgb {
            Some((r, g, b)) => write!(f, "\x1b[38;2;{r};{g};{b}m{}\x1b[0m", self.text),
            None => write!(f, "{}", self.text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Style;

    #[test]
    fn color_enabled_only_on_tty_without_no_color() {
        assert!(Style::for_env(true, false).color);
        assert!(!Style::for_env(false, false).color);
        assert!(!Style::for_env(true, true).color);
        assert!(!Style::for_env(false, true).color);
    }

    #[test]
    fn painted_emits_sgr_only_when_enabled() {
        let on = Style::for_env(true, false);
        let off = Style::for_env(false, false);
        assert_eq!(format!("{}", on.red("err")), "\x1b[31merr\x1b[0m");
        assert_eq!(format!("{}", on.green("ok")), "\x1b[32mok\x1b[0m");
        assert_eq!(format!("{}", off.red("err")), "err");
        assert_eq!(format!("{}", off.green("ok")), "ok");
    }

    #[test]
    fn rgb_painted_emits_24bit_only_when_enabled() {
        let on = Style::for_env(true, false);
        let off = Style::for_env(false, false);
        assert_eq!(
            format!("{}", on.fg_rgb("█", 0xab, 0xcd, 0xef)),
            "\x1b[38;2;171;205;239m█\x1b[0m"
        );
        assert_eq!(format!("{}", off.fg_rgb("█", 0xab, 0xcd, 0xef)), "█");
    }
}
