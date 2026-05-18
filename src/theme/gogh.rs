//! Gogh remote source — fetches palettes from
//! <https://github.com/Gogh-Co/Gogh> and converts their YAML files into
//! `ParsedPalette`s the rest of colorant already understands.
//!
//! The catalog (list of available theme names) is fetched once via the
//! GitHub Git Trees API and cached on disk; individual themes are fetched
//! lazily from raw.githubusercontent.com on install. Network only happens
//! during `colorant themes sync` and `colorant themes apply`/`install` for
//! a previously-unfetched theme.

use crate::config::cache_dir;
use crate::fs_util::atomic_write;
use crate::theme::model::{HexColor, ParsedPalette, ThemeLayer};
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

/// Where Gogh's individual theme YAMLs live on the raw CDN. Each file is
/// fetched as `<RAW_BASE>/<name>.yml`.
const RAW_BASE: &str = "https://raw.githubusercontent.com/Gogh-Co/Gogh/master/themes";

/// GitHub Git Trees endpoint for listing the contents of `themes/` on the
/// `master` branch. Recursive so we get every blob in one round trip.
const TREE_URL: &str = "https://api.github.com/repos/Gogh-Co/Gogh/git/trees/master?recursive=1";

/// HTTP timeout — Gogh is a small repo, this is plenty even on slow links.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Identifier this source is known by from the CLI, e.g. `themes apply --source gogh`.
pub const NAME: &str = "gogh";

/// Resolve the Gogh-specific cache subdirectory.
fn gogh_cache_dir() -> Result<PathBuf> {
    let base = cache_dir().ok_or_else(|| anyhow!("could not resolve cache dir"))?;
    Ok(base.join("remotes").join("gogh"))
}

fn index_path() -> Result<PathBuf> {
    Ok(gogh_cache_dir()?.join("index.json"))
}

/// Refresh the local catalog from the GitHub API. Writes the trimmed list
/// of theme names to `<cache>/remotes/gogh/index.json` (a JSON array of
/// strings). Idempotent — calling repeatedly just rewrites the file.
pub fn sync() -> Result<Vec<String>> {
    let response = ureq::get(TREE_URL)
        .timeout(HTTP_TIMEOUT)
        // GitHub returns 403 without a User-Agent.
        .set("User-Agent", "colorant")
        .call()
        .with_context(|| format!("fetching {TREE_URL}"))?;
    let body = response.into_string().context("reading github tree body")?;
    let names = parse_tree_response(&body)?;

    let dir = gogh_cache_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("creating cache dir {}", dir.display()))?;
    let path = index_path()?;
    let json = serialize_names(&names);
    atomic_write(&path, &json)?;
    Ok(names)
}

/// Return the cached list of theme names. `None` means the user hasn't
/// run `sync` yet — callers should tell them to do so rather than silently
/// reaching out to the network.
pub fn cached_names() -> Result<Option<Vec<String>>> {
    let path = index_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(&path)
        .with_context(|| format!("reading cached index {}", path.display()))?;
    let names = parse_names(&body).with_context(|| {
        format!(
            "cached index {} is corrupted — re-run `colorant themes sync`",
            path.display()
        )
    })?;
    Ok(Some(names))
}

/// Download one Gogh theme by its bare name (e.g. `"Dracula"` or
/// `"3024 Day"`) and parse it into a `ParsedPalette`. The caller decides
/// what to do with the palette (typically: install it into the user's
/// themes dir). The name is percent-encoded for the URL path so themes
/// with spaces/parens etc. work.
pub fn fetch(name: &str) -> Result<ParsedPalette> {
    let url = format!("{RAW_BASE}/{}.yml", percent_encode_path(name));
    let response = ureq::get(&url)
        .timeout(HTTP_TIMEOUT)
        .set("User-Agent", "colorant")
        .call()
        .with_context(|| format!("fetching {url}"))?;
    let body = response.into_string().context("reading gogh yaml body")?;
    parse_gogh_yaml(&body).with_context(|| format!("parsing {url}"))
}

/// Percent-encode the characters in a path segment that need it for an
/// HTTP URL. Conservative — we only special-case the characters that
/// actually appear in Gogh theme names (space, parens). Reserved
/// characters like `/`, `?`, `#` are intentionally left alone: theme names
/// containing them would be rejected at index parse time anyway.
fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(c),
            other => {
                let mut buf = [0u8; 4];
                for byte in other.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    out
}

/// Parse a Gogh `themes/*.yml` payload into a `ParsedPalette`.
///
/// The Gogh format is intentionally simple — flat top-level keys, no
/// nesting, single-line scalar values. We hand-parse it to avoid pulling
/// in a YAML dep for what amounts to a key/value stream.
pub fn parse_gogh_yaml(content: &str) -> Result<ParsedPalette> {
    let mut layer = ThemeLayer::default();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = split_kv(line) else {
            continue;
        };
        // Only color-bearing keys produce side effects; everything else
        // (name, license, author, …) is parsed and ignored.
        match key.as_str() {
            "foreground" => assign(&mut layer.fg, &value),
            "background" => assign(&mut layer.bg, &value),
            "cursor" => assign(&mut layer.cursor, &value),
            k if k.starts_with("color_") => {
                if let Ok(idx) = k[6..].parse::<usize>()
                    && (1..=16).contains(&idx)
                {
                    // Gogh palette slots are 1-indexed (color_01..color_16);
                    // colorant's are 0-indexed.
                    assign(&mut layer.palette[idx - 1], &value);
                }
            }
            _ => {}
        }
    }
    // If we recognized nothing, the upstream file isn't in Gogh format
    // (server returned a redirect HTML page, the schema changed, the
    // theme was renamed). Don't silently install an empty palette —
    // surface it so the user can take action.
    //
    // TODO: a follow-up could route Gogh parser drops through the same
    // DropReason channel used by parse_rc_str_with_diagnostics so
    // `colorant doctor` surfaces specific Gogh issues. For now we error
    // on the "completely empty" case, which is the worst failure mode.
    if layer.fg.is_none()
        && layer.bg.is_none()
        && layer.cursor.is_none()
        && layer.palette.iter().all(Option::is_none)
    {
        return Err(anyhow!(
            "no recognized color keys in Gogh YAML — is the upstream format intact?"
        ));
    }
    Ok(ParsedPalette { layer })
}

/// Try to set `slot` to the color parsed from `value`. Mirrors the rest
/// of the codebase's "drop silently on invalid color" policy — Gogh files
/// are well-formed enough that this almost never fires.
fn assign(slot: &mut Option<HexColor>, value: &str) {
    if let Some(c) = HexColor::parse(value) {
        *slot = Some(c);
    }
}

/// Split `key: value` (Gogh-flavored YAML), stripping surrounding quotes
/// and any trailing `# comment` from the value. Returns `None` for lines
/// without a `:` or empty keys.
///
/// Quoted values: take everything between the opening and matching closing
/// quote (so the trailing comment in `'#abc'  # remark` is discarded).
/// Unquoted values: strip from the first `#` onwards (YAML's comment
/// marker) before trimming whitespace. Gogh's color values are always
/// quoted, so the unquoted path only matters for non-color keys.
fn split_kv(line: &str) -> Option<(String, String)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim().to_ascii_lowercase();
    if key.is_empty() {
        return None;
    }
    let value = line[colon + 1..].trim();
    let value = if let Some(rest) = value.strip_prefix('\'') {
        rest.split_once('\'').map(|(v, _)| v).unwrap_or(rest)
    } else if let Some(rest) = value.strip_prefix('"') {
        rest.split_once('"').map(|(v, _)| v).unwrap_or(rest)
    } else {
        value.split('#').next().unwrap_or("").trim()
    };
    Some((key, value.to_string()))
}

/// Parse a GitHub Git Trees API response and return the list of theme
/// names (filenames directly under `themes/` with the `.yml` extension,
/// sans path and extension). The response is JSON with a `tree` array of
/// objects shaped `{ "path": "themes/foo.yml", "type": "blob", ... }`.
///
/// Avoids a JSON dep — we only need `path` strings, and we look for the
/// literal `"path":` pattern (with the colon, so a value-side `"path"`
/// substring elsewhere in the JSON doesn't false-positive). Subdirectory
/// entries (`themes/sub/foo.yml`) are skipped so we only pick up
/// top-level themes.
fn parse_tree_response(body: &str) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let mut cursor = 0;
    while let Some(off) = body[cursor..].find("\"path\"") {
        let after = cursor + off + "\"path\"".len();
        // Require a `:` (optionally with whitespace) immediately after the
        // key — this is what distinguishes a JSON key from a value that
        // happens to contain the bytes `"path"`. Without this guard, a
        // body like `{"message":"no path found"}` would be misparsed.
        let post_key = body[after..].trim_start();
        if !post_key.starts_with(':') {
            cursor = after;
            continue;
        }
        let colon_off = after + (body[after..].len() - post_key.len());
        let Some(open_rel) = body[colon_off + 1..].find('"') else {
            break;
        };
        let value_start = colon_off + 1 + open_rel + 1;
        let Some(close_rel) = body[value_start..].find('"') else {
            break;
        };
        let value_end = value_start + close_rel;
        let path = &body[value_start..value_end];
        if let Some(stripped) = path
            .strip_prefix("themes/")
            .and_then(|s| s.strip_suffix(".yml"))
            && !stripped.contains('/')
        {
            names.push(stripped.to_string());
        }
        cursor = value_end + 1;
    }
    if names.is_empty() {
        // GitHub surfaces rate-limit and auth errors as
        // `{"message":"API rate limit exceeded ..."}` — bubble that up
        // instead of the generic "no themes found" so the user knows
        // it's not a parser issue.
        if let Some(msg) = extract_json_string_value(body, "message") {
            return Err(anyhow!("GitHub API error: {msg}"));
        }
        return Err(anyhow!(
            "no themes/*.yml entries found in Gogh tree response"
        ));
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// Find a top-level JSON string field `"<key>": "<value>"` and return
/// the value. Used to fish out GitHub's `message` field on error
/// responses without pulling in a JSON parser.
fn extract_json_string_value(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let pos = body.find(&needle)?;
    let after = pos + needle.len();
    let post_key = body[after..].trim_start();
    if !post_key.starts_with(':') {
        return None;
    }
    let colon_off = after + (body[after..].len() - post_key.len());
    let open_rel = body[colon_off + 1..].find('"')?;
    let value_start = colon_off + 1 + open_rel + 1;
    let close_rel = body[value_start..].find('"')?;
    Some(body[value_start..value_start + close_rel].to_string())
}

/// Serialize a `Vec<String>` as a tiny JSON array. Mirrors the parser so
/// we don't need serde_json for one persistent file.
fn serialize_names(names: &[String]) -> String {
    let parts: Vec<String> = names
        .iter()
        .map(|n| format!("\"{}\"", json_escape(n)))
        .collect();
    format!("[{}]\n", parts.join(","))
}

fn parse_names(body: &str) -> Result<Vec<String>> {
    let trimmed = body.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| anyhow!("malformed gogh index: expected JSON array"))?;
    let mut names: Vec<String> = Vec::new();
    let mut in_str = false;
    let mut cur = String::new();
    let mut escaped = false;
    for c in inner.chars() {
        if in_str {
            if escaped {
                cur.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                names.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            in_str = true;
        }
    }
    // A truncated cache (interrupted write, corrupted by external editor)
    // would end mid-string. Reject explicitly so the user gets a clear
    // re-sync prompt instead of a silently-shorter list.
    if in_str {
        return Err(anyhow!(
            "malformed gogh index: unterminated string — re-run `colorant themes sync`"
        ));
    }
    Ok(names)
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "name: Dracula\n\
        foreground: '#f8f8f2'\n\
        background: '#282a36'\n\
        cursor: '#bbbbbb'\n\
        color_01: '#000000'\n\
        color_02: '#ff5555'\n\
        color_03: '#50fa7b'\n\
        color_04: '#f1fa8c'\n\
        color_05: '#bd93f9'\n\
        color_06: '#ff79c6'\n\
        color_07: '#8be9fd'\n\
        color_08: '#bbbbbb'\n\
        color_09: '#555555'\n\
        color_10: '#ff5555'\n\
        color_11: '#50fa7b'\n\
        color_12: '#f1fa8c'\n\
        color_13: '#bd93f9'\n\
        color_14: '#ff79c6'\n\
        color_15: '#8be9fd'\n\
        color_16: '#ffffff'\n";

    #[test]
    fn parse_gogh_yaml_extracts_all_color_slots() {
        let palette = parse_gogh_yaml(SAMPLE).unwrap();
        let layer = &palette.layer;
        assert_eq!(layer.fg, HexColor::parse("#f8f8f2"));
        assert_eq!(layer.bg, HexColor::parse("#282a36"));
        assert_eq!(layer.cursor, HexColor::parse("#bbbbbb"));
        // Gogh 1-indexed → colorant 0-indexed.
        assert_eq!(layer.palette[0], HexColor::parse("#000000"));
        assert_eq!(layer.palette[15], HexColor::parse("#ffffff"));
    }

    #[test]
    fn parse_gogh_yaml_handles_double_and_single_quoted_values() {
        let content = "foreground: \"#abcdef\"\nbackground: '#001122'\n";
        let palette = parse_gogh_yaml(content).unwrap();
        assert_eq!(palette.layer.fg, HexColor::parse("#abcdef"));
        assert_eq!(palette.layer.bg, HexColor::parse("#001122"));
    }

    #[test]
    fn parse_gogh_yaml_strips_trailing_yaml_comments() {
        // The real Gogh format puts a comment after every color value, e.g.
        // `color_01: '#363636'    # Black (Host)`. Earlier versions of
        // `split_kv` left the comment in the value, `HexColor::parse`
        // rejected it, every color slot stayed None, and the empty-palette
        // guard fired — so every Gogh preview failed with "preview
        // unavailable". Lock down the parse so that regression can't
        // sneak back.
        let content = "\
            foreground: '#abcdef'    # Foreground (Text)\n\
            background: '#001122'    # Background\n\
            cursor: \"#bbbbbb\"     # Cursor\n\
            color_01: '#363636'    # Black (Host)\n\
        ";
        let palette = parse_gogh_yaml(content).unwrap();
        assert_eq!(palette.layer.fg, HexColor::parse("#abcdef"));
        assert_eq!(palette.layer.bg, HexColor::parse("#001122"));
        assert_eq!(palette.layer.cursor, HexColor::parse("#bbbbbb"));
        assert_eq!(palette.layer.palette[0], HexColor::parse("#363636"));
    }

    #[test]
    fn parse_gogh_yaml_ignores_unknown_top_level_keys() {
        let content = "name: Whatever\nlicense: MIT\nfg: nonstandard\nforeground: '#abcdef'\n";
        let palette = parse_gogh_yaml(content).unwrap();
        assert_eq!(palette.layer.fg, HexColor::parse("#abcdef"));
        // `fg: nonstandard` doesn't match colorant's key — Gogh uses
        // `foreground`, not `fg`. We intentionally don't map between the
        // two; the canonical Gogh key wins.
    }

    #[test]
    fn parse_gogh_yaml_skips_blank_and_comment_lines() {
        let content = "# comment\n\nforeground: '#abcdef'\n";
        let palette = parse_gogh_yaml(content).unwrap();
        assert_eq!(palette.layer.fg, HexColor::parse("#abcdef"));
    }

    #[test]
    fn parse_gogh_yaml_skips_out_of_range_color_indices() {
        let content = "color_00: '#000000'\ncolor_17: '#000000'\ncolor_01: '#abcdef'\n";
        let palette = parse_gogh_yaml(content).unwrap();
        // Only the in-range one (color_01 → palette[0]) lands.
        assert_eq!(palette.layer.palette[0], HexColor::parse("#abcdef"));
    }

    #[test]
    fn parse_tree_response_filters_to_themes_yaml() {
        let body = r#"{"tree":[
            {"path":"README.md","type":"blob"},
            {"path":"themes/Dracula.yml","type":"blob"},
            {"path":"themes/Nord.yml","type":"blob"},
            {"path":"themes/subdir/Foo.yml","type":"blob"},
            {"path":"themes/Foo.json","type":"blob"},
            {"path":"themes","type":"tree"}
        ]}"#;
        let names = parse_tree_response(body).unwrap();
        assert_eq!(names, vec!["Dracula".to_string(), "Nord".to_string()]);
    }

    #[test]
    fn parse_tree_response_errors_when_empty() {
        let body = r#"{"tree":[{"path":"README.md","type":"blob"}]}"#;
        assert!(parse_tree_response(body).is_err());
    }

    #[test]
    fn names_round_trip_through_serialize_and_parse() {
        let names = vec![
            "Dracula".to_string(),
            "Nord".to_string(),
            "Tokyo Night".to_string(),
        ];
        let json = serialize_names(&names);
        let back = parse_names(&json).unwrap();
        assert_eq!(back, names);
    }

    #[test]
    fn names_round_trip_escapes_quotes() {
        let names = vec!["weird\"name".to_string()];
        let json = serialize_names(&names);
        let back = parse_names(&json).unwrap();
        assert_eq!(back, names);
    }

    #[test]
    fn parse_names_rejects_truncated_input() {
        // Cache cut mid-string: parser should error rather than silently
        // returning the names that did parse — otherwise the user sees a
        // mysteriously short list.
        let body = "[\"a\",\"b";
        assert!(parse_names(body).is_err());
    }

    #[test]
    fn parse_tree_response_recognizes_github_message_error() {
        // GitHub returns this shape on rate limit / auth failure — no
        // `"path"` keys, just a `"message"` field. We should surface the
        // server message instead of the generic "no themes found".
        let body = r#"{"message":"API rate limit exceeded for ...","documentation_url":"..."}"#;
        let err = parse_tree_response(body).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("GitHub API error"), "{msg}");
        assert!(msg.contains("rate limit"), "{msg}");
    }

    #[test]
    fn parse_tree_response_ignores_path_substring_in_values() {
        // Decoy: `"description"` contains the bytes `"path"`. Without the
        // `"path":` (colon) guard, this would slurp the description text
        // as a theme path.
        let body =
            r#"{"description":"see path docs","tree":[{"path":"themes/Foo.yml","type":"blob"}]}"#;
        let names = parse_tree_response(body).unwrap();
        assert_eq!(names, vec!["Foo".to_string()]);
    }

    #[test]
    fn parse_gogh_yaml_errors_when_no_color_keys() {
        // Upstream redirect / 200-with-html / unrelated YAML: surface
        // instead of silently writing an empty palette.
        let body = "name: Whatever\nlicense: MIT\nauthor: Someone\n";
        assert!(parse_gogh_yaml(body).is_err());
    }

    #[test]
    fn percent_encode_path_handles_spaces_and_parens() {
        assert_eq!(percent_encode_path("3024 Day"), "3024%20Day");
        assert_eq!(
            percent_encode_path("Flatland (Palenight)"),
            "Flatland%20%28Palenight%29"
        );
        // Unreserved characters pass through.
        assert_eq!(percent_encode_path("Dracula"), "Dracula");
        assert_eq!(percent_encode_path("tokyo-night-day"), "tokyo-night-day");
    }

    #[test]
    fn split_kv_unquoted_hash_starts_comment() {
        // Documented limitation: in an unquoted value, '#' is treated
        // as YAML's comment marker, so an unquoted hex literal would
        // be parsed as empty. Gogh always quotes its color values so
        // this only affects non-color keys we ignore anyway — but if
        // upstream ever stops quoting, every color slot will silently
        // become empty and parse_gogh_yaml will error with "no
        // recognized color keys". Test locks down the current behavior
        // so the surprise can't sneak in unannounced.
        assert_eq!(
            split_kv("foreground: #abcdef"),
            Some(("foreground".to_string(), "".to_string()))
        );
    }

    #[test]
    fn split_kv_returns_none_for_lines_without_colon() {
        assert_eq!(split_kv("just-a-bare-line"), None);
        assert_eq!(split_kv(""), None);
    }
}
