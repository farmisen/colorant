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

    /// When true and the resolved theme does not set `tab_bg` explicitly,
    /// `apply` derives it from the resolved `bg` so the tab matches the
    /// terminal background. Set to false to leave the tab color alone
    /// unless a `.colorantrc` / palette names it directly. Defaults to
    /// true. Only affects terminals that support a tab-color escape
    /// (today: iTerm2).
    pub tab_follows_window: bool,
}

impl Default for Config {
    fn default() -> Self {
        let base_theme_dir = config_dir()
            .map(|p| p.join("themes"))
            .unwrap_or_else(|| PathBuf::from(".colorant/themes"));
        Self {
            base_theme_dir,
            default_theme: None,
            tab_follows_window: true,
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

/// The directory holding colorant's cached state — remote theme catalogs,
/// fetched palette files, anything that's *derived* and safe to wipe.
///
/// Mirrors `config_dir`'s XDG convention: `$XDG_CACHE_HOME/colorant` if
/// set, otherwise `$HOME/.cache/colorant`. Returns `None` only when
/// neither `$XDG_CACHE_HOME` nor `$HOME` resolves — the caller treats
/// that as a hard error rather than silently fabricating a path.
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_for(std::env::var("XDG_CACHE_HOME").ok(), dirs::home_dir())
}

/// Pure decision split out from `cache_dir` for unit-testability.
/// Mutating `XDG_CACHE_HOME` from within `#[test]` is racy under
/// parallel tests, so the policy lives here and the env probe stays in
/// the wrapper.
fn cache_dir_for(xdg: Option<String>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(xdg) = xdg
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("colorant"));
    }
    home.map(|h| h.join(".cache").join("colorant"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_uses_xdg_when_set() {
        let xdg = Some("/tmp/xdg".to_string());
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            cache_dir_for(xdg, home),
            Some(PathBuf::from("/tmp/xdg/colorant"))
        );
    }

    #[test]
    fn cache_dir_falls_back_when_xdg_empty() {
        // Empty XDG_CACHE_HOME is treated as unset — otherwise a stray
        // `XDG_CACHE_HOME=` in the env would produce a relative cache
        // path like `colorant/`, polluting the cwd.
        let xdg = Some(String::new());
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            cache_dir_for(xdg, home),
            Some(PathBuf::from("/home/me/.cache/colorant"))
        );
    }

    #[test]
    fn cache_dir_falls_back_when_xdg_unset() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            cache_dir_for(None, home),
            Some(PathBuf::from("/home/me/.cache/colorant"))
        );
    }

    #[test]
    fn cache_dir_returns_none_when_no_home_and_no_xdg() {
        assert_eq!(cache_dir_for(None, None), None);
        assert_eq!(cache_dir_for(Some(String::new()), None), None);
    }

    #[test]
    fn tab_follows_window_defaults_to_true() {
        // Pinned so a future cleanup of `Default` can't silently flip the
        // user-facing default. The README's config.toml example and the
        // apply-time auto-derive both rely on this.
        assert!(Config::default().tab_follows_window);
    }
}
