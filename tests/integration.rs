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
        "TERM_PROGRAM",
        "TERM",
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
