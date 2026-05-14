//! Helpers for editing `.colorantrc` files in place.
//!
//! Shared between the interactive `set` TUI and the non-interactive
//! `themes apply` CLI command — both need to splice the user's chosen
//! `extends` / `extends.dark` / `extends.light` keys into an existing rc
//! without disturbing other base-section keys or `[dark]` / `[light]`
//! sections.

/// Splice the given extends assignments into the existing rc content.
/// Removes any existing top-level extends* lines and writes the new set at
/// the top of the file. All other lines (including the entire `[dark]` /
/// `[light]` sections and any other base-section keys) are preserved
/// verbatim and in order.
pub fn rewrite_extends(
    existing: &str,
    both: Option<&str>,
    dark: Option<&str>,
    light: Option<&str>,
) -> String {
    let mut base_kept: Vec<String> = Vec::new();
    let mut section_kept: Vec<String> = Vec::new();
    let mut in_base = true;

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_base = false;
        }
        if in_base {
            if let Some(eq) = trimmed.find('=') {
                let key = trimmed[..eq].trim();
                if matches!(key, "extends" | "extends.dark" | "extends.light") {
                    // Drop existing extends* — they get replaced.
                    continue;
                }
            }
            base_kept.push(line.to_string());
        } else {
            section_kept.push(line.to_string());
        }
    }

    // Trim trailing blank lines from base so we don't accumulate empty rows
    // between blocks on repeated applies.
    while base_kept.last().is_some_and(|s| s.trim().is_empty()) {
        base_kept.pop();
    }

    let mut new_extends: Vec<String> = Vec::new();
    if let Some(name) = both {
        new_extends.push(format!("extends = {name}"));
    }
    if let Some(name) = dark {
        new_extends.push(format!("extends.dark = {name}"));
    }
    if let Some(name) = light {
        new_extends.push(format!("extends.light = {name}"));
    }

    let mut out: Vec<String> = Vec::new();
    out.extend(new_extends);
    if !base_kept.is_empty() {
        if !out.is_empty() {
            out.push(String::new());
        }
        out.extend(base_kept);
    }
    if !section_kept.is_empty() {
        if !out.is_empty() && !out.last().is_some_and(|s| s.trim().is_empty()) {
            out.push(String::new());
        }
        out.extend(section_kept);
    }

    let mut result = out.join("\n");
    if !result.is_empty() && !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrite_extends_into_empty_writes_just_the_block() {
        let out = rewrite_extends("", Some("ayu"), None, None);
        assert_eq!(out, "extends = ayu\n");
    }

    #[test]
    fn rewrite_extends_writes_all_three_slots() {
        let out = rewrite_extends("", Some("ayu"), Some("nord"), Some("solarized"));
        assert_eq!(
            out,
            "extends = ayu\nextends.dark = nord\nextends.light = solarized\n"
        );
    }

    #[test]
    fn rewrite_extends_replaces_existing_extends_lines() {
        let input = "extends = old\nextends.dark = old-dark\nfg = #ffffff\n";
        let out = rewrite_extends(input, Some("new"), None, None);
        assert_eq!(out, "extends = new\n\nfg = #ffffff\n");
    }

    #[test]
    fn rewrite_extends_preserves_base_keys_and_sections() {
        let input = "extends = old\nfg = #ffffff\n[dark]\ncursor = #ff00ff\n";
        let out = rewrite_extends(input, None, Some("nord"), None);
        assert_eq!(
            out,
            "extends.dark = nord\n\nfg = #ffffff\n\n[dark]\ncursor = #ff00ff\n"
        );
    }

    #[test]
    fn rewrite_extends_with_all_nones_strips_existing_extends() {
        let input = "extends = old\nfg = #ffffff\n";
        let out = rewrite_extends(input, None, None, None);
        assert_eq!(out, "fg = #ffffff\n");
    }

    #[test]
    fn rewrite_extends_no_trailing_blank_lines() {
        // Two applies in a row shouldn't accumulate empty lines.
        let mut current = String::new();
        for _ in 0..3 {
            current = rewrite_extends(&current, Some("ayu"), None, None);
        }
        assert_eq!(current, "extends = ayu\n");
    }

    #[test]
    fn rewrite_extends_normalizes_crlf_to_lf() {
        let input = "extends = old\r\nfg = #ffffff\r\n";
        let out = rewrite_extends(input, Some("new"), None, None);
        assert_eq!(out, "extends = new\n\nfg = #ffffff\n");
    }

    #[test]
    fn rewrite_extends_handles_missing_trailing_newline() {
        let input = "fg = #ffffff";
        let out = rewrite_extends(input, Some("ayu"), None, None);
        assert_eq!(out, "extends = ayu\n\nfg = #ffffff\n");
    }

    #[test]
    fn rewrite_extends_preserves_unclosed_section_header_as_base() {
        let input = "[dark\nextends.dark = something\nfg = #ffffff\n";
        let out = rewrite_extends(input, None, Some("nord"), None);
        assert_eq!(out, "extends.dark = nord\n\n[dark\nfg = #ffffff\n");
    }
}
