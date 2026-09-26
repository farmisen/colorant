# colorant Project Guide

colorant is a per-directory terminal theme switcher. It walks up from the cwd to the
nearest `.colorantrc`, applies that theme via xterm OSC sequences, resets when you leave
the tree, and follows the OS dark/light mode. Supported today: Ghostty + iTerm2 +
WezTerm, zsh, macOS. The support matrix lives in the README Status section; keep it
current.

## Critical rules

- **Never `git commit` or `git push` without explicit approval for this change.** Applies to all branches.
- **Never merge a pull request.** Opening the PR is where the work stops (merges are squash-and-merge, done by the user).
- **Never commit secrets or env files.** `.env*` stays untracked.
- **Never bump the version by hand.** `Cargo.toml`, `Cargo.lock` and the README's pinned `--tag` are bumped by the `release-bump` workflow (manual dispatch), which also tags and triggers the cargo-dist release.
- **Ask before adding a dependency.**
- **No `#[allow(...)]` to silence clippy without explicit user confirmation.** Fix the code instead.

## Workflow

- One branch per work item, cut from an up-to-date `main`: `farmisen/<feature|fix|refactor|docs|chore>/<kebab-summary>`.
- Outline the approach and discuss it with the user before implementing.
- Minimal code: the least code that solves the task within existing patterns. Nothing that wasn't asked for.
- Before presenting: fmt, clippy and tests pass; new behavior has tests; `README.md` is updated if user-facing behavior changed (command, flag, config key, install step).
- Present a summary and get approval before committing; iterate on requested changes.
- Commit message: one imperative line. No body, no conventional-commit prefix (`feat:`), no attribution trailer (no `Co-Authored-By:`, no "Generated with" line).
- Before opening the PR: `git fetch origin main && git rebase origin/main`, then re-run checks on the rebased tree.
- PR: plain descriptive title; body is a Summary of what changed and why. No test plan, no attribution trailer (no `Co-Authored-By:`, no "Generated with" line).

## Commands

- Build: `cargo build` (release: `cargo build --release`)
- Run: `cargo run -- <subcommand>`, e.g. `cargo run -- init zsh`, `COLORANT_MODE=dark cargo run -- show --all`
- Test (all): `cargo test --all-features` (what CI runs)
- Test (single): `cargo test <name_substring>`; integration only: `cargo test --test integration <name>`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`
- Format: `cargo fmt --all` (CI checks with `cargo fmt --all -- --check`)

Proven = fmt check, clippy and tests clean on **stable** (CI's `test` job on ubuntu).
CI also compiles on macos-14 (`cargo build --all-features`, no tests), so macOS-only code
is compile-checked but never exercised.

## Layout

- `src/main.rs`, `src/cli.rs`: clap setup and routing. `src/commands/` has one module per subcommand (apply, reset, current, init, doctor, show, `themes/` including the ratatui TUI).
- `src/theme/`: model (HexColor, ThemeLayer, Mode), parse (`.colorant` palettes + `.colorantrc` with `[dark]`/`[light]`), resolve (extends chain + per-mode merge), rc (in-place `.colorantrc` edits), source/bundled/gogh (where themes come from).
- `src/terminal/`: `osc.rs` OSC emitter with tmux DCS wrapping, `utils.rs` terminal detection, `style.rs` ANSI output helpers. `src/shell/zsh.rs`: the `init zsh` hook snippet.
- `src/config.rs` global config, `src/mode.rs` dark/light detection, `src/walk.rs` nearest-rc lookup.
- `themes/*.colorant`: bundled palettes, embedded at compile time by `build.rs`. Adding a palette means dropping a file here.
- `tests/integration.rs`: end-to-end tests that drive the binary as a subprocess.

## Code conventions (not enforced by tooling)

- `anyhow::Result` at the binary boundary (`main.rs`, `commands/`); `thiserror` types in deeper modules when callers match on the error kind.
- No `.unwrap()` / `.expect()` outside tests unless the type system enforces the invariant.
- `let ... else` to keep happy paths flat instead of nested `if let`.
- `///` on every public item; `//!` module docs saying what the module does and where it sits in the flow.
- Unit tests inline (`#[cfg(test)] mod tests`). Anything observable through the CLI (new subcommand, flag, precedence rule) also gets an integration test.

## Tracker & status

Work items live in GitHub Issues (`gh issue list`, `gh issue view <n>`).

## Project-specific gotchas

- The config dir is XDG-style on every OS: `$XDG_CONFIG_HOME/colorant`, else `$HOME/.config/colorant`, with themes in `<config>/themes/`. Never use `dirs::config_dir()` (it gives `~/Library/Application Support` on macOS); `dirs` is for `home_dir` only. Same for the cache dir (`XDG_CACHE_HOME`).
- Dark/light detection is macOS-only (`cfg(target_os = "macos")`); elsewhere `mode::detect` returns `Unknown`. Tests that depend on mode must set `COLORANT_MODE=dark|light`.
- The zsh hook runs `colorant apply` on every `chpwd` and `precmd` with stderr discarded. `apply` must stay fast, offline and non-interactive, and must be a silent no-op on unsupported terminals. Network access happens only in the Gogh paths (`themes sync`, on-demand fetches in `themes apply`/`install` and the TUI preview). Tests must never hit the network.
- `build.rs` duplicates `PALETTE_EXTENSION` from `src/theme/resolve.rs`; change both together.
- The toolchain is unpinned (no `rust-toolchain.toml`) and CI uses whatever `stable` is current. A new stable release can break clippy on untouched code. Check clippy with `cargo +stable` (local default may be nightly, which flags even more). Fix new stable lints in a separate change, not mixed into unrelated work.

<!-- bootstrap-project · axes: rust + cli · 2026-09-25 -->
