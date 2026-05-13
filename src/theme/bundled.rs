//! Compile-time bundled palette files.
//!
//! Populated by `build.rs` scanning the repo's `themes/` directory; each
//! `.colorant` file becomes one `(name, contents)` pair in
//! `BUNDLED_THEMES`. `commands::themes` exposes these to the user via the
//! `colorant themes` command group.

include!(concat!(env!("OUT_DIR"), "/bundled_themes.rs"));

#[cfg(test)]
mod tests {
    use super::BUNDLED_THEMES;

    #[test]
    fn includes_known_palettes() {
        let names: Vec<&str> = BUNDLED_THEMES.iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"catppuccin-mocha"));
        assert!(names.contains(&"tokyo-night"));
        assert!(names.contains(&"nord"));
    }

    #[test]
    fn entries_are_sorted_by_name() {
        let names: Vec<&str> = BUNDLED_THEMES.iter().map(|(n, _)| *n).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
    }

    #[test]
    fn contents_are_non_empty() {
        for (name, content) in BUNDLED_THEMES {
            assert!(!content.is_empty(), "palette {} has empty contents", name);
        }
    }
}
