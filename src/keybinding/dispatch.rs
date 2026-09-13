// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The action state machine (rill-ed keybindingPressed), split per category.

use super::reload_config;
use crate::actions::KeybindingAction;
use crate::types::Status;
use KeybindingAction as A;
/// The action state machine (rill-ed keybindingPressed), split per category.
/// Every state-changing action ends with layout::update + status Layout; the
/// caller requests the manage sequence.
pub fn dispatch_action(state: &mut crate::app::AppData, action: &KeybindingAction) {
    match action {
        // Early-return actions without layout updates.
        A::Spawn(argv) => {
            if let Err(e) = crate::spawn::spawn_detached(argv) {
                eprintln!("tailrace: cannot spawn {argv:?}: {e}");
            }
            return;
        }
        A::Exit => {
            state.wm.status = Status::Exit;
            return;
        }
        A::ReloadConfig => {
            reload_config(state);
            return;
        }
        A::EnterOverview => {
            crate::overview::enter(state);
            if state.wm.overview_state.is_some() {
                // overview.enter assigned grid finish rects; commit them.
                crate::layout::update(&mut state.wm);
                state.wm.status = Status::Overview;
                state.manage_dirty();
            }
            return;
        }
        // No-op outside overview (fall through to the layout tail).
        A::OverviewCancel
        | A::OverviewConfirm
        | A::OverviewNavUp
        | A::OverviewNavDown
        | A::OverviewNavLeft
        | A::OverviewNavRight => {}

        A::CloseWindow
        | A::ToggleFullscreen
        | A::ToggleMaximizeColumn
        | A::AdjustWindowWidth(_)
        | A::SetWindowWidth(_)
        | A::AdjustFloatingWindowSize(_)
        | A::SetFloatingWindowHeight(_)
        | A::FocusWindowLeft
        | A::FocusWindowOrOutputLeft
        | A::FocusWindowRight
        | A::FocusWindowOrOutputRight
        | A::MoveWindowLeft
        | A::MoveWindowRight
        | A::MoveFloatingWindowLeft
        | A::MoveFloatingWindowRight
        | A::MoveFloatingWindowUp
        | A::MoveFloatingWindowDown
        | A::MoveWindowLeftOrToOutputLeft
        | A::MoveWindowRightOrToOutputRight
        | A::ToggleWorkspaceFloating
        | A::FocusWorkspaceAbove
        | A::FocusWorkspaceBelow
        | A::FocusWorkspaceOrOutputAbove
        | A::FocusWorkspaceOrOutputBelow
        | A::FocusWorkspacePrevious
        | A::FocusWorkspaceNumber(_)
        | A::MoveWindowToWorkspaceAbove
        | A::MoveWindowToWorkspaceBelow
        | A::MoveWindowToWorkspaceOrOutputAbove
        | A::MoveWindowToWorkspaceOrOutputBelow
        | A::MoveWindowToWorkspaceNumber(_)
        | A::FocusOutputLeft
        | A::FocusOutputRight
        | A::FocusOutputAbove
        | A::FocusOutputBelow
        | A::MoveWindowToOutputLeft
        | A::MoveWindowToOutputRight
        | A::MoveWindowToOutputAbove
        | A::MoveWindowToOutputBelow => {}
    }

    let Some((output_idx, workspace_idx)) = state.wm.current_ws_idx() else {
        return;
    };
    let config = state.wm.config.clone();

    match action {
        A::CloseWindow => {
            if let Some(window) = state.wm.focused_window_mut() {
                window.geom.is_closing = true;
            }
        }
        A::ToggleFullscreen => {
            if let Some(window) = state.wm.focused_window_mut() {
                window.geom.is_fullscreen = !window.geom.is_fullscreen;
            }
        }
        A::ToggleMaximizeColumn => {
            if let Some(window) = state.wm.focused_window_mut() {
                window.geom.proportion = if window.geom.proportion == 1.0 {
                    0.5
                } else {
                    1.0
                };
            }
        }
        A::AdjustWindowWidth(increment) => {
            let non_exclusive = state.wm.outputs[output_idx].non_exclusive;
            let output_rect = state.wm.outputs[output_idx].rectangle;
            if let Some(window) = state.wm.focused_window_mut() {
                if window.geom.is_fullscreen {
                    return;
                }
                if window.geom.is_floating {
                    let dw = (non_exclusive.width as f32 * increment) as i32;
                    let min_size = 2 * config.border.width as i32;
                    crate::layout::common::resize_floating(
                        &mut window.geom,
                        output_rect,
                        dw,
                        0,
                        min_size,
                    );
                } else {
                    let gap = config.horizontal_gap;
                    let base_width = (non_exclusive.width - gap) as f32;
                    let width_with_gap = (base_width * (window.geom.proportion + increment)) as i32;
                    if width_with_gap - gap < 2 * config.border.width as i32 {
                        return;
                    }
                    window.geom.proportion += increment;
                }
            }
        }
        A::SetWindowWidth(proportion) => {
            let output_rect = state.wm.outputs[output_idx].rectangle;
            let non_exclusive = state.wm.outputs[output_idx].non_exclusive;
            if let Some(window) = state.wm.focused_window_mut() {
                if window.geom.is_floating {
                    if window.geom.is_fullscreen {
                        return;
                    }
                    let w = (non_exclusive.width as f32 * proportion) as i32;
                    let min_size = 2 * config.border.width as i32;
                    window.geom.floating.width = w.clamp(
                        min_size,
                        output_rect.x + output_rect.width - window.geom.floating.x,
                    );
                } else {
                    window.geom.proportion = *proportion;
                }
            }
        }
        A::AdjustFloatingWindowSize(increment) => {
            let output_rect = state.wm.outputs[output_idx].rectangle;
            let non_exclusive = state.wm.outputs[output_idx].non_exclusive;
            if let Some(window) = state.wm.focused_window_mut() {
                if !window.geom.is_floating || window.geom.is_fullscreen {
                    return;
                }
                let dw = (non_exclusive.width as f32 * increment) as i32;
                let dh = (non_exclusive.height as f32 * increment) as i32;
                let min_size = 2 * config.border.width as i32;
                crate::layout::common::scale_floating(
                    &mut window.geom,
                    output_rect,
                    dw,
                    dh,
                    min_size,
                );
            }
        }
        A::SetFloatingWindowHeight(proportion) => {
            let output_rect = state.wm.outputs[output_idx].rectangle;
            let non_exclusive = state.wm.outputs[output_idx].non_exclusive;
            if let Some(window) = state.wm.focused_window_mut() {
                if !window.geom.is_floating || window.geom.is_fullscreen {
                    return;
                }
                let h = (non_exclusive.height as f32 * proportion) as i32;
                let min_size = 2 * config.border.width as i32;
                window.geom.floating.height = h.clamp(
                    min_size,
                    output_rect.y + output_rect.height - window.geom.floating.y,
                );
            }
        }
        A::FocusWindowLeft => {
            let workspace = state.wm.workspace_mut(output_idx, workspace_idx).unwrap();
            let Some(window_idx) = workspace.focused_window_idx else {
                return;
            };
            if window_idx >= workspace.window_list.len() || window_idx == 0 {
                return;
            }
            workspace.focused_window_idx = Some(window_idx - 1);
        }
        A::FocusWindowRight => {
            let workspace = state.wm.workspace_mut(output_idx, workspace_idx).unwrap();
            let Some(window_idx) = workspace.focused_window_idx else {
                return;
            };
            if window_idx >= workspace.window_list.len()
                || window_idx == workspace.window_list.len() - 1
            {
                return;
            }
            workspace.focused_window_idx = Some(window_idx + 1);
        }
        A::FocusWindowOrOutputLeft | A::FocusWindowOrOutputRight => {
            let right = *action == A::FocusWindowOrOutputRight;
            let workspace = state.wm.workspace(output_idx, workspace_idx).unwrap();
            let Some(window_idx) = workspace.focused_window_idx else {
                return;
            };
            if window_idx >= workspace.window_list.len() {
                return;
            }
            let at_edge = if right {
                window_idx == workspace.window_list.len() - 1
            } else {
                window_idx == 0
            };
            let redirect = if right {
                KeybindingAction::FocusOutputRight
            } else {
                KeybindingAction::FocusOutputLeft
            };
            if at_edge {
                dispatch_action(state, &redirect);
                return;
            }
            let inner = if right {
                KeybindingAction::FocusWindowRight
            } else {
                KeybindingAction::FocusWindowLeft
            };
            dispatch_action(state, &inner);
            return;
        }
        A::MoveWindowLeft | A::MoveWindowRight => {
            let right = *action == A::MoveWindowRight;
            let workspace = state.wm.workspace_mut(output_idx, workspace_idx).unwrap();
            let Some(window_idx) = workspace.focused_window_idx else {
                return;
            };
            if window_idx >= workspace.window_list.len() {
                return;
            }
            let Some(target) = move_target(window_idx, workspace.window_list.len(), right) else {
                return;
            };
            workspace.window_list.swap(window_idx, target);
            workspace.focused_window_idx = Some(target);
        }
        A::MoveFloatingWindowLeft
        | A::MoveFloatingWindowRight
        | A::MoveFloatingWindowUp
        | A::MoveFloatingWindowDown => {
            let (dx, dy) = match action {
                A::MoveFloatingWindowLeft => (-crate::layout::common::FLOATING_MOVE_STEP, 0),
                A::MoveFloatingWindowRight => (crate::layout::common::FLOATING_MOVE_STEP, 0),
                A::MoveFloatingWindowUp => (0, -crate::layout::common::FLOATING_MOVE_STEP),
                _ => (0, crate::layout::common::FLOATING_MOVE_STEP),
            };
            let output_rect = state.wm.outputs[output_idx].rectangle;
            if let Some(window) = state.wm.focused_window_mut() {
                if !window.geom.is_floating {
                    return;
                }
                crate::layout::common::move_floating(&mut window.geom, output_rect, dx, dy);
            }
        }
        A::MoveWindowLeftOrToOutputLeft | A::MoveWindowRightOrToOutputRight => {
            let right = *action == A::MoveWindowRightOrToOutputRight;
            let workspace = state.wm.workspace(output_idx, workspace_idx).unwrap();
            let Some(window_idx) = workspace.focused_window_idx else {
                return;
            };
            if window_idx >= workspace.window_list.len() {
                return;
            }
            let at_edge = if right {
                window_idx == workspace.window_list.len() - 1
            } else {
                window_idx == 0
            };
            let redirect = if right {
                KeybindingAction::MoveWindowToOutputRight
            } else {
                KeybindingAction::MoveWindowToOutputLeft
            };
            if at_edge {
                dispatch_action(state, &redirect);
                return;
            }
            let inner = if right {
                KeybindingAction::MoveWindowRight
            } else {
                KeybindingAction::MoveWindowLeft
            };
            dispatch_action(state, &inner);
            return;
        }
        A::ToggleWorkspaceFloating => {
            let non_exclusive = state.wm.outputs[output_idx].non_exclusive;
            if let Some(window) = state.wm.focused_window_mut() {
                window.geom.is_floating = !window.geom.is_floating;
                if window.geom.is_floating {
                    window.geom.floating =
                        crate::layout::common::center_rectangle(non_exclusive, &config);
                    // Don't snap current to floating; layout.update sets
                    // finish so the change is committed in one pass.
                }
            }
        }
        A::FocusWorkspaceAbove | A::FocusWorkspaceBelow | A::FocusWorkspaceNumber(_) => {
            let target: Option<usize> = match action {
                A::FocusWorkspaceAbove => {
                    if workspace_idx == 0 {
                        None
                    } else {
                        Some(workspace_idx - 1)
                    }
                }
                A::FocusWorkspaceBelow => {
                    if workspace_idx == 9 {
                        None
                    } else {
                        Some(workspace_idx + 1)
                    }
                }
                A::FocusWorkspaceNumber(n) => {
                    if *n == 0 || *n > 10 || n - 1 == workspace_idx {
                        None
                    } else {
                        Some(n - 1)
                    }
                }
                _ => unreachable!(),
            };
            let Some(target) = target else { return };
            state.wm.outputs[output_idx].focused_workspace_idx = target;
            state.wm.previous_workspace = Some(crate::types::OverviewHome {
                output_idx,
                workspace_idx,
            });
        }
        A::FocusWorkspaceOrOutputAbove | A::FocusWorkspaceOrOutputBelow => {
            let at_edge = if *action == A::FocusWorkspaceOrOutputAbove {
                workspace_idx == 0
            } else {
                workspace_idx == 9
            };
            let redirect = if *action == A::FocusWorkspaceOrOutputAbove {
                KeybindingAction::FocusOutputAbove
            } else {
                KeybindingAction::FocusOutputBelow
            };
            if at_edge {
                dispatch_action(state, &redirect);
                return;
            }
            let inner = if *action == A::FocusWorkspaceOrOutputAbove {
                KeybindingAction::FocusWorkspaceAbove
            } else {
                KeybindingAction::FocusWorkspaceBelow
            };
            dispatch_action(state, &inner);
            return;
        }
        A::FocusWorkspacePrevious => {
            let Some(previous) = state.wm.previous_workspace else {
                return;
            };
            if previous.output_idx >= state.wm.outputs.len() {
                return;
            }
            state.wm.focused_output_idx = Some(previous.output_idx);
            state.wm.outputs[previous.output_idx].focused_workspace_idx = previous.workspace_idx;
            state.wm.previous_workspace = Some(crate::types::OverviewHome {
                output_idx,
                workspace_idx,
            });
        }
        A::MoveWindowToWorkspaceAbove
        | A::MoveWindowToWorkspaceBelow
        | A::MoveWindowToWorkspaceNumber(_) => {
            let target: Option<usize> = match action {
                A::MoveWindowToWorkspaceAbove => {
                    if workspace_idx == 0 {
                        None
                    } else {
                        Some(workspace_idx - 1)
                    }
                }
                A::MoveWindowToWorkspaceBelow => {
                    if workspace_idx == 9 {
                        None
                    } else {
                        Some(workspace_idx + 1)
                    }
                }
                A::MoveWindowToWorkspaceNumber(n) => {
                    if *n == 0 || *n > 10 || n - 1 == workspace_idx {
                        None
                    } else {
                        Some(n - 1)
                    }
                }
                _ => unreachable!(),
            };
            let Some(target_ws_idx) = target else { return };
            let window_idx = state
                .wm
                .workspace(output_idx, workspace_idx)
                .and_then(|ws| ws.focused_window_idx);
            let Some(window_idx) = window_idx else { return };
            if window_idx
                >= state
                    .wm
                    .workspace(output_idx, workspace_idx)
                    .unwrap()
                    .window_list
                    .len()
            {
                return;
            }
            state.wm.move_window_to_workspace(
                (output_idx, workspace_idx),
                (output_idx, target_ws_idx),
                window_idx,
            );
            state.wm.outputs[output_idx].focused_workspace_idx = target_ws_idx;
            state.wm.previous_workspace = Some(crate::types::OverviewHome {
                output_idx,
                workspace_idx,
            });
        }
        A::FocusOutputLeft | A::FocusOutputRight | A::FocusOutputAbove | A::FocusOutputBelow => {
            if let Some(target) = adjacent_output(&state.wm, output_idx, action) {
                state.wm.focused_output_idx = Some(target);
                state.wm.needs_pointer_warp = true;
                state.wm.previous_workspace = Some(crate::types::OverviewHome {
                    output_idx,
                    workspace_idx,
                });
            }
        }
        A::MoveWindowToOutputLeft
        | A::MoveWindowToOutputRight
        | A::MoveWindowToOutputAbove
        | A::MoveWindowToOutputBelow => {
            let Some(target_idx) = adjacent_output(&state.wm, output_idx, action) else {
                return;
            };
            let window_idx = state
                .wm
                .workspace(output_idx, workspace_idx)
                .and_then(|ws| ws.focused_window_idx);
            let Some(window_idx) = window_idx else { return };
            if window_idx
                >= state
                    .wm
                    .workspace(output_idx, workspace_idx)
                    .unwrap()
                    .window_list
                    .len()
            {
                return;
            }
            let target_ws_idx = state.wm.outputs[target_idx].focused_workspace_idx;
            state.wm.move_window_to_workspace(
                (output_idx, workspace_idx),
                (target_idx, target_ws_idx),
                window_idx,
            );
            // The moved window (now at the target's focus) gets a fresh
            // floating rect on its new output.
            let non_exclusive = state.wm.outputs[target_idx].non_exclusive;
            let target_focus =
                state.wm.outputs[target_idx].workspace_list[target_ws_idx].focused_window_idx;
            if let Some(tw_idx) = target_focus
                && let Some(window) = state.wm.outputs[target_idx].workspace_list[target_ws_idx]
                    .window_list
                    .get_mut(tw_idx)
            {
                window.geom.floating =
                    crate::layout::common::initial_rectangle(non_exclusive, &config);
            }
            state.wm.focused_output_idx = Some(target_idx);
            state.wm.needs_pointer_warp = true;
            state.wm.previous_workspace = Some(crate::types::OverviewHome {
                output_idx,
                workspace_idx,
            });
        }
        _ => {}
    }

    crate::layout::update(&mut state.wm);
    state.wm.status = Status::Layout;
}

/// Swap target for a horizontal window move; None when the move stays in
/// place (list edge, or the underflow that used to panic with index
/// 18446744073709551615 when focus sat on window 0 and `window_idx - 1`
/// wrapped). Mirrors the `window_idx == 0` guard of FocusWindowLeft.
pub(super) fn move_target(window_idx: usize, len: usize, right: bool) -> Option<usize> {
    let target = if right {
        window_idx.checked_add(1)?
    } else {
        window_idx.checked_sub(1)?
    };
    (target < len).then_some(target)
}

/// Find the output adjacent to `output_idx` in the action's direction
/// (rectangle adjacency, matching rill-ed).
fn adjacent_output(
    wm: &crate::wm::WindowManager,
    output_idx: usize,
    action: &KeybindingAction,
) -> Option<usize> {
    use KeybindingAction as A;
    let output = &wm.outputs[output_idx];
    wm.outputs.iter().enumerate().position(|(i, target)| {
        if i == output_idx || target.is_removed {
            return false;
        }
        match action {
            A::FocusOutputLeft | A::MoveWindowToOutputLeft => {
                target.rectangle.x + target.rectangle.width == output.rectangle.x
            }
            A::FocusOutputRight | A::MoveWindowToOutputRight => {
                target.rectangle.x == output.rectangle.x + output.rectangle.width
            }
            A::FocusOutputAbove | A::MoveWindowToOutputAbove => {
                target.rectangle.y + target.rectangle.height == output.rectangle.y
            }
            A::FocusOutputBelow | A::MoveWindowToOutputBelow => {
                target.rectangle.y == output.rectangle.y + output.rectangle.height
            }
            _ => false,
        }
    })
}
