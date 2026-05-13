//! Parsers for the two colorant file shapes.
//!
//! ## Palettes (`.colorant`)
//!
//! Flat key/value, no sections, no inheritance:
//!
//! ```ini
//! # Catppuccin Mocha
//! fg     = #cdd6f4
//! bg     = #1e1e2e
//! cursor = #f5e0dc
//! color0 = #45475a
//! # ...
//! color15 = #a6adc8
//! ```
//!
//! ## Config files (`.colorantrc`)
//!
//! Can declare parent palettes (globally or per mode), plus its own keys.
//! Top-level keys apply in both modes; `[dark]` / `[light]` sections override
//! per mode.
//!
//! ```ini
//! # Use Catppuccin Mocha in dark mode, Latte in light
//! extends.dark  = catppuccin-mocha
//! extends.light = catppuccin-latte
//!
//! # Project-wide override regardless of mode
//! fg = #ffffff
//!
//! [dark]
//! # In dark mode only, recolor the cursor
//! cursor = #ff00ff
//! ```
//!
//! Unknown keys and unknown sections are silently ignored to keep forward
//! compatibility easy. Invalid color values are dropped.

use super::model::{HexColor, ParsedPalette, ParsedRc, ThemeLayer, ThemeName};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

#[derive(Copy, Clone, PartialEq, Eq)]
enum Section {
    Base,
    Dark,
    Light,
    /// Any section name other than `[dark]` / `[light]`. Keys inside are dropped.
    Unknown,
}

/// Parse a `.colorant` palette from disk.
pub fn parse_palette_file(path: &Path) -> Result<ParsedPalette> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(parse_palette_str(&content))
}

/// Parse a `.colorant` palette from a string. Sections and `extends` lines
/// are silently ignored — palettes are flat by design.
pub fn parse_palette_str(content: &str) -> ParsedPalette {
    let mut layer = ThemeLayer::default();

    for raw in content.lines() {
        let Some((key, value)) = split_kv(raw) else {
            continue;
        };
        if key.starts_with('[') {
            // section header — ignore entirely for palette files
            continue;
        }
        apply_kv(&mut layer, &key, &value);
    }

    ParsedPalette { layer }
}

/// Parse a `.colorantrc` from disk.
pub fn parse_rc_file(path: &Path) -> Result<ParsedRc> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(parse_rc_str(&content))
}

/// Parse a `.colorantrc` from a string. Never fails — malformed lines are
/// silently skipped.
pub fn parse_rc_str(content: &str) -> ParsedRc {
    let mut rc = ParsedRc::default();
    let mut section = Section::Base;

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Section header: [name]
        if let Some(stripped) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let name = stripped.trim().to_ascii_lowercase();
            section = match name.as_str() {
                "dark" => Section::Dark,
                "light" => Section::Light,
                _ => Section::Unknown,
            };
            continue;
        }

        // key = value
        let Some(eq_idx) = line.find('=') else {
            continue;
        };
        let key = line[..eq_idx].trim();
        let value = line[eq_idx + 1..].trim();
        if key.is_empty() {
            continue;
        }

        match section {
            Section::Unknown => continue,
            Section::Base => apply_rc_base_kv(&mut rc, key, value),
            Section::Dark => apply_kv(&mut rc.dark, key, value),
            Section::Light => apply_kv(&mut rc.light, key, value),
        }
    }

    rc
}

/// Top-level keys in a `.colorantrc`: `extends`, `extends.dark`,
/// `extends.light`, plus the regular palette keys (which form the file's
/// `base` layer).
fn apply_rc_base_kv(rc: &mut ParsedRc, key: &str, value: &str) {
    // Duplicate keys: last valid value wins, matching the rule used by
    // `apply_kv` for color keys. An invalid value is dropped silently and
    // leaves any previously-set valid value in place.
    match key {
        "extends" => {
            if let Ok(name) = ThemeName::parse(value) {
                rc.extends = Some(name);
            }
        }
        "extends.dark" => {
            if let Ok(name) = ThemeName::parse(value) {
                rc.extends_dark = Some(name);
            }
        }
        "extends.light" => {
            if let Ok(name) = ThemeName::parse(value) {
                rc.extends_light = Some(name);
            }
        }
        _ => apply_kv(&mut rc.base, key, value),
    }
}

/// Apply a single palette key/value to a layer. Keys we don't recognize are
/// silently ignored (forward-compat). Values that aren't valid colors are
/// dropped.
fn apply_kv(layer: &mut ThemeLayer, key: &str, value: &str) {
    match key {
        "fg" => {
            if let Some(c) = HexColor::parse(value) {
                layer.fg = Some(c);
            }
        }
        "bg" => {
            if let Some(c) = HexColor::parse(value) {
                layer.bg = Some(c);
            }
        }
        "cursor" => {
            if let Some(c) = HexColor::parse(value) {
                layer.cursor = Some(c);
            }
        }
        k if k.starts_with("color") => {
            if let Ok(idx) = k[5..].parse::<usize>()
                && idx < 16
                && let Some(c) = HexColor::parse(value)
            {
                layer.palette[idx] = Some(c);
            }
        }
        _ => {}
    }
}

/// Used by the palette parser to skip section headers cheaply.
fn split_kv(raw: &str) -> Option<(String, String)> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if line.starts_with('[') {
        return Some((line.to_string(), String::new())); // signal "skip" to caller
    }
    let eq_idx = line.find('=')?;
    let key = line[..eq_idx].trim().to_string();
    let value = line[eq_idx + 1..].trim().to_string();
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_palette() {
        let p = parse_palette_str("fg = #cdd6f4\nbg = #1e1e2e\ncolor0 = #45475a\n");
        assert_eq!(p.layer.fg.unwrap().as_str(), "#cdd6f4");
        assert_eq!(p.layer.bg.unwrap().as_str(), "#1e1e2e");
        assert_eq!(p.layer.palette[0].as_ref().unwrap().as_str(), "#45475a");
    }

    #[test]
    fn palette_ignores_sections_and_extends() {
        // Anything section-like or extends-y is silently dropped — palettes
        // are flat by design.
        let p = parse_palette_str(
            "extends = nope\n[dark]\nfg = #111111\nextends.dark = also-nope\nbg = #222222\n",
        );
        // We never enter "[dark]" specially, but our palette parser also
        // ignores the section header line itself. The bg= line below it gets
        // applied to the flat layer.
        assert_eq!(p.layer.fg.unwrap().as_str(), "#111111");
        assert_eq!(p.layer.bg.unwrap().as_str(), "#222222");
    }

    #[test]
    fn parses_rc_with_per_mode_extends() {
        let rc = parse_rc_str(
            "extends.dark = tokyo-night\nextends.light = catppuccin-latte\nfg = #abcdef\n[dark]\nbg = #111111\n",
        );
        assert_eq!(rc.extends, None);
        assert_eq!(
            rc.extends_dark.as_ref().map(ThemeName::as_str),
            Some("tokyo-night")
        );
        assert_eq!(
            rc.extends_light.as_ref().map(ThemeName::as_str),
            Some("catppuccin-latte")
        );
        assert_eq!(rc.base.fg.unwrap().as_str(), "#abcdef");
        assert_eq!(rc.dark.bg.unwrap().as_str(), "#111111");
    }

    #[test]
    fn rc_global_extends_still_works() {
        let rc = parse_rc_str("extends = catppuccin-mocha\n");
        assert_eq!(
            rc.extends.as_ref().map(ThemeName::as_str),
            Some("catppuccin-mocha")
        );
    }

    #[test]
    fn rc_drops_invalid_extends_name() {
        // `extends = ../etc` would be a path-traversal in disguise. The
        // parser must reject it silently so the resolver never sees it.
        let rc = parse_rc_str("extends = ../etc\nextends.dark = name with space\n");
        assert!(rc.extends.is_none());
        assert!(rc.extends_dark.is_none());
    }

    #[test]
    fn rc_duplicate_extends_last_wins() {
        // Matches the "last writer wins" rule used by color keys.
        let rc = parse_rc_str("extends = first\nextends = second\n");
        assert_eq!(rc.extends.as_ref().map(ThemeName::as_str), Some("second"));
    }

    #[test]
    fn rc_invalid_duplicate_does_not_clobber_prior_valid() {
        // A later invalid value is dropped silently; the previously-set
        // valid value stays.
        let rc = parse_rc_str("extends = valid\nextends = ../etc\n");
        assert_eq!(rc.extends.as_ref().map(ThemeName::as_str), Some("valid"));
    }

    #[test]
    fn rc_rejects_invalid_color() {
        let rc = parse_rc_str("fg = not-a-color\nbg = #abc\n");
        assert!(rc.base.fg.is_none());
        assert!(rc.base.bg.is_none());
    }

    #[test]
    fn rc_unknown_keys_and_sections_dropped() {
        let rc = parse_rc_str("[bogus]\nfg = #ffffff\n[dark]\nbg = #000000\nweird = #ffeeaa\n");
        assert!(rc.base.fg.is_none());
        assert_eq!(rc.dark.bg.unwrap().as_str(), "#000000");
    }

    #[test]
    fn rc_case_insensitive_sections() {
        let rc = parse_rc_str("[DARK]\nbg = #abcdef\n");
        assert_eq!(rc.dark.bg.unwrap().as_str(), "#abcdef");
    }

    #[test]
    fn palette_keys_in_rc() {
        let rc = parse_rc_str("color0 = #111111\ncolor15 = #ffffff\ncolor16 = #aaaaaa\n");
        assert_eq!(rc.base.palette[0].as_ref().unwrap().as_str(), "#111111");
        assert_eq!(rc.base.palette[15].as_ref().unwrap().as_str(), "#ffffff");
        // color16 is out of range and silently dropped
    }

    #[test]
    fn parent_for_picks_specific_then_global() {
        use super::super::model::Mode;
        let rc = ParsedRc {
            extends: Some(ThemeName::parse("base").unwrap()),
            extends_dark: Some(ThemeName::parse("d").unwrap()),
            extends_light: Some(ThemeName::parse("l").unwrap()),
            ..Default::default()
        };
        assert_eq!(rc.parent_for(Mode::Dark).map(ThemeName::as_str), Some("d"));
        assert_eq!(rc.parent_for(Mode::Light).map(ThemeName::as_str), Some("l"));
        assert_eq!(
            rc.parent_for(Mode::Unknown).map(ThemeName::as_str),
            Some("base")
        );
    }

    #[test]
    fn parent_for_falls_back_to_global() {
        use super::super::model::Mode;
        let rc = ParsedRc {
            extends: Some(ThemeName::parse("base").unwrap()),
            ..Default::default()
        };
        assert_eq!(
            rc.parent_for(Mode::Dark).map(ThemeName::as_str),
            Some("base")
        );
        assert_eq!(
            rc.parent_for(Mode::Light).map(ThemeName::as_str),
            Some("base")
        );
        assert_eq!(
            rc.parent_for(Mode::Unknown).map(ThemeName::as_str),
            Some("base")
        );
    }
}
