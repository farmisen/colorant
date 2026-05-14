# colorant

[![CI](https://github.com/farmisen/colorant/actions/workflows/ci.yml/badge.svg)](https://github.com/farmisen/colorant/actions/workflows/ci.yml)

Per-directory terminal theme switcher with system dark/light mode support.

`colorant` walks up from your current directory looking for a `.colorantrc`
file and applies the theme it describes to your terminal. When you `cd` out
of the tree, the theme resets. When the OS flips between dark and light, the
active theme follows on the next shell prompt.

## Status

Early development. v1 scope: Ghostty + zsh + macOS. Other terminals, shells,
and OSes will land incrementally.

## Install

Three install paths, all of which ship the same `colorant` binary built for
both Intel and Apple Silicon Macs.

### Homebrew

```sh
brew install farmisen/tap/colorant
```

The tap is auto-added on first install. Easiest path if you already use
Homebrew.

### Shell installer

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/farmisen/colorant/releases/latest/download/colorant-installer.sh | sh
```

No Rust toolchain or Homebrew required. Detects your architecture, downloads
the matching binary, and verifies its checksum. Installs to `$CARGO_HOME/bin`
(typically `~/.cargo/bin`) — make sure that's on your `$PATH`. To install
elsewhere, set `COLORANT_INSTALL_DIR=/some/path` before running.

### From source (cargo)

For Rust developers, or to pin a specific version:

```sh
# latest main
cargo install --git https://github.com/farmisen/colorant.git

# pinned to a release tag
cargo install --git https://github.com/farmisen/colorant.git --tag v0.5.0
```

Installs to `~/.cargo/bin/colorant`.

### Verifying downloads

Each release publishes a `.sha256` file alongside every archive, plus an
aggregated `sha256.sum`. If you download a tarball directly (rather than via
the shell installer, which verifies for you), check it with:

```sh
shasum -a 256 -c colorant-aarch64-apple-darwin.tar.xz.sha256
```

## Quick start

```sh
# 1. Wire the zsh hook into your shell, then restart your shell.
echo 'eval "$(colorant init zsh)"' >> ~/.zshrc
exec zsh

# 2. Install the bundled palettes into your themes dir. (`colorant themes list`
#    enumerates what's available; `--all` copies every bundled palette.)
colorant themes install --all

# 3. Tag a directory with that palette.
cd ~/work/myproject
echo 'extends = catppuccin-mocha' > .colorantrc

# 4. cd back in (or just press Enter at the prompt) to see the theme apply.
cd .
```

The hook fires on every `cd` and on every prompt redraw, so themes update
both when you change directories and when macOS flips dark/light mode —
the latter applies on your next prompt (your next command or Enter press).

## Commands

`colorant --help` is the canonical reference. The subcommands:

| Command | What it does |
|---|---|
| `colorant apply` | Walk up from the current directory to find the nearest `.colorantrc`, resolve it for the current dark/light mode, and emit OSC sequences to repaint the terminal. Silent no-op on unsupported terminals. If no rc is found, falls back to `default_theme` from `~/.config/colorant/config.toml` (optional), otherwise emits a terminal reset. |
| `colorant reset` | Reset the terminal's foreground, background, cursor, and 16 palette entries to their defaults. |
| `colorant current` | Print the path of the `.colorantrc` that would be applied for the current directory. Empty output if none is found. |
| `colorant init <shell>` | Print a shell-specific integration snippet to stdout, intended for `eval`. Currently `zsh` only. |
| `colorant themes <action>` | Manage themes from bundled and remote sources (currently `gogh`). `list [--source X] [--installed]` enumerates available themes; `search <q> [--source X]` filters by substring; `sync [--source X]` refreshes the remote catalog cache (network only happens here); `apply <name>` writes `extends` to the cwd's `.colorantrc` (or `--dark <name> --light <name>` for per-mode), auto-installing the palette from any source — qualify a remote theme as `gogh:Dracula`; `install [<name>\|--all] [--force]` bulk-installs bundled palettes; `path` prints the themes dir. |
| `colorant doctor [path]` | Diagnose silent failures in a `.colorantrc`: unknown keys, invalid colors, malformed lines, unknown sections, and `extends` references whose palette files aren't on disk. Without `path`, walks up from the current directory like `current`. Exits 0 if nothing is wrong, 1 otherwise. |
| `colorant show [--all]` | Print the resolved colors that would apply for the current directory — each slot with its hex code and a 24-bit swatch. Defaults to the current OS mode; `--all` prints both dark and light. |
| `colorant set` | Open an interactive TUI to browse installed and bundled palettes with a live preview, then write `extends` / `extends.dark` / `extends.light` to the current directory's `.colorantrc`. Bundled palettes that aren't on disk yet are installed automatically on apply. Other keys in the rc are preserved. Keys: `j/k`=navigate, `b`=both, `d`=dark, `l`=light, `c`=clear, `Enter`=apply, `q`=quit. |

The `apply` command is what the shell hook calls on every `chpwd` /
`precmd`; you generally don't need to invoke it manually unless debugging.

### Environment variables

- `COLORANT_MODE=dark|light` — force a specific mode, bypassing macOS dark/light detection.

## Config files

colorant reads up to three config files, all optional. See [`examples/`](./examples)
for annotated templates of the two main ones.

**Palettes (`.colorant`)** — flat color sets. No modes, no inheritance.
Live under `~/.config/colorant/themes/`. Examples ship in this repo:
`catppuccin-mocha.colorant`, `gruvbox-light.colorant`, `tokyo-night.colorant`,
and a dozen more.

```ini
# tokyo-night.colorant
fg     = #c0caf5
bg     = #1a1b26
cursor = #c0caf5
color0 = #15161e
# ...
```

**Per-directory config (`.colorantrc`)** — the file you author per
project. Picks parent palettes (globally or per-mode), and optionally
overrides specific keys.

```ini
# ~/work/myproject/.colorantrc
extends.dark  = tokyo-night
extends.light = catppuccin-latte

# Project-wide override regardless of mode
fg = #ffffff

[dark]
# In dark mode only, recolor the cursor
cursor = #ff00ff
```

**Global config (`~/.config/colorant/config.toml`)** — applied when no
`.colorantrc` is found while walking up from the current directory.
Without it, `colorant apply` emits a terminal reset when no rc is found.

```toml
default_theme = "catppuccin-mocha"
```

## Uninstall

Match the method you used to install:

```sh
# Installed via Homebrew
brew uninstall colorant
brew untap farmisen/tap        # optional, removes the tap

# Installed via cargo install
cargo uninstall colorant

# Installed via shell installer
rm ~/.cargo/bin/colorant       # or wherever COLORANT_INSTALL_DIR pointed
```

Then remove the shell hook line from `~/.zshrc`:

```sh
eval "$(colorant init zsh)"
```

Optionally clear the config dir and any palettes you downloaded:

```sh
rm -rf ~/.config/colorant
```

## License

MIT. See [LICENSE](./LICENSE).
