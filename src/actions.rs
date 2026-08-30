// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Button {
    Left,
    Right,
    Middle,
}

impl Button {
    /// linux/input-event-codes.h BTN_* codes.
    pub fn code(self) -> u32 {
        match self {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerAction {
    MoveWindow,
    ResizeWindow,
}

/// Default serde (externally tagged) representation matches the config shape:
/// unit variants as plain strings (`action = "close_window"`), payload
/// variants as one-key tables (`action = { adjust_window_width = -0.1 }`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeybindingAction {
    CloseWindow,
    ToggleFullscreen,
    ToggleMaximizeColumn,
    AdjustWindowWidth(f32),
    SetWindowWidth(f32),
    AdjustFloatingWindowSize(f32),
    SetFloatingWindowHeight(f32),
    FocusWindowLeft,
    FocusWindowOrOutputLeft,
    FocusWindowRight,
    FocusWindowOrOutputRight,
    MoveWindowLeft,
    MoveWindowRight,
    MoveFloatingWindowLeft,
    MoveFloatingWindowRight,
    MoveFloatingWindowUp,
    MoveFloatingWindowDown,
    MoveWindowLeftOrToOutputLeft,
    MoveWindowRightOrToOutputRight,
    ToggleWorkspaceFloating,
    FocusWorkspaceAbove,
    FocusWorkspaceBelow,
    FocusWorkspaceOrOutputAbove,
    FocusWorkspaceOrOutputBelow,
    FocusWorkspacePrevious,
    FocusWorkspaceNumber(usize),
    MoveWindowToWorkspaceAbove,
    MoveWindowToWorkspaceBelow,
    MoveWindowToWorkspaceOrOutputAbove,
    MoveWindowToWorkspaceOrOutputBelow,
    MoveWindowToWorkspaceNumber(usize),
    FocusOutputLeft,
    FocusOutputRight,
    FocusOutputAbove,
    FocusOutputBelow,
    MoveWindowToOutputLeft,
    MoveWindowToOutputRight,
    MoveWindowToOutputAbove,
    MoveWindowToOutputBelow,
    Exit,
    ReloadConfig,
    EnterOverview,
    OverviewCancel,
    OverviewConfirm,
    OverviewNavUp,
    OverviewNavDown,
    OverviewNavLeft,
    OverviewNavRight,
    Spawn(Vec<String>),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_unit_variant_parses_from_string() {
        let a: KeybindingAction = toml::from_str("action = \"close_window\"")
            .map(|t: TomlAction| t.action)
            .unwrap();
        assert_eq!(a, KeybindingAction::CloseWindow);
    }

    #[test]
    fn action_payload_variant_parses_from_table() {
        let a: KeybindingAction = toml::from_str("action = { adjust_window_width = -0.1 }")
            .map(|t: TomlAction| t.action)
            .unwrap();
        assert_eq!(a, KeybindingAction::AdjustWindowWidth(-0.1));

        let a: KeybindingAction = toml::from_str("action = { spawn = [\"alacritty\"] }")
            .map(|t: TomlAction| t.action)
            .unwrap();
        assert_eq!(a, KeybindingAction::Spawn(vec!["alacritty".into()]));
    }

    #[derive(Deserialize)]
    struct TomlAction {
        action: KeybindingAction,
    }
}
