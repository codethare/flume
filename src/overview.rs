// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Overview grid mode, ported from rill-ed overview.zig.
//!
//! Virtual overview: windows stay in their home workspace lists; only their
//! finish rect is rewritten to a grid cell on the focused output. Canceling
//! is therefore a strict no-op on the layout data model.

use crate::app::AppData;
use crate::layout;
use crate::river::river_seat_v1::RiverSeatV1;
use crate::types::{OverviewEntry, OverviewHome, OverviewState, Rectangle, Status};
use crate::wm::WindowManager;

/// Compute the grid cell rectangles for `total` windows inside `area`.
/// Returns (columns, cells).
pub fn compute_grid(
    area: Rectangle,
    total: usize,
    h_gap: i32,
    v_gap: i32,
) -> (usize, Vec<Rectangle>) {
    let cols = grid_columns(total, &area);
    let rows = total.div_ceil(cols);

    let cell_w = (area.width - h_gap * (cols as i32 + 1)) / cols as i32;
    let cell_h = (area.height - v_gap * (rows as i32 + 1)) / rows as i32;

    let cells = (0..total)
        .map(|i| {
            let row = (i / cols) as i32;
            let col = (i % cols) as i32;
            Rectangle {
                x: area.x + h_gap + col * (cell_w + h_gap),
                y: area.y + v_gap + row * (cell_h + v_gap),
                width: cell_w,
                height: cell_h,
            }
        })
        .collect();
    (cols, cells)
}

fn grid_columns(total: usize, rect: &Rectangle) -> usize {
    if total <= 1 {
        return 1;
    }
    let aspect = rect.width as f32 / 1f32.max(rect.height as f32);
    let cols_f = (total as f32 * aspect).sqrt();
    1usize.max((cols_f).ceil() as usize)
}

pub fn enter(state: &mut AppData) {
    let Some(output_idx) = state.wm.focused_output_idx else {
        return;
    };
    if state.wm.overview_state.is_some() {
        return;
    }

    let total: usize = state
        .wm
        .outputs
        .iter()
        .flat_map(|out| out.workspace_list.iter())
        .map(|ws| ws.window_list.len())
        .sum();
    if total == 0 {
        return;
    }

    let (columns, cells) = compute_grid(
        state.wm.outputs[output_idx].non_exclusive,
        total,
        state.wm.config.horizontal_gap,
        state.wm.config.vertical_gap,
    );

    // Capture before any mutation below.
    let previous_workspace = Some(OverviewHome {
        output_idx,
        workspace_idx: state.wm.outputs[output_idx].focused_workspace_idx,
    });

    let mut entries = Vec::with_capacity(total);
    let mut slot = 0usize;
    for output in &mut state.wm.outputs {
        for workspace in &mut output.workspace_list {
            for window in &mut workspace.window_list {
                let mut was_fullscreen = false;
                if window.geom.is_fullscreen {
                    was_fullscreen = true;
                    window.geom.is_fullscreen = false;
                    // exit_fullscreen is deferred to apply_borders, which
                    // runs inside a manage sequence; calling it here would
                    // be a protocol error.
                }
                window.geom.finish = Some(cells[slot]);
                entries.push(OverviewEntry {
                    window: window.river_window.clone(),
                    was_fullscreen,
                });
                slot += 1;
            }
        }
    }

    state.wm.overview_state = Some(OverviewState {
        entries,
        highlighted: 0,
        columns,
        previous_workspace,
    });

    state.wm.status = Status::Overview;
    // Bare confirm/cancel keys are only grabbed while the overview is open,
    // so apps keep receiving Return/Escape at all other times.
    super::keybinding::set_overview_keybinds(state, true);
}

/// Cancel overview: restore fullscreen flags, refocus the workspace that was
/// focused at enter. No other state changed while overview was active.
pub fn cancel(state: &mut AppData) {
    let Some(ov_state) = state.wm.overview_state.take() else {
        return;
    };
    let prev = ov_state.previous_workspace;
    exit_overview(state, ov_state);

    if let Some(p) = prev {
        state.wm.focused_output_idx = Some(p.output_idx);
        state.wm.outputs[p.output_idx].focused_workspace_idx = p.workspace_idx;
    }
}

pub fn select(state: &mut AppData) {
    let Some(ov_state) = state.wm.overview_state.as_ref() else {
        return;
    };
    let selected_idx = ov_state.highlighted;
    if selected_idx >= ov_state.entries.len() {
        cancel(state);
        return;
    }
    let entry = ov_state.entries[selected_idx].clone();
    let ov_taken = state.wm.overview_state.take().unwrap();

    exit_overview(state, ov_taken);

    // The window may have closed while highlighted; fall back to the
    // workspace focused at enter rather than focusing a ghost.
    let Some(loc) = locate(&state.wm, &entry.window) else {
        return;
    };

    state.wm.focused_output_idx = Some(loc.output_idx);
    let output = &mut state.wm.outputs[loc.output_idx];
    output.focused_workspace_idx = loc.workspace_idx;
    output.workspace_list[loc.workspace_idx].focused_window_idx = Some(loc.window_idx);
}

/// Select the window at the given index (from mouse click).
pub fn select_index(state: &mut AppData, index: usize) {
    if let Some(ov_state) = &mut state.wm.overview_state {
        ov_state.highlighted = index;
    }
    select(state);
}

fn exit_overview(state: &mut AppData, ov_state: OverviewState) {
    super::keybinding::set_overview_keybinds(state, false);

    for entry in &ov_state.entries {
        if !entry.was_fullscreen {
            continue;
        }
        if let Some(loc) = locate(&state.wm, &entry.window) {
            state.wm.outputs[loc.output_idx].workspace_list[loc.workspace_idx].window_list
                [loc.window_idx]
                .geom
                .is_fullscreen = true;
        }
    }
}

pub struct Located {
    pub output_idx: usize,
    pub workspace_idx: usize,
    pub window_idx: usize,
}

/// Locate a window's current home by scanning live lists. None once the
/// window has closed — entries never rely on captured indices.
pub fn locate(
    wm: &WindowManager,
    river_window: &crate::river::river_window_v1::RiverWindowV1,
) -> Option<Located> {
    for (output_idx, output) in wm.outputs.iter().enumerate() {
        for (workspace_idx, workspace) in output.workspace_list.iter().enumerate() {
            if let Some(window_idx) = workspace
                .window_list
                .iter()
                .position(|w| &w.river_window == river_window)
            {
                return Some(Located {
                    output_idx,
                    workspace_idx,
                    window_idx,
                });
            }
        }
    }
    None
}

/// Drop overview entries whose windows no longer exist, so navigation never
/// highlights a ghost slot. Cancels the overview when nothing is left.
pub fn prune(state: &mut AppData) {
    let any_dead = {
        let Some(ov_state) = &state.wm.overview_state else {
            return;
        };
        ov_state
            .entries
            .iter()
            .any(|entry| locate(&state.wm, &entry.window).is_none())
    };
    if !any_dead {
        return;
    }

    let kept: Vec<OverviewEntry> = state
        .wm
        .overview_state
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| locate(&state.wm, &entry.window).is_some())
        .cloned()
        .collect();
    let ov_state = state.wm.overview_state.as_mut().unwrap();
    if kept.is_empty() {
        // Let cancel() take the state: clearing it here would make cancel()
        // return immediately, leaving the overview-only key grabs held and
        // fullscreen windows unrestored. Dead entries are skipped by locate().
        cancel(state);
        return;
    }
    if kept.len() == ov_state.entries.len() {
        return;
    }
    let highlighted = ov_state.highlighted.min(kept.len() - 1);
    ov_state.highlighted = highlighted;
    ov_state.entries = kept;
}

/// Apply border colors and focus for the overview state without
/// recalculating layout.
pub fn apply_borders(wm: &mut WindowManager, river_seat: &RiverSeatV1) {
    let Some(ov_state) = wm.overview_state.as_ref() else {
        return;
    };
    let config = wm.config.clone();

    river_seat.clear_focus();

    for (idx, entry) in ov_state.entries.iter().enumerate() {
        let Some(loc) = locate(wm, &entry.window) else {
            continue;
        };
        let window = &mut wm.outputs[loc.output_idx].workspace_list[loc.workspace_idx].window_list
            [loc.window_idx];

        window.river_window.exit_fullscreen();

        let is_focused = idx == ov_state.highlighted;
        layout::apply_window_border(&window.river_window, &mut window.geom, is_focused, &config);
        if is_focused {
            // Deliberate relaxation of the floats-on-top invariant: in the
            // overview grid windows do not overlap, so raising the
            // highlighted window is only a UI affordance.
            window.river_node.place_top();
            river_seat.focus_window(&window.river_window);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_grid_fills_area_with_even_cells() {
        let area = Rectangle {
            x: 100,
            y: 50,
            width: 1920,
            height: 1080,
        };
        let (cols, cells) = compute_grid(area, 6, 9, 9);
        let rows = 6_usize.div_ceil(cols);
        assert!(cols * rows >= 6);
        for cell in &cells {
            assert!(cell.x >= area.x && cell.y >= area.y);
            assert!(cell.x + cell.width <= area.x + area.width);
            assert!(cell.y + cell.height <= area.y + area.height);
            assert!(cell.width > 0 && cell.height > 0);
        }
        // Same column => same x (cell `cols` is directly below cell 0).
        assert_eq!(cells[0].x, cells[cols].x);
        // Same row => same y.
        assert_eq!(cells[0].y, cells[1].y);
    }

    #[test]
    fn grid_columns_edge_cases() {
        assert_eq!(
            grid_columns(
                0,
                &Rectangle {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100
                }
            ),
            1
        );
        assert_eq!(
            grid_columns(
                1,
                &Rectangle {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100
                }
            ),
            1
        );
        // Wide aspect => more columns than rows for many windows.
        let wide = Rectangle {
            x: 0,
            y: 0,
            width: 3840,
            height: 1080,
        };
        let tall = Rectangle {
            x: 0,
            y: 0,
            width: 1080,
            height: 3840,
        };
        assert!(grid_columns(16, &wide) >= grid_columns(16, &tall));
    }
}
