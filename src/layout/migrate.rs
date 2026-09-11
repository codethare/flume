// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Output removal: migrating windows to a surviving output, and detaching
//! workspaces when no output is left.

use crate::types::{DetachedOutput, Window, WindowGeom};
use crate::wm::WindowManager;

pub(super) fn reset_sent_caches(geom: &mut WindowGeom) {
    geom.sent_visible = None;
    geom.sent_current = None;
    geom.sent_clip = None;
    geom.sent_border_focused = None;
    geom.sent_border_width = None;
}

pub(super) fn exit_fullscreen_and_close_closing(wm: &mut WindowManager, output_idx: usize) {
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
pub(super) fn migrate_output_windows(
    wm: &mut WindowManager,
    removed_idx: usize,
    survivor_idx: usize,
) {
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
pub(super) fn detach_output(wm: &mut WindowManager, removed_idx: usize) {
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
pub(super) fn migrate_detached_into(
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
pub(super) fn migrate_windows_by_name(
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
pub(super) fn fixup_indices_after_output_removal(
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
