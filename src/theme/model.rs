//! Core theme data types.
//!
//! Two distinct file shapes:
//!
//! - **Palette** (`.colorant`): a flat set of color keys (fg, bg, cursor, 16
//!   palette entries). No modes, no inheritance — just colors. Parsed into
//!   `ParsedPalette`.
//! - **Config** (`.colorantrc`): the per-directory file users actually edit.
//!   Carries mode-aware inheritance via `extends` / `extends.dark` /
//!   `extends.light`, plus its own top-level keys and optional `[dark]` /
//!   `[light]` sections. Parsed into `ParsedRc`.
//!
//! `Mode` is the system dark/light mode (or `Unknown` when we can't tell).

use serde::Deserialize;
use thiserror::Error;

/// A `#rrggbb` color, validated at construction. Stored as a 7-character
/// lowercase string for direct OSC emission. Input case is normalized so
/// `#AABBCC` and `#aabbcc` compare equal under `Eq` and `Hash`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HexColor(String);

impl HexColor {
    /// Parse a `#rrggbb` value. Returns None for anything that isn't exactly
    /// a `#` followed by six hex digits. Both upper and lower case are
    /// accepted on input; the stored value is always lowercase.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit()) {
            Some(HexColor(s.to_ascii_lowercase()))
        } else {
            None
        }
    }

    /// Borrow the underlying `#rrggbb` string for direct OSC emission.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reasons a string fails to be a valid theme name.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ThemeNameError {
    #[error("theme name must not be empty")]
    Empty,
    #[error(
        "theme name {0:?} contains invalid character {1:?}; allowed: alphanumerics (Unicode), '.', '-', '_', ' ', '(', ')', '+'"
    )]
    InvalidChar(String, char),
}

/// A validated theme name.
///
/// A theme name is what `extends`, `extends.dark`, `extends.light` in
/// `.colorantrc` files and `default_theme` in `config.toml` refer to. It is
/// joined to `base_theme_dir` to locate `<name>.colorant` on disk.
///
/// Allowed characters: Unicode alphanumerics (so accented Latin like
/// `é` and CJK both work), plus `.`, `-`, `_`, ` ` (interior only),
/// `(`, `)`, and `+`. This is wide enough to cover the Gogh catalog
/// (`3024 Day`, `Catppuccin Frappé`, `Flatland (Palenight)`,
/// `Vs Code Dark+`) while still ruling out path traversal (`/`, `\`),
/// shell-quoting hazards (`"`, `'`), and control characters. Leading
/// or trailing whitespace is rejected to avoid surprising filesystem
/// behavior and asymmetric rc-line trim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThemeName(String);

impl ThemeName {
    /// Parse a theme name. See the [`ThemeName`] doc comment for the
    /// accepted character set.
    pub fn parse(s: &str) -> Result<Self, ThemeNameError> {
        if s.is_empty() {
            return Err(ThemeNameError::Empty);
        }
        // Leading/trailing whitespace would be a silent footgun:
        // filesystems with trailing-space filenames are weird, and the
        // rc parser trims surrounding whitespace before validation
        // anyway (so accepting it here would produce a name the parser
        // would never round-trip).
        let first = s.chars().next().expect("non-empty");
        if first.is_whitespace() {
            return Err(ThemeNameError::InvalidChar(s.to_string(), first));
        }
        let last = s.chars().next_back().expect("non-empty");
        if last.is_whitespace() {
            return Err(ThemeNameError::InvalidChar(s.to_string(), last));
        }
        if let Some(c) = s.chars().find(|c| !is_allowed(*c)) {
            return Err(ThemeNameError::InvalidChar(s.to_string(), c));
        }
        Ok(Self(s.to_string()))
    }

    /// Borrow the underlying string for path joins and display.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_allowed(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ' | '(' | ')' | '+')
}

impl std::fmt::Display for ThemeName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ThemeName {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        ThemeName::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// A flat collection of theme keys. Any field set to `Some` overrides the
/// corresponding field in a layer it is merged into.
///
/// Fields are `pub(crate)` because the parser writes to them directly during
/// rc traversal. There is no external boundary to enforce against today; the
/// crate-local visibility just signals that consumers outside the theme stack
/// (e.g. `commands`, `terminal::osc`) should treat this as read-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeLayer {
    /// Default foreground color (OSC 10).
    pub(crate) fg: Option<HexColor>,
    /// Default background color (OSC 11).
    pub(crate) bg: Option<HexColor>,
    /// Cursor color (OSC 12).
    pub(crate) cursor: Option<HexColor>,
    /// Palette entries 0..15 (OSC 4).
    pub(crate) palette: [Option<HexColor>; 16],
}

impl ThemeLayer {
    /// Merge `other` into `self`. Any `Some` value in `other` replaces the
    /// corresponding field in `self`. `None` values leave `self` untouched.
    pub fn merge(&mut self, other: &ThemeLayer) {
        if let Some(v) = &other.fg {
            self.fg = Some(v.clone());
        }
        if let Some(v) = &other.bg {
            self.bg = Some(v.clone());
        }
        if let Some(v) = &other.cursor {
            self.cursor = Some(v.clone());
        }
        for i in 0..16 {
            if let Some(v) = &other.palette[i] {
                self.palette[i] = Some(v.clone());
            }
        }
    }

    /// True if no fields are set. Used to decide whether to emit anything.
    pub fn is_empty(&self) -> bool {
        self.fg.is_none()
            && self.bg.is_none()
            && self.cursor.is_none()
            && self.palette.iter().all(|c| c.is_none())
    }
}

/// The parsed contents of a `.colorant` palette file. Just colors, no
/// inheritance or mode logic.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedPalette {
    /// The flat color set declared in the palette file.
    pub(crate) layer: ThemeLayer,
}

/// The parsed contents of a `.colorantrc` config file. Carries optional
/// parent palette names (one global, optionally one per mode), plus the
/// file's own keys split into a `base` layer (always applied) and per-mode
/// overlays (`dark`, `light`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedRc {
    /// Palette name applied in both modes when no per-mode override is set.
    pub(crate) extends: Option<ThemeName>,
    /// Palette name applied only in dark mode. When set, overrides `extends`
    /// in dark mode.
    pub(crate) extends_dark: Option<ThemeName>,
    /// Palette name applied only in light mode. When set, overrides `extends`
    /// in light mode.
    pub(crate) extends_light: Option<ThemeName>,
    /// Color keys at the top level of the rc, applied in every mode.
    pub(crate) base: ThemeLayer,
    /// Color keys under `[dark]`, applied only when the system is in dark mode.
    pub(crate) dark: ThemeLayer,
    /// Color keys under `[light]`, applied only when the system is in light mode.
    pub(crate) light: ThemeLayer,
}

/// System color-scheme mode.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
    /// We weren't able to query the OS. The apply step will skip both
    /// per-mode `extends` and the `[dark]` / `[light]` sections, falling back
    /// to the global `extends` (if any) and the file's base keys.
    Unknown,
}

impl ParsedRc {
    /// Pick the effective parent palette name for `mode`:
    /// `extends.<mode>` if set, else top-level `extends`, else none. For
    /// `Mode::Unknown` only the global `extends` is consulted.
    pub fn parent_for(&self, mode: Mode) -> Option<&ThemeName> {
        let specific = match mode {
            Mode::Dark => self.extends_dark.as_ref(),
            Mode::Light => self.extends_light.as_ref(),
            Mode::Unknown => None,
        };
        specific.or(self.extends.as_ref())
    }
}

#[cfg(test)]
mod hex_color_tests {
    use super::HexColor;

    #[test]
    fn normalizes_to_lowercase() {
        let a = HexColor::parse("#AABBCC").unwrap();
        let b = HexColor::parse("#aabbcc").unwrap();
        assert_eq!(a.as_str(), "#aabbcc");
        assert_eq!(a, b);
    }

    #[test]
    fn accepts_mixed_case() {
        let c = HexColor::parse("#AbCdEf").unwrap();
        assert_eq!(c.as_str(), "#abcdef");
    }
}

#[cfg(test)]
mod theme_name_tests {
    use super::{ThemeName, ThemeNameError};

    #[test]
    fn accepts_alnum_dot_dash_underscore() {
        assert_eq!(
            ThemeName::parse("catppuccin-mocha").unwrap().as_str(),
            "catppuccin-mocha"
        );
        assert!(ThemeName::parse("one_dark").is_ok());
        assert!(ThemeName::parse("solarized.v2").is_ok());
        assert!(ThemeName::parse("ayu123").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(ThemeName::parse(""), Err(ThemeNameError::Empty));
    }

    #[test]
    fn rejects_path_traversal_chars() {
        assert!(matches!(
            ThemeName::parse("../etc"),
            Err(ThemeNameError::InvalidChar(_, '/'))
        ));
        assert!(matches!(
            ThemeName::parse("foo/bar"),
            Err(ThemeNameError::InvalidChar(_, '/'))
        ));
        assert!(matches!(
            ThemeName::parse("~/themes"),
            Err(ThemeNameError::InvalidChar(_, '~'))
        ));
    }

    #[test]
    fn accepts_gogh_style_names() {
        // Spaces inside the name and parens are accepted so we don't
        // drop the bulk of the Gogh catalog.
        assert_eq!(ThemeName::parse("3024 Day").unwrap().as_str(), "3024 Day");
        assert_eq!(
            ThemeName::parse("Flatland (Palenight)").unwrap().as_str(),
            "Flatland (Palenight)"
        );
        assert!(ThemeName::parse("Vs Code Dark+").is_ok());
        // Unicode alphabetics (accented Latin, CJK) are alphanumeric
        // under Unicode rules — useful for `Catppuccin Frappé` etc.
        assert!(ThemeName::parse("Catppuccin Frappé").is_ok());
        assert!(ThemeName::parse("테마").is_ok());
    }

    #[test]
    fn rejects_leading_or_trailing_whitespace() {
        // Interior spaces are allowed (see accepts_gogh_style_names),
        // but surrounding whitespace would create silent footguns.
        assert!(matches!(
            ThemeName::parse(" leading"),
            Err(ThemeNameError::InvalidChar(_, ' '))
        ));
        assert!(matches!(
            ThemeName::parse("trailing "),
            Err(ThemeNameError::InvalidChar(_, ' '))
        ));
        assert!(matches!(
            ThemeName::parse("\tlead-tab"),
            Err(ThemeNameError::InvalidChar(_, '\t'))
        ));
    }

    #[test]
    fn rejects_interior_tab_and_quotes() {
        // Spaces are interior-OK; tabs and quotes are not.
        assert!(matches!(
            ThemeName::parse("name\tx"),
            Err(ThemeNameError::InvalidChar(_, '\t'))
        ));
        assert!(matches!(
            ThemeName::parse("\"quoted\""),
            Err(ThemeNameError::InvalidChar(_, '"'))
        ));
    }
}
