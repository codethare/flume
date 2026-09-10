// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Window event scenarios: fullscreen requests, close handling and the
//! fullscreen geometry sent to the compositor.

use super::*;

/// Regression: a fullscreen window fills its output rect exactly, so any
/// window border (and the -border clip offset) would be drawn 3px beyond the
/// output onto a neighboring monitor's adjoining edge ("the edge of A facing
/// B shows the edge of B's fullscreen window"). Fullscreen must send border
/// width 0 and a
/// zero-origin clip box.
#[test]
fn fullscreen_window_sends_no_border_and_zero_origin_clip() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    // A at origin, B to the right (adjoining edge = A's right / B's left).
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_output(&mut sv, "B", (1920, 0), (1920, 1080));
    s.manage(&mut sv);
    s.state.wm.focused_output_idx = Some(0);
    s.manage(&mut sv);
    s.add_window(&mut sv); // window on A
    s.manage(&mut sv);
    s.state.wm.focused_output_idx = Some(1);
    s.manage(&mut sv);
    let b_win = s.add_window(&mut sv); // window to be fullscreened
    s.manage(&mut sv);
    s.state.wm.outputs[1].workspace_list[0].focused_window_idx = Some(0);
    s.manage(&mut sv);

    sv.clear_request_log();
    crate::keybinding::dispatch_action(
        &mut s.state,
        &crate::actions::KeybindingAction::ToggleFullscreen,
    );
    s.manage(&mut sv);

    for (_obj, op, args) in sv.requests_for(&b_win) {
        if op == 8 {
            // set_borders(edges, width, r, g, b, a)
            let width: i32 = args
                .split(',')
                .nth(1)
                .unwrap()
                .trim_start_matches('i')
                .parse()
                .unwrap();
            assert_eq!(
                width, 0,
                "fullscreen window must not set a border, got {args}"
            );
        }
        if op == 21 {
            // set_clip_box(x, y, w, h)
            let v: Vec<i32> = args
                .split(',')
                .map(|a| a.trim_start_matches('i').parse().unwrap())
                .collect();
            assert_eq!(
                (v[0], v[1]),
                (0, 0),
                "fullscreen clip box must be zero-origin, got {args}"
            );
        }
    }
}

/// window.rs: a client-side fullscreen request fills the window's output and
/// the inverse request restores the tiled layout.
#[test]
fn window_fullscreen_requests_toggle_and_fill_output() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    let out = s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    let win = s.add_window(&mut sv);
    s.manage(&mut sv);

    s.send(
        &mut sv,
        win.clone(),
        EVT_WIN_FULLSCREEN_REQUESTED,
        vec![Argument::Object(out)],
    );
    s.manage(&mut sv);
    let output_rect = s.state.wm.outputs[0].rectangle;
    let geom = s.state.wm.outputs[0].workspace_list[0].window_list[0]
        .geom
        .current;
    assert!(
        s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .is_fullscreen,
        "fullscreen_requested must set the flag"
    );
    assert!(
        geom.eql(output_rect),
        "fullscreen window must fill its output: {geom:?} vs {output_rect:?}"
    );

    s.send(&mut sv, win, EVT_WIN_EXIT_FULLSCREEN_REQUESTED, vec![]);
    s.manage(&mut sv);
    assert!(
        !s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .is_fullscreen,
        "exit_fullscreen_requested must clear the flag"
    );
}

/// window.rs: closing windows keeps the workspace focus index valid — the
/// off-by-one class that used to panic in window moves.
#[test]
fn closing_windows_keeps_focus_index_valid() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);
    let w1 = s.add_window(&mut sv);
    s.manage(&mut sv);
    let w2 = s.add_window(&mut sv);
    s.manage(&mut sv);
    s.check_consistent();

    // Focus the middle window, close it: focus shifts to the one before it.
    s.state.wm.outputs[0].workspace_list[0].focused_window_idx = Some(1);
    s.manage(&mut sv);
    s.send(&mut sv, w1, EVT_WIN_CLOSED, vec![]);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(0),
        "closing the focused middle window moves focus down"
    );
    assert_eq!(s.state.wm.outputs[0].workspace_list[0].window_list.len(), 2);
    s.check_consistent();

    // Closing a window after the focused one leaves focus alone.
    s.send(&mut sv, w2, EVT_WIN_CLOSED, vec![]);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(0),
        "closing a later window must not move focus"
    );
    s.check_consistent();
}
