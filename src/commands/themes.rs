//! `colorant themes <list|install|path>` — manage bundled palettes.
//!
//! Bundled palettes are compiled into the binary at build time (see
//! `theme::bundled`). The `install` sub-action copies them onto disk under
//! `Config::base_theme_dir` so the resolver picks them up like any other
//! palette file.

use crate::cli::ThemesAction;
use crate::config::Config;
use crate::theme::bundled::BUNDLED_THEMES;
use crate::theme::resolve::PALETTE_EXTENSION;
use anyhow::{Context, Result, anyhow};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

pub fn run(config: &Config, action: ThemesAction) -> Result<()> {
    match action {
        ThemesAction::List => run_list(config),
        ThemesAction::Install { name, all, force } => run_install(config, name, all, force),
        ThemesAction::Path => run_path(config),
    }
}

fn run_list(config: &Config) -> Result<()> {
    let mut out = io::stdout().lock();
    for (name, _) in BUNDLED_THEMES {
        let marker = if dest_path(config, name).is_file() {
            " (installed)"
        } else {
            ""
        };
        writeln!(out, "{name}{marker}")?;
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
fn write_palette(dest: &std::path::Path, content: &str, force: bool) -> Result<()> {
    if dest.exists() && !force {
        return Err(anyhow!(
            "{} already exists; pass --force to overwrite",
            dest.display()
        ));
    }
    fs::write(dest, content).with_context(|| format!("writing {}", dest.display()))
}
