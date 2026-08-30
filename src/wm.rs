// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Core window-manager state, ported from rill-ed types.zig `WindowManager`.
//!
//! Protocol globals (river_window_manager, seat, xkb/pointer binding lists)
//! live in the dispatch layer (`AppData`); this struct holds the WM state and
//! config only, so pure logic here is testable without Wayland.

use std::collections::HashMap;

use crate::river::river_window_v1::RiverWindowV1;
use crate::types::{
    Config, DetachedOutput, Output, OverviewHome, OverviewState, PendingWindow, Status, Window,
    Workspace,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerShellFocus {
    #[default]
    None,
    NonExclusive,
    Exclusive,
}

pub struct WindowManager {
    pub config: Config,
    pub outputs: Vec<Output>,
    pub focused_output_idx: Option<usize>,
    pub previous_workspace: Option<OverviewHome>,
    pub pending_windows: Vec<PendingWindow>,
    /// Workspaces preserved when their output was removed, keyed by output
    /// name; restored when an output with the same name reappears.
    pub detached_outputs: HashMap<String, DetachedOutput>,
    pub overview_state: Option<OverviewState>,
    pub status: Status,
    pub needs_refocus: bool,
    pub needs_setup_bindings: bool,
    pub should_exit_loop: bool,
    /// Last regular window we requested the compositor to focus. Used to
    /// avoid sending redundant focus/clear_focus requests, which break input
    /// method clients (fcitx5) on every layout cycle.
    pub last_focused_window: Option<RiverWindowV1>,
    pub layer_shell_focus: LayerShellFocus,
    /// True when the compositor has sent session_locked.
    pub session_locked: bool,
    /// Window focused when the session was locked, restored on unlock.
    pub lock_focus: Option<RiverWindowV1>,
    /// Set when the focused output changed because an output was
    /// added/removed; the next layout pass warps the pointer.
    pub needs_pointer_warp: bool,
}

impl WindowManager {
    pub fn new(config: Config) -> WindowManager {
        WindowManager {
            config,
            outputs: Vec::new(),
            focused_output_idx: None,
            previous_workspace: None,
            pending_windows: Vec::new(),
            detached_outputs: HashMap::new(),
            overview_state: None,
            status: Status::None,
            needs_refocus: false,
            needs_setup_bindings: false,
            should_exit_loop: false,
            last_focused_window: None,
            layer_shell_focus: LayerShellFocus::None,
            session_locked: false,
            lock_focus: None,
            needs_pointer_warp: false,
        }
    }

    pub fn focused_output(&self) -> Option<&Output> {
        self.outputs.get(self.focused_output_idx?)
    }

    /// (output_idx, workspace_idx) of the focused workspace.
    pub fn current_ws_idx(&self) -> Option<(usize, usize)> {
        let output = self.focused_output()?;
        Some((self.focused_output_idx?, output.focused_workspace_idx))
    }

    pub fn workspace(&self, output_idx: usize, workspace_idx: usize) -> Option<&Workspace> {
        self.outputs
            .get(output_idx)?
            .workspace_list
            .get(workspace_idx)
    }

    pub fn workspace_mut(
        &mut self,
        output_idx: usize,
        workspace_idx: usize,
    ) -> Option<&mut Workspace> {
        self.outputs
            .get_mut(output_idx)?
            .workspace_list
            .get_mut(workspace_idx)
    }

    pub fn focused_window(&self) -> Option<&Window> {
        let (oi, wi) = self.current_ws_idx()?;
        self.workspace(oi, wi)?.focused_window()
    }

    pub fn focused_window_mut(&mut self) -> Option<&mut Window> {
        let (oi, wi) = self.current_ws_idx()?;
        self.workspace_mut(oi, wi)?.focused_window_mut()
    }

    /// Locate a window by its river proxy across all outputs/workspaces.
    /// Returns (output_idx, workspace_idx, window_idx).
    pub fn locate_window(&self, river_window: &RiverWindowV1) -> Option<(usize, usize, usize)> {
        for (output_idx, output) in self.outputs.iter().enumerate() {
            for (workspace_idx, workspace) in output.workspace_list.iter().enumerate() {
                if let Some(window_idx) = workspace
                    .window_list
                    .iter()
                    .position(|w| &w.river_window == river_window)
                {
                    return Some((output_idx, workspace_idx, window_idx));
                }
            }
        }
        None
    }

    /// Move a window between workspaces, adjusting both focus indices
    /// (rill-ed `moveWindowToWorkspace`). The moved window lands right after
    /// the target's focused window.
    pub fn move_window_to_workspace(
        &mut self,
        source: (usize, usize),
        target: (usize, usize),
        window_idx: usize,
    ) {
        // mem::take dance: two &mut Workspace from the same [Workspace; 10]
        // cannot coexist, so the source list is taken out first.
        let (mut source_list, mut source_focus) = {
            let ws = self.workspace_mut(source.0, source.1);
            match ws {
                Some(ws) => (
                    std::mem::take(&mut ws.window_list),
                    std::mem::take(&mut ws.focused_window_idx),
                ),
                None => return,
            }
        };
        let Some(target_ws) = self.workspace_mut(target.0, target.1) else {
            // Put the source back untouched.
            let ws = self.workspace_mut(source.0, source.1).unwrap();
            ws.window_list = source_list;
            ws.focused_window_idx = source_focus;
            return;
        };
        move_window(
            &mut source_list,
            &mut source_focus,
            &mut target_ws.window_list,
            &mut target_ws.focused_window_idx,
            window_idx,
        );
        let ws = self.workspace_mut(source.0, source.1).unwrap();
        ws.window_list = source_list;
        ws.focused_window_idx = source_focus;
    }
}

/// Move `window_idx` from `source` to `target`, fixing up both focus indices.
/// Generic so the index semantics are testable without Wayland.
pub fn move_window<T>(
    source: &mut Vec<T>,
    source_focus: &mut Option<usize>,
    target: &mut Vec<T>,
    target_focus: &mut Option<usize>,
    window_idx: usize,
) {
    if window_idx >= source.len() {
        return;
    }
    let window = source.remove(window_idx);

    if source.is_empty() {
        *source_focus = None;
    } else if window_idx != 0 {
        *source_focus = Some(window_idx - 1);
    }

    let target_idx = target_focus.map_or(0, |i| i + 1);
    target.insert(target_idx, window);
    *target_focus = Some(target_idx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn move_in_list(
        source: &mut Vec<i32>,
        source_focus: &mut Option<usize>,
        target: &mut Vec<i32>,
        target_focus: &mut Option<usize>,
        window_idx: usize,
    ) {
        move_window(source, source_focus, target, target_focus, window_idx);
    }

    #[test]
    fn move_window_adjusts_source_focus() {
        let mut src = vec![1, 2, 3];
        let mut src_focus = Some(2);
        let mut dst = vec![];
        let mut dst_focus: Option<usize> = None;

        move_in_list(&mut src, &mut src_focus, &mut dst, &mut dst_focus, 2);
        assert_eq!(src, vec![1, 2]);
        assert_eq!(src_focus, Some(1));
        assert_eq!(dst, vec![3]);
        assert_eq!(dst_focus, Some(0));
    }

    #[test]
    fn move_window_lands_after_target_focus() {
        let mut src = vec![1];
        let mut src_focus = Some(0);
        let mut dst = vec![7, 8, 9];
        let mut dst_focus = Some(1);

        move_in_list(&mut src, &mut src_focus, &mut dst, &mut dst_focus, 0);
        assert_eq!(src, vec![]);
        assert_eq!(src_focus, None);
        assert_eq!(dst, vec![7, 8, 1, 9]);
        assert_eq!(dst_focus, Some(2));
    }

    #[test]
    fn move_window_index_zero_keeps_focus_at_zero() {
        // rill-ed behavior: focus stays at position 0 (the next window
        // slides into focus) when the removed window was at index 0.
        let mut src = vec![1, 2];
        let mut src_focus = Some(0);
        let mut dst = vec![];
        let mut dst_focus: Option<usize> = None;

        move_in_list(&mut src, &mut src_focus, &mut dst, &mut dst_focus, 0);
        assert_eq!(src, vec![2]);
        assert_eq!(src_focus, Some(0));
    }

    #[test]
    fn move_window_out_of_bounds_is_noop() {
        let mut src = vec![1];
        let mut src_focus = Some(0);
        let mut dst: Vec<i32> = vec![];
        let mut dst_focus: Option<usize> = None;

        move_in_list(&mut src, &mut src_focus, &mut dst, &mut dst_focus, 5);
        assert_eq!(src, vec![1]);
        assert!(dst.is_empty());
    }
}
