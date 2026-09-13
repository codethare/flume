<!--
SPDX-FileCopyrightText: © 2026 Julian Andrews
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Architecture

tailrace is a window manager *client*: river owns input, rendering and the
protocol, and tailrace decides where windows go and which one has focus. This
document is the map for changing it; `README.md` covers user-facing behaviour.

## Where things live

| File | Contents |
|---|---|
| `main.rs` | Arguments, the Wayland connection and the event loop |
| `app.rs` | `AppData` (protocol globals + binding lists), registry binds, `manage()` |
| `wm.rs`, `types.rs` | Derived state: outputs, workspaces, windows, status, config |
| `window.rs`, `output.rs`, `seat.rs` | Event handlers for the three river global objects |
| `layout/` | Geometry: `update` (targets), `apply`/`snap_to_finish` (commit), `focus.rs` (borders, focus, raise), `migrate.rs` (output hotplug) |
| `keybinding/` | Default tables (`defaults.rs`), parsing/setup (`keybinding.rs`), action dispatch (`dispatch.rs`) |
| `overview.rs` | Grid overlay |
| `hotplug/` | The test harness and all scenario tests (see below) |
| `protocol/` | Vendored river protocol XMLs; provenance in `protocol/README.md` |

## The manage cycle

river drives everything: it sends events, then a `manage_start`, and tailrace
answers with requests and finally `manage_finish`. `app.rs::manage()` runs the
state machine once per sequence:

| `Status` | What the sequence does |
|---|---|
| `SetupBindings` | Create xkb and pointer bindings from the config, then go to `Layout` |
| `Layout` | `layout::apply` (init pending windows, hotplug, then `update` + focus/borders) and `layout::snap_to_finish` (commit geometry), then `None` |
| `Overview` | `overview::apply_borders` + `snap_to_finish`, then `None` |
| `PointerAction` | `op_start_pointer` and follow the drag in `seat::pointer_action`; ends on `OpRelease` |
| `Exit` | `river_window_manager_v1.exit_session` |
| `None` | `seat.op_end()` |

Anything that changes state sets `status = Layout` (or calls
`manage_dirty()`) so the next sequence commits it. `render_start` is answered
with `render_finish`.

## Layout pipeline

1. `layout::update` recomputes every window's `geom.finish` from the workspace
   layout (`layout::scroller`), clamping floats into the output. Fullscreen
   wins over everything: the focused one is handed to the compositor
   (`river_window_v1.fullscreen`) which then owns its geometry and clipping.
2. `layout::apply` handles pending windows, removed outputs (migrate or
   detach) and restored outputs, then `apply_focus_and_borders`.
3. `layout::snap_to_finish` sets `current = finish` and issues the protocol
   requests through `layout::common::place_window`: `propose_dimensions`,
   `set_position`, `show`/`hide` and `set_clip_box`, each deduplicated against
   the `sent_*` caches on `WindowGeom`. `raise_floating_windows` then raises
   every float so floats stay above tiles.

Focus is derived state: `focused_output_idx` → `focused_workspace_idx` →
`focused_window_idx`. `apply_focus_and_borders` is the only place that talks
to river about focus, and it only does so when the target differs from
`last_focused_window` — so any code that changes focus must not also update
that cache. Focus commands are suppressed entirely while a layer-shell surface
holds exclusive focus or the session is locked; `needs_refocus` makes the next
sequence retry.

## The test harness (`src/hotplug/`)

`hotplug.rs` is an in-process fake compositor built on `wayland-backend`'s
server side, wired to the real client code over a socketpair:

- `MiniServer` advertises the river globals (and `wl_seat`, and
  `wp_cursor_shape_manager_v1`), answers requests, and records every request
  as `(object, opcode, args)` — that log is what most tests assert on.
- `Session` owns the same `AppData` the binary uses, so scenarios drive the
  real `Dispatch` implementations, the real `manage()` and the real layout.

A scenario is: `build()`, add a seat and outputs, add windows, send the events
under test, call `manage()`, then assert on `Session.state` or on the request
log. Helpers: `add_output`, `add_pending_window` + `map_window`,
`add_window`, `add_wl_seat`, `manage`, `config_mut`, plus `children_with_interface`,
`count_requests` and `requests_for` for assertions. Each scenario module
(`outputs.rs`, `pointer.rs`, `windows.rs`, `manage.rs`, `overview.rs`,
`keybindings.rs`, `cursor.rs`, `rules.rs`) covers one area.

Limits: there is no compositor, so requests are recorded rather than executed,
nothing is rendered, and protocol errors only surface as panics inside the
backend. Passing tests prove what tailrace *asks for*, not what river does.

## Traps worth knowing

- **State pokes need a status.** Setting fields and calling `manage()` does
  nothing unless `status == Status::Layout`: the focus gates and the layout
  pass only run from there. A test asserting "no request was sent" after a
  bare poke proves nothing — that bug was already fixed once in
  `manage.rs::exclusive_layer_shell_focus_skips_focus_commands`.
- **Windows are mapped on `Dimensions`.** `window.rs` moves a pending window
  into a workspace when its `dimensions` event arrives, which is also where
  `window_rules` are evaluated; river sends `app_id`/`title` before
  `dimensions`, and the harness mirrors that with `add_pending_window` +
  `map_window`.
- **Some requests must not be assigned as status.** A seat event that wants
  binding setup sets `needs_setup_bindings` instead of `status`, because any
  output or window event in the same batch would clobber the status and the
  bindings would never be created.
- **rill-ed is the reference implementation.** When behaviour looks wrong,
  compare with `src/` in rill-ed (`animation.zig` ↔ `snap_to_finish`,
  `layout.zig` ↔ `layout/`); two bugs in tailrace were single dropped lines from
  it, and one deliberate divergence (fullscreen floats) is documented in the
  code.
- **Protocol updates** are a `protocol/update.sh <ref>` plus a bind version
  bump in `app.rs`; see `protocol/README.md` for the current revision and what
  is still unused.
