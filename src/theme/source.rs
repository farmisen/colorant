//! Source dispatch — where a theme can come from.
//!
//! v1 has two sources: `Bundled` (compiled into the binary via
//! `theme::bundled::BUNDLED_THEMES`) and `Gogh` (fetched from
//! <https://github.com/Gogh-Co/Gogh> on demand). The enum centralizes
//! source-specific logic so the CLI (and later the TUI) treats them
//! uniformly: list names, fetch a palette, sync any catalog cache.

use crate::theme::bundled::BUNDLED_THEMES;
use crate::theme::gogh;
use crate::theme::model::ParsedPalette;
use crate::theme::parse::parse_palette_str;
use anyhow::{Result, anyhow};
use std::fmt;

/// One known source of themes. Adding a third source means a new variant
/// plus a match arm in each method here — keeping the list small is
/// deliberate (cf. `colorant themes` only really makes sense with a
/// handful of curated sources).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Themes compiled into the binary. Always available, no network.
    Bundled,
    /// Themes fetched from the Gogh repository. Catalog is cached locally
    /// after `colorant themes sync`.
    Gogh,
}

impl Source {
    /// Identifier used at the CLI surface (`--source <name>`, `gogh:foo`).
    pub fn name(self) -> &'static str {
        match self {
            Source::Bundled => "bundled",
            Source::Gogh => gogh::NAME,
        }
    }

    /// Every source colorant knows about. Iterated in a stable order so
    /// `themes list` / `themes search` output is deterministic.
    pub fn all() -> &'static [Source] {
        &[Source::Bundled, Source::Gogh]
    }

    /// Parse a CLI identifier into a `Source`. Returns `None` for unknown
    /// names so callers can surface a helpful error.
    pub fn parse(name: &str) -> Option<Source> {
        match name {
            "bundled" => Some(Source::Bundled),
            n if n == gogh::NAME => Some(Source::Gogh),
            _ => None,
        }
    }

    /// List the theme names available from this source. For `Bundled` this
    /// is the compile-time list; for remote sources it's whatever the last
    /// `sync` put in the cache (returns an empty list with an error message
    /// in stderr if the cache is missing).
    pub fn list(self) -> Result<Vec<String>> {
        match self {
            Source::Bundled => Ok(BUNDLED_THEMES
                .iter()
                .map(|(n, _)| (*n).to_string())
                .collect()),
            Source::Gogh => match gogh::cached_names()? {
                Some(names) => Ok(names),
                None => Err(anyhow!(
                    "gogh catalog not synced yet — run `colorant themes sync` first"
                )),
            },
        }
    }

    /// Refresh any cached state. No-op for sources that don't need it.
    pub fn sync(self) -> Result<()> {
        match self {
            Source::Bundled => Ok(()),
            Source::Gogh => gogh::sync().map(|_| ()),
        }
    }

    /// Fetch one theme by name. Returns a `ParsedPalette` the caller can
    /// either install (`fs::write` into `base_theme_dir`) or use directly.
    pub fn fetch(self, name: &str) -> Result<ParsedPalette> {
        match self {
            Source::Bundled => BUNDLED_THEMES
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, content)| parse_palette_str(content))
                .ok_or_else(|| anyhow!("no bundled theme named {name}")),
            Source::Gogh => gogh::fetch(name),
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Split a theme reference like `gogh:Dracula` or `dracula` into
/// `(Option<Source>, name)`. An unqualified name returns `None` for the
/// source — the caller decides the lookup order (typically: installed →
/// bundled → error-and-suggest-source-prefix).
pub fn parse_ref(s: &str) -> (Option<Source>, &str) {
    if let Some((prefix, name)) = s.split_once(':')
        && let Some(source) = Source::parse(prefix)
    {
        return (Some(source), name);
    }
    (None, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_round_trip() {
        for source in Source::all() {
            assert_eq!(Source::parse(source.name()), Some(*source));
        }
    }

    #[test]
    fn source_parse_unknown_returns_none() {
        assert!(Source::parse("definitely-not-a-source").is_none());
    }

    #[test]
    fn parse_ref_recognizes_source_prefix() {
        assert_eq!(parse_ref("gogh:Dracula"), (Some(Source::Gogh), "Dracula"));
        assert_eq!(
            parse_ref("bundled:catppuccin-mocha"),
            (Some(Source::Bundled), "catppuccin-mocha")
        );
    }

    #[test]
    fn parse_ref_unqualified_returns_none_source() {
        assert_eq!(parse_ref("dracula"), (None, "dracula"));
    }

    #[test]
    fn parse_ref_unknown_prefix_treats_whole_string_as_name() {
        // `wezterm:` isn't a v1 source, so the whole thing is the name.
        // The CLI will then try the name as-is in installed/bundled and
        // surface a clear "not found" message.
        assert_eq!(parse_ref("wezterm:Foo"), (None, "wezterm:Foo"));
    }

    #[test]
    fn bundled_list_returns_known_themes() {
        let names = Source::Bundled.list().unwrap();
        assert!(names.contains(&"catppuccin-mocha".to_string()));
    }
}
