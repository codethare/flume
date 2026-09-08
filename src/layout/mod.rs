// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Layout coordinator, ported from rill-ed layout.zig.
//!
//! No animation: `update` computes `finish` targets, `snap_to_finish`
//! commits them immediately (`current = finish`).

pub mod common;
pub mod floating;
pub mod scroller;

use crate::river::river_seat_v1::RiverSeatV1;
use crate::river::wayland_client::Proxy;
use crate::types::{Color, DetachedOutput, Layout, Window, WindowGeom};
use crate::wm::{LayerShellFocus, WindowManager};

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
            match workspace.layout {
                Layout::Floating => floating::apply(&mut geoms, output.rectangle, y_offset),
                Layout::Scroller => scroller::apply(
                    &mut geoms,
                    workspace.focused_window_idx,
                    output.rectangle,
                    output.non_exclusive,
                    &config,
                    y_offset,
                ),
            }
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

fn reset_sent_caches(geom: &mut WindowGeom) {
    geom.sent_visible = None;
    geom.sent_current = None;
    geom.sent_clip = None;
    geom.sent_border_focused = None;
    geom.sent_border_width = None;
}

fn exit_fullscreen_and_close_closing(wm: &mut WindowManager, output_idx: usize) {
    let output_rect = wm.outputs[output_idx].rectangle;
    for workspace in &mut wm.outputs[output_idx].workspace_list {
        for window in &mut workspace.window_list {
            // Windows sitting at (or near) the fullscreen rect still hold
            // compositor-side fullscreen; release it. Over-calling
            // exit_fullscreen is a no-op, so bias toward calling.
            let pos = window.geom.current;
            let at_fullscreen_rect = window.geom.is_fullscreen
                || ((pos.x - output_rect.x).abs() <= 4
                    && (pos.y - output_rect.y).abs() <= 4
                    && (pos.width - output_rect.width).abs() <= 4
                    && (pos.height - output_rect.height).abs() <= 4);
            if at_fullscreen_rect {
                window.river_window.exit_fullscreen();
            }
            if window.geom.is_closing {
                window.river_window.close();
            }
        }
    }
}

/// Migrate all windows of a removed output to a surviving output,
/// workspace-by-workspace. Must run inside the manage sequence; the event
/// handler only sets is_removed.
fn migrate_output_windows(wm: &mut WindowManager, removed_idx: usize, survivor_idx: usize) {
    let src_name = wm.outputs[removed_idx].name.clone();
    let mut src_workspaces = std::mem::take(&mut wm.outputs[removed_idx].workspace_list);
    {
        let target = &mut wm.outputs[survivor_idx];
        for (src_ws, dst_ws) in src_workspaces
            .iter_mut()
            .zip(target.workspace_list.iter_mut())
        {
            for mut window in src_ws.window_list.drain(..) {
                if window.geom.is_fullscreen {
                    window.river_window.exit_fullscreen();
                }
                window.geom.former_output_name = src_name.clone();
                dst_ws.window_list.push(window);
                if dst_ws.focused_window_idx.is_none() {
                    dst_ws.focused_window_idx = Some(dst_ws.window_list.len() - 1);
                }
            }
            src_ws.focused_window_idx = None;
        }
    }
    wm.outputs[removed_idx].workspace_list = src_workspaces;
}

/// Preserve a removed output's workspaces keyed by its name. Any previously
/// detached entry under the same name is closed out.
fn detach_output(wm: &mut WindowManager, removed_idx: usize) {
    let Some(name) = wm.outputs[removed_idx].name.clone() else {
        return; // no name: windows stay in the output for the cleanup pass
    };
    let detached = DetachedOutput {
        workspace_list: std::mem::take(&mut wm.outputs[removed_idx].workspace_list),
        focused_workspace_idx: wm.outputs[removed_idx].focused_workspace_idx,
    };
    if let Some(old) = wm.detached_outputs.insert(name, detached) {
        for workspace in old.workspace_list {
            for window in workspace.window_list {
                window.river_window.close();
            }
        }
    }
}

/// Move a detached output's windows into the fallback output's matching
/// workspaces, tagging them with the detached output's name so they can
/// return if it reappears.
fn migrate_detached_into(
    wm: &mut WindowManager,
    target_idx: usize,
    key: &str,
    detached: DetachedOutput,
) {
    let mut detached = detached;
    {
        let target = &mut wm.outputs[target_idx];
        for (src_ws, dst_ws) in detached
            .workspace_list
            .iter_mut()
            .zip(target.workspace_list.iter_mut())
        {
            for mut window in src_ws.window_list.drain(..) {
                if window.geom.is_fullscreen {
                    window.river_window.exit_fullscreen();
                }
                if window.geom.former_output_name.is_none() {
                    window.geom.former_output_name = Some(key.to_string());
                }
                dst_ws.window_list.push(window);
                if dst_ws.focused_window_idx.is_none() {
                    dst_ws.focused_window_idx = Some(dst_ws.window_list.len() - 1);
                }
            }
        }
    }
}

/// Move windows whose former_output_name matches `dst_name` from src to dst.
/// Returns true if anything moved. Replicates rill-ed's focus-index fixup.
fn migrate_windows_by_name(
    wm: &mut WindowManager,
    src_idx: usize,
    dst_idx: usize,
    ws_idx: usize,
    dst_name: &str,
) -> bool {
    let src_list = std::mem::take(&mut wm.outputs[src_idx].workspace_list[ws_idx].window_list);
    let src_focus = wm.outputs[src_idx].workspace_list[ws_idx].focused_window_idx;

    let mut moved: Vec<Window> = Vec::new();
    let mut kept: Vec<Window> = Vec::new();
    let mut removed_idxs: Vec<usize> = Vec::new();
    for (i, mut window) in src_list.into_iter().enumerate() {
        if window.geom.former_output_name.as_deref() == Some(dst_name) {
            window.geom.former_output_name = None;
            removed_idxs.push(i);
            moved.push(window);
        } else {
            kept.push(window);
        }
    }
    if moved.is_empty() {
        wm.outputs[src_idx].workspace_list[ws_idx].window_list = kept;
        return false;
    }

    // rill-ed removes indices in descending order and fixes up focus per
    // removal; apply the same rule over the same order.
    let mut focus = src_focus;
    for &i in removed_idxs.iter().rev() {
        if let Some(f) = focus
            && f >= i
        {
            focus = if f > 0 { Some(f - 1) } else { None };
        }
    }

    // Insert at the front in reverse so the moved windows keep their
    // original relative order at the destination.
    {
        let dst_ws = &mut wm.outputs[dst_idx].workspace_list[ws_idx];
        for window in moved.into_iter().rev() {
            dst_ws.window_list.insert(0, window);
        }
        if dst_ws.focused_window_idx.is_none() {
            dst_ws.focused_window_idx = Some(0);
        }
    }

    let src_ws = &mut wm.outputs[src_idx].workspace_list[ws_idx];
    src_ws.window_list = kept;
    src_ws.focused_window_idx = focus;
    true
}

/// Fixup `focused` after `removed_idx` is swap-removed from a list of
/// `old_len` entries (rill-ed layout.zig, unit-tested there too).
fn fixup_indices_after_output_removal(
    focused: &mut Option<usize>,
    removed_idx: usize,
    old_len: usize,
) {
    if let Some(foi) = *focused {
        if foi == removed_idx {
            if old_len > 1 {
                *focused = Some(removed_idx.min(old_len - 2));
            } else {
                *focused = None;
            }
        } else if foi > removed_idx {
            *focused = Some(foi - 1);
        }
    }
}

/// Set border colors and keyboard focus to match the current focused
/// window/output. Safe to call every manage pass: redundant border and focus
/// requests are skipped so IME clients (fcitx5) are not disrupted.
pub fn apply_focus_and_borders(wm: &mut WindowManager, seat: &RiverSeatV1) {
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
pub fn raise_floating_windows(wm: &mut WindowManager) {
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
    for output in &mut wm.outputs {
        if output.is_removed {
            continue;
        }
        for workspace in &mut output.workspace_list {
            for window in &mut workspace.window_list {
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

#[cfg(test)]
mod tests {
    use super::*;

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
