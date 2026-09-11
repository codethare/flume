// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Default keybindings and pointer bindings (ported from rill-ed keybinding.zig).

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
