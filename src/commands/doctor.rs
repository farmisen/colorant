//! `colorant doctor` — diagnose silent failures in colorant config files.
//!
//! Re-parses the target `.colorantrc` with the diagnostics-collecting
//! parser so users can see every line that was silently dropped: unknown
//! keys, invalid colors, malformed lines, unknown sections, invalid theme
//! names. For each mode, checks whether the chosen `extends` palette file
//! exists on disk under `base_theme_dir` — and, if it does, parses the
//! palette with the same diagnostics so typos in the user's themes dir
//! aren't hidden either.
//!
//! Exits 0 if nothing is wrong, 1 otherwise. The report goes to stdout in
//! either case so it's safe to pipe. Colors are auto-disabled when stdout
//! isn't a TTY or when `NO_COLOR` is set.

use crate::config::{Config, THEME_FILE_NAME};
use crate::terminal::style::Style;
use crate::theme::model::{Mode, ParsedRc};
use crate::theme::parse::{
    DropReason, parse_palette_str_with_diagnostics, parse_rc_str_with_diagnostics,
};
use crate::theme::resolve::PALETTE_EXTENSION;
use crate::walk;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Entry point routed from `main.rs`. Returns `ExitCode::FAILURE` if any
/// issue was reported so shell users can gate on it
/// (`colorant doctor && ...`). Returning rather than calling `exit` lets the
/// runtime flush stdout on the way out — otherwise piped output (e.g.
/// `colorant doctor | tee log`) can truncate the final lines.
pub fn run(config: &Config, explicit_path: Option<PathBuf>) -> Result<ExitCode> {
    let style = Style::detect();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let issues = check(config, explicit_path, &mut out, style)?;
    Ok(if issues == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Run the checks against `path` (or the walked-up rc) and write a report
/// to `out`. Returns the number of issues so callers can decide on exit
/// code; broken out from `run` to keep the writeln/Result plumbing
/// separate from the process-level concerns (stdout lock, exit code).
fn check<W: Write>(
    config: &Config,
    explicit_path: Option<PathBuf>,
    out: &mut W,
    style: Style,
) -> Result<usize> {
    let path = match explicit_path {
        Some(p) => p,
        None => {
            let cwd = std::env::current_dir()?;
            match walk::find_nearest(&cwd, THEME_FILE_NAME) {
                Some(p) => p,
                None => {
                    writeln!(
                        out,
                        "No {} found while walking up from {}",
                        THEME_FILE_NAME,
                        cwd.display()
                    )?;
                    return Ok(1);
                }
            }
        }
    };

    let content =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let (rc, diags) = parse_rc_str_with_diagnostics(&content);

    writeln!(out, "Checking {}", path.display())?;
    writeln!(out)?;

    let mut issues = 0usize;
    write_diagnostics(out, &diags, "  ", style)?;
    issues += diags.len();

    writeln!(out)?;
    writeln!(out, "  Palette(s):")?;
    // Only Dark and Light are audited. Mode::Unknown only fires when OS
    // detection fails (non-macOS today). Add a third row once Linux support
    // lands and Unknown becomes a path real users can hit.
    let mut audited_palettes: HashSet<PathBuf> = HashSet::new();
    issues += report_mode(
        &rc,
        Mode::Dark,
        "dark ",
        config,
        out,
        &mut audited_palettes,
        style,
    )?;
    issues += report_mode(
        &rc,
        Mode::Light,
        "light",
        config,
        out,
        &mut audited_palettes,
        style,
    )?;

    writeln!(out)?;
    // Indent the summary to line up with the "No parsing errors." /
    // "Parsing errors:" headers above.
    if issues == 0 {
        writeln!(out, "  {}", style.green("No issues found."))?;
    } else {
        let summary = format!(
            "Found {issues} issue{}.",
            if issues == 1 { "" } else { "s" }
        );
        writeln!(out, "  {}", style.red(&summary))?;
    }
    Ok(issues)
}

/// Print a `Parsing errors:` block under `indent` if `diags` is non-empty,
/// otherwise a `No parsing errors.` confirmation. The indent is applied to
/// the header line; individual issues are indented one extra step.
fn write_diagnostics<W: Write>(
    out: &mut W,
    diags: &[(usize, DropReason)],
    indent: &str,
    style: Style,
) -> Result<()> {
    if diags.is_empty() {
        writeln!(out, "{indent}{}", style.green("No parsing errors."))?;
        return Ok(());
    }
    writeln!(out, "{indent}Parsing errors:")?;
    for (line, reason) in diags {
        let msg = format!("line {line}: {}", format_reason(reason));
        writeln!(out, "{indent}  {}", style.red(&msg))?;
    }
    Ok(())
}

/// Returns the number of issues for this mode: 1 if the palette file is
/// missing, otherwise the count of palette parsing errors. "No parent
/// palette" is not an issue — it's a valid choice for rcs that only set
/// their own keys. Palettes already inspected (by an earlier mode pointing
/// at the same file) are not re-audited; their drops were counted then.
fn report_mode<W: Write>(
    rc: &ParsedRc,
    mode: Mode,
    label: &str,
    config: &Config,
    out: &mut W,
    audited_palettes: &mut HashSet<PathBuf>,
    style: Style,
) -> Result<usize> {
    let Some(name) = rc.parent_for(mode) else {
        writeln!(out, "    {label}: no parent palette (rc's own keys only)")?;
        return Ok(0);
    };
    let palette_path =
        config
            .base_theme_dir
            .join(format!("{}.{}", name.as_str(), PALETTE_EXTENSION));
    if !palette_path.exists() {
        writeln!(
            out,
            "    {label}: extends {name} -> {} {}",
            palette_path.display(),
            style.red("(NOT FOUND)")
        )?;
        return Ok(1);
    }
    writeln!(
        out,
        "    {label}: extends {name} -> {}",
        palette_path.display()
    )?;
    if !audited_palettes.insert(palette_path.clone()) {
        // Same palette already audited via the other mode (e.g. global
        // `extends`); skip re-parsing but keep the report symmetric so the
        // user isn't left wondering whether this row was checked.
        writeln!(out, "      (same palette as above; not re-audited)")?;
        return Ok(0);
    }
    audit_palette(&palette_path, out, style)
}

/// Read and parse the palette file, indent-listing any drops under the
/// resolution line. Returns the number of palette-level drops. A read
/// failure is reported as a single issue rather than aborting doctor — the
/// rc-level diagnostics are still useful even if a palette read failed.
fn audit_palette<W: Write>(path: &Path, out: &mut W, style: Style) -> Result<usize> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            // Echo the path: io::Error's Display doesn't include it, so
            // grep-ing the report later still tells you which file failed.
            let msg = format!("Could not read palette {}: {e}", path.display());
            writeln!(out, "      {}", style.red(&msg))?;
            return Ok(1);
        }
    };
    let (_palette, diags) = parse_palette_str_with_diagnostics(&content);
    // Always emit the per-palette confirmation — matches the rc block's
    // "No parsing errors." line so the user gets a clear yes/no for each
    // file colorant would load.
    write_diagnostics(out, &diags, "      ", style)?;
    Ok(diags.len())
}

fn format_reason(r: &DropReason) -> String {
    match r {
        // The parser hits this branch either when the line has no '=' or
        // when the key half is empty (e.g. `= value`), so neither "missing
        // '='" nor "empty key" alone is accurate. Stay neutral.
        DropReason::MalformedLine => "malformed line (expected 'key = value')".to_string(),
        DropReason::UnknownSection(name) => format!("unknown section [{name}]"),
        DropReason::UnknownKey(key) => format!("unknown key '{key}'"),
        DropReason::InvalidColor { key, value } => {
            format!("invalid color '{value}' for key '{key}' (expected #rrggbb)")
        }
        DropReason::InvalidExtendsName { key, value, error } => {
            format!("invalid theme name '{value}' for key '{key}': {error}")
        }
    }
}
