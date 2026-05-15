//! Interactive TUI for `colorant themes` (no sub-action).
//!
//! Launches a ratatui app that lists themes from every known source —
//! installed (in `base_theme_dir`), bundled (compiled in), and remote
//! (Gogh, when its catalog has been synced) — with a live preview of
//! each palette's colors. The user assigns themes to one or more of
//! three slots — `both` / `dark` / `light` — and on apply the cwd's
//! `.colorantrc` is updated with the corresponding `extends*` keys.
//! Themes that aren't yet on disk (bundled or remote) get installed
//! as part of apply. Other keys in the rc are preserved.

use crate::config::{Config, THEME_FILE_NAME};
use crate::theme::bundled::BUNDLED_THEMES;
use crate::theme::gogh;
use crate::theme::model::{HexColor, ThemeLayer, ThemeName};
use crate::theme::parse::{parse_palette_str, parse_rc_str};
use crate::theme::rc::rewrite_extends;
use crate::theme::resolve::PALETTE_EXTENSION;
use crate::theme::source::Source;
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
///
/// `origin` is where the theme is known from — `Some(Bundled)` /
/// `Some(Gogh)` for entries that appeared in a catalog, `None` for
/// installed-on-disk palettes whose origin we can't determine.
///
/// `layer` is `Some` when we have the colors loaded — either eagerly
/// (bundled / installed) or after a successful lazy fetch (gogh). It's
/// `None` for gogh entries that haven't been fetched yet during this TUI
/// session. `fetch_failed` short-circuits retries on every navigation
/// when a fetch errored (typically: offline).
struct ThemeEntry {
    name: ThemeName,
    origin: Option<Source>,
    layer: Option<ThemeLayer>,
    installed: bool,
    fetch_failed: bool,
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
    /// Indices into `themes` for the rows currently shown in the list,
    /// reflecting `source_filter` + `filter`. The list cursor (`list_state`)
    /// indexes into this `visible` slice, not into `themes` directly.
    /// Recomputed by [`App::recompute_visible`] whenever a filter changes.
    visible: Vec<usize>,
    list_state: ListState,
    picks: Picks,
    /// Source filter cycle: `None` = all sources, `Some(s)` = restrict to `s`.
    source_filter: Option<Source>,
    /// Case-insensitive substring filter applied to theme names. Empty
    /// means no text filter.
    filter: String,
    /// True while the user is typing into the filter input (`/` mode).
    /// Key handling routes differently in this mode.
    editing_filter: bool,
    rc_path: PathBuf,
    themes_dir: PathBuf,
    /// Filled in only on apply: the path actually written, for the final
    /// summary printed after the TUI exits.
    applied: Option<PathBuf>,
}

impl App {
    /// Theme at the current cursor position, if any. Resolves through
    /// the visible-index cache so we honor the active filter.
    fn selected(&self) -> Option<&ThemeEntry> {
        let v = self.list_state.selected()?;
        let i = *self.visible.get(v)?;
        self.themes.get(i)
    }

    /// Original (themes-vec) index of the currently selected entry.
    fn selected_theme_idx(&self) -> Option<usize> {
        let v = self.list_state.selected()?;
        self.visible.get(v).copied()
    }

    fn assign_selected_to<F: FnOnce(&mut Picks, usize)>(&mut self, set: F) {
        if let Some(i) = self.selected_theme_idx() {
            set(&mut self.picks, i);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0) as isize;
        let len = self.visible.len() as isize;
        let next = (cur + delta).rem_euclid(len);
        self.list_state.select(Some(next as usize));
    }

    /// Rebuild `visible` based on the current source + text filters. Keeps
    /// the cursor on the same theme when it survives the filter; otherwise
    /// clamps to the first matching row.
    fn recompute_visible(&mut self) {
        let prior_theme_idx = self.selected_theme_idx();
        let needle = self.filter.to_lowercase();
        self.visible = self
            .themes
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                if let Some(src) = self.source_filter
                    && t.origin != Some(src)
                {
                    return false;
                }
                if !needle.is_empty() && !t.name.as_str().to_lowercase().contains(&needle) {
                    return false;
                }
                true
            })
            .map(|(i, _)| i)
            .collect();
        // Try to preserve the prior selection; else snap to row 0 (or
        // None when the filter eliminated every row).
        let new_pos = prior_theme_idx
            .and_then(|t| self.visible.iter().position(|i| *i == t))
            .or(if self.visible.is_empty() {
                None
            } else {
                Some(0)
            });
        self.list_state.select(new_pos);
    }

    /// Cycle the source filter: None → Bundled → Gogh → None → …
    fn cycle_source_filter(&mut self) {
        self.source_filter = match self.source_filter {
            None => Some(Source::Bundled),
            Some(Source::Bundled) => Some(Source::Gogh),
            Some(Source::Gogh) => None,
        };
        self.recompute_visible();
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
        visible: Vec::new(),
        list_state: ListState::default(),
        picks,
        source_filter: None,
        filter: String::new(),
        editing_filter: false,
        rc_path,
        themes_dir: config.base_theme_dir.clone(),
        applied: None,
    };
    app.recompute_visible();

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
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if app.editing_filter {
                    handle_filter_key(app, key.code);
                } else {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('j') | KeyCode::Down => {
                            app.move_cursor(1);
                            ensure_loaded_selected(app);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            app.move_cursor(-1);
                            ensure_loaded_selected(app);
                        }
                        KeyCode::Char('g') | KeyCode::Home if !app.visible.is_empty() => {
                            app.list_state.select(Some(0));
                            ensure_loaded_selected(app);
                        }
                        KeyCode::Char('G') | KeyCode::End if !app.visible.is_empty() => {
                            let last = app.visible.len().saturating_sub(1);
                            app.list_state.select(Some(last));
                            ensure_loaded_selected(app);
                        }
                        KeyCode::Char('b') => app.assign_selected_to(|p, i| p.toggle_both(i)),
                        KeyCode::Char('d') => app.assign_selected_to(|p, i| p.toggle_dark(i)),
                        KeyCode::Char('l') => app.assign_selected_to(|p, i| p.toggle_light(i)),
                        KeyCode::Char('c') => app.picks.clear(),
                        KeyCode::Char('s') => app.cycle_source_filter(),
                        KeyCode::Char('/') => {
                            app.editing_filter = true;
                        }
                        KeyCode::Enter => {
                            apply(app)?;
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// Key handling while `/` filter mode is active. Typing appends to the
/// filter, backspace removes a char, Esc cancels (clears the filter and
/// exits), Enter commits (keeps the filter but exits the input mode).
fn handle_filter_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.filter.clear();
            app.editing_filter = false;
            app.recompute_visible();
            ensure_loaded_selected(app);
        }
        KeyCode::Enter => {
            app.editing_filter = false;
            ensure_loaded_selected(app);
        }
        KeyCode::Backspace => {
            app.filter.pop();
            app.recompute_visible();
        }
        KeyCode::Char(c) => {
            app.filter.push(c);
            app.recompute_visible();
        }
        _ => {}
    }
}

/// Ensure the currently-selected theme has its palette loaded. For Gogh
/// entries whose layer hasn't been fetched yet, this issues one network
/// request and caches the result back into the entry. Errors set
/// `fetch_failed` so we don't retry on every navigation — the preview
/// shows "(preview unavailable — offline?)" until the user re-navigates
/// to a working entry.
fn ensure_loaded_selected(app: &mut App) {
    let Some(idx) = app.selected_theme_idx() else {
        return;
    };
    let entry = &app.themes[idx];
    if entry.layer.is_some() || entry.fetch_failed {
        return;
    }
    let Some(Source::Gogh) = entry.origin else {
        return; // Nothing to do for non-remote sources.
    };
    let name = entry.name.as_str().to_string();
    match gogh::fetch(&name) {
        Ok(palette) => {
            if let Some(e) = app.themes.get_mut(idx) {
                e.layer = Some(palette.layer);
            }
        }
        Err(_) => {
            if let Some(e) = app.themes.get_mut(idx) {
                e.fetch_failed = true;
            }
        }
    }
}

fn draw(frame: &mut ratatui::Frame, app: &App) {
    let area = frame.area();
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" colorant themes ");
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
    draw_keybinds(frame, app, chunks[2]);
}

fn draw_theme_list(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| {
            let t = &app.themes[i];
            let installed = if t.installed { "*" } else { " " };
            // Short single-letter source marker before the name so users
            // can tell bundled from remote at a glance.
            let origin_marker = match t.origin {
                Some(Source::Bundled) => "B",
                Some(Source::Gogh) => "G",
                None => "·",
            };
            let tag = app.picks.slot_tag(i);
            ListItem::new(format!("{installed}{origin_marker} {}{tag}", t.name))
        })
        .collect();

    // Title reflects the current source filter so the user knows what
    // they're looking at when the list is shorter than expected.
    let source_label = match app.source_filter {
        None => "all".to_string(),
        Some(s) => s.to_string(),
    };
    let title = format!(" Themes ({} / {source_label}) ", app.visible.len());
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

    let lines: Vec<Line> = match (&theme.layer, theme.fetch_failed) {
        (Some(layer), _) => {
            let mut out = vec![
                swatch_line("fg", layer.fg.as_ref()),
                swatch_line("bg", layer.bg.as_ref()),
                swatch_line("cursor", layer.cursor.as_ref()),
                Line::default(),
            ];
            for i in 0..8 {
                out.push(palette_row(layer, i));
            }
            out
        }
        (None, true) => vec![Line::from(Span::styled(
            "  (preview unavailable — offline?)",
            Style::default().fg(Color::DarkGray),
        ))],
        (None, false) => vec![Line::from(Span::styled(
            "  (palette not loaded yet)",
            Style::default().fg(Color::DarkGray),
        ))],
    };

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

fn draw_keybinds(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    // While the filter input is active, show a different hint and the
    // current filter buffer so the user can see what they're typing.
    let line = if app.editing_filter {
        Line::from(vec![
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(app.filter.clone()),
            Span::styled(
                "    (esc=cancel  enter=commit)",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    } else {
        let filter_indicator = if app.filter.is_empty() {
            String::new()
        } else {
            format!("  /{}  ", app.filter)
        };
        Line::from(Span::styled(
            format!(
                "j/k=nav  b=both  d=dark  l=light  c=clear  /=filter  s=source  enter=apply  q=quit{filter_indicator}"
            ),
            Style::default().fg(Color::DarkGray),
        ))
    };
    frame.render_widget(Paragraph::new(line), area);
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

fn palette_row(layer: &ThemeLayer, row: usize) -> Line<'static> {
    let left = swatch_inline(&color_name(row), layer.palette[row].as_ref());
    let right = swatch_inline(&color_name(row + 8), layer.palette[row + 8].as_ref());
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
    // Install any picked theme that isn't yet on disk: bundled themes are
    // copied from the in-memory blob; gogh themes are fetched if their
    // palette hasn't already been loaded into the entry during browsing.
    for i in [both_idx, dark_idx, light_idx].into_iter().flatten() {
        let theme = &app.themes[i];
        if theme.installed {
            continue;
        }
        match theme.origin {
            Some(Source::Bundled) => install_bundled_palette(&theme.name, &app.themes_dir)?,
            Some(Source::Gogh) => install_gogh_palette(theme, &app.themes_dir)?,
            None => {
                return Err(anyhow!(
                    "theme {} is not installed and has no known origin",
                    theme.name
                ));
            }
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

/// Install a Gogh palette into `themes_dir`. Reuses the already-fetched
/// layer when the user previewed it during this TUI session (no extra
/// network); otherwise fetches now. The written file is colorant's flat
/// `.colorant` format — same shape `parse_palette_str` reads.
fn install_gogh_palette(entry: &ThemeEntry, themes_dir: &Path) -> Result<()> {
    let layer = match &entry.layer {
        Some(l) => l.clone(),
        None => {
            gogh::fetch(entry.name.as_str())
                .with_context(|| format!("fetching gogh theme {}", entry.name))?
                .layer
        }
    };
    fs::create_dir_all(themes_dir)
        .with_context(|| format!("creating themes dir {}", themes_dir.display()))?;
    let dest = themes_dir.join(format!("{}.{}", entry.name, PALETTE_EXTENSION));
    let content = render_palette_layer(&layer);
    fs::write(&dest, content).with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

/// Render a `ThemeLayer` as the flat `.colorant` key/value format used on
/// disk. Skips slots that aren't set so a partial Gogh theme rounds-trips
/// without empty `colorN = ` lines.
fn render_palette_layer(layer: &ThemeLayer) -> String {
    let mut out = String::new();
    if let Some(c) = &layer.fg {
        out.push_str(&format!("fg = {}\n", c.as_str()));
    }
    if let Some(c) = &layer.bg {
        out.push_str(&format!("bg = {}\n", c.as_str()));
    }
    if let Some(c) = &layer.cursor {
        out.push_str(&format!("cursor = {}\n", c.as_str()));
    }
    for (i, slot) in layer.palette.iter().enumerate() {
        if let Some(c) = slot {
            out.push_str(&format!("color{i} = {}\n", c.as_str()));
        }
    }
    out
}

/// Build the merged theme list from every known source (bundled, gogh
/// cache, and what's already installed under `base_theme_dir`). Sorted by
/// name. Installed entries override catalog entries so the preview
/// reflects whatever's actually on disk.
///
/// Gogh themes whose names can't be ThemeName-parsed (spaces, parens, …)
/// are dropped at this stage — they'd fail later when written to the rc.
/// TODO: relax `ThemeName` to accept Gogh's broader naming so we don't
/// strand entries like `3024 Day` or `Flatland (Palenight)`.
fn load_themes(config: &Config) -> Result<Vec<ThemeEntry>> {
    let mut entries: BTreeMap<String, ThemeEntry> = BTreeMap::new();
    let mut gogh_skipped: usize = 0;

    // Bundled (compiled in). Loaded eagerly — they're already in memory.
    for (name, content) in BUNDLED_THEMES {
        let Ok(theme_name) = ThemeName::parse(name) else {
            continue;
        };
        entries.insert(
            name.to_string(),
            ThemeEntry {
                name: theme_name,
                origin: Some(Source::Bundled),
                layer: Some(parse_palette_str(content).layer),
                installed: false,
                fetch_failed: false,
            },
        );
    }

    // Gogh themes from the cached catalog (no network here — `themes sync`
    // populated it). The palette stays unloaded; we fetch lazily when the
    // user navigates to one. Bundled themes with the same name keep their
    // entry — `or_insert` doesn't overwrite.
    if let Ok(Some(names)) = gogh::cached_names() {
        for name in names {
            let Ok(theme_name) = ThemeName::parse(&name) else {
                gogh_skipped += 1;
                continue;
            };
            entries.entry(name.clone()).or_insert(ThemeEntry {
                name: theme_name,
                origin: Some(Source::Gogh),
                layer: None,
                installed: false,
                fetch_failed: false,
            });
        }
    }

    // Installed on disk under base_theme_dir. Overrides anything above
    // with the on-disk content (so user edits to a bundled theme show
    // through). Origin is preserved when the name matches a catalog entry.
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
            let prior_origin = entries.get(stem).and_then(|e| e.origin);
            let origin = prior_origin.or_else(|| {
                if BUNDLED_THEMES.iter().any(|(n, _)| *n == stem) {
                    Some(Source::Bundled)
                } else {
                    None
                }
            });
            entries.insert(
                stem.to_string(),
                ThemeEntry {
                    name,
                    origin,
                    layer: Some(parse_palette_str(&content).layer),
                    installed: true,
                    fetch_failed: false,
                },
            );
        }
    }

    if gogh_skipped > 0 {
        eprintln!(
            "note: {gogh_skipped} gogh theme(s) skipped (names contain characters \
             colorant can't yet handle, e.g. spaces or parens)"
        );
    }

    Ok(entries.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
            origin: Some(Source::Bundled),
            layer: Some(ThemeLayer::default()),
            installed: false,
            fetch_failed: false,
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

    // --- apply() end-to-end via tempdir (filesystem-touching path) ---

    use tempfile::tempdir;

    fn make_app_with_picks(picks: Picks, tmp: &std::path::Path) -> App {
        let themes = vec![
            ThemeEntry {
                name: ThemeName::parse("ayu").unwrap(),
                origin: None,
                layer: Some(ThemeLayer::default()),
                installed: true,
                fetch_failed: false,
            },
            ThemeEntry {
                name: ThemeName::parse("nord").unwrap(),
                origin: None,
                layer: Some(ThemeLayer::default()),
                installed: true,
                fetch_failed: false,
            },
        ];
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        let visible: Vec<usize> = (0..themes.len()).collect();
        App {
            themes,
            visible,
            list_state,
            picks,
            source_filter: None,
            filter: String::new(),
            editing_filter: false,
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
    fn apply_errors_when_theme_missing_and_origin_unknown() {
        let dir = tempdir().unwrap();
        let mut picks = Picks::default();
        picks.toggle_both(0);
        let mut app = make_app_with_picks(picks, dir.path());
        // Force the theme to look uninstalled with no known origin —
        // apply should error rather than fabricate one.
        app.themes[0].installed = false;
        app.themes[0].origin = None;
        let err = apply(&mut app).unwrap_err();
        assert!(err.to_string().contains("no known origin"));
        // No partial write.
        assert!(!app.rc_path.exists());
    }

    // --- filter + source cycle ---

    fn entry(name: &str, origin: Option<Source>) -> ThemeEntry {
        ThemeEntry {
            name: ThemeName::parse(name).unwrap(),
            origin,
            layer: Some(ThemeLayer::default()),
            installed: false,
            fetch_failed: false,
        }
    }

    fn app_with_themes(themes: Vec<ThemeEntry>) -> App {
        let visible: Vec<usize> = (0..themes.len()).collect();
        let mut state = ListState::default();
        if !themes.is_empty() {
            state.select(Some(0));
        }
        let mut app = App {
            themes,
            visible,
            list_state: state,
            picks: Picks::default(),
            source_filter: None,
            filter: String::new(),
            editing_filter: false,
            rc_path: PathBuf::from("/tmp/.colorantrc"),
            themes_dir: PathBuf::from("/tmp/themes"),
            applied: None,
        };
        app.recompute_visible();
        app
    }

    #[test]
    fn cycle_source_filter_rotates_none_bundled_gogh() {
        let mut app = app_with_themes(vec![
            entry("alpha", Some(Source::Bundled)),
            entry("beta", Some(Source::Gogh)),
            entry("gamma", None),
        ]);
        assert_eq!(app.source_filter, None);
        app.cycle_source_filter();
        assert_eq!(app.source_filter, Some(Source::Bundled));
        // Visible drops to only the bundled entry (alpha).
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.themes[app.visible[0]].name.as_str(), "alpha");
        app.cycle_source_filter();
        assert_eq!(app.source_filter, Some(Source::Gogh));
        assert_eq!(app.themes[app.visible[0]].name.as_str(), "beta");
        app.cycle_source_filter();
        assert_eq!(app.source_filter, None);
        assert_eq!(app.visible.len(), 3);
    }

    #[test]
    fn text_filter_narrows_visible_case_insensitively() {
        let mut app = app_with_themes(vec![
            entry("Catppuccin-Mocha", Some(Source::Bundled)),
            entry("catppuccin-latte", Some(Source::Bundled)),
            entry("tokyo-night", Some(Source::Bundled)),
        ]);
        app.filter = "MOCHA".to_string();
        app.recompute_visible();
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.themes[app.visible[0]].name.as_str(), "Catppuccin-Mocha");
    }

    #[test]
    fn text_filter_and_source_filter_combine() {
        let mut app = app_with_themes(vec![
            entry("catppuccin-mocha", Some(Source::Bundled)),
            entry("catppuccin-mocha-2", Some(Source::Gogh)),
            entry("tokyo-night", Some(Source::Bundled)),
        ]);
        app.source_filter = Some(Source::Bundled);
        app.filter = "catppuccin".to_string();
        app.recompute_visible();
        // Only the bundled catppuccin survives — gogh is filtered out
        // by source, tokyo is filtered out by text.
        assert_eq!(app.visible.len(), 1);
        assert_eq!(app.themes[app.visible[0]].name.as_str(), "catppuccin-mocha");
    }

    #[test]
    fn recompute_visible_preserves_cursor_when_possible() {
        let mut app = app_with_themes(vec![
            entry("alpha", Some(Source::Bundled)),
            entry("beta", Some(Source::Bundled)),
            entry("gamma", Some(Source::Bundled)),
        ]);
        app.list_state.select(Some(1)); // beta
        app.filter = "bet".to_string();
        app.recompute_visible();
        // beta survives, cursor stays on it (now row 0 since list shrank).
        assert_eq!(app.list_state.selected(), Some(0));
        assert_eq!(app.themes[app.visible[0]].name.as_str(), "beta");
    }

    #[test]
    fn recompute_visible_clamps_when_cursor_filtered_out() {
        let mut app = app_with_themes(vec![
            entry("alpha", Some(Source::Bundled)),
            entry("beta", Some(Source::Bundled)),
        ]);
        app.list_state.select(Some(1)); // beta
        app.filter = "alph".to_string();
        app.recompute_visible();
        // beta got filtered out; cursor snaps to first surviving row.
        assert_eq!(app.list_state.selected(), Some(0));
        assert_eq!(app.themes[app.visible[0]].name.as_str(), "alpha");
    }

    #[test]
    fn recompute_visible_with_no_matches_clears_cursor() {
        let mut app = app_with_themes(vec![entry("alpha", Some(Source::Bundled))]);
        app.filter = "definitely-no-match".to_string();
        app.recompute_visible();
        assert!(app.visible.is_empty());
        assert_eq!(app.list_state.selected(), None);
    }
}
