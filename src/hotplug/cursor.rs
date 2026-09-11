// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: BSD-0-Clause

//! Cursor feedback during pointer operations: river applies a WM-set cursor
//! shape while no client has pointer focus, which is exactly during a drag.

use super::*;

/// Shape requests (opcode 1) on the cursor shape device, as raw args.
fn shape_requests(server: &MiniServer, device: &ObjectId) -> Vec<String> {
    server
        .requests_for(device)
        .into_iter()
        .filter(|(_, op, _)| *op == REQ_CURSOR_SHAPE_SET_SHAPE)
        .map(|(_, _, args)| args)
        .collect()
}

/// A drag sets the move cursor and hands the cursor back on release.
#[test]
fn drag_sets_and_restores_the_cursor_shape() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_wl_seat(&mut sv, seat.clone());

    let move_binding = pointer_binding_objects(&sv)[0].clone();
    // Created by wp_cursor_shape_manager_v1.get_pointer during add_wl_seat.
    let device = children_with_interface(&sv, "wp_cursor_shape_device_v1")
        .pop()
        .expect("cursor shape device created");

    s.add_window(&mut sv);
    s.manage(&mut sv);
    crate::keybinding::dispatch_action(
        &mut s.state,
        &crate::actions::KeybindingAction::ToggleWorkspaceFloating,
    );
    s.manage(&mut sv);
    assert!(
        s.state.cursor_shape.is_some(),
        "the client must create the cursor shape device when the compositor offers it"
    );

    sv.clear_request_log();
    s.send(&mut sv, move_binding, EVT_PTR_BINDING_PRESSED, vec![]);
    assert_eq!(
        shape_requests(&sv, &device),
        vec!["u0,u13".to_string()],
        "a move drag must set Shape::Move (13)"
    );

    s.send(&mut sv, seat, EVT_SEAT_OP_RELEASE, vec![]);
    assert_eq!(
        shape_requests(&sv, &device),
        vec!["u0,u13".to_string(), "u0,u1".to_string()],
        "release must hand the cursor back (Shape::Default = 1)"
    );
}

/// Without the cursor shape manager (compositors that do not offer it) the
/// drag still works and nothing is requested.
#[test]
fn drag_without_cursor_shape_manager_is_silent() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    let move_binding = pointer_binding_objects(&sv)[0].clone();

    s.add_window(&mut sv);
    s.manage(&mut sv);
    crate::keybinding::dispatch_action(
        &mut s.state,
        &crate::actions::KeybindingAction::ToggleWorkspaceFloating,
    );
    s.manage(&mut sv);

    sv.clear_request_log();
    s.send(&mut sv, move_binding, EVT_PTR_BINDING_PRESSED, vec![]);
    s.manage(&mut sv);
    assert!(
        matches!(
            s.state.wm.status,
            crate::types::Status::PointerAction(crate::actions::PointerAction::MoveWindow)
        ),
        "the drag must still start without a cursor shape device"
    );
    assert!(
        sv.request_log.lock().unwrap().is_empty()
            || sv
                .request_log
                .lock()
                .unwrap()
                .iter()
                .all(|(object, _, _)| object.interface().name != "wp_cursor_shape_device_v1"),
        "no cursor requests without the manager"
    );
}
