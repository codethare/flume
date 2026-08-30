// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::Deserialize;

use crate::actions::{Button, KeybindingAction, PointerAction};
use crate::river::river_layer_shell_output_v1::RiverLayerShellOutputV1;
use crate::river::river_node_v1::RiverNodeV1;
use crate::river::river_output_v1::RiverOutputV1;
use crate::river::river_window_v1::RiverWindowV1;
use crate::river::wl_output::WlOutput;

/// Window pending initialization: held in `pending_windows` until
/// `dimensions` arrives, then moved to a workspace.
pub struct PendingWindow {
    pub river_window: RiverWindowV1,
    pub initialized: bool,
    pub title: Option<String>,
    pub app_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rectangle {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rectangle {
    pub fn eql(self, other: Rectangle) -> bool {
        self.x == other.x
            && self.y == other.y
            && self.width == other.width
            && self.height == other.height
    }
}

/// Per-window layout state. Kept separate from the Wayland proxies so the
/// geometry code (common/scroller/floating) is pure and unit-testable.
#[derive(Clone)]
pub struct WindowGeom {
    /// Fraction of the output's available width this column occupies.
    pub proportion: f32,
    pub is_fullscreen: bool,
    pub is_floating: bool,
    pub is_closing: bool,
    /// Geometry while floating (rest position).
    pub floating: Rectangle,
    pub current: Rectangle,
    /// Floating-rect anchor captured at pointer-drag start.
    pub drag_origin: Option<Rectangle>,
    /// Last geometry sent to the compositor; used to skip redundant requests.
    pub sent_current: Option<Rectangle>,
    /// Layout target computed by the layout pass; the coordinator snaps
    /// `current` to it (no animation).
    pub finish: Option<Rectangle>,
    pub sent_clip: Option<Rectangle>,
    pub sent_visible: Option<bool>,
    pub sent_border_focused: Option<bool>,
    pub sent_border_width: Option<u8>,
    /// Name of the output this window was migrated from (if any). Used to
    /// return windows to their original output when it reappears.
    pub former_output_name: Option<String>,
}

impl WindowGeom {
    pub fn new(proportion: f32, current: Rectangle) -> WindowGeom {
        WindowGeom {
            proportion,
            is_fullscreen: false,
            is_floating: false,
            is_closing: false,
            floating: current,
            current,
            drag_origin: None,
            sent_current: None,
            finish: None,
            sent_clip: None,
            sent_visible: None,
            sent_border_focused: None,
            sent_border_width: None,
            former_output_name: None,
        }
    }
}

pub struct Window {
    pub river_window: RiverWindowV1,
    pub river_node: RiverNodeV1,
    pub geom: WindowGeom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    #[default]
    Scroller,
    Floating,
}

pub struct Workspace {
    pub window_list: Vec<Window>,
    pub focused_window_idx: Option<usize>,
    pub is_floating: bool,
    pub layout: Layout,
}

impl Workspace {
    pub fn new() -> Workspace {
        Workspace {
            window_list: Vec::new(),
            focused_window_idx: None,
            is_floating: false,
            layout: Layout::Scroller,
        }
    }

    pub fn focused_window(&self) -> Option<&Window> {
        let idx = self.focused_window_idx?;
        self.window_list.get(idx)
    }

    pub fn focused_window_mut(&mut self) -> Option<&mut Window> {
        let idx = self.focused_window_idx?;
        self.window_list.get_mut(idx)
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace::new()
    }
}

pub struct Output {
    pub river_output: RiverOutputV1,
    pub river_layer_shell_output: Option<RiverLayerShellOutputV1>,
    pub wl_output: Option<WlOutput>,
    pub name: Option<String>,
    pub workspace_list: [Workspace; 10],
    pub focused_workspace_idx: usize,
    pub rectangle: Rectangle,
    /// Output rectangle minus exclusive zone (e.g. bars).
    pub non_exclusive: Rectangle,
    pub is_removed: bool,
}

/// A detached output: workspaces and windows preserved when the output is
/// removed (e.g. laptop panel off during lock / TTY switch), restored when an
/// output with the same name reappears.
pub struct DetachedOutput {
    pub workspace_list: [Workspace; 10],
    pub focused_workspace_idx: usize,
}

pub struct OverviewState {
    pub entries: Vec<OverviewEntry>,
    pub highlighted: usize,
    pub columns: usize,
    pub previous_workspace: Option<OverviewHome>,
}

#[derive(Clone)]
pub struct OverviewEntry {
    /// Stable identity: resolved by scanning live lists at use time, so
    /// closing or reordering windows mid-overview cannot corrupt entries.
    pub window: RiverWindowV1,
    /// Fullscreen state captured at enter; restored on exit.
    pub was_fullscreen: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct OverviewHome {
    pub output_idx: usize,
    pub workspace_idx: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Layout,
    PointerAction(PointerAction),
    Overview,
    SetupBindings,
    Exit,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CenterFocused {
    Never,
    Always,
    Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Border {
    pub width: u8,
    pub focused_color: Color,
    pub unfocused_color: Color,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Cursor {
    pub theme: String,
    pub size: u32,
}

/// Rule matching windows by exact app_id and glob title. All set fields must
/// match. Title supports `*` (any run, including empty) and `?` (single char).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct WindowRule {
    pub app_id: Option<String>,
    pub title: Option<String>,
    #[serde(default)]
    pub floating: bool,
}

impl WindowRule {
    pub fn matches(&self, app_id: Option<&str>, title: Option<&str>) -> bool {
        if self.app_id.is_none() && self.title.is_none() {
            return false;
        }
        if let Some(a) = &self.app_id
            && app_id != Some(a.as_str())
        {
            return false;
        }
        if let Some(t) = &self.title {
            match title {
                Some(title) if glob_match(t, title) => {}
                _ => return false,
            }
        }
        true
    }
}

pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut matched) = (None::<usize>, 0usize);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            matched = t;
            p += 1;
        } else if let Some(s) = star {
            p = s + 1;
            matched += 1;
            t = matched;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Keybinding {
    pub key: String,
    /// Modifier names: shift, ctrl, mod1, mod3, mod4, mod5.
    pub modifiers: Vec<String>,
    pub action: KeybindingAction,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PointerBinding {
    pub button: Button,
    pub modifiers: Vec<String>,
    pub action: PointerAction,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Gap between windows and the output's top/bottom edge.
    pub vertical_gap: i32,
    /// Gap between adjacent windows.
    pub horizontal_gap: i32,
    /// Default proportion of the output's available width for a new window.
    pub default_window_width: f32,
    pub center_focused_window: CenterFocused,
    pub no_csd: bool,
    pub border: Border,
    pub cursor: Option<Cursor>,
    pub spawn_at_startup: Vec<Vec<String>>,
    pub keybindings: Vec<Keybinding>,
    pub pointer_bindings: Vec<PointerBinding>,
    pub window_rules: Vec<WindowRule>,
}

impl Default for Border {
    fn default() -> Self {
        Border {
            width: 3,
            focused_color: Color {
                r: 141,
                g: 214,
                b: 0,
                a: 1.0,
            },
            unfocused_color: Color {
                r: 160,
                g: 160,
                b: 160,
                a: 1.0,
            },
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            vertical_gap: 9,
            horizontal_gap: 9,
            default_window_width: 0.5,
            center_focused_window: CenterFocused::Never,
            no_csd: true,
            border: Border::default(),
            cursor: None,
            spawn_at_startup: Vec::new(),
            keybindings: Vec::new(),
            pointer_bindings: Vec::new(),
            window_rules: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_eql() {
        let r = Rectangle {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert!(r.eql(r));
        assert!(!r.eql(Rectangle { width: 1919, ..r }));
    }

    #[test]
    fn window_rule_matches() {
        let r = WindowRule {
            app_id: Some("footclient".into()),
            title: None,
            floating: true,
        };
        assert!(r.matches(Some("footclient"), None));
        assert!(!r.matches(Some("foot"), None));
        assert!(!r.matches(None, None));

        let both = WindowRule {
            app_id: Some("a".into()),
            title: Some("t".into()),
            floating: false,
        };
        assert!(both.matches(Some("a"), Some("t")));
        assert!(!both.matches(Some("a"), Some("x")));

        let empty = WindowRule::default();
        assert!(!empty.matches(Some("a"), Some("t")));

        let glob = WindowRule {
            app_id: None,
            title: Some("file-*".into()),
            floating: false,
        };
        assert!(glob.matches(None, Some("file-a.txt")));
        assert!(glob.matches(None, Some("file-")));
        assert!(!glob.matches(None, Some("dir/file-a.txt")));

        let q = WindowRule {
            app_id: None,
            title: Some("file-?.txt".into()),
            floating: false,
        };
        assert!(q.matches(None, Some("file-a.txt")));
        assert!(!q.matches(None, Some("file-ab.txt")));

        let star_only = WindowRule {
            app_id: None,
            title: Some("*".into()),
            floating: false,
        };
        assert!(star_only.matches(None, Some("")));
        assert!(star_only.matches(None, Some("anything")));

        let backtrack = WindowRule {
            app_id: None,
            title: Some("a*b".into()),
            floating: false,
        };
        assert!(backtrack.matches(None, Some("aXbYb")));
        assert!(!backtrack.matches(None, Some("aXbY")));
    }

    #[test]
    fn config_defaults_deserialize_from_empty() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.vertical_gap, 9);
        assert_eq!(config.horizontal_gap, 9);
        assert_eq!(config.default_window_width, 0.5);
        assert_eq!(config.center_focused_window, CenterFocused::Never);
        assert!(config.no_csd);
        assert_eq!(config.border.width, 3);
        assert!(config.cursor.is_none());
        assert!(config.keybindings.is_empty());
    }

    #[test]
    fn config_parses_keybindings_and_rules() {
        let config: Config = toml::from_str(
            r#"
            [[keybindings]]
            key = "q"
            modifiers = ["mod4"]
            action = "close_window"

            [[keybindings]]
            key = "minus"
            modifiers = ["mod4", "ctrl"]
            action = { adjust_window_width = -0.1 }

            [[keybindings]]
            key = "t"
            modifiers = ["mod4"]
            action = { spawn = ["alacritty"] }

            [[pointer_bindings]]
            button = "left"
            modifiers = ["mod4"]
            action = "move_window"

            [[window_rules]]
            app_id = "footclient"
            floating = true

            [[window_rules]]
            title = "file-*"
            floating = true
            "#,
        )
        .unwrap();
        assert_eq!(config.keybindings.len(), 3);
        assert_eq!(
            config.keybindings[0].action,
            crate::actions::KeybindingAction::CloseWindow
        );
        assert_eq!(
            config.keybindings[1].action,
            crate::actions::KeybindingAction::AdjustWindowWidth(-0.1)
        );
        assert_eq!(
            config.keybindings[2].action,
            crate::actions::KeybindingAction::Spawn(vec!["alacritty".into()])
        );
        assert_eq!(config.pointer_bindings.len(), 1);
        assert_eq!(config.pointer_bindings[0].button, Button::Left);
        assert_eq!(config.window_rules.len(), 2);
        assert!(config.window_rules[0].matches(Some("footclient"), None));
        assert!(config.window_rules[1].matches(None, Some("file-a.txt")));
    }

    #[test]
    fn config_rejects_invalid_values() {
        assert!(toml::from_str::<Config>("vertical_gap = \"nine\"").is_err());
        assert!(toml::from_str::<Config>("center_focused_window = \"sometimes\"").is_err());
        assert!(
            toml::from_str::<Config>(
                "[[keybindings]]\nkey = \"q\"\nmodifiers = [\"mod4\"]\naction = \"nonexistent\""
            )
            .is_err()
        );
    }
}
