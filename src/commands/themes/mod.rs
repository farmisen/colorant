//! `colorant themes` — manage themes from bundled and remote sources.
//!
//! With no sub-action, opens the interactive TUI in [`tui`]. With one,
//! dispatches to the corresponding non-interactive operation:
//! - `list` (with `--source` / `--installed`): show available themes.
//! - `install` (bundled only): copy a bundled palette onto disk. Kept for
//!   bulk first-time setup (`--all`); single-name installs are usually
//!   better done via `apply` which writes the rc too.
//! - `path`: print `base_theme_dir`.
//! - `search`: substring match across known sources.
//! - `sync`: refresh remote-source catalog caches (Gogh).
//! - `apply`: write `extends*` keys to the cwd's `.colorantrc`,
//!   auto-installing any required palette (bundled or remote).

pub mod tui;

use crate::cli::ThemesAction;
use crate::config::Config;
use crate::theme::bundled::BUNDLED_THEMES;
use crate::theme::model::ParsedPalette;
use crate::theme::rc::rewrite_extends;
use crate::theme::resolve::PALETTE_EXTENSION;
use crate::theme::source::{self, Source};
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const RC_FILE_NAME: &str = ".colorantrc";

pub fn run(config: &Config, action: Option<ThemesAction>) -> Result<()> {
    match action {
        None => tui::run(config),
        Some(ThemesAction::List { source, installed }) => run_list(config, source, installed),
        Some(ThemesAction::Install { name, all, force }) => run_install(config, name, all, force),
        Some(ThemesAction::Path) => run_path(config),
        Some(ThemesAction::Search { query, source }) => run_search(config, &query, source),
        Some(ThemesAction::Sync { source }) => run_sync(source),
        Some(ThemesAction::Apply { name, dark, light }) => run_apply(config, name, dark, light),
    }
}

fn run_list(config: &Config, source_filter: Option<String>, installed_only: bool) -> Result<()> {
    let mut out = io::stdout().lock();
    let sources = resolve_sources(source_filter)?;
    for source in sources {
        let names = match source.list() {
            Ok(n) => n,
            // A remote source that isn't synced shouldn't kill the whole
            // list — surface the issue (to stderr so it's pipe-friendly)
            // and move on to the next source.
            Err(e) => {
                eprintln!("warning: [{source}] {e:#}");
                continue;
            }
        };
        for name in names {
            let is_installed = dest_path(config, &name).is_file();
            if installed_only && !is_installed {
                continue;
            }
            let marker = if is_installed { " (installed)" } else { "" };
            writeln!(out, "[{source}] {name}{marker}")?;
        }
    }
    Ok(())
}

fn run_install(config: &Config, name: Option<String>, all: bool, force: bool) -> Result<()> {
    match (name, all) {
        (None, false) => Err(anyhow!(
            "specify a palette name or pass --all to install every bundled palette"
        )),
        (Some(name), false) => install_one(config, &name, force),
        (None, true) => install_all(config, force),
        // clap's `conflicts_with` on `--all` rejects this combination before
        // we reach here, but we surface a clear error in case clap is bypassed.
        (Some(_), true) => Err(anyhow!("--all conflicts with a named palette")),
    }
}

fn install_one(config: &Config, name: &str, force: bool) -> Result<()> {
    let content = BUNDLED_THEMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, c)| *c)
        .ok_or_else(|| anyhow!("no bundled palette named {name:?}"))?;

    ensure_themes_dir(config)?;
    let dest = dest_path(config, name);
    write_palette(&dest, content, force)?;
    println!("installed {}", dest.display());
    Ok(())
}

fn install_all(config: &Config, force: bool) -> Result<()> {
    ensure_themes_dir(config)?;
    // --all is a batch op: existing files are skipped gracefully when
    // `force` is unset. (`install_one` instead errors so a user typing a
    // specific name hears about the conflict.)
    let mut installed = 0;
    let mut overwritten = 0;
    let mut skipped = 0;
    for (name, content) in BUNDLED_THEMES {
        let dest = dest_path(config, name);
        if dest.exists() {
            if !force {
                println!("skipping {} (already installed)", dest.display());
                skipped += 1;
                continue;
            }
            overwritten += 1;
        }
        write_palette(&dest, content, force)?;
        installed += 1;
    }
    println!("installed {installed} themes ({overwritten} overwritten, {skipped} skipped)");
    Ok(())
}

fn run_path(config: &Config) -> Result<()> {
    println!("{}", config.base_theme_dir.display());
    Ok(())
}

fn run_search(config: &Config, query: &str, source_filter: Option<String>) -> Result<()> {
    let needle = query.to_lowercase();
    let mut out = io::stdout().lock();
    let mut any_hits = false;
    for source in resolve_sources(source_filter)? {
        let names = match source.list() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("warning: [{source}] {e:#}");
                continue;
            }
        };
        for name in names
            .into_iter()
            .filter(|n| n.to_lowercase().contains(&needle))
        {
            let marker = if dest_path(config, &name).is_file() {
                " (installed)"
            } else {
                ""
            };
            writeln!(out, "[{source}] {name}{marker}")?;
            any_hits = true;
        }
    }
    if !any_hits {
        writeln!(out, "no themes matched {query:?}")?;
    }
    Ok(())
}

fn run_sync(source_filter: Option<String>) -> Result<()> {
    let sources = resolve_sources(source_filter)?;
    let mut failed = 0usize;
    for source in sources {
        // Bundled sources are no-ops; skip the noise instead of printing
        // "synced bundled" or similar.
        if matches!(source, Source::Bundled) {
            continue;
        }
        print!("syncing {source}... ");
        io::stdout().flush().ok();
        match source.sync() {
            Ok(()) => println!("ok"),
            Err(e) => {
                println!("failed");
                eprintln!("warning: [{source}] {e:#}");
                failed += 1;
            }
        }
    }
    // Exit non-zero on any failure so `themes sync && themes apply ...`
    // does the right thing in scripts.
    if failed > 0 {
        return Err(anyhow!(
            "{failed} source(s) failed to sync — see warnings above"
        ));
    }
    Ok(())
}

fn run_apply(
    config: &Config,
    name: Option<String>,
    dark: Option<String>,
    light: Option<String>,
) -> Result<()> {
    if name.is_none() && dark.is_none() && light.is_none() {
        return Err(anyhow!(
            "pass a theme name (applies to both modes) or --dark / --light"
        ));
    }

    let install = |spec: &str| -> Result<String> {
        let (source_hint, theme_name) = source::parse_ref(spec);
        install_for_apply(config, source_hint, theme_name)
    };
    let both = name.as_deref().map(install).transpose()?;
    let dark_name = dark.as_deref().map(install).transpose()?;
    let light_name = light.as_deref().map(install).transpose()?;

    let rc_path = std::env::current_dir()?.join(RC_FILE_NAME);
    let existing = match fs::read_to_string(&rc_path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", rc_path.display()));
        }
    };
    let rewritten = rewrite_extends(
        &existing,
        both.as_deref(),
        dark_name.as_deref(),
        light_name.as_deref(),
    );
    fs::write(&rc_path, rewritten).with_context(|| format!("writing {}", rc_path.display()))?;

    println!("Updated {}", rc_path.display());
    if let Some(n) = both {
        println!("  extends = {n}");
    }
    if let Some(n) = dark_name {
        println!("  extends.dark = {n}");
    }
    if let Some(n) = light_name {
        println!("  extends.light = {n}");
    }
    Ok(())
}

/// Look up `spec` and install it locally if needed. Returns the bare theme
/// name (without any `source:` prefix) for writing into the rc. The lookup
/// order when no explicit source prefix is given is: installed → bundled.
/// To install from a remote source the user must say so explicitly (e.g.
/// `gogh:Dracula`) — implicit remote fetches would be a network surprise.
fn install_for_apply(config: &Config, source_hint: Option<Source>, name: &str) -> Result<String> {
    let dest = dest_path(config, name);

    if let Some(source) = source_hint {
        // Explicit source prefix — always fetch and (re)install so the
        // disk version matches the chosen source, even if a same-named
        // file already exists. Warn the user when we're about to clobber
        // a file they may have hand-edited.
        if dest.is_file() {
            eprintln!(
                "warning: overwriting {} with the {source} version",
                dest.display()
            );
        }
        let palette = source.fetch(name)?;
        write_parsed_palette(config, name, &palette)?;
        return Ok(name.to_string());
    }

    if dest.is_file() {
        return Ok(name.to_string());
    }

    if let Some((_, content)) = BUNDLED_THEMES.iter().find(|(n, _)| *n == name) {
        ensure_themes_dir(config)?;
        write_palette(&dest, content, false)?;
        return Ok(name.to_string());
    }

    Err(anyhow!(
        "no theme named {name:?} installed or bundled — try `colorant themes search {name}` \
         or qualify with a source prefix (e.g. `gogh:{name}`)"
    ))
}

/// Materialize a `ParsedPalette` back as a `.colorant` file under
/// `base_theme_dir`. We emit the same flat key/value format the local
/// parser already understands, so a roundtrip is lossless.
fn write_parsed_palette(config: &Config, name: &str, palette: &ParsedPalette) -> Result<()> {
    ensure_themes_dir(config)?;
    let dest = dest_path(config, name);
    let content = render_palette(palette);
    // Atomic: stage to <dest>.tmp then rename, so an interrupted write
    // doesn't leave a half-written palette that future commands treat as
    // authoritative.
    let tmp = dest.with_extension(format!("{PALETTE_EXTENSION}.tmp"));
    fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &dest)
        .with_context(|| format!("renaming {} to {}", tmp.display(), dest.display()))?;
    Ok(())
}

fn render_palette(palette: &ParsedPalette) -> String {
    let mut out = String::new();
    if let Some(c) = &palette.layer.fg {
        out.push_str(&format!("fg = {}\n", c.as_str()));
    }
    if let Some(c) = &palette.layer.bg {
        out.push_str(&format!("bg = {}\n", c.as_str()));
    }
    if let Some(c) = &palette.layer.cursor {
        out.push_str(&format!("cursor = {}\n", c.as_str()));
    }
    for (i, slot) in palette.layer.palette.iter().enumerate() {
        if let Some(c) = slot {
            out.push_str(&format!("color{i} = {}\n", c.as_str()));
        }
    }
    out
}

fn resolve_sources(source_filter: Option<String>) -> Result<Vec<Source>> {
    match source_filter {
        Some(name) => Source::parse(&name).map(|s| vec![s]).ok_or_else(|| {
            anyhow!(
                "unknown source {name:?}; known sources: {}",
                Source::all()
                    .iter()
                    .map(|s| s.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }),
        None => Ok(Source::all().to_vec()),
    }
}

fn dest_path(config: &Config, name: &str) -> PathBuf {
    config
        .base_theme_dir
        .join(format!("{name}.{PALETTE_EXTENSION}"))
}

fn ensure_themes_dir(config: &Config) -> Result<()> {
    fs::create_dir_all(&config.base_theme_dir).with_context(|| {
        format!(
            "creating themes directory {}",
            config.base_theme_dir.display()
        )
    })
}

/// Write `content` to `dest`. Errors when `dest` already exists and `force`
/// is `false`; overwrites silently when `force` is `true`. Callers that want
/// graceful skipping for existing files (e.g. `install_all`) should check
/// `dest.exists()` themselves before invoking this.
fn write_palette(dest: &Path, content: &str, force: bool) -> Result<()> {
    if dest.exists() && !force {
        return Err(anyhow!(
            "{} already exists; pass --force to overwrite",
            dest.display()
        ));
    }
    fs::write(dest, content).with_context(|| format!("writing {}", dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::model::HexColor;

    #[test]
    fn render_palette_emits_set_keys_only() {
        let mut palette = ParsedPalette::default();
        palette.layer.fg = HexColor::parse("#abcdef");
        palette.layer.palette[0] = HexColor::parse("#001122");
        let out = render_palette(&palette);
        assert_eq!(out, "fg = #abcdef\ncolor0 = #001122\n");
    }
}
