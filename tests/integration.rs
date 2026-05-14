//! End-to-end tests driving the binary as a subprocess. We don't expose a
//! library target, so the tests exercise the full CLI surface.

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_colorant")
}

fn run_in(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> (String, String, i32) {
    let mut cmd = Command::new(binary());
    cmd.current_dir(cwd).args(args);
    // Strip every env var colorant looks at by default, so tests don't pick
    // up the developer's real config, mode, or terminal. Tests that care set
    // these explicitly via `envs`.
    for var in [
        "COLORANT_MODE",
        "TMUX",
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "TERM_PROGRAM",
        "TERM",
        "NO_COLOR",
    ] {
        cmd.env_remove(var);
    }
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("running colorant");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn make_workspace() -> TempDir {
    tempfile::tempdir().expect("creating tempdir")
}

/// Set up an XDG_CONFIG_HOME with a colorant/ dir and a themes/ subdir.
/// Returns (xdg_root, themes_dir).
fn setup_xdg(ws: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let xdg = ws.path().join("xdg");
    let themes = xdg.join("colorant").join("themes");
    fs::create_dir_all(&themes).unwrap();
    (xdg, themes)
}

#[test]
fn current_finds_nearest_rc() {
    let ws = make_workspace();
    let nested = ws.path().join("a/b/c");
    fs::create_dir_all(&nested).unwrap();
    fs::write(ws.path().join(".colorantrc"), "fg = #abcdef\n").unwrap();

    let (stdout, _, code) = run_in(&nested, &["current"], &[]);
    assert_eq!(code, 0);
    assert!(
        stdout.trim().ends_with(".colorantrc"),
        "stdout was: {stdout:?}"
    );
}

#[test]
fn current_prints_nothing_when_missing() {
    let ws = make_workspace();
    let nested = ws.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();
    let (stdout, _, code) = run_in(&nested, &["current"], &[]);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "");
}

#[test]
fn init_zsh_emits_hook() {
    let ws = make_workspace();
    let (stdout, _, code) = run_in(ws.path(), &["init", "zsh"], &[]);
    assert_eq!(code, 0);
    assert!(stdout.contains("add-zsh-hook chpwd"));
    assert!(stdout.contains("add-zsh-hook precmd"));
    assert!(stdout.contains("# >>> colorant init >>>"));
}

#[test]
fn apply_no_op_on_unsupported_terminal() {
    let ws = make_workspace();
    fs::write(ws.path().join(".colorantrc"), "fg = #abcdef\n").unwrap();
    let (stdout, stderr, code) =
        run_in(ws.path(), &["apply"], &[("TERM_PROGRAM", "Apple_Terminal")]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "");
}

#[test]
fn apply_emits_osc_in_ghostty() {
    let ws = make_workspace();
    fs::write(
        ws.path().join(".colorantrc"),
        "fg = #abcdef\nbg = #112233\n",
    )
    .unwrap();

    let (stdout, stderr, code) = run_in(
        ws.path(),
        &["apply"],
        &[("TERM_PROGRAM", "ghostty"), ("COLORANT_MODE", "dark")],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("\x1b]10;#abcdef\x07"),
        "missing fg OSC, got: {stdout:?}"
    );
    assert!(
        stdout.contains("\x1b]11;#112233\x07"),
        "missing bg OSC, got: {stdout:?}"
    );
}

#[test]
fn apply_wraps_osc_in_tmux_dcs_when_tmux_is_set() {
    // Inside tmux, every OSC sequence must be wrapped in DCS passthrough so
    // it actually reaches the outer terminal. The unit test in
    // src/terminal/osc.rs only exercises the helper with a hardcoded payload;
    // this test drives the full `emit()` path with the runtime env check.
    let ws = make_workspace();
    fs::write(ws.path().join(".colorantrc"), "fg = #abcdef\n").unwrap();

    let (stdout, stderr, code) = run_in(
        ws.path(),
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "dark"),
            ("TMUX", "/tmp/tmux-fake,0,0"),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    // DCS passthrough envelope: ESC P tmux ; ESC ESC ] payload BEL ESC \
    assert!(
        stdout.contains("\x1bPtmux;\x1b\x1b]10;#abcdef\x07\x1b\\"),
        "expected DCS-wrapped fg OSC, got: {stdout:?}"
    );
}

#[test]
fn apply_respects_dark_section_override() {
    let ws = make_workspace();
    fs::write(
        ws.path().join(".colorantrc"),
        "fg = #aaaaaa\n[dark]\nfg = #111111\n[light]\nfg = #eeeeee\n",
    )
    .unwrap();

    let (stdout, _, _) = run_in(
        ws.path(),
        &["apply"],
        &[("TERM_PROGRAM", "ghostty"), ("COLORANT_MODE", "dark")],
    );
    assert!(
        stdout.contains("\x1b]10;#111111\x07"),
        "expected dark fg override, got: {stdout:?}"
    );

    let (stdout, _, _) = run_in(
        ws.path(),
        &["apply"],
        &[("TERM_PROGRAM", "ghostty"), ("COLORANT_MODE", "light")],
    );
    assert!(
        stdout.contains("\x1b]10;#eeeeee\x07"),
        "expected light fg override, got: {stdout:?}"
    );
}

#[test]
fn child_top_level_beats_extended_palette() {
    // The rc's own top-level keys must beat anything pulled in via `extends`.
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);

    fs::write(
        themes_dir.join("parent.colorant"),
        "fg = #ff0000\nbg = #ff0000\n",
    )
    .unwrap();

    let work_dir = ws.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(
        work_dir.join(".colorantrc"),
        "extends = parent\nbg = #00ff00\n",
    )
    .unwrap();

    let (stdout, stderr, code) = run_in(
        &work_dir,
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "dark"),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("\x1b]10;#ff0000\x07"),
        "expected inherited fg, got: {stdout:?}"
    );
    assert!(
        stdout.contains("\x1b]11;#00ff00\x07"),
        "expected child top-level bg to win, got: {stdout:?}"
    );
}

#[test]
fn per_mode_extends_pick_different_palettes() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);

    fs::write(
        themes_dir.join("nightly.colorant"),
        "fg = #111111\nbg = #000000\n",
    )
    .unwrap();
    fs::write(
        themes_dir.join("daily.colorant"),
        "fg = #eeeeee\nbg = #ffffff\n",
    )
    .unwrap();

    let work_dir = ws.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(
        work_dir.join(".colorantrc"),
        "extends.dark = nightly\nextends.light = daily\n",
    )
    .unwrap();

    // Dark mode → nightly palette.
    let (stdout, _, _) = run_in(
        &work_dir,
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "dark"),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ],
    );
    assert!(
        stdout.contains("\x1b]10;#111111\x07") && stdout.contains("\x1b]11;#000000\x07"),
        "dark mode should use nightly: {stdout:?}"
    );

    // Light mode → daily palette.
    let (stdout, _, _) = run_in(
        &work_dir,
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "light"),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ],
    );
    assert!(
        stdout.contains("\x1b]10;#eeeeee\x07") && stdout.contains("\x1b]11;#ffffff\x07"),
        "light mode should use daily: {stdout:?}"
    );
}

#[test]
fn per_mode_extends_overrides_global_extends_in_that_mode() {
    // Top-level `extends = base`, plus `extends.dark = override`. In dark mode
    // we get `override`; in light mode we get `base`.
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);

    fs::write(themes_dir.join("base.colorant"), "fg = #aaaaaa\n").unwrap();
    fs::write(themes_dir.join("override.colorant"), "fg = #bbbbbb\n").unwrap();

    let work_dir = ws.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(
        work_dir.join(".colorantrc"),
        "extends = base\nextends.dark = override\n",
    )
    .unwrap();

    let (stdout, _, _) = run_in(
        &work_dir,
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "dark"),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ],
    );
    assert!(
        stdout.contains("\x1b]10;#bbbbbb\x07"),
        "dark should pick `override`: {stdout:?}"
    );

    let (stdout, _, _) = run_in(
        &work_dir,
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "light"),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ],
    );
    assert!(
        stdout.contains("\x1b]10;#aaaaaa\x07"),
        "light should fall through to `base`: {stdout:?}"
    );
}

#[test]
fn dark_only_palette_in_light_mode_emits_reset() {
    // The user picked a dark-only theme (no extends.light, no top-level
    // extends, no own keys). In light mode there's nothing to apply, so we
    // emit the OSC reset rather than leaving stale colors.
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);

    fs::write(
        themes_dir.join("nightly.colorant"),
        "fg = #111111\nbg = #000000\n",
    )
    .unwrap();

    let work_dir = ws.path().join("work");
    fs::create_dir_all(&work_dir).unwrap();
    fs::write(work_dir.join(".colorantrc"), "extends.dark = nightly\n").unwrap();

    let (stdout, _, _) = run_in(
        &work_dir,
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "light"),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ],
    );
    // OSC 110/111/112 = reset fg/bg/cursor; absence of any "10;#..." setter.
    assert!(
        stdout.contains("\x1b]110\x07")
            && stdout.contains("\x1b]111\x07")
            && stdout.contains("\x1b]112\x07"),
        "expected reset OSCs, got: {stdout:?}"
    );
    assert!(
        !stdout.contains("\x1b]10;#"),
        "should not have emitted any fg setter: {stdout:?}"
    );
}

#[test]
fn default_theme_applies_when_no_rc_found() {
    // No .colorantrc in the parent chain, but a `default_theme` in
    // config.toml — apply that palette directly.
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);

    fs::write(
        themes_dir.join("fallback.colorant"),
        "fg = #abcdef\nbg = #123456\n",
    )
    .unwrap();
    let cfg_path = xdg.join("colorant").join("config.toml");
    fs::write(&cfg_path, "default_theme = \"fallback\"\n").unwrap();

    // Operate from a dir guaranteed to have no .colorantrc above it within
    // the tempdir. (The tempdir root is below /tmp on macOS/Linux, so no
    // user-level rc will pollute this run.)
    let nested = ws.path().join("nowhere");
    fs::create_dir_all(&nested).unwrap();

    let (stdout, stderr, code) = run_in(
        &nested,
        &["apply"],
        &[
            ("TERM_PROGRAM", "ghostty"),
            ("COLORANT_MODE", "dark"),
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("\x1b]10;#abcdef\x07") && stdout.contains("\x1b]11;#123456\x07"),
        "expected fallback palette OSCs: {stdout:?}"
    );
}

// ---------------------------------------------------------------------------
// `colorant themes` command group.
// ---------------------------------------------------------------------------

#[test]
fn themes_list_enumerates_bundled_palettes() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (stdout, stderr, code) = run_in(
        ws.path(),
        &["themes", "list", "--source", "bundled"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    // At least the well-known palettes show up. The list is now prefixed
    // with the source — e.g. `[bundled] catppuccin-mocha`.
    for expected in ["catppuccin-mocha", "tokyo-night", "nord"] {
        assert!(
            stdout
                .lines()
                .any(|l| l.starts_with("[bundled] ") && l.contains(expected)),
            "expected {expected:?} in list output: {stdout:?}"
        );
    }
}

#[test]
fn themes_list_marks_installed_palettes() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);
    fs::write(themes_dir.join("nord.colorant"), "fg = #aaaaaa\n").unwrap();

    let (stdout, _, code) = run_in(
        ws.path(),
        &["themes", "list", "--source", "bundled"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0);
    assert!(
        stdout.lines().any(|l| l == "[bundled] nord (installed)"),
        "expected nord to be marked installed: {stdout:?}"
    );
    assert!(
        stdout.lines().any(|l| l == "[bundled] catppuccin-mocha"),
        "expected catppuccin-mocha unmarked: {stdout:?}"
    );
}

#[test]
fn themes_path_prints_resolved_themes_dir() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);
    let (stdout, _, code) = run_in(
        ws.path(),
        &["themes", "path"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), themes_dir.to_str().unwrap());
}

#[test]
fn themes_install_one_creates_file() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);
    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "nord"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    let installed = themes_dir.join("nord.colorant");
    assert!(
        installed.is_file(),
        "expected nord.colorant at {installed:?}"
    );
    let content = fs::read_to_string(&installed).unwrap();
    assert!(
        content.contains("fg") && content.contains("bg"),
        "palette looks empty: {content:?}"
    );
}

#[test]
fn themes_install_creates_missing_themes_dir() {
    // The themes dir doesn't exist yet — install must create it.
    let ws = make_workspace();
    let xdg = ws.path().join("xdg");
    fs::create_dir_all(xdg.join("colorant")).unwrap();
    let themes_dir = xdg.join("colorant").join("themes");
    assert!(!themes_dir.exists());

    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "nord"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(themes_dir.join("nord.colorant").is_file());
}

#[test]
fn themes_install_one_refuses_overwrite_without_force() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);
    fs::write(themes_dir.join("nord.colorant"), "old\n").unwrap();

    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "nord"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_ne!(code, 0, "expected non-zero exit; stderr: {stderr}");
    assert!(
        stderr.contains("--force"),
        "expected --force hint in stderr: {stderr:?}"
    );
    // Original file is untouched.
    assert_eq!(
        fs::read_to_string(themes_dir.join("nord.colorant")).unwrap(),
        "old\n"
    );
}

#[test]
fn themes_install_one_force_overwrites() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);
    fs::write(themes_dir.join("nord.colorant"), "old\n").unwrap();

    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "nord", "--force"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    let content = fs::read_to_string(themes_dir.join("nord.colorant")).unwrap();
    assert_ne!(content, "old\n", "expected overwrite");
    assert!(content.contains("fg"));
}

#[test]
fn themes_install_unknown_name_errors() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "definitely-not-a-real-theme"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("no bundled palette"),
        "expected 'no bundled palette' in stderr: {stderr:?}"
    );
}

#[test]
fn themes_install_all_populates_themes_dir() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);

    let (stdout, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "--all"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("installed"),
        "expected install summary: {stdout:?}"
    );
    // Spot-check a few known palettes exist on disk.
    for name in ["catppuccin-mocha", "tokyo-night", "nord"] {
        let path = themes_dir.join(format!("{name}.colorant"));
        assert!(path.is_file(), "expected {path:?} after --all");
    }
}

#[test]
fn themes_install_all_is_idempotent_skipping_existing() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);

    // First run installs everything.
    let (_, _, code) = run_in(
        ws.path(),
        &["themes", "install", "--all"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0);

    // Sentinel content on one file proves it isn't rewritten without --force.
    let nord = themes_dir.join("nord.colorant");
    fs::write(&nord, "sentinel\n").unwrap();

    let (stdout, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "--all"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "second run must exit 0; stderr: {stderr}");
    assert!(
        stdout.contains("skipping") && stdout.contains("nord"),
        "expected skip messages: {stdout:?}"
    );
    assert_eq!(
        fs::read_to_string(&nord).unwrap(),
        "sentinel\n",
        "sentinel must survive a no-force --all"
    );
}

#[test]
fn themes_install_all_force_overwrites_everything() {
    let ws = make_workspace();
    let (xdg, themes_dir) = setup_xdg(&ws);
    fs::write(themes_dir.join("nord.colorant"), "sentinel\n").unwrap();

    let (stdout, stderr, code) = run_in(
        ws.path(),
        &["themes", "install", "--all", "--force"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    let content = fs::read_to_string(themes_dir.join("nord.colorant")).unwrap();
    assert_ne!(content, "sentinel\n");
    // Summary line must honestly report overwrites (not "0 already present").
    assert!(
        stdout.contains("1 overwritten"),
        "expected summary to report 1 overwritten, got: {stdout:?}"
    );
}

#[test]
fn themes_search_filters_bundled_substring_case_insensitively() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (stdout, _, code) = run_in(
        ws.path(),
        &["themes", "search", "Mocha", "--source", "bundled"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0);
    assert!(
        stdout.lines().any(|l| l.contains("catppuccin-mocha")),
        "expected catppuccin-mocha hit: {stdout:?}"
    );
    assert!(
        !stdout.lines().any(|l| l.contains("ayu-")),
        "non-matching theme should not appear: {stdout:?}"
    );
}

#[test]
fn themes_search_reports_no_hits_for_unknown_query() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (stdout, _, code) = run_in(
        ws.path(),
        &[
            "themes",
            "search",
            "zzzzz-no-such-theme",
            "--source",
            "bundled",
        ],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("no themes matched"), "{stdout:?}");
}

#[test]
fn themes_apply_writes_global_extends_to_rc() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "apply", "catppuccin-mocha"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    let rc = fs::read_to_string(ws.path().join(".colorantrc")).unwrap();
    assert!(
        rc.contains("extends = catppuccin-mocha"),
        "expected global extends: {rc:?}"
    );
    // Auto-install side effect: the bundled palette was written to disk.
    assert!(
        xdg.join("colorant")
            .join("themes")
            .join("catppuccin-mocha.colorant")
            .exists(),
        "expected catppuccin-mocha.colorant in themes dir"
    );
}

#[test]
fn themes_apply_dark_and_light_writes_per_mode_extends() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (_, _, code) = run_in(
        ws.path(),
        &[
            "themes",
            "apply",
            "--dark",
            "tokyo-night",
            "--light",
            "catppuccin-latte",
        ],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0);
    let rc = fs::read_to_string(ws.path().join(".colorantrc")).unwrap();
    assert!(rc.contains("extends.dark = tokyo-night"), "{rc:?}");
    assert!(rc.contains("extends.light = catppuccin-latte"), "{rc:?}");
    assert!(!rc.contains("extends = "), "no global extends: {rc:?}");
}

#[test]
fn themes_apply_preserves_other_base_keys() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let rc_path = ws.path().join(".colorantrc");
    fs::write(
        &rc_path,
        "extends = old\nfg = #ff00ff\n[dark]\ncursor = #abcdef\n",
    )
    .unwrap();

    let (_, _, code) = run_in(
        ws.path(),
        &["themes", "apply", "catppuccin-mocha"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0);
    let rc = fs::read_to_string(&rc_path).unwrap();
    assert!(rc.contains("extends = catppuccin-mocha"), "{rc:?}");
    assert!(rc.contains("fg = #ff00ff"), "{rc:?}");
    assert!(rc.contains("[dark]"), "{rc:?}");
    assert!(rc.contains("cursor = #abcdef"), "{rc:?}");
    assert!(!rc.contains("extends = old"), "{rc:?}");
}

#[test]
fn themes_apply_per_mode_preserves_existing_dark_section_block() {
    // Locks the contiguous-section invariant: applying --dark/--light to
    // an rc that already has a `[dark]` block must keep that block intact
    // and in place, not just present-somewhere.
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let rc_path = ws.path().join(".colorantrc");
    fs::write(
        &rc_path,
        "fg = #ff00ff\n[dark]\ncursor = #abcdef\ncolor0 = #001122\n",
    )
    .unwrap();

    let (_, stderr, code) = run_in(
        ws.path(),
        &[
            "themes",
            "apply",
            "--dark",
            "tokyo-night",
            "--light",
            "catppuccin-latte",
        ],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    let rc = fs::read_to_string(&rc_path).unwrap();
    // The [dark] block must stay contiguous — keys neither reordered nor
    // displaced into a separate section.
    assert!(
        rc.contains("[dark]\ncursor = #abcdef\ncolor0 = #001122"),
        "[dark] block should be intact: {rc:?}"
    );
    // And the new extends keys land above the base content.
    assert!(rc.starts_with("extends.dark = tokyo-night"), "{rc:?}");
}

#[test]
fn themes_list_continues_past_unsynced_remote() {
    // Without `--source`, list iterates every source. Gogh isn't synced
    // in this test (fresh workspace), so it should surface a warning on
    // stderr but still print the bundled list on stdout — exit 0.
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    // Point the cache somewhere empty so gogh definitely isn't synced.
    let cache = ws.path().join("cache");
    fs::create_dir_all(&cache).unwrap();

    let (stdout, stderr, code) = run_in(
        ws.path(),
        &["themes", "list"],
        &[
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
            ("XDG_CACHE_HOME", cache.to_str().unwrap()),
        ],
    );
    assert_eq!(code, 0);
    assert!(
        stdout.lines().any(|l| l.starts_with("[bundled] ")),
        "expected bundled themes on stdout: {stdout:?}"
    );
    assert!(
        stderr.contains("[gogh]") && stderr.contains("not synced"),
        "expected gogh warning on stderr: {stderr:?}"
    );
}

#[test]
fn themes_apply_errors_on_unknown_theme() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "apply", "definitely-not-a-real-theme"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("not") && stderr.contains("installed") && stderr.contains("bundled"),
        "expected guidance about lookup failure: {stderr:?}"
    );
}

#[test]
fn themes_apply_requires_at_least_one_target() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "apply"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("--dark") || stderr.contains("theme name"),
        "{stderr:?}"
    );
}

#[test]
fn themes_install_without_name_or_all_errors() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let (_, stderr, code) = run_in(
        ws.path(),
        &["themes", "install"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("--all") || stderr.contains("palette name"),
        "expected guidance on how to call install: {stderr:?}"
    );
}

#[test]
fn doctor_clean_rc_exits_zero() {
    let ws = make_workspace();
    let (xdg, themes) = setup_xdg(&ws);
    fs::write(themes.join("ayu.colorant"), "fg = #abcdef\n").unwrap();
    fs::write(
        ws.path().join(".colorantrc"),
        "extends = ayu\nfg = #ffffff\n",
    )
    .unwrap();

    let rc = ws.path().join(".colorantrc");
    let (stdout, _, code) = run_in(
        ws.path(),
        &["doctor", rc.to_str().unwrap()],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("Palette(s):"), "{stdout}");
    assert!(stdout.contains("No issues found."), "{stdout}");
    // Symmetric confirmation: one "No parsing errors." line for the rc
    // itself, one for the audited palette (light and dark resolve to the
    // same palette, so dedup keeps it to one audit).
    assert_eq!(
        stdout.matches("No parsing errors.").count(),
        2,
        "expected one confirmation per audited file: {stdout}"
    );
    // Doctor no longer appends a "(found)" suffix on successful resolutions;
    // the absence of "(NOT FOUND)" + exit 0 is the signal.
    assert!(!stdout.contains("(found)"), "{stdout}");
    assert!(!stdout.contains("NOT FOUND"), "{stdout}");
}

#[test]
fn doctor_surfaces_palette_parsing_errors() {
    // A clean rc that extends a palette with drops should still exit 1 and
    // surface the palette's drops indented under its resolution line.
    let ws = make_workspace();
    let (xdg, themes) = setup_xdg(&ws);
    fs::write(
        themes.join("dirty.colorant"),
        "fg = #abcdef\nforground = #112233\nbg = nope\n",
    )
    .unwrap();
    fs::write(ws.path().join(".colorantrc"), "extends = dirty\n").unwrap();

    let rc = ws.path().join(".colorantrc");
    let (stdout, _, code) = run_in(
        ws.path(),
        &["doctor", rc.to_str().unwrap()],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(stdout.contains("unknown key 'forground'"), "{stdout}");
    assert!(stdout.contains("invalid color 'nope'"), "{stdout}");
    // Both modes resolve to the same palette via global `extends`, but the
    // palette is audited only once.
    assert_eq!(
        stdout.matches("unknown key 'forground'").count(),
        1,
        "palette should be audited once even when both modes share it: {stdout}"
    );
    // The second mode's row gets an explicit "not re-audited" note so the
    // user isn't left guessing whether it was checked.
    assert!(
        stdout.contains("(same palette as above; not re-audited)"),
        "{stdout}"
    );
    assert!(stdout.contains("Found 2 issues."), "{stdout}");
}

#[test]
fn doctor_walks_up_when_no_path_given() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let nested = ws.path().join("a/b/c");
    fs::create_dir_all(&nested).unwrap();
    fs::write(ws.path().join(".colorantrc"), "fg = #ffffff\n").unwrap();

    let (stdout, _, _) = run_in(
        &nested,
        &["doctor"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert!(stdout.contains("Checking "), "{stdout}");
    assert!(stdout.contains(".colorantrc"), "{stdout}");
}

#[test]
fn doctor_no_rc_anywhere_exits_one() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let nested = ws.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();

    let (stdout, _, code) = run_in(
        &nested,
        &["doctor"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 1);
    assert!(stdout.contains("No .colorantrc found"), "{stdout}");
}

#[test]
fn doctor_reports_every_drop_kind() {
    let ws = make_workspace();
    let (xdg, themes) = setup_xdg(&ws);
    fs::write(themes.join("good.colorant"), "fg = #abcdef\n").unwrap();
    let rc = ws.path().join("dirty.colorantrc");
    fs::write(
        &rc,
        "extends = good\n\
         extends.light = bad/name\n\
         forground = #112233\n\
         bg = nope\n\
         [lite]\n\
         cursor = #ff00ff\n\
         no equals here\n",
    )
    .unwrap();

    let (stdout, _, code) = run_in(
        ws.path(),
        &["doctor", rc.to_str().unwrap()],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(stdout.contains("invalid theme name 'bad/name'"), "{stdout}");
    assert!(stdout.contains("unknown key 'forground'"), "{stdout}");
    assert!(stdout.contains("invalid color 'nope'"), "{stdout}");
    assert!(stdout.contains("unknown section [lite]"), "{stdout}");
    assert!(stdout.contains("malformed line"), "{stdout}");
    assert!(stdout.contains("Found 5 issues."), "{stdout}");
}

#[test]
fn doctor_reports_missing_extends_palette() {
    // Clean rc, but the named palette file isn't installed in themes/.
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let rc = ws.path().join(".colorantrc");
    fs::write(&rc, "extends = does-not-exist\n").unwrap();

    let (stdout, _, code) = run_in(
        ws.path(),
        &["doctor", rc.to_str().unwrap()],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 1, "stdout: {stdout}");
    assert!(stdout.contains("NOT FOUND"), "{stdout}");
    assert!(stdout.contains("does-not-exist"), "{stdout}");
    assert!(
        stdout.contains("Found 2 issue"),
        "missing palette is reported once per mode: {stdout}"
    );
}

#[test]
fn doctor_explicit_path_that_doesnt_exist_errors_nonzero() {
    // doctor on a path that isn't a real file should fail nonzero (the
    // read_to_string propagates as anyhow::Error → stderr → exit 1).
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let missing = ws.path().join("nope.colorantrc");

    let (_, stderr, code) = run_in(
        ws.path(),
        &["doctor", missing.to_str().unwrap()],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("reading") && stderr.contains("nope.colorantrc"),
        "expected anyhow context naming the file, got: {stderr:?}"
    );
}

#[test]
fn show_prints_resolved_colors_with_rc_and_palette() {
    let ws = make_workspace();
    let (xdg, themes) = setup_xdg(&ws);
    fs::write(
        themes.join("ayu.colorant"),
        "fg = #abcdef\nbg = #001122\ncolor0 = #112233\n",
    )
    .unwrap();
    fs::write(
        ws.path().join(".colorantrc"),
        "extends = ayu\ncursor = #ff00ff\n",
    )
    .unwrap();

    let (stdout, _, code) = run_in(
        ws.path(),
        &["show"],
        &[
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
            ("COLORANT_MODE", "dark"),
        ],
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("Active theme for"), "{stdout}");
    assert!(stdout.contains("Mode: dark"), "{stdout}");
    assert!(stdout.contains("#abcdef"), "{stdout}");
    assert!(stdout.contains("#001122"), "{stdout}");
    // The rc's own cursor key should override the palette (it had none).
    assert!(stdout.contains("#ff00ff"), "{stdout}");
    assert!(stdout.contains("color0"), "{stdout}");
    assert!(stdout.contains("color15"), "{stdout}");
    // Unset palette entries are flagged so the user can see what's missing.
    assert!(stdout.contains("(unset)"), "{stdout}");
}

#[test]
fn show_all_prints_both_modes() {
    let ws = make_workspace();
    let (xdg, themes) = setup_xdg(&ws);
    fs::write(themes.join("dark-pal.colorant"), "fg = #000001\n").unwrap();
    fs::write(themes.join("light-pal.colorant"), "fg = #fffffe\n").unwrap();
    fs::write(
        ws.path().join(".colorantrc"),
        "extends.dark = dark-pal\nextends.light = light-pal\n",
    )
    .unwrap();

    let (stdout, _, code) = run_in(
        ws.path(),
        &["show", "--all"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("Dark mode:"), "{stdout}");
    assert!(stdout.contains("Light mode:"), "{stdout}");
    assert!(stdout.contains("#000001"), "{stdout}");
    assert!(stdout.contains("#fffffe"), "{stdout}");
    // With --all there's no "Mode:" line — the section headers do that job.
    assert!(!stdout.contains("Mode: dark"), "{stdout}");
}

#[test]
fn show_falls_back_to_default_theme_when_no_rc() {
    let ws = make_workspace();
    let (xdg, themes) = setup_xdg(&ws);
    fs::write(themes.join("solo.colorant"), "fg = #777777\n").unwrap();
    fs::write(
        xdg.join("colorant").join("config.toml"),
        "default_theme = \"solo\"\n",
    )
    .unwrap();

    let nested = ws.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();
    let (stdout, _, code) = run_in(
        &nested,
        &["show"],
        &[
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
            ("COLORANT_MODE", "dark"),
        ],
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("Default theme: solo"), "{stdout}");
    assert!(stdout.contains("#777777"), "{stdout}");
}

#[test]
fn show_warns_when_default_theme_palette_is_missing() {
    // default_theme names a palette that isn't installed. Without the
    // warning, show would print the theme header followed by 19 (unset)
    // rows with no explanation — which is the silent-failure pattern
    // this tool exists to surface.
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    fs::write(
        xdg.join("colorant").join("config.toml"),
        "default_theme = \"ghost\"\n",
    )
    .unwrap();
    let nested = ws.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();

    let (stdout, _, code) = run_in(
        &nested,
        &["show"],
        &[
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
            ("COLORANT_MODE", "dark"),
        ],
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("Default theme: ghost"), "{stdout}");
    assert!(
        stdout.contains("(palette file not found"),
        "expected missing-palette warning, got: {stdout}"
    );
    assert!(stdout.contains("ghost.colorant"), "{stdout}");
}

#[test]
fn show_with_no_rc_and_no_default_theme_prints_message() {
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let nested = ws.path().join("a/b");
    fs::create_dir_all(&nested).unwrap();

    let (stdout, _, code) = run_in(
        &nested,
        &["show"],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(
        stdout.contains("No theme applies in this directory."),
        "{stdout}"
    );
}

#[test]
fn show_omits_ansi_escapes_when_piped() {
    // Subprocess stdout is captured (not a TTY), so swatches should be
    // plain block characters with no `\x1b[48;2;...m` codes.
    let ws = make_workspace();
    let (xdg, themes) = setup_xdg(&ws);
    fs::write(themes.join("p.colorant"), "fg = #abcdef\n").unwrap();
    fs::write(ws.path().join(".colorantrc"), "extends = p\n").unwrap();

    let (stdout, _, _) = run_in(
        ws.path(),
        &["show"],
        &[
            ("XDG_CONFIG_HOME", xdg.to_str().unwrap()),
            ("COLORANT_MODE", "dark"),
        ],
    );
    assert!(
        !stdout.contains("\x1b[38;2;"),
        "expected no 24-bit color escapes in piped output: {stdout:?}"
    );
    // Block characters are still printed (kept visible for piped layout).
    assert!(stdout.contains('█'), "{stdout}");
}

#[test]
fn doctor_no_parent_palette_is_not_an_issue() {
    // An rc with only its own keys (no extends) is valid — doctor should
    // call that out as "no parent palette" without counting it as an issue.
    let ws = make_workspace();
    let (xdg, _) = setup_xdg(&ws);
    let rc = ws.path().join(".colorantrc");
    fs::write(&rc, "fg = #ffffff\nbg = #000000\n").unwrap();

    let (stdout, _, code) = run_in(
        ws.path(),
        &["doctor", rc.to_str().unwrap()],
        &[("XDG_CONFIG_HOME", xdg.to_str().unwrap())],
    );
    assert_eq!(code, 0, "stdout: {stdout}");
    assert!(stdout.contains("no parent palette"), "{stdout}");
}
