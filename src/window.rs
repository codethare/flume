// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Window lifecycle, ported from rill-ed window.zig.

use wayland_client::{Dispatch, Proxy, QueueHandle};

use crate::app::AppData;
use crate::layout::{self, common};
use crate::river::river_window_v1::{self, RiverWindowV1};
use crate::types::{Layout, PendingWindow, Status, Window, WindowGeom};
use crate::wm::WindowManager;

/// Handle a river_window_v1 event. Returns true when a manage sequence
/// should be requested.
pub fn window_event(
    state: &mut AppData,
    river_window: &RiverWindowV1,
    event: river_window_v1::Event,
    qh: &QueueHandle<AppData>,
) -> bool {
    match &event {
        river_window_v1::Event::Title { title: Some(title) } => {
            set_pending_string(
                &mut state.wm,
                river_window,
                title.clone(),
                PendingField::Title,
            );
        }
        river_window_v1::Event::AppId {
            app_id: Some(app_id),
        } => {
            set_pending_string(
                &mut state.wm,
                river_window,
                app_id.clone(),
                PendingField::AppId,
            );
        }
        _ => {}
    }

    if let river_window_v1::Event::Closed = event {
        // Closed before ever getting dimensions: drop the pending window.
        if let Some(idx) = state
            .wm
            .pending_windows
            .iter()
            .position(|p| &p.river_window == river_window)
        {
            state.wm.pending_windows.swap_remove(idx);
            river_window.destroy();
            return false;
        }
    }

    if let river_window_v1::Event::Dimensions { .. } = event
        && let Some((output_idx, _ws_idx)) = state.wm.current_ws_idx()
        && let Some(idx) = state
            .wm
            .pending_windows
            .iter()
            .position(|p| &p.river_window == river_window)
    {
        let pending = state.wm.pending_windows.swap_remove(idx);
        add_window(&mut state.wm, pending, output_idx, qh);

        layout::update(&mut state.wm);
        state.wm.status = Status::Layout;
        return true;
    }

    for output_idx in 0..state.wm.outputs.len() {
        for workspace_idx in 0..10 {
            let Some(focused_idx) = state
                .wm
                .workspace(output_idx, workspace_idx)
                .and_then(|ws| ws.focused_window_idx)
            else {
                continue;
            };
            let Some(win_idx) = state
                .wm
                .workspace(output_idx, workspace_idx)
                .unwrap()
                .window_list
                .iter()
                .position(|w| &w.river_window == river_window)
            else {
                continue;
            };

            match event {
                river_window_v1::Event::Closed => {
                    let workspace = state.wm.workspace_mut(output_idx, workspace_idx).unwrap();
                    if workspace.window_list.len() == 1 {
                        workspace.focused_window_idx = None;
                    } else if win_idx <= focused_idx && focused_idx != 0 {
                        workspace.focused_window_idx = Some(focused_idx - 1);
                    }
                    workspace.window_list.remove(win_idx);
                    if state.wm.last_focused_window.as_ref() == Some(river_window) {
                        state.wm.last_focused_window = None;
                    }
                    if state.wm.lock_focus.as_ref() == Some(river_window) {
                        state.wm.lock_focus = None;
                    }
                    river_window.destroy();
                    // Keep the overview grid consistent when a window is
                    // closed while the overview is showing it (may cancel
                    // the overview and refocus the pre-overview workspace).
                    crate::overview::prune(state);
                }
                river_window_v1::Event::FullscreenRequested { .. } => {
                    state
                        .wm
                        .workspace_mut(output_idx, workspace_idx)
                        .unwrap()
                        .window_list[win_idx]
                        .geom
                        .is_fullscreen = true;
                }
                river_window_v1::Event::ExitFullscreenRequested => {
                    state
                        .wm
                        .workspace_mut(output_idx, workspace_idx)
                        .unwrap()
                        .window_list[win_idx]
                        .geom
                        .is_fullscreen = false;
                }
                _ => return false,
            }
            layout::update(&mut state.wm);
            state.wm.status = if state.wm.overview_state.is_some() {
                Status::Overview
            } else {
                Status::Layout
            };
            return false;
        }
    }

    // The window may belong to a detached (removed) output. A closed event
    // for a detached window must still be honored, otherwise the proxy ends
    // up in a restored workspace as a dead window.
    if let river_window_v1::Event::Closed = event {
        for detached in state.wm.detached_outputs.values_mut() {
            for workspace in &mut detached.workspace_list {
                let Some(focused_idx) = workspace.focused_window_idx else {
                    continue;
                };
                let Some(win_idx) = workspace
                    .window_list
                    .iter()
                    .position(|w| &w.river_window == river_window)
                else {
                    continue;
                };

                if workspace.window_list.len() == 1 {
                    workspace.focused_window_idx = None;
                } else if win_idx <= focused_idx && focused_idx != 0 {
                    workspace.focused_window_idx = Some(focused_idx - 1);
                }
                workspace.window_list.remove(win_idx);
                if state.wm.last_focused_window.as_ref() == Some(river_window) {
                    state.wm.last_focused_window = None;
                }
                if state.wm.lock_focus.as_ref() == Some(river_window) {
                    state.wm.lock_focus = None;
                }
                river_window.destroy();
                return false;
            }
        }
    }

    false
}

enum PendingField {
    Title,
    AppId,
}

fn set_pending_string(
    wm: &mut WindowManager,
    river_window: &RiverWindowV1,
    value: String,
    field: PendingField,
) {
    for pending in &mut wm.pending_windows {
        if &pending.river_window != river_window {
            continue;
        }
        match field {
            PendingField::Title => pending.title = Some(value),
            PendingField::AppId => pending.app_id = Some(value),
        }
        return;
    }
}

/// Move a pending window into the output's focused workspace (rill-ed `add`).
fn add_window(
    wm: &mut WindowManager,
    pending: PendingWindow,
    output_idx: usize,
    qh: &QueueHandle<AppData>,
) {
    let config = wm.config.clone();
    let ws_idx = wm.outputs[output_idx].focused_workspace_idx;
    let output = &mut wm.outputs[output_idx];
    let workspace = &mut output.workspace_list[ws_idx];

    // A new window landing here means the user is actively using this
    // workspace; clear former_output_name so previously-migrated windows
    // stay on this output instead of jumping back on reconnection.
    for w in &mut workspace.window_list {
        w.geom.former_output_name = None;
    }

    let is_floating = config.window_rules.iter().any(|rule| {
        rule.floating && rule.matches(pending.app_id.as_deref(), pending.title.as_deref())
    });

    // New windows start just off the right edge (or at the centered floating
    // rect) so the first manage pass moves them into place.
    let rightmost = common::initial_rectangle(output.non_exclusive, &config);
    let floating_rect = if workspace.layout == Layout::Floating || is_floating {
        common::center_rectangle(output.non_exclusive, &config)
    } else {
        rightmost
    };

    let mut start_rect = rightmost;
    start_rect.x = output.non_exclusive.x + output.non_exclusive.width;

    let river_node = pending.river_window.get_node(qh, ());
    let mut geom = WindowGeom::new(config.default_window_width, start_rect);
    geom.is_floating = is_floating;
    geom.floating = floating_rect;
    let window = Window {
        river_window: pending.river_window,
        river_node,
        geom,
    };

    let window_idx = workspace.focused_window_idx.map_or(0, |idx| idx + 1);
    workspace.window_list.insert(window_idx, window);
    workspace.focused_window_idx = Some(window_idx);
}

impl Dispatch<RiverWindowV1, ()> for AppData {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowV1,
        event: <RiverWindowV1 as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        qh: &QueueHandle<Self>,
    ) {
        let needs_manage = window_event(state, proxy, event, qh);
        if needs_manage {
            state.manage_dirty();
        }
    }
}
