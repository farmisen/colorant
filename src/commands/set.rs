//! `colorant set` — interactive theme picker.
//!
//! Launches a ratatui TUI that lists installed and bundled palettes with a
//! live preview of each palette's colors. The user assigns themes to one
//! or more of three slots — `both` / `dark` / `light` — and on apply the
//! cwd's `.colorantrc` is updated with the corresponding `extends` /
//! `extends.dark` / `extends.light` keys. Other keys in the rc are
//! preserved. Bundled themes that aren't yet installed on disk are copied
//! into `base_theme_dir` automatically as part of apply.

use crate::config::{Config, THEME_FILE_NAME};
use crate::theme::bundled::BUNDLED_THEMES;
use crate::theme::model::{HexColor, ThemeLayer, ThemeName};
use crate::theme::parse::{parse_palette_str, parse_rc_str};
use crate::theme::resolve::PALETTE_EXTENSION;
use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};

/// One row in the browseable theme list.
struct ThemeEntry {
    name: ThemeName,
    layer: ThemeLayer,
    installed: bool,
    bundled: bool,
}

/// Which slot a theme is currently assigned to. A single theme can occupy
/// multiple slots; the apply step writes one `extends*` line per slot.
#[derive(Default)]
struct Picks {
    both: Option<usize>,
    dark: Option<usize>,
    light: Option<usize>,
}

/// Toggle assignment of `idx` to `slot`: if already set to `Some(idx)`,
/// clear it; otherwise assign. Pressing the same slot key twice on the
/// same theme unpicks it — handy for undoing without going through `c`.
fn toggle(slot: &mut Option<usize>, idx: usize) {
    *slot = if *slot == Some(idx) { None } else { Some(idx) };
}

impl Picks {
    fn clear(&mut self) {
        self.both = None;
        self.dark = None;
        self.light = None;
    }

    /// Toggle the `both` slot for `idx`. Assigning `both` clears `dark` and
    /// `light` — the two modes are orthogonal in the file format
    /// (`extends = X` vs `extends.dark = Y` + `extends.light = Z`) and
    /// letting them coexist in the UI would only confuse the apply step.
    fn toggle_both(&mut self, idx: usize) {
        toggle(&mut self.both, idx);
        if self.both.is_some() {
            self.dark = None;
            self.light = None;
        }
    }

    /// Toggle the `dark` slot for `idx`. When `both` is currently set, it
    /// gets decomposed: the previous `both` target moves to `light` (so
    /// the user's earlier intent is preserved on the other side), unless
    /// the user is picking the same theme for dark — in which case `both`
    /// is simply dropped and `dark` gets the pick (they're disambiguating
    /// to dark-only).
    fn toggle_dark(&mut self, idx: usize) {
        if let Some(prev_both) = self.both.take() {
            if prev_both != idx {
                self.light = Some(prev_both);
            }
            self.dark = Some(idx);
            return;
        }
        toggle(&mut self.dark, idx);
    }

    /// Toggle the `light` slot for `idx`. Mirror of `toggle_dark`: when
    /// `both` is set, the previous target moves to `dark` (or is dropped
    /// if the user picked the same theme for light).
    fn toggle_light(&mut self, idx: usize) {
        if let Some(prev_both) = self.both.take() {
            if prev_both != idx {
                self.dark = Some(prev_both);
            }
            self.light = Some(idx);
            return;
        }
        toggle(&mut self.light, idx);
    }

    /// Resolve the slot state into the trio of effective extends keys to
    /// write. `dark == light` collapses to a single global `extends` —
    /// semantically equivalent but a cleaner rc.
    fn effective(&self) -> (Option<usize>, Option<usize>, Option<usize>) {
        if let Some(b) = self.both {
            return (Some(b), None, None);
        }
        if let (Some(d), Some(l)) = (self.dark, self.light)
            && d == l
        {
            return (Some(d), None, None);
        }
        (None, self.dark, self.light)
    }

    /// Display tag for the theme list. Shows what modes the theme is
    /// currently active for: `[d/l]` for the `both` slot (since `both`
    /// means dark *and* light), or `[d]` / `[l]` / `[d/l]` for the
    /// per-mode slots. Empty when the theme is unassigned.
    fn slot_tag(&self, idx: usize) -> String {
        let dark_hit = self.both == Some(idx) || self.dark == Some(idx);
        let light_hit = self.both == Some(idx) || self.light == Some(idx);
        match (dark_hit, light_hit) {
            (false, false) => String::new(),
            (true, false) => " [d]".to_string(),
            (false, true) => " [l]".to_string(),
            (true, true) => " [d/l]".to_string(),
        }
    }
}

struct App {
    themes: Vec<ThemeEntry>,
    list_state: ListState,
    picks: Picks,
    rc_path: PathBuf,
    themes_dir: PathBuf,
    /// Filled in only on apply: the path actually written, for the final
    /// summary printed after the TUI exits.
    applied: Option<PathBuf>,
}

impl App {
    fn selected(&self) -> Option<&ThemeEntry> {
        self.list_state.selected().and_then(|i| self.themes.get(i))
    }

    fn assign_selected_to<F: FnOnce(&mut Picks, usize)>(&mut self, set: F) {
        if let Some(i) = self.list_state.selected() {
            set(&mut self.picks, i);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.themes.is_empty() {
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0) as isize;
        let len = self.themes.len() as isize;
        let next = (cur + delta).rem_euclid(len);
        self.list_state.select(Some(next as usize));
    }

    fn picks_summary(&self) -> Vec<String> {
        let line = |slot: Option<usize>| {
            slot.and_then(|i| self.themes.get(i))
                .map(|t| t.name.as_str().to_string())
                .unwrap_or_else(|| "–".to_string())
        };
        vec![
            format!("both = {}", line(self.picks.both)),
            format!("dark = {}", line(self.picks.dark)),
            format!("light = {}", line(self.picks.light)),
        ]
    }

    fn pending_rc_block(&self) -> Vec<String> {
        let (both, dark, light) = self.picks.effective();
        let mut lines = Vec::new();
        if let Some(i) = both
            && let Some(t) = self.themes.get(i)
        {
            lines.push(format!("extends = {}", t.name));
        }
        if let Some(i) = dark
            && let Some(t) = self.themes.get(i)
        {
            lines.push(format!("extends.dark = {}", t.name));
        }
        if let Some(i) = light
            && let Some(t) = self.themes.get(i)
        {
            lines.push(format!("extends.light = {}", t.name));
        }
        lines
    }
}

pub fn run(config: &Config) -> Result<()> {
    let themes = load_themes(config)?;
    if themes.is_empty() {
        eprintln!(
            "No themes available. Run `colorant themes install --all` or drop \
             a .colorant palette into {}.",
            config.base_theme_dir.display()
        );
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let rc_path = cwd.join(THEME_FILE_NAME);

    // Pre-populate the slots from any existing rc so the user sees what's
    // currently in effect when the TUI opens. Read errors other than
    // NotFound are surfaced — they usually mean a permissions problem the
    // user should know about before they apply (which would then fail).
    let (picks, missing) = match fs::read_to_string(&rc_path) {
        Ok(content) => picks_from_rc_content(&content, &themes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Picks::default(), Vec::new()),
        Err(e) => {
            eprintln!(
                "warning: could not read {}: {e} (starting with empty picks)",
                rc_path.display()
            );
            (Picks::default(), Vec::new())
        }
    };
    for name in &missing {
        eprintln!(
            "warning: {} references theme {:?} which isn't installed or bundled — \
             it won't be preserved on apply",
            rc_path.display(),
            name
        );
    }

    let mut app = App {
        themes,
        list_state: {
            let mut s = ListState::default();
            s.select(Some(0));
            s
        },
        picks,
        rc_path,
        themes_dir: config.base_theme_dir.clone(),
        applied: None,
    };

    // Install a panic hook that restores the terminal before the panic
    // unwinds. Without this, a panic anywhere in the draw/event path
    // leaves the user in raw mode with the alt screen still active —
    // unrecoverable without typing `stty sane; reset` blind.
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(info);
    }));

    let mut terminal = setup_terminal()?;
    let outcome = event_loop(&mut terminal, &mut app);
    let restore = restore_terminal(&mut terminal);
    // Event-loop errors take precedence — the user cares about why the
    // TUI failed, not about cleanup hiccups.
    outcome?;
    restore?;

    if let Some(written) = &app.applied {
        println!("Updated {}", written.display());
        for line in app.pending_rc_block() {
            println!("  {line}");
        }
    }

    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("entering raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("entering alternate screen")?;
    Terminal::new(CrosstermBackend::new(stdout)).context("constructing terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    // Try every step even if an earlier one fails so the user has the
    // best chance of getting a usable terminal back. Surface the first
    // failure (with a hint), if any.
    let raw = disable_raw_mode();
    let alt = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let cursor = terminal.show_cursor();
    for (label, result) in [
        ("disabling raw mode", raw),
        ("leaving alternate screen", alt),
        ("restoring cursor", cursor),
    ] {
        if let Err(e) = result {
            eprintln!(
                "warning: {label} failed during cleanup: {e}. Run `reset` if your terminal looks broken."
            );
            return Err(e).context(label);
        }
    }
    Ok(())
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        match event::read()? {
            // Resize: next iteration of the loop redraws against the new
            // size. Without this match arm, the TUI would stay frozen on
            // the old size until a key was pressed.
            Event::Resize(_, _) => continue,
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('j') | KeyCode::Down => app.move_cursor(1),
                KeyCode::Char('k') | KeyCode::Up => app.move_cursor(-1),
                KeyCode::Char('g') | KeyCode::Home => app.list_state.select(Some(0)),
                KeyCode::Char('G') | KeyCode::End => {
                    let last = app.themes.len().saturating_sub(1);
                    app.list_state.select(Some(last));
                }
                KeyCode::Char('b') => app.assign_selected_to(|p, i| p.toggle_both(i)),
                KeyCode::Char('d') => app.assign_selected_to(|p, i| p.toggle_dark(i)),
                KeyCode::Char('l') => app.assign_selected_to(|p, i| p.toggle_light(i)),
                KeyCode::Char('c') => app.picks.clear(),
                KeyCode::Enter => {
                    apply(app)?;
                    return Ok(());
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn draw(frame: &mut ratatui::Frame, app: &App) {
    let area = frame.area();
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" colorant set ");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),    // main area
            Constraint::Length(5), // status (slot assignments + rc preview)
            Constraint::Length(1), // keybinds
        ])
        .split(inner);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[0]);

    draw_theme_list(frame, app, main[0]);
    draw_preview(frame, app, main[1]);
    draw_status(frame, app, chunks[1]);
    draw_keybinds(frame, chunks[2]);
}

fn draw_theme_list(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .themes
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let installed = if t.installed { "*" } else { " " };
            let tag = app.picks.slot_tag(i);
            ListItem::new(format!("{installed} {}{tag}", t.name))
        })
        .collect();

    let title = format!(" Themes ({}) ", app.themes.len());
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut state = app.list_state.clone();
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_preview(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let title = match app.selected() {
        Some(t) => format!(
            " Preview: {}{} ",
            t.name,
            if t.installed { " (installed)" } else { "" }
        ),
        None => " Preview ".to_string(),
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(theme) = app.selected() else {
        return;
    };

    let mut lines: Vec<Line> = vec![
        swatch_line("fg", theme.layer.fg.as_ref()),
        swatch_line("bg", theme.layer.bg.as_ref()),
        swatch_line("cursor", theme.layer.cursor.as_ref()),
        Line::default(),
    ];
    for i in 0..8 {
        lines.push(palette_row(theme, i));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn draw_status(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let mut lines = Vec::new();
    lines.push(Line::from(format!(
        "Picks: {}",
        app.picks_summary().join("  ")
    )));
    let pending = app.pending_rc_block();
    if pending.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no extends to write — pick a theme with b/d/l)",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        lines.push(Line::from(format!(
            "Will write to {}:",
            app.rc_path.display()
        )));
        for entry in pending {
            lines.push(Line::from(format!("  {entry}")));
        }
    }
    let block = Block::default().borders(Borders::TOP);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_keybinds(frame: &mut ratatui::Frame, area: Rect) {
    let text = "j/k=nav  b=both  d=dark  l=light  c=clear  enter=apply  q=quit";
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::DarkGray),
        ))),
        area,
    );
}

fn swatch_line(name: &str, color: Option<&HexColor>) -> Line<'static> {
    let mut spans = vec![Span::raw(format!("  {name:<7}  "))];
    match color {
        Some(c) => {
            spans.push(Span::raw(format!("{}  ", c.as_str())));
            let (r, g, b) = hex_to_rgb(c.as_str());
            spans.push(Span::styled(
                "    ",
                Style::default().bg(Color::Rgb(r, g, b)),
            ));
        }
        None => spans.push(Span::raw("(unset)      ".to_string())),
    }
    Line::from(spans)
}

fn palette_row(theme: &ThemeEntry, row: usize) -> Line<'static> {
    let left = swatch_inline(&color_name(row), theme.layer.palette[row].as_ref());
    let right = swatch_inline(&color_name(row + 8), theme.layer.palette[row + 8].as_ref());
    let mut spans = Vec::with_capacity(left.len() + right.len() + 1);
    spans.extend(left);
    spans.push(Span::raw("   "));
    spans.extend(right);
    Line::from(spans)
}

/// Build the spans for one palette entry (used twice per row).
fn swatch_inline(name: &str, color: Option<&HexColor>) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(format!("  {name:<7}  "))];
    match color {
        Some(c) => {
            spans.push(Span::raw(format!("{}  ", c.as_str())));
            let (r, g, b) = hex_to_rgb(c.as_str());
            spans.push(Span::styled(
                "    ",
                Style::default().bg(Color::Rgb(r, g, b)),
            ));
        }
        None => spans.push(Span::raw("(unset)      ".to_string())),
    }
    spans
}

fn color_name(idx: usize) -> String {
    format!("color{idx:<2}")
}

fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let r = u8::from_str_radix(hex.get(1..3).unwrap_or("00"), 16).unwrap_or(0);
    let g = u8::from_str_radix(hex.get(3..5).unwrap_or("00"), 16).unwrap_or(0);
    let b = u8::from_str_radix(hex.get(5..7).unwrap_or("00"), 16).unwrap_or(0);
    (r, g, b)
}

fn apply(app: &mut App) -> Result<()> {
    let pending = app.pending_rc_block();
    if pending.is_empty() {
        // Nothing to write — treat enter as a no-op exit. Caller will see
        // applied stays None and won't print a summary.
        return Ok(());
    }

    let (both_idx, dark_idx, light_idx) = app.picks.effective();
    // Auto-install bundled palettes that haven't been copied to disk yet.
    for i in [both_idx, dark_idx, light_idx].into_iter().flatten() {
        let theme = &app.themes[i];
        if !theme.installed && theme.bundled {
            install_bundled_palette(&theme.name, &app.themes_dir)?;
        } else if !theme.installed && !theme.bundled {
            return Err(anyhow!(
                "theme {} is not installed and not bundled",
                theme.name
            ));
        }
    }

    let names = |slot: Option<usize>| slot.and_then(|i| app.themes.get(i).map(|t| t.name.clone()));
    let both = names(both_idx);
    let dark = names(dark_idx);
    let light = names(light_idx);

    let existing = match fs::read_to_string(&app.rc_path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", app.rc_path.display()));
        }
    };
    let rewritten = rewrite_extends(
        &existing,
        both.as_ref().map(|n| n.as_str()),
        dark.as_ref().map(|n| n.as_str()),
        light.as_ref().map(|n| n.as_str()),
    );
    fs::write(&app.rc_path, rewritten)
        .with_context(|| format!("writing {}", app.rc_path.display()))?;
    app.applied = Some(app.rc_path.clone());
    Ok(())
}

/// Map the extends keys of an existing `.colorantrc` onto our three slots
/// so the TUI opens with the user's current picks pre-selected. Mirrors
/// the resolver's fallback logic: `extends.dark`/`extends.light` win, with
/// the global `extends` filling in any side that didn't have a per-mode
/// override. When both effective sides end up equal, they collapse into
/// the `both` slot — same consolidation `effective()` does on the way out.
///
/// Returns the populated `Picks` plus the list of theme names referenced
/// by the rc that aren't in the loaded theme list (sorted, deduped). The
/// caller surfaces those as a warning so the user knows their old picks
/// won't be preserved on apply.
fn picks_from_rc_content(content: &str, themes: &[ThemeEntry]) -> (Picks, Vec<String>) {
    let rc = parse_rc_str(content);
    let mut missing: Vec<String> = Vec::new();
    let resolve = |name: Option<&ThemeName>, missing: &mut Vec<String>| -> Option<usize> {
        let name = name?;
        let idx = themes.iter().position(|t| t.name.as_str() == name.as_str());
        if idx.is_none() {
            let s = name.as_str().to_string();
            if !missing.contains(&s) {
                missing.push(s);
            }
        }
        idx
    };
    let dark = resolve(
        rc.extends_dark.as_ref().or(rc.extends.as_ref()),
        &mut missing,
    );
    let light = resolve(
        rc.extends_light.as_ref().or(rc.extends.as_ref()),
        &mut missing,
    );
    let picks = match (dark, light) {
        (Some(d), Some(l)) if d == l => Picks {
            both: Some(d),
            dark: None,
            light: None,
        },
        (d, l) => Picks {
            both: None,
            dark: d,
            light: l,
        },
    };
    (picks, missing)
}

fn install_bundled_palette(name: &ThemeName, themes_dir: &Path) -> Result<()> {
    let content = BUNDLED_THEMES
        .iter()
        .find(|(n, _)| *n == name.as_str())
        .map(|(_, c)| *c)
        .ok_or_else(|| anyhow!("no bundled palette named {}", name))?;
    fs::create_dir_all(themes_dir)
        .with_context(|| format!("creating themes dir {}", themes_dir.display()))?;
    let dest = themes_dir.join(format!("{}.{}", name, PALETTE_EXTENSION));
    fs::write(&dest, content).with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

/// Build the merged installed + bundled theme list, sorted by name.
fn load_themes(config: &Config) -> Result<Vec<ThemeEntry>> {
    let mut entries: BTreeMap<String, ThemeEntry> = BTreeMap::new();

    // Bundled (compiled in).
    for (name, content) in BUNDLED_THEMES {
        let Ok(theme_name) = ThemeName::parse(name) else {
            continue;
        };
        let layer = parse_palette_str(content).layer;
        entries.insert(
            name.to_string(),
            ThemeEntry {
                name: theme_name,
                layer,
                installed: false,
                bundled: true,
            },
        );
    }

    // Installed on disk under base_theme_dir. Overrides bundled with the
    // disk version so user edits show up in the preview.
    if config.base_theme_dir.is_dir() {
        for entry in fs::read_dir(&config.base_theme_dir)
            .with_context(|| format!("reading themes dir {}", config.base_theme_dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some(PALETTE_EXTENSION) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(name) = ThemeName::parse(stem) else {
                continue;
            };
            let content =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let layer = parse_palette_str(&content).layer;
            let bundled = BUNDLED_THEMES.iter().any(|(n, _)| *n == stem);
            entries.insert(
                stem.to_string(),
                ThemeEntry {
                    name,
                    layer,
                    installed: true,
                    bundled,
                },
            );
        }
    }

    Ok(entries.into_values().collect())
}

/// Splice the given extends assignments into the existing rc content.
/// Removes any existing top-level extends* lines and writes the new set at
/// the top of the file. All other lines (including the entire `[dark]` /
/// `[light]` sections and any other base-section keys) are preserved
/// verbatim and in order.
fn rewrite_extends(
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
    fn picks_slot_tag_renders_practical_effect() {
        // `both` expands to "d/l" because that's what the theme actually
        // applies to. Other themes that aren't assigned get no tag.
        let mut p = Picks::default();
        p.toggle_both(0);
        assert_eq!(p.slot_tag(0), " [d/l]");
        assert_eq!(p.slot_tag(1), "");

        // Per-mode only.
        let mut p = Picks::default();
        p.toggle_dark(0);
        assert_eq!(p.slot_tag(0), " [d]");
        p.toggle_light(1);
        assert_eq!(p.slot_tag(0), " [d]");
        assert_eq!(p.slot_tag(1), " [l]");

        // Same theme picked for both modes explicitly.
        let mut p = Picks::default();
        p.toggle_dark(0);
        p.toggle_light(0);
        assert_eq!(p.slot_tag(0), " [d/l]");
    }

    #[test]
    fn picks_both_then_per_mode_clears_both() {
        // Setting `both` after `dark`/`light` clears those per-mode picks.
        let mut p = Picks::default();
        p.toggle_dark(1);
        p.toggle_light(2);
        p.toggle_both(0);
        assert_eq!(p.both, Some(0));
        assert_eq!(p.dark, None);
        assert_eq!(p.light, None);
    }

    #[test]
    fn dark_after_both_on_different_theme_decomposes_into_dark_and_light() {
        // Spec: `b` on X then `d` on Y → dark=Y, light=X (the previous
        // `both` migrates to the other mode so the user's earlier intent
        // is preserved).
        let mut p = Picks::default();
        p.toggle_both(0); // X = 0
        p.toggle_dark(1); // Y = 1
        assert_eq!(p.both, None);
        assert_eq!(p.dark, Some(1));
        assert_eq!(p.light, Some(0));
    }

    #[test]
    fn light_after_both_on_different_theme_decomposes_into_dark_and_light() {
        let mut p = Picks::default();
        p.toggle_both(0);
        p.toggle_light(1);
        assert_eq!(p.both, None);
        assert_eq!(p.dark, Some(0));
        assert_eq!(p.light, Some(1));
    }

    #[test]
    fn dark_after_both_on_same_theme_disambiguates_to_dark_only() {
        // Spec: `b` on X then `d` on X → only dark=X (user is saying "I
        // only want this theme on the dark side").
        let mut p = Picks::default();
        p.toggle_both(0);
        p.toggle_dark(0);
        assert_eq!(p.both, None);
        assert_eq!(p.dark, Some(0));
        assert_eq!(p.light, None);
    }

    #[test]
    fn light_after_both_on_same_theme_disambiguates_to_light_only() {
        let mut p = Picks::default();
        p.toggle_both(0);
        p.toggle_light(0);
        assert_eq!(p.both, None);
        assert_eq!(p.dark, None);
        assert_eq!(p.light, Some(0));
    }

    #[test]
    fn toggling_a_slot_off_does_not_clear_the_other_mode() {
        // Pressing `b` twice (toggle off) shouldn't have any side effect
        // on dark/light, which were already cleared when `b` was set.
        let mut p = Picks::default();
        p.toggle_both(0);
        p.toggle_both(0); // toggle off
        assert_eq!(p.both, None);
        // dark/light remain whatever they were (None here, untouched).
        assert_eq!(p.dark, None);
        assert_eq!(p.light, None);
    }

    #[test]
    fn toggle_clears_when_slot_matches_current_index() {
        let mut slot = Some(3);
        toggle(&mut slot, 3);
        assert_eq!(slot, None);
        toggle(&mut slot, 3);
        assert_eq!(slot, Some(3));
        toggle(&mut slot, 7);
        assert_eq!(slot, Some(7));
    }

    fn make_theme(name: &str) -> ThemeEntry {
        ThemeEntry {
            name: ThemeName::parse(name).unwrap(),
            layer: ThemeLayer::default(),
            installed: false,
            bundled: true,
        }
    }

    #[test]
    fn effective_collapses_equal_dark_and_light_into_both() {
        let mut p = Picks::default();
        p.toggle_dark(0);
        p.toggle_light(0);
        assert_eq!(p.effective(), (Some(0), None, None));
    }

    #[test]
    fn effective_keeps_separate_dark_and_light_when_different() {
        let mut p = Picks::default();
        p.toggle_dark(0);
        p.toggle_light(1);
        assert_eq!(p.effective(), (None, Some(0), Some(1)));
    }

    #[test]
    fn effective_passes_through_partial_picks() {
        let mut p = Picks::default();
        p.toggle_dark(2);
        assert_eq!(p.effective(), (None, Some(2), None));
    }

    #[test]
    fn picks_from_rc_global_extends() {
        let themes = [make_theme("alpha"), make_theme("beta")];
        let (picks, missing) = picks_from_rc_content("extends = beta\n", &themes);
        assert_eq!(picks.both, Some(1));
        assert_eq!(picks.dark, None);
        assert_eq!(picks.light, None);
        assert!(missing.is_empty());
    }

    #[test]
    fn picks_from_rc_per_mode_extends() {
        let themes = [make_theme("alpha"), make_theme("beta")];
        let (picks, missing) =
            picks_from_rc_content("extends.dark = alpha\nextends.light = beta\n", &themes);
        assert_eq!(picks.both, None);
        assert_eq!(picks.dark, Some(0));
        assert_eq!(picks.light, Some(1));
        assert!(missing.is_empty());
    }

    #[test]
    fn picks_from_rc_same_per_mode_consolidates_into_both() {
        let themes = [make_theme("alpha")];
        let (picks, _) =
            picks_from_rc_content("extends.dark = alpha\nextends.light = alpha\n", &themes);
        assert_eq!(picks.both, Some(0));
    }

    #[test]
    fn picks_from_rc_global_with_one_per_mode_override() {
        // Resolver fallback semantics: dark uses extends.dark, light falls
        // back to the global extends. So picks reflect the *effective*
        // theme each mode would resolve to.
        let themes = [make_theme("alpha"), make_theme("beta")];
        let (picks, _) = picks_from_rc_content("extends = alpha\nextends.dark = beta\n", &themes);
        assert_eq!(picks.both, None);
        assert_eq!(picks.dark, Some(1)); // beta
        assert_eq!(picks.light, Some(0)); // alpha (from global)
    }

    #[test]
    fn picks_from_rc_unknown_theme_yields_none_and_reports_missing() {
        let themes = [make_theme("alpha")];
        let (picks, missing) = picks_from_rc_content("extends = missing-theme\n", &themes);
        assert_eq!(picks.both, None);
        assert_eq!(picks.dark, None);
        assert_eq!(picks.light, None);
        assert_eq!(missing, vec!["missing-theme".to_string()]);
    }

    #[test]
    fn picks_from_rc_missing_per_mode_extends_dedupes() {
        // Both dark and light references the same missing name — should
        // only appear once in the missing list.
        let themes = [make_theme("alpha")];
        let (_picks, missing) =
            picks_from_rc_content("extends.dark = gone\nextends.light = gone\n", &themes);
        assert_eq!(missing, vec!["gone".to_string()]);
    }

    #[test]
    fn hex_to_rgb_round_trip() {
        assert_eq!(hex_to_rgb("#abcdef"), (0xab, 0xcd, 0xef));
        assert_eq!(hex_to_rgb("#000000"), (0, 0, 0));
        assert_eq!(hex_to_rgb("#ffffff"), (0xff, 0xff, 0xff));
    }

    // --- rewrite_extends edge cases ---

    #[test]
    fn rewrite_extends_normalizes_crlf_to_lf() {
        // `str::lines` accepts both line endings but we always emit LF —
        // applying to a CRLF file flips it. Lock the behavior.
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
        // `[dark` is not a valid section header (no `]`), so the rewriter
        // leaves it in the base section. The following extends.dark line
        // gets stripped because `in_base` is still true — that's an
        // edge-case interaction worth pinning down so a future refactor
        // doesn't subtly change behavior on malformed input.
        let input = "[dark\nextends.dark = something\nfg = #ffffff\n";
        let out = rewrite_extends(input, None, Some("nord"), None);
        assert_eq!(out, "extends.dark = nord\n\n[dark\nfg = #ffffff\n");
    }

    // --- apply() end-to-end via tempdir (filesystem-touching path) ---

    use tempfile::tempdir;

    fn make_app_with_picks(picks: Picks, tmp: &std::path::Path) -> App {
        let themes = vec![
            ThemeEntry {
                name: ThemeName::parse("ayu").unwrap(),
                layer: ThemeLayer::default(),
                installed: true,
                bundled: false,
            },
            ThemeEntry {
                name: ThemeName::parse("nord").unwrap(),
                layer: ThemeLayer::default(),
                installed: true,
                bundled: false,
            },
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        App {
            themes,
            list_state,
            picks,
            rc_path: tmp.join(".colorantrc"),
            themes_dir: tmp.join("themes"),
            applied: None,
        }
    }

    #[test]
    fn apply_creates_rc_when_missing() {
        let dir = tempdir().unwrap();
        let mut picks = Picks::default();
        picks.toggle_both(0); // ayu
        let mut app = make_app_with_picks(picks, dir.path());
        apply(&mut app).unwrap();
        let content = fs::read_to_string(&app.rc_path).unwrap();
        assert_eq!(content, "extends = ayu\n");
        assert_eq!(app.applied.as_deref(), Some(app.rc_path.as_path()));
    }

    #[test]
    fn apply_preserves_existing_base_keys_and_sections() {
        let dir = tempdir().unwrap();
        let rc = dir.path().join(".colorantrc");
        fs::write(
            &rc,
            "extends = old\nfg = #ffffff\n[dark]\ncursor = #ff00ff\n",
        )
        .unwrap();
        let mut picks = Picks::default();
        picks.toggle_dark(1); // nord
        let mut app = make_app_with_picks(picks, dir.path());
        apply(&mut app).unwrap();
        let content = fs::read_to_string(&rc).unwrap();
        // Old `extends = old` is replaced with `extends.dark = nord`; the
        // base-section `fg` and the `[dark]` section block are preserved.
        assert!(content.contains("extends.dark = nord"));
        assert!(content.contains("fg = #ffffff"));
        assert!(content.contains("[dark]"));
        assert!(content.contains("cursor = #ff00ff"));
        assert!(!content.contains("extends = old"));
    }

    #[test]
    fn apply_is_noop_when_no_picks() {
        let dir = tempdir().unwrap();
        let app_picks = Picks::default();
        let mut app = make_app_with_picks(app_picks, dir.path());
        apply(&mut app).unwrap();
        // No rc written, applied flag stays None so the post-exit summary
        // is suppressed.
        assert!(!app.rc_path.exists());
        assert_eq!(app.applied, None);
    }

    #[test]
    fn apply_errors_when_theme_missing_and_not_bundled() {
        let dir = tempdir().unwrap();
        let mut picks = Picks::default();
        picks.toggle_both(0);
        let mut app = make_app_with_picks(picks, dir.path());
        // Force the theme to look uninstalled + not bundled so apply has
        // to error rather than auto-install.
        app.themes[0].installed = false;
        app.themes[0].bundled = false;
        let err = apply(&mut app).unwrap_err();
        assert!(err.to_string().contains("not installed and not bundled"));
        // No partial write.
        assert!(!app.rc_path.exists());
    }
}
