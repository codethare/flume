// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Layout coordinator, ported from rill-ed layout.zig.
//!
//! No animation: `update` computes `finish` targets, `snap_to_finish`
//! commits them immediately (`current = finish`).

pub mod common;
pub mod scroller;

mod focus;
mod migrate;

pub use focus::apply_window_border;
use focus::{apply_focus_and_borders, raise_floating_windows};
use migrate::{
    detach_output, exit_fullscreen_and_close_closing, fixup_indices_after_output_removal,
    migrate_detached_into, migrate_output_windows, migrate_windows_by_name, reset_sent_caches,
};

use crate::river::river_seat_v1::RiverSeatV1;
use crate::river::wayland_client::Proxy;
use crate::types::{Window, WindowGeom};
use crate::wm::WindowManager;

/// Recompute layout targets for every output/workspace. Pure state
/// computation; no protocol requests.
pub fn update(wm: &mut WindowManager) {
    // Virtual overview: enter() already assigned grid finishes; skip the
    // regular per-workspace layouts so they can't clobber them.
    if wm.overview_state.is_some() {
        return;
    }

    let config = wm.config.clone();
    for output in wm.outputs.iter_mut() {
        // Freshly re-added outputs start at 0x0 until the dimensions event
        // arrives. Laying out on a 0x0 output produces bad geometry
        // (rill-ed: "fix panic on HDMI disconnect").
        if output.rectangle.width <= 0 || output.rectangle.height <= 0 {
            continue;
        }
        for (workspace_idx, workspace) in output.workspace_list.iter_mut().enumerate() {
            let workspace_offset = workspace_idx as i32 - output.focused_workspace_idx as i32;
            let y_offset = workspace_offset * output.rectangle.height;

            // Floating rest rects are absolute; when the output is replaced
            // by a smaller one (HDMI unplug back to a 1360x768 laptop) the
            // old large-screen rect overhangs the new screen unreachable.
            // Clamp it back in (mirrors the move/resize clamps). Only the
            // visible workspace: others are stacked off-screen by y_offset
            // by design.
            if workspace_idx == output.focused_workspace_idx {
                for window in &mut workspace.window_list {
                    if window.geom.is_floating {
                        common::clamp_floating_into_output(&mut window.geom, output.rectangle);
                    }
                }
            }

            let mut geoms: Vec<WindowGeom> = workspace
                .window_list
                .iter()
                .map(|w| w.geom.clone())
                .collect();
            scroller::apply(
                &mut geoms,
                workspace.focused_window_idx,
                output.rectangle,
                output.non_exclusive,
                &config,
                y_offset,
            );
            for (window, geom) in workspace.window_list.iter_mut().zip(geoms) {
                window.geom = geom;
            }
        }
    }
}

/// Full manage-pass layout: pending window init, removed-output handling
/// (migrate or detach), detached restore, window migration back to their
/// former output, focus/borders, float raising, pointer warp.
pub fn apply(wm: &mut WindowManager, seat: &RiverSeatV1) {
    // Initialize pending windows so they don't flash on screen.
    for pending in wm.pending_windows.iter_mut() {
        if pending.initialized {
            continue;
        }
        let river_window = pending.river_window.clone();
        if wm.config.no_csd {
            river_window.use_ssd();
        }
        river_window.set_tiled(common::edges_all());
        river_window.propose_dimensions(0, 0);
        river_window.hide();
        pending.initialized = true;
    }

    let mut needs_update = false;

    let mut output_idx = wm.outputs.len();
    while output_idx > 0 {
        output_idx -= 1;
        if !wm.outputs[output_idx].is_removed {
            exit_fullscreen_and_close_closing(wm, output_idx);
            continue;
        }

        // Count surviving (non-removed) outputs to decide migration vs
        // detachment strategy.
        let survivors: Vec<usize> = wm
            .outputs
            .iter()
            .enumerate()
            .filter(|(i, o)| !o.is_removed && *i != output_idx)
            .map(|(i, _)| i)
            .collect();

        if let Some(&survivor_idx) = survivors.first() {
            migrate_output_windows(wm, output_idx, survivor_idx);
        } else {
            detach_output(wm, output_idx);
        }

        // Close any windows remaining in the output's workspaces. After
        // successful migration workspaces are empty; after successful
        // detachment they were taken out.
        for workspace in &mut wm.outputs[output_idx].workspace_list {
            for window in workspace.window_list.drain(..) {
                window.river_window.close();
            }
        }

        {
            let output = &wm.outputs[output_idx];
            if let Some(layer_shell_output) = &output.river_layer_shell_output {
                layer_shell_output.destroy();
            }
            if let Some(wl_output) = &output.wl_output {
                // wl_output has no destructor; release (v3+) lets the
                // compositor reclaim it early.
                if wl_output.version() >= 3 {
                    wl_output.release();
                }
            }
            output.river_output.destroy();
        }

        fixup_indices_after_output_removal(
            &mut wm.focused_output_idx,
            output_idx,
            wm.outputs.len(),
        );
        if let Some(pw) = &mut wm.previous_workspace {
            if pw.output_idx == output_idx {
                wm.previous_workspace = None;
            } else if pw.output_idx > output_idx {
                pw.output_idx -= 1;
            }
        }

        // When the last output is removed, the previously focused window
        // proxy is no longer meaningful.
        if wm.focused_output_idx.is_none() {
            wm.last_focused_window = None;
            wm.previous_workspace = None;
        }

        wm.outputs.swap_remove(output_idx);
        needs_update = true;
    }

    // Restore workspaces (with windows) that were detached when an output
    // was removed. Match by output name so windows return to the correct
    // display even if the compositor re-creates the output with a new
    // river_output_v1.
    let mut restored_any = false;
    for output in wm.outputs.iter_mut() {
        if output.is_removed {
            continue;
        }
        let Some(name) = output.name.clone() else {
            continue;
        };
        let Some(detached) = wm.detached_outputs.remove(&name) else {
            continue;
        };
        output.workspace_list = detached.workspace_list;
        output.focused_workspace_idx = detached.focused_workspace_idx;
        // The previous output's compositor-side state was destroyed on
        // removal; reset sent_* caches so show/proposeDimensions/setBorders
        // are re-issued for the fresh output.
        for workspace in &mut output.workspace_list {
            for window in &mut workspace.window_list {
                reset_sent_caches(&mut window.geom);
            }
        }
        restored_any = true;
    }

    // Fallback: any detached outputs NOT matched by name (e.g. HDMI was
    // unplugged, only eDP reappeared) get migrated to the first active
    // output with valid dimensions so windows are not left orphaned.
    if !wm.detached_outputs.is_empty() {
        let fallback_idx = wm
            .outputs
            .iter()
            .position(|o| !o.is_removed && o.rectangle.width > 0 && o.rectangle.height > 0);
        if let Some(fallback_idx) = fallback_idx {
            let keys: Vec<String> = wm.detached_outputs.keys().cloned().collect();
            for key in keys {
                let Some(detached) = wm.detached_outputs.remove(&key) else {
                    continue;
                };
                migrate_detached_into(wm, fallback_idx, &key, detached);
                restored_any = true;
            }
        }
    }

    // Migrate windows back to the output whose name matches their
    // former_output_name (e.g. DPMS on after screen lock).
    for dst_idx in 0..wm.outputs.len() {
        if wm.outputs[dst_idx].is_removed {
            continue;
        }
        let Some(dst_name) = wm.outputs[dst_idx].name.clone() else {
            continue;
        };
        for src_idx in 0..wm.outputs.len() {
            if src_idx == dst_idx || wm.outputs[src_idx].is_removed {
                continue;
            }
            for ws_idx in 0..10 {
                restored_any |= migrate_windows_by_name(wm, src_idx, dst_idx, ws_idx, &dst_name);
            }
        }
    }

    if needs_update || restored_any {
        update(wm);
    }

    apply_focus_and_borders(wm, seat);

    // Focus raising in apply_focus_and_borders (including a focused tile
    // raising above floats) is overridden here within the same transaction.
    raise_floating_windows(wm);

    // Warp the pointer to the focused output's center when focus moved to a
    // different output (niri/hyprland behavior; keeps the cursor off
    // disabled or newly-connected displays).
    if wm.needs_pointer_warp {
        wm.needs_pointer_warp = false;
        if let Some(output) = wm.focused_output() {
            seat.pointer_warp(
                output.rectangle.x + output.rectangle.width / 2,
                output.rectangle.y + output.rectangle.height / 2,
            );
        }
    }
}

/// Commit layout targets immediately (no animation): current = finish.
pub fn snap_to_finish(wm: &mut WindowManager) {
    let config = wm.config.clone();
    // During overview every window's grid cell is drawn on the focused
    // output (overview::enter), so visibility and clipping must be measured
    // against that output, not each window's home output — otherwise all
    // windows from the other displays are hidden/clipped off the grid.
    let clip = if wm.overview_state.is_some() {
        wm.focused_output_idx
            .and_then(|i| wm.outputs.get(i))
            .map(|o| o.rectangle)
    } else {
        None
    };
    // The focused fullscreen window is handed to the compositor, which then
    // owns its position and clips content, borders and decorations to the
    // output (river-window-management-v1.fullscreen). Only the focused one:
    // fullscreen windows on other workspaces must stay on the WM's own
    // off-screen stacking, and the overview grid clears is_fullscreen anyway.
    let focused_output_idx = wm.focused_output_idx;
    for (output_idx, output) in wm.outputs.iter_mut().enumerate() {
        if output.is_removed {
            continue;
        }
        let river_output = output.river_output.clone();
        let focused_ws_idx = output.focused_workspace_idx;
        for (workspace_idx, workspace) in output.workspace_list.iter_mut().enumerate() {
            let ws_focus = workspace.focused_window_idx;
            for (window_idx, window) in workspace.window_list.iter_mut().enumerate() {
                let Window {
                    river_window,
                    river_node,
                    geom,
                } = window;
                if let Some(finish) = geom.finish {
                    geom.current = finish;
                    common::place_window(
                        river_window,
                        river_node,
                        geom,
                        clip.unwrap_or(output.rectangle),
                        &config,
                    );
                    if geom.is_fullscreen {
                        let is_focused = focused_output_idx == Some(output_idx)
                            && workspace_idx == focused_ws_idx
                            && Some(window_idx) == ws_focus;
                        if is_focused {
                            river_window.fullscreen(&river_output);
                        }
                        river_window.inform_fullscreen();
                    } else {
                        river_window.inform_not_fullscreen();
                    }
                    geom.finish = None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::focus::color_to_river;
    use crate::types::Color;

    #[test]
    fn focused_output_idx_stays_valid_after_swap_remove() {
        let mut focused: Option<usize> = Some(0);

        // Remove output 0 from a list of 2: focus moves to the survivor.
        fixup_indices_after_output_removal(&mut focused, 0, 2);
        assert_eq!(focused, Some(0));

        // Remove output 1 from a list of 2: focus (0) unchanged.
        focused = Some(0);
        fixup_indices_after_output_removal(&mut focused, 1, 2);
        assert_eq!(focused, Some(0));

        // Remove output 0 from a list of 1: no outputs left.
        focused = Some(0);
        fixup_indices_after_output_removal(&mut focused, 0, 1);
        assert_eq!(focused, None);

        // Remove output 0 from a list of 3: focus lands on the new output 0.
        focused = Some(0);
        fixup_indices_after_output_removal(&mut focused, 0, 3);
        assert_eq!(focused, Some(0));

        // Focus beyond the removed index shifts down.
        focused = Some(2);
        fixup_indices_after_output_removal(&mut focused, 0, 3);
        assert_eq!(focused, Some(1));
    }

    #[test]
    fn color_to_river_full_alpha_scales_channels() {
        let color = Color {
            r: 141,
            g: 214,
            b: 0,
            a: 1.0,
        };
        let (r, g, b, a) = color_to_river(color);
        assert_eq!(a, u32::MAX);
        assert_eq!(r, ((141.0f32 / 255.0) as f64 * u32::MAX as f64) as u32);
        assert_eq!(g, ((214.0f32 / 255.0) as f64 * u32::MAX as f64) as u32);
        assert_eq!(b, 0);
    }

    #[test]
    fn color_to_river_half_alpha() {
        let color = Color {
            r: 0,
            g: 0,
            b: 0,
            a: 0.5,
        };
        let (r, g, b, a) = color_to_river(color);
        assert_eq!(a, (0.5f64 * u32::MAX as f64) as u32);
        assert_eq!((r, g, b), (0, 0, 0));
    }
}
