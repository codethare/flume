// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::actions::KeybindingAction;

mod defaults;
mod dispatch;

pub use defaults::{default_keybindings, default_pointer_bindings};
pub use dispatch::dispatch_action;

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
        eprintln!("tailrace: river_xkb_bindings_v1 missing, no keybindings");
        return;
    };
    let Some(seat) = state.river_seat.clone() else {
        return;
    };
    for kb in &state.wm.config.keybindings {
        let Some(keysym) = parse_key(&kb.key) else {
            eprintln!("tailrace: unknown keysym '{}'", kb.key);
            continue;
        };
        let Ok(mods) = parse_modifiers(&kb.modifiers) else {
            eprintln!(
                "tailrace: invalid modifiers {:?} for key '{}'",
                kb.modifiers, kb.key
            );
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

fn reload_config(state: &mut crate::app::AppData) {
    let Some(new_config) = crate::config::reload(state.config_path.as_deref()) else {
        eprintln!("tailrace: config reload failed, keeping the running config");
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
    use crate::keybinding::dispatch::move_target;

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
