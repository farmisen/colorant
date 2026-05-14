//! `colorant show` — print the resolved colors that would apply for the
//! current directory.
//!
//! Walks up like `current` to find the `.colorantrc`, resolves it against
//! the detected (or `COLORANT_MODE`-forced) mode, and prints each color
//! slot with its hex code and a 24-bit swatch. Pass `--all` to print both
//! dark and light sequentially instead of just the current mode.

use crate::config::{Config, THEME_FILE_NAME};
use crate::mode;
use crate::terminal::style::Style;
use crate::theme::model::{HexColor, Mode, ThemeLayer};
use crate::theme::parse;
use crate::theme::resolve::{PALETTE_EXTENSION, Resolver};
use crate::walk;
use anyhow::Result;
use std::io::Write;
use std::path::Path;

/// Width of the key column (fits both "cursor" and "color15").
const KEY_WIDTH: usize = 7;
/// Width of the value column (fits "#rrggbb" and "(unset)").
const VALUE_WIDTH: usize = 7;
/// Swatch glyph — four full-block characters drawn with a 24-bit fg color
/// in TTYs (each cell renders as the requested RGB). In piped output the
/// blocks stay visible in the terminal's default foreground so the column
/// still aligns even without color.
const SWATCH: &str = "████";

pub fn run(config: &Config, all: bool) -> Result<()> {
    let style = Style::detect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let cwd = std::env::current_dir()?;
    let rc_path = walk::find_nearest(&cwd, THEME_FILE_NAME);

    match &rc_path {
        Some(path) => writeln!(out, "Active theme for {}", path.display())?,
        None => match &config.default_theme {
            Some(name) => {
                writeln!(out, "Default theme: {name}")?;
                // Warn explicitly if the palette file is missing rather
                // than just printing all-(unset) rows — without this the
                // user sees a confident header followed by no colors and
                // has no clue why.
                let palette_path =
                    config
                        .base_theme_dir
                        .join(format!("{}.{}", name.as_str(), PALETTE_EXTENSION));
                if !palette_path.exists() {
                    writeln!(
                        out,
                        "  {}",
                        style.red(&format!(
                            "(palette file not found at {})",
                            palette_path.display()
                        ))
                    )?;
                }
            }
            None => {
                writeln!(out, "No theme applies in this directory.")?;
                return Ok(());
            }
        },
    }

    let resolver = Resolver::new(config.base_theme_dir.clone());
    if all {
        for (mode, label) in [(Mode::Dark, "Dark"), (Mode::Light, "Light")] {
            writeln!(out)?;
            writeln!(out, "  {label} mode:")?;
            let layer = resolve_layer(&resolver, rc_path.as_deref(), config, mode)?;
            print_layer(&mut out, &layer, style, "    ")?;
        }
    } else {
        let current = mode::detect();
        writeln!(out, "Mode: {}", mode_label(current))?;
        writeln!(out)?;
        let layer = resolve_layer(&resolver, rc_path.as_deref(), config, current)?;
        print_layer(&mut out, &layer, style, "  ")?;
    }

    Ok(())
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Dark => "dark",
        Mode::Light => "light",
        Mode::Unknown => "unknown",
    }
}

/// Compute the flat layer for `mode`: resolve the rc if present, else load
/// the default-theme palette if configured, else an empty layer.
fn resolve_layer(
    resolver: &Resolver,
    rc_path: Option<&Path>,
    config: &Config,
    mode: Mode,
) -> Result<ThemeLayer> {
    if let Some(path) = rc_path {
        return resolver.resolve(path, mode);
    }
    if let Some(default_name) = &config.default_theme {
        let palette_path =
            config
                .base_theme_dir
                .join(format!("{}.{}", default_name.as_str(), PALETTE_EXTENSION));
        if palette_path.exists() {
            let palette = parse::parse_palette_file(&palette_path)?;
            return Ok(palette.layer);
        }
    }
    Ok(ThemeLayer::default())
}

fn print_layer<W: Write>(
    out: &mut W,
    layer: &ThemeLayer,
    style: Style,
    indent: &str,
) -> Result<()> {
    writeln!(
        out,
        "{indent}{}",
        format_slot("fg", layer.fg.as_ref(), style)
    )?;
    writeln!(
        out,
        "{indent}{}",
        format_slot("bg", layer.bg.as_ref(), style)
    )?;
    writeln!(
        out,
        "{indent}{}",
        format_slot("cursor", layer.cursor.as_ref(), style)
    )?;
    writeln!(out)?;
    // Palette: two columns. Left holds 0..7, right holds 8..15 — matches
    // the conventional normal/bright split so users can scan a row to see
    // both faces of the same logical color.
    for i in 0..8 {
        let left = format_slot(&color_name(i), layer.palette[i].as_ref(), style);
        let right = format_slot(&color_name(i + 8), layer.palette[i + 8].as_ref(), style);
        writeln!(out, "{indent}{left}   {right}")?;
    }
    Ok(())
}

fn color_name(idx: usize) -> String {
    // Pad the index to two digits so "color0 " and "color15" align at the
    // same column width.
    format!("color{idx:<2}")
}

fn format_slot(name: &str, color: Option<&HexColor>, style: Style) -> String {
    match color {
        Some(c) => {
            let hex = c.as_str();
            let (r, g, b) = hex_to_rgb(hex);
            let swatch = style.fg_rgb(SWATCH, r, g, b);
            format!("{name:<KEY_WIDTH$}  {hex:<VALUE_WIDTH$}  {swatch}")
        }
        None => {
            // Pad in the swatch position (2-space separator + 4-cell block
            // glyph = 6 chars) so the right column still lines up with the
            // set rows above and below.
            let placeholder = "(unset)";
            format!("{name:<KEY_WIDTH$}  {placeholder:<VALUE_WIDTH$}      ")
        }
    }
}

/// Parse the validated "#rrggbb" into its RGB octets. `HexColor` enforces
/// the shape, so the radix parses can't fail — but we fall back to 0 on the
/// off-chance the invariant is ever broken upstream.
fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let r = u8::from_str_radix(hex.get(1..3).unwrap_or("00"), 16).unwrap_or(0);
    let g = u8::from_str_radix(hex.get(3..5).unwrap_or("00"), 16).unwrap_or(0);
    let b = u8::from_str_radix(hex.get(5..7).unwrap_or("00"), 16).unwrap_or(0);
    (r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_to_rgb_parses_canonical_form() {
        assert_eq!(hex_to_rgb("#000000"), (0, 0, 0));
        assert_eq!(hex_to_rgb("#abcdef"), (0xab, 0xcd, 0xef));
        assert_eq!(hex_to_rgb("#ffffff"), (0xff, 0xff, 0xff));
    }

    #[test]
    fn color_name_pads_for_alignment() {
        assert_eq!(color_name(0), "color0 ");
        assert_eq!(color_name(9), "color9 ");
        assert_eq!(color_name(10), "color10");
        assert_eq!(color_name(15), "color15");
    }

    #[test]
    fn mode_label_covers_every_variant() {
        // Every Mode arm needs a stable user-facing label. The
        // single-mode integration tests force COLORANT_MODE, so Unknown
        // would otherwise never be exercised in CI.
        assert_eq!(mode_label(Mode::Dark), "dark");
        assert_eq!(mode_label(Mode::Light), "light");
        assert_eq!(mode_label(Mode::Unknown), "unknown");
    }
}
