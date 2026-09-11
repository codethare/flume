// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::actions::{KeybindingAction, PointerAction};
use crate::types::{Keybinding, PointerBinding};

/// Default keybindings, ported from rill-ed (keybinding.zig).
pub fn default_keybindings() -> Vec<Keybinding> {
    use KeybindingAction as A;
    fn kb(key: &str, modifiers: &[&str], action: KeybindingAction) -> Keybinding {
        Keybinding {
            key: key.into(),
            modifiers: modifiers.iter().map(|m| m.to_string()).collect(),
            action,
        }
    }

    let mut v = vec![
        kb("q", &["mod4"], A::CloseWindow),
        kb("f", &["mod4"], A::ToggleFullscreen),
        kb("minus", &["mod4"], A::AdjustWindowWidth(-0.1)),
        kb("equal", &["mod4"], A::AdjustWindowWidth(0.1)),
        kb("BackSpace", &["mod4"], A::SetWindowWidth(0.5)),
        kb(
            "minus",
            &["mod4", "ctrl"],
            A::AdjustFloatingWindowSize(-0.1),
        ),
        kb("equal", &["mod4", "ctrl"], A::AdjustFloatingWindowSize(0.1)),
        kb(
            "BackSpace",
            &["mod4", "ctrl"],
            A::SetFloatingWindowHeight(0.5),
        ),
        kb("Left", &["mod4"], A::FocusWindowLeft),
        kb("Right", &["mod4"], A::FocusWindowRight),
        kb("Left", &["mod4", "shift"], A::MoveWindowLeft),
        kb("Right", &["mod4", "shift"], A::MoveWindowRight),
        kb("Left", &["mod4", "ctrl"], A::MoveFloatingWindowLeft),
        kb("Right", &["mod4", "ctrl"], A::MoveFloatingWindowRight),
        kb("Up", &["mod4", "ctrl"], A::MoveFloatingWindowUp),
        kb("Down", &["mod4", "ctrl"], A::MoveFloatingWindowDown),
        kb("v", &["mod4"], A::ToggleWorkspaceFloating),
        kb("Up", &["mod4"], A::FocusWorkspaceAbove),
        kb("Down", &["mod4"], A::FocusWorkspaceBelow),
        kb("grave", &["mod4"], A::FocusWorkspacePrevious),
        kb("Up", &["mod4", "shift"], A::MoveWindowToWorkspaceAbove),
        kb("Down", &["mod4", "shift"], A::MoveWindowToWorkspaceBelow),
        kb("h", &["mod4"], A::FocusOutputLeft),
        kb("l", &["mod4"], A::FocusOutputRight),
        kb("k", &["mod4"], A::FocusOutputAbove),
        kb("j", &["mod4"], A::FocusOutputBelow),
        kb("h", &["mod4", "shift"], A::MoveWindowToOutputLeft),
        kb("l", &["mod4", "shift"], A::MoveWindowToOutputRight),
        kb("k", &["mod4", "shift"], A::MoveWindowToOutputAbove),
        kb("j", &["mod4", "shift"], A::MoveWindowToOutputBelow),
        kb("Escape", &["mod4"], A::Exit),
        // Bare keys: only meaningful while the overview is open.
        kb("Escape", &[], A::OverviewCancel),
        kb("Return", &[], A::OverviewConfirm),
        kb("h", &[], A::OverviewNavLeft),
        kb("j", &[], A::OverviewNavDown),
        kb("k", &[], A::OverviewNavUp),
        kb("l", &[], A::OverviewNavRight),
        kb("r", &["mod4"], A::ReloadConfig),
        kb("t", &["mod4"], A::Spawn(vec!["alacritty".into()])),
        kb("Space", &["mod4"], A::EnterOverview),
        kb(
            "XF86AudioRaiseVolume",
            &[],
            A::Spawn(vec![
                "wpctl".into(),
                "set-volume".into(),
                "@DEFAULT_AUDIO_SINK@".into(),
                "0.05+".into(),
                "--limit".into(),
                "1.0".into(),
            ]),
        ),
        kb(
            "XF86AudioLowerVolume",
            &[],
            A::Spawn(vec![
                "wpctl".into(),
                "set-volume".into(),
                "@DEFAULT_AUDIO_SINK@".into(),
                "0.05-".into(),
            ]),
        ),
        kb(
            "XF86AudioMute",
            &[],
            A::Spawn(vec![
                "wpctl".into(),
                "set-mute".into(),
                "@DEFAULT_AUDIO_SINK@".into(),
                "toggle".into(),
            ]),
        ),
        kb(
            "XF86AudioMicMute",
            &[],
            A::Spawn(vec![
                "wpctl".into(),
                "set-mute".into(),
                "@DEFAULT_AUDIO_SOURCE@".into(),
                "toggle".into(),
            ]),
        ),
    ];
    // Workspace numbers 1-10, focus and move.
    for n in 1..=10usize {
        let key = if n == 10 { "0" } else { &n.to_string()[..] };
        v.push(kb(key, &["mod4"], A::FocusWorkspaceNumber(n)));
        v.push(kb(
            key,
            &["mod4", "shift"],
            A::MoveWindowToWorkspaceNumber(n),
        ));
    }
    v
}

pub fn default_pointer_bindings() -> Vec<PointerBinding> {
    vec![
        PointerBinding {
            button: crate::actions::Button::Left,
            modifiers: vec!["mod4".into()],
            action: PointerAction::MoveWindow,
        },
        PointerBinding {
            button: crate::actions::Button::Right,
            modifiers: vec!["mod4".into()],
            action: PointerAction::ResizeWindow,
        },
    ]
}

/// Parse modifier names (shift, ctrl, mod1, mod3, mod4, mod5) into the
/// river `Modifiers` bitfield. Returns the offending name on error.
pub fn parse_modifiers(names: &[String]) -> Result<crate::river::river_seat_v1::Modifiers, String> {
    use crate::river::river_seat_v1::Modifiers;
    let mut mods = Modifiers::None;
    for name in names {
        let flag = match name.as_str() {
            "shift" => Modifiers::Shift,
            "ctrl" => Modifiers::Ctrl,
            "mod1" => Modifiers::Mod1,
            "mod3" => Modifiers::Mod3,
            "mod4" => Modifiers::Mod4,
            "mod5" => Modifiers::Mod5,
            other => return Err(other.to_string()),
        };
        mods = mods.union(flag);
    }
    Ok(mods)
}

/// Toggle the bare-key bindings that are only meaningful while the overview
/// is open (Return to confirm, Escape to cancel, vi navigation). They are
/// registered but kept disabled otherwise so apps keep receiving those keys.
pub fn set_overview_keybinds(state: &mut crate::app::AppData, enabled: bool) {
    for binding in &state.xkb_bindings {
        let overview_only = matches!(
            binding.action,
            KeybindingAction::OverviewConfirm
                | KeybindingAction::OverviewCancel
                | KeybindingAction::OverviewNavUp
                | KeybindingAction::OverviewNavDown
                | KeybindingAction::OverviewNavLeft
                | KeybindingAction::OverviewNavRight
        );
        if overview_only {
            if enabled {
                binding.proxy.enable();
            } else {
                binding.proxy.disable();
            }
        }
    }
}

/// Parse a keysym name (xkbcommon-keysyms.h names, case insensitive) to its
/// raw keysym value.
pub fn parse_key(key: &str) -> Option<u32> {
    use xkbcommon::xkb;
    let keysym = xkb::keysym_from_name(key, xkb::KEYSYM_CASE_INSENSITIVE);
    (keysym.raw() != 0).then_some(keysym.raw())
}

/// Rebuild xkb bindings from config (rill-ed setupKeybindings).
pub fn setup_keybindings(state: &mut crate::app::AppData) {
    for binding in state.xkb_bindings.drain(..) {
        binding.proxy.destroy();
    }
    let Some(xkb_bindings) = state.river_xkb.clone() else {
        eprintln!("Failed to find xkb bindings");
        return;
    };
    let Some(seat) = state.river_seat.clone() else {
        return;
    };
    for kb in &state.wm.config.keybindings {
        let Some(keysym) = parse_key(&kb.key) else {
            eprintln!("Failed to parse key '{}'", kb.key);
            continue;
        };
        let Ok(mods) = parse_modifiers(&kb.modifiers) else {
            eprintln!("Invalid modifiers {:?} for key '{}'", kb.modifiers, kb.key);
            continue;
        };
        let proxy = xkb_bindings.get_xkb_binding(&seat, keysym, mods, &state.qh, ());
        proxy.enable();
        state.xkb_bindings.push(crate::app::XkbBinding {
            proxy,
            action: kb.action.clone(),
        });
    }
    // Overview-only keys start disabled; enter() enables them.
    set_overview_keybinds(state, false);
}

fn binding_action(
    state: &crate::app::AppData,
    proxy: &crate::river::river_xkb_binding_v1::RiverXkbBindingV1,
) -> Option<KeybindingAction> {
    state
        .xkb_bindings
        .iter()
        .find(|b| &b.proxy == proxy)
        .map(|b| b.action.clone())
}

/// Dispatch a pressed binding (rill-ed xkbBindingListener).
pub fn binding_pressed(
    state: &mut crate::app::AppData,
    proxy: &crate::river::river_xkb_binding_v1::RiverXkbBindingV1,
) {
    use crate::types::Status;

    // Pointer ops swallow keyboard events.
    if matches!(state.wm.status, Status::PointerAction(_)) {
        return;
    }
    // Suppress keybindings while the session is locked.
    if state.wm.session_locked {
        return;
    }
    // During overview, intercept all key events for navigation.
    if state.wm.overview_state.is_some() {
        overview_key_pressed(state, proxy);
        return;
    }

    let Some(action) = binding_action(state, proxy) else {
        return;
    };
    dispatch_action(state, &action);
    state.manage_dirty();
}

/// The action state machine (rill-ed keybindingPressed), split per category.
/// Every state-changing action ends with layout::update + status Layout; the
/// caller requests the manage sequence.
pub fn dispatch_action(state: &mut crate::app::AppData, action: &KeybindingAction) {
    use crate::types::Status;
    use KeybindingAction as A;

    match action {
        // Early-return actions without layout updates.
        A::Spawn(argv) => {
            let _ = crate::spawn::spawn_detached(argv);
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
            if state
                .wm
                .workspace(output_idx, workspace_idx)
                .unwrap()
                .is_floating
            {
                return;
            }
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
            if state
                .wm
                .workspace(output_idx, workspace_idx)
                .unwrap()
                .is_floating
            {
                return;
            }
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
            if state
                .wm
                .workspace(output_idx, workspace_idx)
                .unwrap()
                .is_floating
            {
                return;
            }
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
                workspace.is_floating || window_idx == workspace.window_list.len() - 1
            } else {
                workspace.is_floating || window_idx == 0
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
fn move_target(window_idx: usize, len: usize, right: bool) -> Option<usize> {
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

fn reload_config(state: &mut crate::app::AppData) {
    let Some(new_config) = crate::config::reload() else {
        eprintln!("Config reload failed — keeping current config");
        return;
    };
    state.wm.config = std::rc::Rc::new(new_config);

    if let Some(cursor) = state.wm.config.cursor.clone()
        && let Some(seat) = &state.river_seat
    {
        seat.set_xcursor_theme(cursor.theme.clone(), cursor.size);
    }
    crate::layout::update(&mut state.wm);

    // Flag only: assigning Status here loses the request if a layout-setting
    // event (output/window) lands before the next manage sequence.
    state.wm.needs_setup_bindings = true;
    state.manage_dirty();
}

/// Overview key interception (rill-ed overviewKeyPressed): all pressed
/// bindings are routed here while the overview is open.
fn overview_key_pressed(
    state: &mut crate::app::AppData,
    proxy: &crate::river::river_xkb_binding_v1::RiverXkbBindingV1,
) {
    use crate::types::Status;
    use KeybindingAction as A;

    let Some(action) = binding_action(state, proxy) else {
        return;
    };

    // Drop entries for windows that closed mid-overview so navigation never
    // highlights a ghost slot.
    crate::overview::prune(state);
    let (total, cols, cur) = {
        let Some(ov) = &state.wm.overview_state else {
            return;
        };
        (ov.entries.len(), ov.columns, ov.highlighted)
    };
    let rows = total.div_ceil(cols);
    let row = cur / cols;
    let col = cur % cols;

    let mut next = cur;
    match action {
        // Toggle: pressing enter_overview again exits overview.
        A::EnterOverview | A::Exit | A::OverviewCancel => {
            crate::overview::cancel(state);
            crate::layout::update(&mut state.wm);
            state.wm.status = Status::Layout;
            return;
        }
        A::OverviewNavLeft | A::FocusWindowLeft | A::FocusOutputLeft => {
            if col > 0 {
                next = cur - 1;
            }
        }
        A::OverviewNavRight | A::FocusWindowRight | A::FocusOutputRight => {
            if col + 1 < cols {
                next = cur + 1;
            }
        }
        A::OverviewNavUp | A::FocusWorkspaceAbove | A::FocusOutputAbove => {
            if row > 0 {
                next = cur - cols;
            }
        }
        A::OverviewNavDown | A::FocusWorkspaceBelow | A::FocusOutputBelow => {
            if row + 1 < rows {
                next = cur + cols;
            }
        }
        A::OverviewConfirm => {
            crate::overview::select(state);
            crate::layout::update(&mut state.wm);
            state.wm.status = Status::Layout;
            return;
        }
        _ => return,
    }

    // The last grid row may be partial, so cap navigation at the real window
    // count instead of the grid bounds.
    if next >= total {
        next = total - 1;
    }
    if next != cur {
        state.wm.overview_state.as_mut().unwrap().highlighted = next;
        state.wm.status = Status::Overview;
        state.manage_dirty();
    }
}

impl wayland_client::Dispatch<crate::river::river_xkb_binding_v1::RiverXkbBindingV1, ()>
    for crate::app::AppData
{
    fn event(
        state: &mut crate::app::AppData,
        proxy: &crate::river::river_xkb_binding_v1::RiverXkbBindingV1,
        event: <crate::river::river_xkb_binding_v1::RiverXkbBindingV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &wayland_client::QueueHandle<Self>,
    ) {
        use crate::river::river_xkb_binding_v1::Event;
        let Event::Pressed = event else {
            return;
        };
        binding_pressed(state, proxy);
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;

    #[test]
    fn default_keysyms_all_parse() {
        for kb in default_keybindings() {
            assert!(parse_key(&kb.key).is_some(), "keysym '{}' invalid", kb.key);
        }
        for pb in default_pointer_bindings() {
            let _ = pb;
        }
    }

    #[test]
    fn parse_key_case_insensitive() {
        assert_eq!(parse_key("Return"), parse_key("return"));
        assert!(parse_key("notakeysym").is_none());
    }

    #[test]
    fn modifiers_parse() {
        use crate::river::river_seat_v1::Modifiers;
        let m = parse_modifiers(&["mod4".into(), "shift".into()]).unwrap();
        assert_eq!(m, Modifiers::Mod4.union(Modifiers::Shift));
        assert!(parse_modifiers(&["bogus".into()]).is_err());
        assert_eq!(parse_modifiers(&[]).unwrap(), Modifiers::None);
    }

    #[test]
    fn move_target_bounds() {
        // Regression: focus on window 0 with 2 windows, move left used to
        // underflow to u64::MAX and panic in window_list.swap.
        assert_eq!(move_target(0, 2, false), None);
        assert_eq!(move_target(1, 2, false), Some(0));
        assert_eq!(move_target(0, 2, true), Some(1));
        assert_eq!(move_target(1, 2, true), None);
        assert_eq!(move_target(0, 1, true), None);
        assert_eq!(move_target(usize::MAX, 2, false), None);
        assert_eq!(move_target(usize::MAX, 2, true), None);
    }
}
