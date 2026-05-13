# colorant Project Guide

**Product:** colorant — per-directory terminal theme switcher with system dark/light mode support
**Repo:** https://github.com/farmisen/colorant

## Critical Rules

- **NEVER push to GitHub without explicit user approval.** Always ask before running `git push`. This applies to all branches.

## Project Overview

colorant walks up from the current working directory looking for a `.colorantrc` file and applies the theme it describes to the terminal via standard xterm OSC escape sequences. When the user `cd`s out of a themed tree, the colors reset. When the OS flips between dark and light mode, the active theme follows.

v1 scope is intentionally narrow: **Ghostty + zsh + macOS**. Additional terminals (Kitty, iTerm2, WezTerm, Alacritty), shells (bash, fish), and operating systems (Linux) land incrementally after v1.

## Tech Stack

- **Language:** Rust (edition 2024, MSRV 1.88)
- **CLI parsing:** clap (derive feature)
- **Errors:** anyhow at the binary entrypoint, thiserror for typed library errors when matching matters
- **Config:** serde + toml
- **Paths:** dirs (used for `home_dir` only; the config directory is resolved via XDG conventions, not the macOS default of `~/Library/Application Support`)
- **Tests:** cargo test — unit tests inline (`#[cfg(test)] mod tests`), integration tests under `tests/` driving the binary as a subprocess
- **CI:** GitHub Actions — `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`

## Project Structure

```
colorant/
├── Cargo.toml
├── README.md
├── LICENSE
├── .github/workflows/ci.yml
├── src/
│   ├── main.rs            # clap setup, command routing
│   ├── cli.rs             # CLI argument types
│   ├── config.rs          # global config (~/.config/colorant/config.toml)
│   ├── mode.rs            # system dark/light detection (macOS for now)
│   ├── walk.rs            # find-nearest .colorantrc
│   ├── theme/
│   │   ├── model.rs       # HexColor, ThemeLayer, ParsedTheme, Mode
│   │   ├── parse.rs       # .colorantrc parser with [dark]/[light] sections
│   │   └── resolve.rs     # extends-chain + per-mode merge
│   ├── terminal/
│   │   └── osc.rs         # OSC escape sequence emitter, tmux DCS-wrapping
│   ├── shell/
│   │   └── zsh.rs         # zsh hook generator for `colorant init zsh`
│   └── commands/
│       ├── apply.rs
│       ├── reset.rs
│       ├── init.rs
│       └── current.rs
├── themes/                # bundled default themes (`.colorantrc` format)
└── tests/
    └── integration.rs     # end-to-end tests that drive the binary
```

## Build & Development Commands

- Build: `cargo build`
- Build release: `cargo build --release`
- Run: `cargo run -- <subcommand>` (e.g. `cargo run -- init zsh`)
- Test all: `cargo test`
- Test specific: `cargo test name_of_test`
- Format: `cargo fmt --all`
- Format check: `cargo fmt --all -- --check`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`

## Implementation Philosophy

**Minimal Code Principle**: Each feature/fix should be done with the **least amount of code possible** while:

- Strictly adhering to existing codebase patterns and conventions
- Maintaining the highest standards of software engineering
- **NEVER** implementing anything not explicitly required for the current task
- Avoiding premature optimization or over-engineering
- Following YAGNI (You Aren't Gonna Need It)

## Code Style

### Rust

- Follow standard `cargo fmt` formatting — no exceptions.
- `snake_case` for functions/variables/modules, `PascalCase` for types, `SCREAMING_SNAKE_CASE` for constants.
- Prefer `Result<T>` over panics in library code; reserve panics for genuine programmer errors that should never reach a release build.
- Use `anyhow::Result` at the binary boundary (`main.rs`, `commands/`) and `thiserror` types in deeper library modules when callers may want to match on the error kind.
- Pattern-match and use `let ... else { return ... };` to keep happy paths flat instead of nesting `if let` blocks.
- `///` doc comments on all public items. `//!` module-level doc comments explaining the module's purpose and where it sits in the larger flow.
- Keep functions focused — extract helpers when a function exceeds ~40 lines.
- Avoid `.unwrap()` / `.expect()` outside of tests unless the invariant is enforced by the type system (e.g., a value just parsed by a validating constructor).
- Tests: inline `#[cfg(test)] mod tests` for unit tests close to the code under test; `tests/` for integration tests that drive the binary as a subprocess.
- Never add `#[allow(...)]` to silence a clippy lint without explicit user confirmation — fix the underlying issue instead.

## Environment Configuration

- `~/.config/colorant/config.toml` — optional global config. Resolves `$XDG_CONFIG_HOME/colorant` first, falls back to `$HOME/.config/colorant`. All fields are optional; missing values use sensible defaults.
- `~/.config/colorant/themes/` — directory holding named themes that `.colorantrc` files reference via `extends`.
- `COLORANT_MODE` env var — forces `dark` or `light` regardless of OS state. Useful for tests and for users who want a fixed mode.

## Development Workflow

**ALWAYS** follow this workflow unless told otherwise. **Never** skip a step unless told otherwise.

**IMPORTANT**: When starting work on a task, IMMEDIATELY create a TodoWrite list with all these steps as individual todos. Mark each as completed as you progress.

1. **Create Branch**: ALWAYS create a new branch from `main` for each work item. See Branch Management below.
2. **Plan**: Before implementing, outline the approach. Always discuss the plan with the user before proceeding.
3. **Implement Feature/Fix**: Write the code following the Code Style guidelines above.
4. **Run Quality Checks**: ALWAYS run `cargo fmt --all`, then `cargo clippy --all-targets --all-features -- -D warnings`. Fix all issues before proceeding.
5. **Test Your Changes**: ALWAYS run `cargo test` and ensure all tests pass. Write new tests for new functionality. Fix all test failures before proceeding.
6. **Self-Review**: Re-read every changed file in the diff — look for leftover debug code, `dbg!()` / `println!()` calls, TODOs, commented-out code, or hardcoded paths. Verify changes match the requirements. Confirm no unrelated changes are included. If user-facing behavior changed (new command, new config key, install step), update `README.md` as part of this step.
7. **User Review**: Present a summary of the changes to the user — what was implemented, key decisions made, any trade-offs. Show the diff if helpful. Ask for approval or change requests. Do NOT proceed to commit until the user explicitly approves. If changes are requested, go back to step 3 and iterate.
8. **Multi-Agent Review**: For non-trivial changes, run `/pr-review-toolkit:review-pr all parallel` to launch the specialized reviewers. Drop the `parallel` flag for very large PRs where applying early findings should inform later agents. Triage findings by severity (Critical / Important / Suggestion), discuss with the user which to act on, and if any need code changes go back to step 3 and iterate.
9. **Commit**: Generate a clear, concise commit message. See Commit Standards below.
10. **Rebase onto main**: ALWAYS run `git fetch origin main && git rebase origin/main` before opening the PR. Keeps the squash-merge graph clean and surfaces conflicts locally before GitHub renders the PR as `CONFLICTING`. Resolve any conflicts locally and re-run quality checks + tests on the rebased tree before continuing.
11. **Create PR**: Create a pull request. See Pull Request Standards below. Do NOT merge — wait for user approval.

### Workflow Checklist Template

When starting a new work item, create todos with this template:

```
1.  [ ] Create branch from main
2.  [ ] Plan the approach (discuss with user)
3.  [ ] Implement the feature/fix
4.  [ ] Run quality checks (cargo fmt, cargo clippy -D warnings)
5.  [ ] Run tests (cargo test), write new tests, fix failures
6.  [ ] Self-review the diff (and update README if needed)
7.  [ ] Present changes to user, get approval (iterate if needed)
8.  [ ] Run multi-agent review if non-trivial
9.  [ ] Commit with clear message
10. [ ] Rebase onto origin/main; re-run checks on rebased tree
11. [ ] Create PR
```

### Branch Management

- ALWAYS create a new branch from `main` for each work item.
- **Branch naming**: `farmisen/<type>/<description>` where `<type>` is one of `feature`, `fix`, `refactor`, `docs`, `chore` and `<description>` is a short kebab-case summary (e.g., `farmisen/feature/kitty-adapter`, `farmisen/fix/dark-mode-detection`).
- **New branch creation**: ALWAYS pull the latest changes from `main` before creating a new branch (`git pull origin main`).
- **Already on the expected branch**: If you're already on the correct branch, rebase it onto `main`:
  ```bash
  git fetch origin main
  git rebase origin/main
  ```

### Commit Standards

- **Message Format**: Keep commit messages short and concise. Use format: `<brief description>` (e.g., `Add Kitty terminal adapter`).
- **No Co-Authors**: NEVER add co-author attribution to commits. Do NOT include lines like "Co-Authored-By: Claude <noreply@anthropic.com>".
- **No Attribution Links**: Do NOT include attribution links like "Generated with [Claude Code]".
- **Types**: NEVER prefix the commit message with conventional commit types like feat, fix, or docs.
- **Single Line**: Commit messages should be a single line description only.

### Pull Request Standards

- **Title Format**: Use a plain descriptive title (e.g., `Add Kitty terminal adapter`).
- **Description Format**:
  - Start with a Summary section explaining the overall changes and their purpose.
  - Focus on WHAT was changed and WHY, not implementation details.
  - Keep it concise but complete — someone should understand the PR without reading the code.
  - No test plan section needed.
  - No co-author attribution.
- **Squash and Merge**: ALWAYS squash and merge when merging a PR to keep `main`'s history clean.

### Code Quality Standards

- **ALWAYS Check Code Quality**: ALWAYS run `cargo fmt --all` and `cargo clippy --all-targets --all-features -- -D warnings` before committing. Fix all issues — never silence them.
- **Never Silence Warnings**: Do not add `#[allow(...)]` attributes to silence clippy lints without explicit user confirmation — fix the underlying issue instead.
- **Test Coverage**: Write unit tests for parser logic, theme resolution rules, and any pure-function helpers. Write integration tests under `tests/` for any behavior the user can observe through the CLI (new subcommand, new flag, new precedence rule).
