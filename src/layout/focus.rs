// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Border colours, keyboard focus and raise order.

use crate::layout::common;
use crate::river::river_seat_v1::RiverSeatV1;
use crate::types::{Color, Window, WindowGeom};
use crate::wm::{LayerShellFocus, WindowManager};

/// Set border colors and keyboard focus to match the current focused
/// window/output. Safe to call every manage pass: redundant border and focus
/// requests are skipped so IME clients (fcitx5) are not disrupted.
pub(super) fn apply_focus_and_borders(wm: &mut WindowManager, seat: &RiverSeatV1) {
    let Some(foi) = wm.focused_output_idx else {
        return;
    };
    let config = wm.config.clone();

    // While overview is active the highlighted grid slot is the focused
    // window; it lives at its home location, not in any single workspace.
    let ov_highlighted: Option<crate::river::river_window_v1::RiverWindowV1> = wm
        .overview_state
        .as_ref()
        .filter(|ov| ov.highlighted < ov.entries.len())
        .and_then(|ov| wm.locate_window(&ov.entries[ov.highlighted].window))
        .and_then(|(oi, wi, wi2)| {
            wm.outputs
                .get(oi)?
                .workspace_list
                .get(wi)?
                .window_list
                .get(wi2)
                .map(|w| w.river_window.clone())
        });

    for (output_idx, output) in wm.outputs.iter_mut().enumerate() {
        if output.is_removed {
            continue;
        }
        let focused_ws = output.focused_workspace_idx;
        let is_focused_output = output_idx == foi;
        for (workspace_idx, workspace) in output.workspace_list.iter_mut().enumerate() {
            let ws_focus = workspace.focused_window_idx;
            for (window_idx, window) in workspace.window_list.iter_mut().enumerate() {
                let is_focused = if wm.overview_state.is_some() {
                    ov_highlighted.as_ref() == Some(&window.river_window)
                } else {
                    is_focused_output && workspace_idx == focused_ws && Some(window_idx) == ws_focus
                };

                let Window {
                    river_window,
                    river_node,
                    geom,
                } = window;
                apply_window_border(river_window, geom, is_focused, &config);

                if !is_focused {
                    continue;
                }
                river_node.place_top();
            }
        }

        if !is_focused_output {
            continue;
        }
        if let Some(layer_shell_output) = &output.river_layer_shell_output {
            layer_shell_output.set_default();
        }
    }

    // Only send focus commands when the target actually changes.
    if wm.layer_shell_focus == LayerShellFocus::Exclusive {
        return;
    }
    // Skip focus management while the session is locked; the lock surface
    // has exclusive keyboard focus managed by the compositor.
    if wm.session_locked {
        return;
    }

    let desired_focus: Option<crate::river::river_window_v1::RiverWindowV1> =
        if wm.overview_state.is_some() {
            ov_highlighted
        } else {
            let output = &wm.outputs[foi];
            let workspace = &output.workspace_list[output.focused_workspace_idx];
            workspace
                .focused_window_idx
                .and_then(|fwi| workspace.window_list.get(fwi))
                .map(|w| w.river_window.clone())
        };

    if desired_focus != wm.last_focused_window {
        if let Some(window) = &desired_focus {
            seat.focus_window(window);
        } else if wm.layer_shell_focus != LayerShellFocus::NonExclusive {
            seat.clear_focus();
        }
        wm.last_focused_window = desired_focus;
    }
}

/// Raise every floating window above all tiled windows. river commits the
/// render list atomically at render_finish and skips reorder work when the
/// order is unchanged, so re-issuing on every manage pass is free.
pub(super) fn raise_floating_windows(wm: &mut WindowManager) {
    for output in &mut wm.outputs {
        for workspace in &mut output.workspace_list {
            for window in &mut workspace.window_list {
                if window.geom.is_floating {
                    window.river_node.place_top();
                }
            }
        }
    }
}

pub fn apply_window_border(
    river_window: &crate::river::river_window_v1::RiverWindowV1,
    geom: &mut WindowGeom,
    is_focused: bool,
    config: &crate::types::Config,
) {
    // Fullscreen windows fill their output rect exactly (place_window uses
    // border 0 for them); any border would be drawn by the compositor beyond
    // the rect, spilling onto a neighboring monitor's adjoining edge. Suppress
    // borders for fullscreen like place_window does.
    let width = if geom.is_fullscreen {
        0
    } else {
        config.border.width
    };
    // Dedup against sent state; if unchanged, skip the request.
    let need =
        geom.sent_border_focused != Some(is_focused) || geom.sent_border_width != Some(width);
    if !need {
        return;
    }
    let color = if is_focused {
        color_to_river(config.border.focused_color)
    } else {
        color_to_river(config.border.unfocused_color)
    };
    river_window.set_borders(
        common::edges_all(),
        width as i32,
        color.0,
        color.1,
        color.2,
        color.3,
    );
    geom.sent_border_focused = Some(is_focused);
    geom.sent_border_width = Some(width);
}

/// Convert a config color to river's 32-bit channel values.
pub fn color_to_river(c: Color) -> (u32, u32, u32, u32) {
    let max = u32::MAX as f64;
    let r = (c.a * c.r as f32 / 255.0) as f64 * max;
    let g = (c.a * c.g as f32 / 255.0) as f64 * max;
    let b = (c.a * c.b as f32 / 255.0) as f64 * max;
    let a = c.a as f64 * max;
    (r as u32, g as u32, b as u32, a as u32)
}
