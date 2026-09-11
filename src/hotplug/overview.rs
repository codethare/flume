// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Overview interaction: enter, navigate, confirm/cancel and prune. The
//! navigation arithmetic lives in the key-press path, so these drive the
//! real xkb binding objects rather than calling overview.rs directly.

use super::*;
use crate::actions::KeybindingAction;

/// Press the default binding carrying `action` on its xkb binding object.
fn press(s: &mut Session, server: &mut MiniServer, action: KeybindingAction) {
    let index = crate::keybinding::default_keybindings()
        .iter()
        .position(|kb| kb.action == action)
        .expect("action has a default binding");
    let binding = children_with_interface(server, "river_xkb_binding_v1")[index].clone();
    s.send(server, binding, EVT_XKB_BINDING_PRESSED, vec![]);
}

fn highlight(s: &Session) -> usize {
    s.state
        .wm
        .overview_state
        .as_ref()
        .expect("overview open")
        .highlighted
}

fn entry_count(s: &Session) -> usize {
    s.state
        .wm
        .overview_state
        .as_ref()
        .expect("overview open")
        .entries
        .len()
}

/// `disable` requests on the overview-only bindings, which is how the WM
/// releases Return/Escape/hjkl back to applications.
fn overview_grabs_released(server: &MiniServer) -> usize {
    children_with_interface(server, "river_xkb_binding_v1")
        .iter()
        .map(|binding| {
            server
                .requests_for(binding)
                .into_iter()
                .filter(|(_, op, _)| *op == REQ_XKB_DISABLE)
                .count()
        })
        .sum()
}

/// Navigation moves the highlight within the grid and confirming focuses the
/// highlighted window, not the one that was focused before.
#[test]
fn navigate_and_confirm_focuses_the_highlighted_window() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv); // entries[0]
    s.manage(&mut sv);
    s.add_window(&mut sv); // entries[1], focused on open
    s.manage(&mut sv);
    s.check_consistent();
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(1)
    );

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::EnterOverview);
    s.manage(&mut sv);
    assert_eq!(entry_count(&s), 2);
    assert_eq!(highlight(&s), 0);

    // Two windows on a 1920x1080 grid are two columns, one row.
    press(&mut s, &mut sv, KeybindingAction::OverviewNavRight);
    assert_eq!(highlight(&s), 1);
    press(&mut s, &mut sv, KeybindingAction::OverviewNavRight); // last column
    assert_eq!(highlight(&s), 1, "navigation must stop at the grid edge");
    press(&mut s, &mut sv, KeybindingAction::OverviewNavDown); // only one row
    assert_eq!(highlight(&s), 1, "navigation must stop at the last row");
    press(&mut s, &mut sv, KeybindingAction::OverviewNavLeft);
    assert_eq!(highlight(&s), 0);

    press(&mut s, &mut sv, KeybindingAction::OverviewConfirm);
    s.manage(&mut sv);
    assert!(s.state.wm.overview_state.is_none(), "confirm closes it");
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(0),
        "the highlighted window is focused, not the previously focused one"
    );
    s.check_consistent();
}

/// Cancel returns to the workspace the overview was entered from.
#[test]
fn cancel_restores_the_workspace_you_came_from() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv); // workspace 0
    s.manage(&mut sv);

    // Second window on workspace 1, then back to workspace 0 before entering.
    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::FocusWorkspaceBelow);
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);
    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::FocusWorkspaceAbove);
    s.manage(&mut sv);
    assert_eq!(s.state.wm.outputs[0].focused_workspace_idx, 0);

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::EnterOverview);
    s.manage(&mut sv);
    assert_eq!(entry_count(&s), 2);
    press(&mut s, &mut sv, KeybindingAction::OverviewNavRight);

    press(&mut s, &mut sv, KeybindingAction::OverviewCancel);
    s.manage(&mut sv);
    assert!(s.state.wm.overview_state.is_none());
    assert_eq!(
        s.state.wm.outputs[0].focused_workspace_idx, 0,
        "cancel must restore the workspace the overview started from"
    );
    s.check_consistent();
}

/// Closing windows while the overview is open drops their grid slots, and
/// closing the last one must also release the overview-only key grabs —
/// otherwise the WM keeps swallowing Return/Escape/hjkl.
#[test]
fn closing_windows_prunes_the_overview_and_releases_the_grabs() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    let w0 = s.add_window(&mut sv);
    s.manage(&mut sv);
    let w1 = s.add_window(&mut sv);
    s.manage(&mut sv);

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::EnterOverview);
    s.manage(&mut sv);
    assert_eq!(entry_count(&s), 2);
    press(&mut s, &mut sv, KeybindingAction::OverviewNavRight);
    assert_eq!(highlight(&s), 1);

    sv.clear_request_log();
    s.send(&mut sv, w1, EVT_WIN_CLOSED, vec![]);
    s.manage(&mut sv);
    assert_eq!(entry_count(&s), 1, "the closed window loses its slot");
    assert_eq!(highlight(&s), 0, "the highlight is clamped to the grid");
    assert_eq!(
        overview_grabs_released(&sv),
        0,
        "the overview is still open, so its keys stay grabbed"
    );

    s.send(&mut sv, w0, EVT_WIN_CLOSED, vec![]);
    s.manage(&mut sv);
    assert!(
        s.state.wm.overview_state.is_none(),
        "no windows left ends the overview"
    );
    assert!(
        overview_grabs_released(&sv) > 0,
        "ending the overview must release Return/Escape/hjkl"
    );
    s.check_consistent();
}
