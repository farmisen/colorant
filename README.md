# colorant

Per-directory terminal theme switcher with system dark/light mode support.

`colorant` walks up from your current directory looking for a `.colorantrc`
file and applies the theme it describes to your terminal. When you `cd` out
of the tree, the theme resets. When the OS flips between dark and light, the
active theme follows.

## Status

Early development. v1 scope: Ghostty + zsh + macOS. Other terminals, shells,
and OSes will land incrementally.

## Two file types

colorant uses two distinct file shapes. See [`examples/example.colorant`](./examples/example.colorant)
and [`examples/example.colorantrc`](./examples/example.colorantrc) for fully
annotated templates.

**Palettes (`.colorant`)** — flat color sets. No modes, no inheritance.
Lives under `~/.config/colorant/themes/`. Examples ship in this repo:
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

**Config (`.colorantrc`)** — the per-directory file you author. Picks parent
palettes (globally or per-mode), and optionally overrides specific keys.

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

## How resolution works

In mode `M` (dark or light):

1. **Pick the parent palette**: `extends.M` if set, otherwise the top-level
   `extends`, otherwise none.
2. **Load that palette** from `~/.config/colorant/themes/<name>.colorant`.
3. **Apply the rc's top-level keys** on top.
4. **Apply the rc's `[M]` section** on top of that.

The result: a child always beats its parent palette, and a mode section
always beats top-level keys within the same file.

## Install

TBD (Homebrew, install script, `cargo install` — coming).

## Build from source

```sh
cargo build --release
./target/release/colorant --help
```

## Shell integration

```sh
# Add to ~/.zshrc
eval "$(colorant init zsh)"
```

The hook fires on every `chpwd` and `precmd`, so the theme changes as you
move between projects.

## Bundled palettes

Sixteen palettes ship in `themes/`:

- Catppuccin: `catppuccin-mocha` (dark), `catppuccin-latte` (light)
- Tokyo Night: `tokyo-night` (dark), `tokyo-night-day` (light)
- Nord (dark)
- Gruvbox: `gruvbox-dark`, `gruvbox-light`
- Solarized: `solarized-dark`, `solarized-light`
- One: `one-dark`, `one-light`
- Owl: `night-owl`, `light-owl`
- Ayu: `ayu-dark`, `ayu-mirage`, `ayu-light`

Drop these into `~/.config/colorant/themes/` and reference them by name in
your `.colorantrc` via `extends`, `extends.dark`, or `extends.light`.

## License

MIT. See [LICENSE](./LICENSE).
