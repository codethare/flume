<!--
SPDX-FileCopyrightText: © 2026 Julian Andrews
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# flume

Tiny scrolling window manager for [river](https://isaacfreund.com/software/river/),
implemented in Rust against the
[river-window-management-v1](https://isaacfreund.com/docs/wayland/river-window-management-v1/)
protocol. A port of [rill-ed](https://github.com/codethare/rill-ed) (Zig) with
**no animations** — layout changes commit instantly.

## Features

* Per-workspace scrolling column layout; individual windows can float
* Floating windows center on screen; drag with mouse to move, resize with mouse or keyboard
* 10 workspaces per output
* Overview mode: grid of all windows across outputs/workspaces, vi-key navigation
* Live-reloading TOML config
* Multi-output with window migration (windows return to their output when it reappears)
* TTY switch resilience — workspaces survive output removal (laptop panel off, lock)
* Window rules by exact `app_id` / glob `title` (force floating)
* Focus follows the mouse pointer (opt-in, sloppy focus)
* Session-lock focus save/restore

## Building

Dependencies: Rust (stable), `libxkbcommon`, `wayland`, `wayland-protocols` (runtime).

```sh
cargo build --release
```

## Running

```
river -c ./target/release/flume
```

## Configuration

Config is searched at, in order:

1. `$XDG_CONFIG_HOME/flume/config.toml`
2. `$HOME/.config/flume/config.toml`

If no file is found, the built-in defaults (identical to rill-ed's defaults,
minus animations) are used. A `[keybindings]` or `[[pointer_bindings]]` section
**replaces the built-in list as a whole** — bindings do not merge — so an empty
(or absent) section is how you keep the defaults. See
[config.example.toml](config.example.toml) for an annotated example and the
default key table below.

Unknown keys are rejected at parse time: a typo is reported rather than
silently ignored.

`Super+r` reloads the config; on parse errors the current config is kept.

### Top-level options

| Option | Default | Meaning |
|---|---|---|
| `vertical_gap` | `9` | Gap between windows and the output's top/bottom edge |
| `horizontal_gap` | `9` | Gap between adjacent windows |
| `default_window_width` | `0.5` | Starting width of a new window, as a proportion of the output |
| `center_focused_window` | `"never"` | Center the focused window: `never`, `always`, `single` |
| `focus_follows_pointer` | `false` | Keyboard focus follows the mouse pointer (sloppy focus) |
| `no_csd` | `true` | Disable client side decorations |
| `border.width` | `3` | Border width in pixels |
| `border.focused_color`, `border.unfocused_color` | | `{ r, g, b, a }` with 0–255 channels and 0.0–1.0 alpha |
| `cursor.theme`, `cursor.size` | | XCursor theme and size (omit for the default cursor) |
| `spawn_at_startup` | `[]` | Commands spawned detached at startup |
| `window_rules` | `[]` | Match by `app_id` and/or glob `title`; matched windows float |

### Default keybindings

| Keybinding | Action |
|---|---|
| `Super` `q` | Close window |
| `Super` `f` | Toggle fullscreen |
| `Super` `minus` / `equal` | Decrease / increase window width by 0.1 |
| `Super` `BackSpace` | Set window width to 0.5 |
| `Super` `Ctrl` `minus` / `equal` | Shrink / grow floating window |
| `Super` `Ctrl` `BackSpace` | Set floating window height to 0.5 |
| `Super` `Left` / `Right` | Focus window left / right (falls through to output focus at edges) |
| `Super` `Shift` `Left` / `Right` | Move window left / right |
| `Super` `Ctrl` `Left`/`Right`/`Up`/`Down` | Move floating window |
| `Super` `v` | Toggle window floating |
| `Super` `Up` / `Down` | Focus workspace above / below |
| `Super` `` ` `` | Previous workspace |
| `Super` `1`–`0` | Focus workspace 1–10 |
| `Super` `Shift` `Up` / `Down` | Move window to workspace above / below |
| `Super` `Shift` `1`–`0` | Move window to workspace 1–10 |
| `Super` `h`/`j`/`k`/`l` | Focus output left/below/above/right |
| `Super` `Shift` `h`/`j`/`k`/`l` | Move window to output left/below/above/right |
| `Super` `Escape` | Exit river |
| `Super` `Space` | Enter overview |
| `Escape` / `Return` / `hjkl` | Overview: cancel / confirm / navigate (overview only) |
| `Super` `r` | Reload config |
| `Super` `t` | Spawn alacritty |
| `XF86Audio*` | Volume via `wpctl` |

### Default pointer bindings

| Pointer binding | Action |
|---|---|
| `Super` `Left Click` | Move floating window |
| `Super` `Right Click` | Resize floating window |

`[[pointer_bindings]]` takes `button = "left" | "right" | "middle" | "side" | "extra"`
and any modifier list, including none. A bare button (empty `modifiers`) is how a
touchpad gesture that a remapper turns into a mouse button — or an extra mouse
button — drives `move_window` / `resize_window`.

## Differences from rill-ed

* No animations (no spring physics, no frame interpolation) — by design
* No kwim hotplug integration (Zig-specific input method)
* TOML config instead of ZON

## License

AGPL-3.0-or-later — see [LICENSE](LICENSE). Ported from rill-ed.
