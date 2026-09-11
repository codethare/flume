<!--
SPDX-FileCopyrightText: © 2026 Julian Andrews
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Vendored river protocols

These XML files are vendored unmodified from the river compositor
(<https://codeberg.org/river/river>, MIT, © Isaac Freund). They are consumed at
build time by `wayland-scanner` (see `src/river.rs`), so changing them changes
the generated client API and can require handler updates in `src/`.

The vendored state matches the river release
[`v0.4.8`](https://codeberg.org/river/river/releases/tag/v0.4.8) (commit
[`b028abc`](https://codeberg.org/river/river/commit/b028abcff33ed7a42ee222dce1abe71c06d3e41a),
2026-08-07), which is also the surface of `main` for these files:

| File | Interface versions | Upstream drift |
|---|---|---|
| `river-window-management-v1.xml` | v5 | none |
| `river-xkb-bindings-v1.xml` | v3 | none |
| `river-layer-shell-v1.xml` | v1 | none |

flume binds `river_window_manager_v1` at v5 and `river_xkb_bindings_v1` at v3
(`src/app.rs`), so the running river must be **0.4.6 or newer**. Two API
additions come with them and are vendored but deliberately unused for now:
`river_window_v1`/`river_output_v1.capture_sessions` (counts of active
ext-image-copy-capture sessions, e.g. for staying awake while captured) and
`river_xkb_bindings_seat_v1.modifiers_{watch,update}` (modifier state changes
before the next input event, e.g. for a mod-key overlay). `src/window.rs` and
`src/output.rs` carry explicit no-op arms for the capture events.

Verified by blob equality rather than by date, e.g. for the window management
protocol:

```sh
git hash-object protocol/river-window-management-v1.xml
# 039a9b65a2e41df220dc2b93d7fa545e26a1c898
git -C <river clone> log --all --find-object=039a9b65a2e41df220dc2b93d7fa545e26a1c898
# the commit carrying the v0.4.8 protocol
```

## Refreshing

```sh
protocol/update.sh          # upstream main
protocol/update.sh <ref>    # any river branch or commit
```

Then build and test: a protocol change is an API change for the code
implementing it, so expect `src/` follow-up work (and check whether the new
revision is still understood by the river build being run).
