//! Global configuration loaded from `~/.config/colorant/config.toml`.
//!
//! All fields are optional; missing values fall back to sensible defaults.
//! Example config:
//!
//! ```toml
//! # ~/.config/colorant/config.toml
//! base_theme_dir = "~/.config/colorant/themes"
//! default_theme = "catppuccin-mocha"
//! ```

use crate::theme::model::ThemeName;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

/// Name of the per-directory config file colorant looks for during walk-up.
pub const THEME_FILE_NAME: &str = ".colorantrc";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directory where palette files referenced via `extends` (in
    /// `.colorantrc` files) and `default_theme` (here) are looked up.
    /// Defaults to `$XDG_CONFIG_HOME/colorant/themes` (or
    /// `$HOME/.config/colorant/themes`).
    pub base_theme_dir: PathBuf,

    /// Palette name (without extension) applied when no `.colorantrc` is
    /// found while walking up from `cwd`. Resolves to
    /// `<base_theme_dir>/<name>.colorant`. When unset, `colorant apply`
    /// emits the reset sequence instead.
    ///
    /// Validated at config load: an invalid name (empty, path traversal,
    /// whitespace, etc.) makes `Config::load` return an error rather than
    /// silently misbehaving downstream.
    pub default_theme: Option<ThemeName>,
}

impl Default for Config {
    fn default() -> Self {
        let base_theme_dir = config_dir()
            .map(|p| p.join("themes"))
            .unwrap_or_else(|| PathBuf::from(".colorant/themes"));
        Self {
            base_theme_dir,
            default_theme: None,
        }
    }
}

impl Config {
    /// Load `<config_dir>/colorant/config.toml` if present, falling back to
    /// `Config::default()` otherwise. Returns an error only when the file
    /// exists but is malformed.
    pub fn load() -> Result<Self> {
        let Some(dir) = config_dir() else {
            return Ok(Self::default());
        };
        let path = dir.join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config =
            toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }
}

/// The directory holding colorant's config and bundled themes.
///
/// We use `$XDG_CONFIG_HOME/colorant` if `XDG_CONFIG_HOME` is set (and
/// non-empty), otherwise `$HOME/.config/colorant`. This matches the
/// convention followed by tools like starship and zoxide rather than the
/// platform-default `dirs::config_dir()` (which uses
/// `~/Library/Application Support` on macOS).
fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("colorant"));
    }
    dirs::home_dir().map(|h| h.join(".config").join("colorant"))
}
