<!--
SPDX-FileCopyrightText: © 2026 Julian Andrews
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# Vendored river protocols

These XML files are vendored unmodified from the river compositor
(<https://codeberg.org/river/river>, MIT, © Isaac Freund). They are consumed at
build time by `wayland-scanner` (see `src/river.rs`), so changing them changes
the generated client API and can require handler updates in `src/`.

The vendored state matches river
[`ffcaf28`](https://codeberg.org/river/river/commit/ffcaf28) (2026-03-10):

| File | Matches | Upstream commits since |
|---|---|---|
| `river-window-management-v1.xml` | `ffcaf28` | 11 |
| `river-xkb-bindings-v1.xml` | the revision before `65947fe` (2026-04-22) | 2 |
| `river-layer-shell-v1.xml` | current `main` | 0 |

The drift is almost entirely documentation. The only interface additions
upstream are `river_window_v1.capture_sessions` and
`river_output_v1.capture_sessions` (since 5: counts of active
ext-image-copy-capture sessions) and, in the xkb protocol,
`river_xkb_bindings_seat_v1.modifiers_watch` / `modifiers_update` (since 3).
Every window management interface moved v4 → v5 and the xkb ones v2 → v3, so
using any of this also means raising the bind versions in `src/app.rs` and
running a river new enough to serve them.

Verified by blob equality rather than by date, e.g. for the window management
protocol:

```sh
git hash-object protocol/river-window-management-v1.xml
# 608225dd5f47e4feb5875742392d501f754b2dea
git -C <river clone> log --all --find-object=608225dd5f47e4feb5875742392d501f754b2dea
# ffcaf28 added this blob, c1771ae replaced it
```

## Refreshing

```sh
protocol/update.sh          # upstream main
protocol/update.sh <ref>    # any river branch or commit
```

Then build and test: a protocol change is an API change for the code
implementing it, so expect `src/` follow-up work (and check whether the new
revision is still understood by the river build being run).
