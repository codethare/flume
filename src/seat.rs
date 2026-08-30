// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Seat handling: pointer bindings, pointer ops (move/resize), layer-shell
//! seat focus tracking. Ported from rill-ed seat.zig.

use wayland_client::{Dispatch, Proxy, QueueHandle};

use crate::actions::PointerAction;
use crate::app::{AppData, PointerBindingEntry};
use crate::layout;
use crate::river::river_layer_shell_seat_v1::RiverLayerShellSeatV1;
use crate::river::river_pointer_binding_v1::RiverPointerBindingV1;
use crate::river::river_seat_v1::{self, RiverSeatV1};
use crate::types::Status;
use crate::wm::LayerShellFocus;

/// A new river seat appeared (river_window_manager_v1.seat event).
pub fn seat_added(state: &mut AppData, qh: &QueueHandle<AppData>) {
    if let Some(cursor) = state.wm.config.cursor.clone()
        && let Some(seat) = &state.river_seat
    {
        seat.set_xcursor_theme(cursor.theme.clone(), cursor.size);
    }
    if let (Some(layer_shell), Some(seat)) = (&state.river_layer_shell, &state.river_seat) {
        state.layer_shell_seat = Some(layer_shell.get_seat(seat, qh, ()));
    }
}

pub fn seat_event(
    state: &mut AppData,
    _seat: &RiverSeatV1,
    event: <RiverSeatV1 as Proxy>::Event,
) -> bool {
    let (output_idx, workspace_idx) = match state.wm.current_ws_idx() {
        Some(v) => v,
        None => return false,
    };
    let focused_win_idx = match state
        .wm
        .workspace(output_idx, workspace_idx)
        .and_then(|ws| ws.focused_window_idx)
    {
        Some(v) => v,
        None => return false,
    };

    match event {
        // During overview, hovering a window highlights it (macOS Mission
        // Control style). Outside overview these events are ignored.
        river_seat_v1::Event::PointerEnter { window } => {
            let Some(ov_state) = &mut state.wm.overview_state else {
                return false;
            };
            if let Some(idx) = ov_state
                .entries
                .iter()
                .position(|entry| entry.window == window)
                && ov_state.highlighted != idx
            {
                ov_state.highlighted = idx;
                state.wm.status = Status::Overview;
                return true;
            }
            false
        }
        // Keep the last hover highlight; pointer_leave does not clear it.
        river_seat_v1::Event::PointerLeave => false,

        river_seat_v1::Event::WindowInteraction { window } => {
            // During overview, clicking a window selects it.
            if state.wm.overview_state.is_some() {
                let idx = state
                    .wm
                    .overview_state
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .position(|entry| entry.window == window);
                if let Some(idx) = idx {
                    crate::overview::select_index(state, idx);
                    layout::update(&mut state.wm);
                    state.wm.status = Status::Layout;
                }
                return false;
            }

            if state
                .wm
                .focused_window()
                .map(|w| w.river_window == window)
                .unwrap_or(false)
            {
                return false;
            }

            for target_output_idx in 0..state.wm.outputs.len() {
                let target_ws_idx = state.wm.outputs[target_output_idx].focused_workspace_idx;
                let Some(target_win_idx) = state
                    .wm
                    .workspace(target_output_idx, target_ws_idx)
                    .unwrap()
                    .window_list
                    .iter()
                    .position(|w| w.river_window == window)
                else {
                    continue;
                };

                state.wm.focused_output_idx = Some(target_output_idx);
                state.wm.outputs[target_output_idx].workspace_list[target_ws_idx]
                    .focused_window_idx = Some(target_win_idx);

                if target_output_idx != output_idx {
                    state.wm.previous_workspace = Some(crate::types::OverviewHome {
                        output_idx,
                        workspace_idx,
                    });
                }

                layout::update(&mut state.wm);
                state.wm.status = Status::Layout;
                return false;
            }
            false
        }
        river_seat_v1::Event::OpDelta { dx, dy } => {
            let action = match &state.wm.status {
                Status::PointerAction(action) => *action,
                _ => return false,
            };
            let Some(origin) = state
                .wm
                .workspace_mut(output_idx, workspace_idx)
                .unwrap()
                .window_list
                .get_mut(focused_win_idx)
                .and_then(|w| w.geom.drag_origin)
            else {
                return false;
            };
            let output_rect = state.wm.outputs[output_idx].rectangle;
            let workspace = state.wm.workspace_mut(output_idx, workspace_idx).unwrap();
            let geom = &mut workspace.window_list[focused_win_idx].geom;

            let output_left = output_rect.x;
            let output_right = output_rect.x + output_rect.width;
            let output_top = output_rect.y;
            let output_bottom = output_rect.y + output_rect.height;

            match action {
                PointerAction::MoveWindow => {
                    let width = geom.current.width;
                    let height = geom.current.height;
                    geom.floating.x = (origin.x + dx).clamp(output_left, output_right - width);
                    geom.floating.y = (origin.y + dy).clamp(output_top, output_bottom - height);
                }
                PointerAction::ResizeWindow => {
                    geom.floating.width =
                        (origin.width + dx).clamp(0, output_right - geom.current.x);
                    geom.floating.height =
                        (origin.height + dy).clamp(0, output_bottom - geom.current.y);
                }
            }
            geom.current = geom.floating;
            false
        }
        river_seat_v1::Event::Removed
        | river_seat_v1::Event::WlSeat { .. }
        | river_seat_v1::Event::ShellSurfaceInteraction { .. }
        | river_seat_v1::Event::PointerPosition { .. } => false,
        river_seat_v1::Event::OpRelease => {
            state.wm.status = Status::None;
            if let Some(window) = state
                .wm
                .workspace_mut(output_idx, workspace_idx)
                .unwrap()
                .window_list
                .get_mut(focused_win_idx)
            {
                window.geom.drag_origin = None;
            }
            false
        }
    }
}

/// Rebuild pointer bindings from config (rill-ed setupPointerBindings).
pub fn setup_pointer_bindings(state: &mut AppData) {
    for binding in state.pointer_bindings.drain(..) {
        binding.proxy.destroy();
    }
    let Some(seat) = state.river_seat.clone() else {
        return;
    };
    for binding in &state.wm.config.pointer_bindings {
        let Ok(mods) = crate::keybinding::parse_modifiers(&binding.modifiers) else {
            eprintln!("Invalid modifiers: {:?}", binding.modifiers);
            continue;
        };
        let proxy = seat.get_pointer_binding(binding.button.code(), mods, &state.qh, ());
        proxy.enable();
        state.pointer_bindings.push(PointerBindingEntry {
            proxy,
            action: binding.action,
        });
    }
}

fn pointer_binding_action(state: &AppData, proxy: &RiverPointerBindingV1) -> Option<PointerAction> {
    state
        .pointer_bindings
        .iter()
        .find(|b| &b.proxy == proxy)
        .map(|b| b.action)
}

pub fn layer_shell_seat_event(
    state: &mut AppData,
    event: <RiverLayerShellSeatV1 as Proxy>::Event,
) -> bool {
    use crate::river::river_layer_shell_seat_v1::Event;
    match event {
        Event::FocusExclusive => {
            state.wm.layer_shell_focus = LayerShellFocus::Exclusive;
            state.wm.status = Status::Layout;
            false
        }
        Event::FocusNonExclusive => {
            state.wm.layer_shell_focus = LayerShellFocus::NonExclusive;
            state.wm.status = Status::Layout;
            false
        }
        Event::FocusNone => {
            state.wm.layer_shell_focus = LayerShellFocus::None;
            // Exclusive layer-shell focus was revoked; invalidate the stale
            // focus cache so focus_window is re-issued.
            state.wm.last_focused_window = None;
            state.wm.needs_refocus = true;
            state.wm.status = Status::Layout;
            true
        }
    }
}

/// Send the current geometry of the focused window for a pointer op
/// (rill-ed pointerAction, called at the start of the manage cycle).
pub fn pointer_action(wm: &mut crate::wm::WindowManager) {
    let Some((output_idx, workspace_idx)) = wm.current_ws_idx() else {
        return;
    };
    let Some(window_idx) = wm
        .workspace(output_idx, workspace_idx)
        .and_then(|ws| ws.focused_window_idx)
    else {
        return;
    };
    let border_width = wm.config.border.width as i32;
    let workspace = wm.workspace_mut(output_idx, workspace_idx).unwrap();
    let WindowRef {
        river_window,
        river_node,
        geom,
    } = {
        let window = &mut workspace.window_list[window_idx];
        WindowRef {
            river_window: window.river_window.clone(),
            river_node: window.river_node.clone(),
            geom: &mut window.geom,
        }
    };

    river_window.set_clip_box(0, 0, 0, 0);

    let border = if geom.is_fullscreen { 0 } else { border_width };
    river_window.propose_dimensions(
        (geom.current.width - 2 * border).max(0),
        (geom.current.height - 2 * border).max(0),
    );
    river_node.set_position(geom.current.x + border, geom.current.y + border);
}

struct WindowRef<'a> {
    river_window: crate::river::river_window_v1::RiverWindowV1,
    river_node: crate::river::river_node_v1::RiverNodeV1,
    geom: &'a mut crate::types::WindowGeom,
}

impl Dispatch<RiverSeatV1, ()> for AppData {
    fn event(
        state: &mut Self,
        seat: &RiverSeatV1,
        event: <RiverSeatV1 as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let _ = seat_event(state, seat, event);
    }
}

impl Dispatch<RiverPointerBindingV1, ()> for AppData {
    fn event(
        state: &mut Self,
        proxy: &RiverPointerBindingV1,
        event: <RiverPointerBindingV1 as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use crate::river::river_pointer_binding_v1::Event;
        let Event::Pressed = event else {
            return;
        };
        let Some(action) = pointer_binding_action(state, proxy) else {
            return;
        };
        // Drag the focused window only; needs a floating, non-fullscreen one.
        let Some(window) = state.wm.focused_window_mut() else {
            return;
        };
        if !window.geom.is_floating || window.geom.is_fullscreen {
            return;
        }
        window.geom.drag_origin = Some(window.geom.current);
        state.wm.status = Status::PointerAction(action);
    }
}

impl Dispatch<RiverLayerShellSeatV1, ()> for AppData {
    fn event(
        state: &mut Self,
        _proxy: &RiverLayerShellSeatV1,
        event: <RiverLayerShellSeatV1 as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let needs_manage = layer_shell_seat_event(state, event);
        if needs_manage {
            state.manage_dirty();
        }
    }
}
