// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! app.rs manage-cycle state machine: status transitions, binding setup
//! ordering and the focus gates (exclusive layer-shell focus, session lock,
//! exit).

use super::*;

/// Binding setup is requested by the seat event but must wait until an output
/// is focused, and must happen exactly once. Assigning Status::SetupBindings
/// directly used to lose the request whenever an output/window event set a
/// different status first (no keybindings at all).
#[test]
fn setup_bindings_waits_for_an_output_and_runs_once() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);

    // No output yet: manage returns early and must not consume the request.
    assert!(
        s.state.wm.needs_setup_bindings,
        "the setup request must survive a manage without a focused output"
    );
    assert!(
        s.state.pointer_bindings.is_empty(),
        "no bindings before an output exists"
    );

    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    assert!(!s.state.wm.needs_setup_bindings, "request consumed");
    assert_eq!(
        s.state.pointer_bindings.len(),
        2,
        "default pointer bindings"
    );
    assert!(
        s.state.xkb_bindings.len() > 50,
        "default keybindings, got {}",
        s.state.xkb_bindings.len()
    );

    // Later manage cycles must not re-create bindings.
    let before = count_requests(&sv, &seat, REQ_SEAT_GET_POINTER_BINDING);
    s.manage(&mut sv);
    s.manage(&mut sv);
    assert_eq!(
        count_requests(&sv, &seat, REQ_SEAT_GET_POINTER_BINDING),
        before,
        "bindings must be created once"
    );
}

/// A plain layout manage commits the layout and ends the pointer op; the
/// cycle returns to None so the next sequence starts clean.
#[test]
fn layout_manage_ends_the_pointer_op_and_returns_to_none() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    sv.clear_request_log();
    s.manage(&mut sv);
    assert_eq!(s.state.wm.status, crate::types::Status::None);
    assert!(
        count_requests(&sv, &seat, REQ_SEAT_OP_END) >= 1,
        "the manage cycle must end the pointer op"
    );
}

/// While a layer-shell surface holds exclusive focus no keyboard focus is
/// handed to any window; once revoked, focus is re-issued.
#[test]
fn exclusive_layer_shell_focus_skips_focus_commands() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    s.state.wm.layer_shell_focus = crate::wm::LayerShellFocus::Exclusive;
    sv.clear_request_log();
    s.manage(&mut sv);
    assert_eq!(
        count_requests(&sv, &seat, REQ_SEAT_FOCUS_WINDOW)
            + count_requests(&sv, &seat, REQ_SEAT_CLEAR_FOCUS),
        0,
        "exclusive layer-shell focus must suppress focus commands"
    );

    // Focus revoked: mirrors layer_shell_seat_event(FocusNone), which clears
    // the focus cache, forces a refocus and re-enters the layout state.
    s.state.wm.layer_shell_focus = crate::wm::LayerShellFocus::None;
    s.state.wm.last_focused_window = None;
    s.state.wm.needs_refocus = true;
    s.state.wm.status = crate::types::Status::Layout;
    sv.clear_request_log();
    s.manage(&mut sv);
    assert_eq!(
        count_requests(&sv, &seat, REQ_SEAT_FOCUS_WINDOW),
        1,
        "focus must be re-issued after exclusive focus is revoked"
    );
}

/// The lock surface owns keyboard focus while the session is locked: the WM
/// must not fight it, and must restore focus on unlock.
#[test]
fn locked_session_skips_focus_commands_and_restores_on_unlock() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    let wm = sv.wm.clone().unwrap();
    s.send(&mut sv, wm.clone(), EVT_SESSION_LOCKED, vec![]);
    assert!(s.state.wm.session_locked);
    assert!(s.state.wm.lock_focus.is_some(), "focused window saved");
    sv.clear_request_log();
    s.manage(&mut sv);
    assert_eq!(
        count_requests(&sv, &seat, REQ_SEAT_FOCUS_WINDOW)
            + count_requests(&sv, &seat, REQ_SEAT_CLEAR_FOCUS),
        0,
        "focus must not be touched while locked"
    );

    s.send(&mut sv, wm, EVT_SESSION_UNLOCKED, vec![]);
    assert!(!s.state.wm.session_locked);
    assert!(s.state.wm.needs_refocus, "unlock must force a refocus");
    sv.clear_request_log();
    s.manage(&mut sv); // retry pass while needs_refocus is set
    s.manage(&mut sv);
    assert_eq!(
        count_requests(&sv, &seat, REQ_SEAT_FOCUS_WINDOW),
        1,
        "the focused window must be restored after unlock"
    );
}

/// Exiting the session is a request on the window manager object, sent from
/// the Exit status.
#[test]
fn exit_status_calls_exit_session() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    crate::keybinding::dispatch_action(&mut s.state, &crate::actions::KeybindingAction::Exit);
    assert_eq!(s.state.wm.status, crate::types::Status::Exit);
    sv.clear_request_log();
    s.manage(&mut sv);
    let wm = sv.wm.clone().unwrap();
    assert_eq!(
        count_requests(&sv, &wm, REQ_WM_EXIT_SESSION),
        1,
        "Exit must call river_window_manager_v1.exit_session"
    );
}
