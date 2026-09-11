// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Seat/pointer scenarios: focus-follows-pointer and the pointer binding drag
//! (move/resize) path.

use super::*;

/// Focus-follows-pointer: hover refocuses exactly like clicking (sloppy
/// focus) when `focus_follows_pointer` is enabled; with the toggle off,
/// hovering must not move focus.
#[test]
fn pointer_enter_focus_follows_toggle() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "eDP-1", (0, 0), (1360, 768));
    s.manage(&mut sv);
    let a = s.add_window(&mut sv);
    s.manage(&mut sv);
    let b = s.add_window(&mut sv);
    s.manage(&mut sv);
    s.check_consistent();

    let ws = &s.state.wm.outputs[0].workspace_list[0];
    let initial = ws.focused_window_idx.expect("window has focus");
    // Server-side id for the event payload, client-side proxy for the check.
    let (hover, hover_proxy) = if initial == 0 {
        (b, ws.window_list[1].river_window.clone())
    } else {
        (a, ws.window_list[0].river_window.clone())
    };

    // Toggle off: hover changes nothing.
    s.config_mut().focus_follows_pointer = false;
    s.send(
        &mut sv,
        seat.clone(),
        EVT_SEAT_POINTER_ENTER,
        vec![Argument::Object(hover.clone())],
    );
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(initial),
        "hover must not move focus when focus_follows_pointer is off"
    );

    // Toggle on: hovering the other window refocuses it.
    s.config_mut().focus_follows_pointer = true;
    s.send(
        &mut sv,
        seat,
        EVT_SEAT_POINTER_ENTER,
        vec![Argument::Object(hover)],
    );
    s.manage(&mut sv);
    let ws = &s.state.wm.outputs[0].workspace_list[0];
    let now = ws.focused_window_idx.expect("window has focus");
    assert_ne!(now, initial, "hover must move focus when enabled");
    assert_eq!(
        ws.window_list[now].river_window, hover_proxy,
        "focused window must be the hovered one"
    );
}

/// seat.rs: a pointer binding press starts a drag, op_delta follows the
/// pointer (clamped to the output), op_release ends it. This is the
/// `move_window` path behind drag-to-move and side-button bindings.
#[test]
fn pointer_binding_drag_moves_floating_window() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    // Bindings are only created once a focused output exists (manage returns
    // early before that), so capture them after the first output manage.
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    let binds = pointer_binding_objects(&sv);
    let move_binding = binds[0].clone();

    s.add_window(&mut sv);
    s.manage(&mut sv);
    s.check_consistent();

    // Drag only applies to floating windows (seat.rs press handler).
    crate::keybinding::dispatch_action(
        &mut s.state,
        &crate::actions::KeybindingAction::ToggleWorkspaceFloating,
    );
    s.manage(&mut sv);
    let origin = s.state.wm.outputs[0].workspace_list[0].window_list[0]
        .geom
        .current;
    assert!(
        s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .is_floating,
        "window must be floating for the drag binding to engage"
    );

    s.send(&mut sv, move_binding, EVT_PTR_BINDING_PRESSED, vec![]);
    s.manage(&mut sv);
    assert!(
        matches!(
            s.state.wm.status,
            crate::types::Status::PointerAction(crate::actions::PointerAction::MoveWindow)
        ),
        "press must enter the move drag state, got {:?}",
        s.state.wm.status
    );

    // Compose the op delta: the window follows the pointer, clamped to the
    // output and pushed to the compositor.
    sv.clear_request_log();
    s.send(
        &mut sv,
        seat.clone(),
        EVT_SEAT_OP_DELTA,
        vec![Argument::Int(100), Argument::Int(50)],
    );
    s.manage(&mut sv);
    let moved = s.state.wm.outputs[0].workspace_list[0].window_list[0]
        .geom
        .current;
    assert_eq!(
        (moved.x, moved.y),
        (origin.x + 100, origin.y + 50),
        "window must follow the pointer delta"
    );
    assert!(
        node_positions(&sv, moved.x + 3, moved.y + 3) > 0,
        "the dragged position must reach the compositor (border inset)"
    );

    s.send(&mut sv, seat, EVT_SEAT_OP_RELEASE, vec![]);
    assert_eq!(
        s.state.wm.status,
        crate::types::Status::None,
        "release ends the drag"
    );
    assert!(
        s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .drag_origin
            .is_none(),
        "drag origin must be cleared on release"
    );
    s.manage(&mut sv);
}

/// A removed seat takes its bindings with it: river asserts that the previous
/// bindings are gone before it hands out a seat object again, and the next
/// seat must get a fresh, working set.
#[test]
fn removed_seat_destroys_its_bindings_and_a_new_seat_gets_new_ones() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_wl_seat(&mut sv, seat.clone());
    s.add_window(&mut sv);
    s.manage(&mut sv);
    assert_eq!(s.state.pointer_bindings.len(), 2);
    assert!(s.state.river_seat.is_some());
    assert!(s.state.wl_seat.is_some());
    assert!(s.state.layer_shell_seat.is_some());

    let bindings: Vec<ObjectId> = children_with_interface(&sv, "river_xkb_binding_v1")
        .into_iter()
        .chain(children_with_interface(&sv, "river_pointer_binding_v1"))
        .collect();
    assert!(!bindings.is_empty());

    sv.clear_request_log();
    s.send(&mut sv, seat.clone(), EVT_SEAT_REMOVED, vec![]);
    assert!(
        s.state.river_seat.is_none(),
        "the stale seat proxy is dropped"
    );
    assert!(s.state.wl_seat.is_none());
    assert!(s.state.layer_shell_seat.is_none());
    assert!(s.state.xkb_bindings.is_empty());
    assert!(s.state.pointer_bindings.is_empty());
    assert_eq!(
        count_requests(&sv, &seat, REQ_SEAT_DESTROY),
        1,
        "the removed seat object is destroyed"
    );
    let destroyed = bindings
        .iter()
        .filter(|b| count_requests(&sv, b, REQ_BINDING_DESTROY) == 1)
        .count();
    assert_eq!(
        destroyed,
        bindings.len(),
        "every xkb and pointer binding is destroyed"
    );

    // A new seat re-establishes bindings.
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    assert!(s.state.river_seat.is_some());
    assert_eq!(s.state.pointer_bindings.len(), 2);
    assert!(s.state.xkb_bindings.len() > 50);
    s.check_consistent();
}
