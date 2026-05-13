//! Resolves a `.colorantrc` into a single flat `ThemeLayer` for a given mode.
//!
//! Composition is intentionally one-level: a `.colorantrc` may inherit colors
//! from at most one `.colorant` palette (chosen per mode), then layers its own
//! keys on top.
//!
//! Order of operations for mode `M`:
//!
//! 1. Pick the parent palette name: `extends.M` if set, else top-level
//!    `extends`, else none.
//! 2. If a parent is named and `<base_theme_dir>/<name>.colorant` exists,
//!    merge its layer into the accumulator.
//! 3. Merge the rc's `base` (top-level) keys.
//! 4. Merge the rc's `[M]` section if `M` is dark or light.
//!
//! This gives the property "child always beats parent, even when the parent
//! is a per-mode palette" — the child's top-level keys override the inherited
//! palette, and the child's mode section overrides everything.

use super::model::{Mode, ParsedRc, ThemeLayer};
use super::parse;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// Extension used for palette files in the base theme directory.
pub const PALETTE_EXTENSION: &str = "colorant";

/// Resolves a config file by loading the right palette (if any) for the
/// requested mode and merging the rc's own keys on top.
pub struct Resolver {
    /// Directory where palette files named via `extends` are looked up.
    pub(crate) base_theme_dir: PathBuf,
}

impl Resolver {
    /// Construct a resolver that looks up palette files in `base_theme_dir`.
    pub fn new(base_theme_dir: PathBuf) -> Self {
        Self { base_theme_dir }
    }

    /// Resolve `path` (a `.colorantrc`) into a flat layer for `mode`.
    pub fn resolve(&self, path: &Path, mode: Mode) -> Result<ThemeLayer> {
        let rc = parse::parse_rc_file(path)?;
        self.flatten(&rc, mode)
    }

    /// Resolve a parsed rc directly (used by callers that already parsed,
    /// e.g. when applying the global default theme).
    pub fn flatten(&self, rc: &ParsedRc, mode: Mode) -> Result<ThemeLayer> {
        let mut acc = ThemeLayer::default();

        if let Some(parent_name) = rc.parent_for(mode) {
            let palette_path =
                self.base_theme_dir
                    .join(format!("{}.{}", parent_name.as_str(), PALETTE_EXTENSION));
            if palette_path.exists() {
                let palette = parse::parse_palette_file(&palette_path)?;
                acc.merge(&palette.layer);
            }
        }

        acc.merge(&rc.base);
        match mode {
            Mode::Dark => acc.merge(&rc.dark),
            Mode::Light => acc.merge(&rc.light),
            Mode::Unknown => {}
        }
        Ok(acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::model::{HexColor, ThemeName};
    use tempfile::tempdir;

    fn layer_with_fg(hex: &str) -> ThemeLayer {
        ThemeLayer {
            fg: HexColor::parse(hex),
            ..Default::default()
        }
    }

    #[test]
    fn unknown_mode_skips_per_mode_layers_and_extends() {
        // Setup: a global `extends = base` palette plus `extends.dark = dark`.
        // Rc carries a base fg, a [dark] override, and a [light] override.
        // In Unknown mode the resolver must:
        //   - load the global `base` palette (not `dark`)
        //   - apply the rc's own base keys
        //   - skip both [dark] and [light] overlays
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("base.colorant"),
            "fg = #aaaaaa\nbg = #bbbbbb\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("dark.colorant"), "fg = #000000\n").unwrap();

        let rc = ParsedRc {
            extends: Some(ThemeName::parse("base").unwrap()),
            extends_dark: Some(ThemeName::parse("dark").unwrap()),
            base: layer_with_fg("#cccccc"),
            dark: layer_with_fg("#111111"),
            light: layer_with_fg("#eeeeee"),
            ..Default::default()
        };

        let resolver = Resolver::new(dir.path().to_path_buf());
        let resolved = resolver.flatten(&rc, Mode::Unknown).unwrap();

        // Global `base` palette loaded (bg inherited), then rc's own base fg
        // wins over the palette's fg. No per-mode overlay applied.
        assert_eq!(resolved.fg.as_ref().unwrap().as_str(), "#cccccc");
        assert_eq!(resolved.bg.as_ref().unwrap().as_str(), "#bbbbbb");
    }
}
