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
//!
//! ## Diagnostics
//!
//! The rc parser exposes a `_with_diagnostics` variant that reports every
//! silently-dropped line and why. `colorant doctor` consumes it so users can
//! find typos that would otherwise lurk in their `.colorantrc` without ever
//! producing an error. The regular `parse_rc_str` shim discards the
//! diagnostics for the hot path.

use super::model::{HexColor, ParsedPalette, ParsedRc, ThemeLayer, ThemeName, ThemeNameError};
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

/// Categorized reasons the rc parser silently dropped a line.
///
/// Each variant carries enough context for `colorant doctor` to print a
/// useful one-liner without the user having to re-read the file. Returned
/// from `parse_rc_str_with_diagnostics` alongside the parsed result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropReason {
    /// Non-empty, non-comment line with no `=` separator.
    MalformedLine,
    /// Section header other than `[dark]` or `[light]`. Reported once for
    /// the header itself; per-line drops inside the section are suppressed.
    UnknownSection(String),
    /// Key isn't `fg`, `bg`, `cursor`, `color0`..`color15`, `extends`,
    /// `extends.dark`, or `extends.light`.
    UnknownKey(String),
    /// Key would set a color but the value isn't a valid `#rrggbb`.
    InvalidColor { key: String, value: String },
    /// Key is one of `extends`, `extends.dark`, `extends.light` but the
    /// value isn't a valid theme name.
    InvalidExtendsName {
        key: String,
        value: String,
        error: ThemeNameError,
    },
}

/// Parse a `.colorant` palette from disk.
pub fn parse_palette_file(path: &Path) -> Result<ParsedPalette> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(parse_palette_str(&content))
}

/// Parse a `.colorant` palette from a string. Sections and `extends` lines
/// are silently ignored — palettes are flat by design. Use
/// `parse_palette_str_with_diagnostics` if you want to surface drops (the
/// shape doctor consumes for palette files referenced via `extends`).
pub fn parse_palette_str(content: &str) -> ParsedPalette {
    parse_palette_str_with_diagnostics(content).0
}

/// Parse a `.colorant` palette and report every silently-dropped line.
/// Section headers (`[anything]`) and comments (`# ...`) are still skipped
/// without a diagnostic — palettes don't have sections by design and a
/// section-looking line is treated as a no-op for forward compatibility.
pub fn parse_palette_str_with_diagnostics(
    content: &str,
) -> (ParsedPalette, Vec<(usize, DropReason)>) {
    let mut layer = ThemeLayer::default();
    let mut diags: Vec<(usize, DropReason)> = Vec::new();

    for (idx, raw) in content.lines().enumerate() {
        let lineno = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some(eq_idx) = line.find('=') else {
            diags.push((lineno, DropReason::MalformedLine));
            continue;
        };
        let key = line[..eq_idx].trim();
        let value = line[eq_idx + 1..].trim();
        if key.is_empty() {
            diags.push((lineno, DropReason::MalformedLine));
            continue;
        }
        apply_kv(&mut layer, key, value, lineno, &mut diags);
    }

    (ParsedPalette { layer }, diags)
}

/// Parse a `.colorantrc` from disk.
pub fn parse_rc_file(path: &Path) -> Result<ParsedRc> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(parse_rc_str(&content))
}

/// Parse a `.colorantrc` from a string. Never fails — malformed lines are
/// silently skipped. Use `parse_rc_str_with_diagnostics` to see what was
/// dropped.
pub fn parse_rc_str(content: &str) -> ParsedRc {
    parse_rc_str_with_diagnostics(content).0
}

/// Parse a `.colorantrc` from a string and report every silently-dropped
/// line. Returns the same `ParsedRc` that `parse_rc_str` would, paired with
/// a list of `(1-based line number, DropReason)` entries in file order.
pub fn parse_rc_str_with_diagnostics(content: &str) -> (ParsedRc, Vec<(usize, DropReason)>) {
    let mut rc = ParsedRc::default();
    let mut diags: Vec<(usize, DropReason)> = Vec::new();
    let mut section = Section::Base;

    for (idx, raw) in content.lines().enumerate() {
        let lineno = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(stripped) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let original = stripped.trim();
            section = match original.to_ascii_lowercase().as_str() {
                "dark" => Section::Dark,
                "light" => Section::Light,
                _ => {
                    // Report the original casing so a typo of [LITE] shows
                    // up as [LITE] in the doctor message, not [lite].
                    diags.push((lineno, DropReason::UnknownSection(original.to_string())));
                    Section::Unknown
                }
            };
            continue;
        }

        let Some(eq_idx) = line.find('=') else {
            diags.push((lineno, DropReason::MalformedLine));
            continue;
        };
        let key = line[..eq_idx].trim();
        let value = line[eq_idx + 1..].trim();
        if key.is_empty() {
            diags.push((lineno, DropReason::MalformedLine));
            continue;
        }

        match section {
            // The section header itself was already reported; per-line
            // drops inside it would only add noise.
            Section::Unknown => continue,
            Section::Base => apply_rc_base_kv(&mut rc, key, value, lineno, &mut diags),
            Section::Dark => apply_kv(&mut rc.dark, key, value, lineno, &mut diags),
            Section::Light => apply_kv(&mut rc.light, key, value, lineno, &mut diags),
        }
    }

    (rc, diags)
}

/// Top-level keys in a `.colorantrc`: `extends`, `extends.dark`,
/// `extends.light`, plus the regular palette keys (which form the file's
/// `base` layer).
fn apply_rc_base_kv(
    rc: &mut ParsedRc,
    key: &str,
    value: &str,
    lineno: usize,
    diags: &mut Vec<(usize, DropReason)>,
) {
    // Duplicate keys: last valid value wins, matching the rule used by
    // `apply_kv` for color keys. An invalid value is dropped silently and
    // leaves any previously-set valid value in place.
    let extends_slot = match key {
        "extends" => Some(&mut rc.extends),
        "extends.dark" => Some(&mut rc.extends_dark),
        "extends.light" => Some(&mut rc.extends_light),
        _ => None,
    };
    if let Some(slot) = extends_slot {
        match ThemeName::parse(value) {
            Ok(name) => *slot = Some(name),
            Err(error) => diags.push((
                lineno,
                DropReason::InvalidExtendsName {
                    key: key.to_string(),
                    value: value.to_string(),
                    error,
                },
            )),
        }
        return;
    }
    apply_kv(&mut rc.base, key, value, lineno, diags);
}

/// Apply a single palette key/value to a layer. Keys we don't recognize are
/// reported as `UnknownKey`. Values that aren't valid colors are reported
/// as `InvalidColor`.
fn apply_kv(
    layer: &mut ThemeLayer,
    key: &str,
    value: &str,
    lineno: usize,
    diags: &mut Vec<(usize, DropReason)>,
) {
    let slot: Option<&mut Option<HexColor>> = match key {
        "fg" => Some(&mut layer.fg),
        "bg" => Some(&mut layer.bg),
        "cursor" => Some(&mut layer.cursor),
        k if k.starts_with("color") => {
            let suffix = &k[5..];
            match suffix.parse::<usize>() {
                Ok(idx) if idx < 16 => Some(&mut layer.palette[idx]),
                // "colorXYZ" or "color99": looks like a palette key but
                // isn't one we recognize, so surface it as UnknownKey.
                _ => {
                    diags.push((lineno, DropReason::UnknownKey(key.to_string())));
                    return;
                }
            }
        }
        _ => None,
    };
    let Some(slot) = slot else {
        diags.push((lineno, DropReason::UnknownKey(key.to_string())));
        return;
    };
    match HexColor::parse(value) {
        Some(c) => *slot = Some(c),
        None => diags.push((
            lineno,
            DropReason::InvalidColor {
                key: key.to_string(),
                value: value.to_string(),
            },
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::model::{HexColor, ThemeName};

    #[test]
    fn parses_simple_palette() {
        let pal = parse_palette_str("fg = #112233\nbg = #aabbcc\n");
        assert_eq!(pal.layer.fg, HexColor::parse("#112233"));
        assert_eq!(pal.layer.bg, HexColor::parse("#aabbcc"));
    }

    #[test]
    fn palette_ignores_sections_and_extends() {
        // Anything section-like or extends-y is silently dropped — palettes
        // are flat by design. The section header line itself is skipped, but
        // there is no section-state tracking: keys *after* the header are
        // still applied to the flat layer.
        let p = parse_palette_str(
            "extends = nope\n[dark]\nfg = #111111\nextends.dark = also-nope\nbg = #222222\n",
        );
        assert_eq!(p.layer.fg.unwrap().as_str(), "#111111");
        assert_eq!(p.layer.bg.unwrap().as_str(), "#222222");
    }

    #[test]
    fn palette_diagnostics_report_extends_as_unknown_key() {
        // Palettes are flat — `extends` is not a recognized key. The
        // diagnostics path surfaces it as UnknownKey so `colorant doctor`
        // can warn the user.
        let (_, diags) = parse_palette_str_with_diagnostics("extends = catppuccin-mocha\n");
        assert_eq!(diags, vec![(1, DropReason::UnknownKey("extends".into()))]);
    }

    #[test]
    fn palette_diagnostics_do_not_report_section_headers() {
        // Section-looking lines in a palette are silently skipped — palettes
        // don't have sections, but lone `[anything]` is treated as a no-op
        // for forward compatibility rather than a parse error. Lock this in
        // so a future refactor doesn't accidentally start flagging them.
        let (_, diags) = parse_palette_str_with_diagnostics("[dark]\nfg = #111111\n");
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    #[test]
    fn diagnostics_palette_path_produces_same_palette_as_shim() {
        // Mirror of the rc shim equivalence test — locks in that
        // `parse_palette_str` and `parse_palette_str_with_diagnostics` agree
        // on the parsed layer.
        let input = "extends = nope\nfg = #111111\nforground = #aabbcc\nbg = bad\n";
        let plain = parse_palette_str(input);
        let (with_diag, diags) = parse_palette_str_with_diagnostics(input);
        assert_eq!(plain, with_diag);
        assert!(!diags.is_empty());
    }

    #[test]
    fn palette_diagnostics_report_malformed_line() {
        let (_, diags) =
            parse_palette_str_with_diagnostics("no equals here\nfg = #111111\n= no key\n");
        assert_eq!(
            diags,
            vec![
                (1, DropReason::MalformedLine),
                (3, DropReason::MalformedLine),
            ]
        );
    }

    #[test]
    fn parses_rc_with_per_mode_extends() {
        let rc = parse_rc_str(
            "extends.dark = catppuccin-mocha\nextends.light = catppuccin-latte\nfg = #ffffff\n",
        );
        assert_eq!(
            rc.extends_dark,
            Some(ThemeName::parse("catppuccin-mocha").unwrap())
        );
        assert_eq!(
            rc.extends_light,
            Some(ThemeName::parse("catppuccin-latte").unwrap())
        );
        assert_eq!(rc.base.fg, HexColor::parse("#ffffff"));
    }

    #[test]
    fn rc_global_extends_still_works() {
        let rc = parse_rc_str("extends = ayu\n");
        assert_eq!(rc.extends, Some(ThemeName::parse("ayu").unwrap()));
    }

    #[test]
    fn rc_drops_invalid_extends_name() {
        let rc = parse_rc_str("extends = bad/name\n");
        assert!(rc.extends.is_none());
    }

    #[test]
    fn rc_duplicate_extends_last_wins() {
        let rc = parse_rc_str("extends = a\nextends = b\n");
        assert_eq!(rc.extends, Some(ThemeName::parse("b").unwrap()));
    }

    #[test]
    fn rc_invalid_duplicate_does_not_clobber_prior_valid() {
        let rc = parse_rc_str("extends = good\nextends = bad/name\n");
        assert_eq!(rc.extends, Some(ThemeName::parse("good").unwrap()));
    }

    #[test]
    fn rc_rejects_invalid_color() {
        let rc = parse_rc_str("fg = not-a-color\n");
        assert!(rc.base.fg.is_none());
    }

    #[test]
    fn rc_unknown_keys_and_sections_dropped() {
        let rc = parse_rc_str("forground = #112233\n[lite]\nfg = #aabbcc\n");
        assert!(rc.base.fg.is_none());
        assert!(rc.dark.fg.is_none());
        assert!(rc.light.fg.is_none());
    }

    #[test]
    fn rc_case_insensitive_sections() {
        let rc = parse_rc_str("[Dark]\nfg = #112233\n");
        assert_eq!(rc.dark.fg, HexColor::parse("#112233"));
    }

    #[test]
    fn palette_keys_in_rc() {
        let rc = parse_rc_str("color0 = #112233\ncolor15 = #aabbcc\n");
        assert_eq!(rc.base.palette[0], HexColor::parse("#112233"));
        assert_eq!(rc.base.palette[15], HexColor::parse("#aabbcc"));
    }

    // --- diagnostics ---

    #[test]
    fn diagnostics_empty_for_clean_rc() {
        let (_, diags) = parse_rc_str_with_diagnostics(
            "extends = catppuccin-mocha\nfg = #ffffff\n[dark]\ncursor = #ff00ff\n",
        );
        assert!(diags.is_empty(), "expected no diagnostics, got {diags:?}");
    }

    #[test]
    fn diagnostics_report_unknown_key() {
        let (_, diags) = parse_rc_str_with_diagnostics("forground = #112233\n");
        assert_eq!(diags, vec![(1, DropReason::UnknownKey("forground".into()))]);
    }

    #[test]
    fn diagnostics_report_invalid_color() {
        let (_, diags) = parse_rc_str_with_diagnostics("fg = nope\n");
        assert_eq!(
            diags,
            vec![(
                1,
                DropReason::InvalidColor {
                    key: "fg".into(),
                    value: "nope".into()
                }
            )]
        );
    }

    #[test]
    fn diagnostics_report_invalid_extends() {
        let (_, diags) = parse_rc_str_with_diagnostics("extends.dark = bad/name\n");
        match diags.as_slice() {
            [(1, DropReason::InvalidExtendsName { key, value, .. })] => {
                assert_eq!(key, "extends.dark");
                assert_eq!(value, "bad/name");
            }
            other => panic!("unexpected diagnostics: {other:?}"),
        }
    }

    #[test]
    fn diagnostics_report_unknown_section_once() {
        // Header on line 2, key on line 3 — only the header is reported,
        // the inner key is suppressed to avoid noise. Also verify the
        // section's own fg key is NOT silently leaked into base/dark/light.
        let (rc, diags) = parse_rc_str_with_diagnostics("\n[lite]\nfg = #112233\n");
        assert_eq!(diags, vec![(2, DropReason::UnknownSection("lite".into()))]);
        assert!(rc.base.fg.is_none());
        assert!(rc.dark.fg.is_none());
        assert!(rc.light.fg.is_none());
    }

    #[test]
    fn diagnostics_path_produces_same_rc_as_shim() {
        // The non-diagnostic `parse_rc_str` is a one-line shim over
        // `parse_rc_str_with_diagnostics`. Lock the equivalence in a test
        // so a future "optimization" of either path can't silently drift.
        let input = "extends = good\n\
                     extends.dark = bad/name\n\
                     fg = #112233\n\
                     forground = #aabbcc\n\
                     [dark]\n\
                     cursor = #ff00ff\n\
                     bg = nope\n\
                     [lite]\n\
                     fg = #000000\n";
        let plain = parse_rc_str(input);
        let (with_diag, diags) = parse_rc_str_with_diagnostics(input);
        assert_eq!(plain, with_diag);
        // Sanity: this input does produce drops, so the equivalence isn't
        // trivially "no diagnostics fired".
        assert!(!diags.is_empty());
    }

    #[test]
    fn diagnostics_preserve_original_section_casing() {
        // Section matching is case-insensitive (so [LITE] still doesn't get
        // misclassified as [light]), but doctor needs to show the user
        // exactly what they typed so they can find the typo in their file.
        let (_, diags) = parse_rc_str_with_diagnostics("[Lite]\n");
        assert_eq!(diags, vec![(1, DropReason::UnknownSection("Lite".into()))]);
    }

    #[test]
    fn diagnostics_report_malformed_line() {
        let (_, diags) = parse_rc_str_with_diagnostics("no equals sign here\n= no key\n");
        assert_eq!(
            diags,
            vec![
                (1, DropReason::MalformedLine),
                (2, DropReason::MalformedLine)
            ]
        );
    }

    #[test]
    fn diagnostics_color_out_of_range_is_unknown_key() {
        // color99 looks like a palette key but the index is out of range:
        // surface it as UnknownKey so the user knows the parser didn't
        // accept it.
        let (_, diags) = parse_rc_str_with_diagnostics("color99 = #112233\n");
        assert_eq!(diags, vec![(1, DropReason::UnknownKey("color99".into()))]);
    }

    #[test]
    fn diagnostics_preserve_file_order() {
        let (_, diags) =
            parse_rc_str_with_diagnostics("fg = #ffffff\nforground = #112233\nbg = bad\n[lite]\n");
        let kinds: Vec<_> = diags.iter().map(|(line, r)| (*line, r.clone())).collect();
        assert_eq!(
            kinds,
            vec![
                (2, DropReason::UnknownKey("forground".into())),
                (
                    3,
                    DropReason::InvalidColor {
                        key: "bg".into(),
                        value: "bad".into()
                    }
                ),
                (4, DropReason::UnknownSection("lite".into())),
            ]
        );
    }

    #[test]
    fn parent_for_picks_specific_then_global() {
        let rc = parse_rc_str("extends = global\nextends.dark = darkonly\n");
        use crate::theme::model::Mode;
        assert_eq!(
            rc.parent_for(Mode::Dark),
            Some(&ThemeName::parse("darkonly").unwrap())
        );
        assert_eq!(
            rc.parent_for(Mode::Light),
            Some(&ThemeName::parse("global").unwrap())
        );
    }

    #[test]
    fn parent_for_falls_back_to_global() {
        let rc = parse_rc_str("extends = global\n");
        use crate::theme::model::Mode;
        assert_eq!(
            rc.parent_for(Mode::Dark),
            Some(&ThemeName::parse("global").unwrap())
        );
    }
}
