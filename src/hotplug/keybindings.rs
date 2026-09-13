// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Keybinding dispatch: every action smoke-tested for panics and index
//! consistency, plus semantic checks for the ones carrying real logic.
//! Only three actions had any end-to-end coverage before this.

use super::*;
use crate::actions::KeybindingAction;

fn every_action() -> Vec<KeybindingAction> {
    use KeybindingAction as A;
    vec![
        A::CloseWindow,
        A::ToggleFullscreen,
        A::ToggleMaximizeColumn,
        A::AdjustWindowWidth(0.1),
        A::SetWindowWidth(0.5),
        A::AdjustFloatingWindowSize(0.1),
        A::SetFloatingWindowHeight(0.5),
        A::FocusWindowLeft,
        A::FocusWindowOrOutputLeft,
        A::FocusWindowRight,
        A::FocusWindowOrOutputRight,
        A::MoveWindowLeft,
        A::MoveWindowRight,
        A::MoveFloatingWindowLeft,
        A::MoveFloatingWindowRight,
        A::MoveFloatingWindowUp,
        A::MoveFloatingWindowDown,
        A::MoveWindowLeftOrToOutputLeft,
        A::MoveWindowRightOrToOutputRight,
        A::ToggleWorkspaceFloating,
        A::FocusWorkspaceAbove,
        A::FocusWorkspaceBelow,
        A::FocusWorkspaceOrOutputAbove,
        A::FocusWorkspaceOrOutputBelow,
        A::FocusWorkspacePrevious,
        A::FocusWorkspaceNumber(2),
        A::MoveWindowToWorkspaceAbove,
        A::MoveWindowToWorkspaceBelow,
        A::MoveWindowToWorkspaceOrOutputAbove,
        A::MoveWindowToWorkspaceOrOutputBelow,
        A::MoveWindowToWorkspaceNumber(2),
        A::FocusOutputLeft,
        A::FocusOutputRight,
        A::FocusOutputAbove,
        A::FocusOutputBelow,
        A::MoveWindowToOutputLeft,
        A::MoveWindowToOutputRight,
        A::MoveWindowToOutputAbove,
        A::MoveWindowToOutputBelow,
        A::Exit,
        A::ReloadConfig,
        A::EnterOverview,
        A::OverviewCancel,
        A::OverviewConfirm,
        A::OverviewNavUp,
        A::OverviewNavDown,
        A::OverviewNavLeft,
        A::OverviewNavRight,
        A::Spawn(vec!["/bin/true".into()]),
    ]
}

/// Output A with two windows, output B focused with one. ReloadConfig is
/// pointed at a file that cannot exist, so it behaves the same everywhere.
fn scene() -> (Session, MiniServer) {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "B", (1920, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);
    s.state.config_path = Some(std::path::PathBuf::from("/nonexistent/tailrace.toml"));
    s.check_consistent();
    (s, sv)
}

fn focused_window(s: &Session) -> &crate::types::Window {
    s.state.wm.focused_window().expect("a window is focused")
}

/// No action may panic or leave an out-of-bounds focus index behind.
#[test]
fn every_action_keeps_the_state_consistent() {
    for action in every_action() {
        let (mut s, mut sv) = scene();
        crate::keybinding::dispatch_action(&mut s.state, &action);
        s.manage(&mut sv);
        s.check_consistent();
        // Some actions only finish on the next cycle (status retries).
        s.manage(&mut sv);
        s.check_consistent();
    }
}

#[test]
fn focus_workspace_number_and_previous() {
    let (mut s, mut sv) = scene();
    let output = s.state.wm.focused_output_idx.unwrap();
    assert_eq!(s.state.wm.outputs[output].focused_workspace_idx, 0);

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::FocusWorkspaceNumber(3));
    s.manage(&mut sv);
    assert_eq!(s.state.wm.outputs[output].focused_workspace_idx, 2);
    assert_eq!(
        s.state.wm.previous_workspace.map(|p| p.workspace_idx),
        Some(0)
    );

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::FocusWorkspacePrevious);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[output].focused_workspace_idx, 0,
        "FocusWorkspacePrevious returns to the pre-switch workspace"
    );
    s.check_consistent();
}

#[test]
fn move_window_to_workspace_number_moves_it() {
    let (mut s, mut sv) = scene();
    let output = s.state.wm.focused_output_idx.unwrap();
    assert_eq!(
        s.state.wm.outputs[output].workspace_list[0]
            .window_list
            .len(),
        1
    );

    crate::keybinding::dispatch_action(
        &mut s.state,
        &KeybindingAction::MoveWindowToWorkspaceNumber(2),
    );
    s.manage(&mut sv);
    let workspaces = &s.state.wm.outputs[output].workspace_list;
    assert_eq!(workspaces[0].window_list.len(), 0, "left the old workspace");
    assert_eq!(workspaces[1].window_list.len(), 1, "landed on workspace 2");
    s.check_consistent();
}

#[test]
fn focus_and_move_across_outputs() {
    let (mut s, mut sv) = scene();
    assert_eq!(s.state.wm.focused_output_idx, Some(1), "focus starts on B");

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::FocusOutputLeft);
    s.manage(&mut sv);
    assert_eq!(s.state.wm.focused_output_idx, Some(0), "moved to A");

    // Move the focused window from A to the output on its right (B).
    let before_a = s.state.wm.outputs[0].workspace_list[0].window_list.len();
    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::MoveWindowToOutputRight);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].window_list.len(),
        before_a - 1,
        "the window left output A"
    );
    assert!(
        s.state.wm.outputs[1]
            .workspace_list
            .iter()
            .any(|ws| !ws.window_list.is_empty()),
        "the window arrived on output B"
    );
    s.check_consistent();
}

#[test]
fn toggle_maximize_column_switches_proportion() {
    let (mut s, mut sv) = scene();
    let output = s.state.wm.focused_output_idx.unwrap();
    assert_eq!(focused_window(&s).geom.proportion, 0.5);

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::ToggleMaximizeColumn);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[output].workspace_list[0].window_list[0]
            .geom
            .proportion,
        1.0
    );

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::ToggleMaximizeColumn);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[output].workspace_list[0].window_list[0]
            .geom
            .proportion,
        0.5
    );
}

#[test]
fn window_width_actions_change_the_proportion() {
    let (mut s, mut sv) = scene();
    let output = s.state.wm.focused_output_idx.unwrap();
    let before = focused_window(&s).geom.proportion;

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::AdjustWindowWidth(0.1));
    assert!(focused_window(&s).geom.proportion > before);

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::SetWindowWidth(0.5));
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[output].workspace_list[0].window_list[0]
            .geom
            .proportion,
        0.5
    );
}

#[test]
fn floating_actions_move_and_resize_the_rest_rect() {
    let (mut s, mut sv) = scene();
    let output = s.state.wm.focused_output_idx.unwrap();
    let geom = |s: &Session| {
        s.state.wm.outputs[output].workspace_list[0].window_list[0]
            .geom
            .clone()
    };

    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::ToggleWorkspaceFloating);
    s.manage(&mut sv);
    assert!(geom(&s).is_floating);

    let before = geom(&s);
    crate::keybinding::dispatch_action(&mut s.state, &KeybindingAction::MoveFloatingWindowRight);
    assert!(
        geom(&s).floating.x > before.floating.x,
        "MoveFloatingWindowRight shifts the rest rect right"
    );

    let before = geom(&s);
    crate::keybinding::dispatch_action(
        &mut s.state,
        &KeybindingAction::AdjustFloatingWindowSize(0.1),
    );
    assert!(
        geom(&s).floating.width > before.floating.width,
        "AdjustFloatingWindowSize grows the rest rect"
    );

    crate::keybinding::dispatch_action(
        &mut s.state,
        &KeybindingAction::SetFloatingWindowHeight(0.5),
    );
    assert!(geom(&s).floating.height > 0);
    s.check_consistent();
}
