// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dispatch root: protocol globals, binding lists, and the window-manager
//! global event dispatch. The manage cycle itself is wired in main.rs.

use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_manager_v1::WpCursorShapeManagerV1;

use crate::actions::{KeybindingAction, PointerAction};
use crate::river::river_layer_shell_seat_v1::RiverLayerShellSeatV1;
use crate::river::river_layer_shell_v1::RiverLayerShellV1;
use crate::river::river_pointer_binding_v1::RiverPointerBindingV1;
use crate::river::river_seat_v1::RiverSeatV1;
use crate::river::river_window_manager_v1::{self, RiverWindowManagerV1};
use crate::river::river_window_v1::RiverWindowV1;
use crate::river::river_xkb_binding_v1::RiverXkbBindingV1;
use crate::river::river_xkb_bindings_v1::RiverXkbBindingsV1;
use crate::types::{PendingWindow, Status};
use crate::wm::WindowManager;

pub struct XkbBinding {
    pub proxy: RiverXkbBindingV1,
    pub action: KeybindingAction,
}

pub struct PointerBindingEntry {
    pub proxy: RiverPointerBindingV1,
    pub action: PointerAction,
}

pub struct AppData {
    pub registry: WlRegistry,
    pub river_wm: Option<RiverWindowManagerV1>,
    pub river_xkb: Option<RiverXkbBindingsV1>,
    pub river_layer_shell: Option<RiverLayerShellV1>,
    pub river_seat: Option<RiverSeatV1>,
    pub layer_shell_seat: Option<RiverLayerShellSeatV1>,
    /// wl_seat of the river seat, bound so the window manager can set the
    /// pointer cursor during pointer operations (river-window-management-v1
    /// v4: the WM may set cursor surface/shape without pointer focus).
    /// Config file given with -c/--config, if any: used for reload too.
    pub config_path: Option<std::path::PathBuf>,
    /// The missing-seat warning is worth printing once, not on every manage.
    pub warned_missing_seat: bool,
    pub wl_seat: Option<WlSeat>,
    /// wl_seat version advertised by the registry, needed to bind it later
    /// from the river seat's `wl_seat` event (which only carries the name).
    pub wl_seat_version: u32,
    pub wl_pointer: Option<WlPointer>,
    pub cursor_shape_manager: Option<WpCursorShapeManagerV1>,
    pub cursor_shape: Option<WpCursorShapeDeviceV1>,
    pub wm: WindowManager,
    pub xkb_bindings: Vec<XkbBinding>,
    pub pointer_bindings: Vec<PointerBindingEntry>,
    /// Queue handle, kept for creating child proxies (get_node, bindings...).
    pub qh: QueueHandle<AppData>,
}

impl AppData {
    /// Request a manage sequence, if the window-manager global is bound.
    pub fn manage_dirty(&self) {
        if let Some(river_wm) = &self.river_wm {
            river_wm.manage_dirty();
        }
    }
}

impl Dispatch<WlRegistry, ()> for AppData {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: <WlRegistry as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        qh: &QueueHandle<Self>,
    ) {
        use wayland_client::protocol::wl_registry::Event;
        let Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "river_window_manager_v1" => {
                const VERSION: u32 = 4;
                if version < VERSION {
                    eprintln!("river_window_manager_v1 v{version}, need at least v{VERSION}");
                    std::process::exit(1);
                }
                state.river_wm = Some(registry.bind(name, VERSION, qh, ()));
            }
            "river_xkb_bindings_v1" => {
                const VERSION: u32 = 1;
                if version < VERSION {
                    eprintln!("river_xkb_bindings_v1 v{version}, need at least v{VERSION}");
                    std::process::exit(1);
                }
                state.river_xkb = Some(registry.bind(name, VERSION, qh, ()));
            }
            "river_layer_shell_v1" => {
                const VERSION: u32 = 1;
                if version < VERSION {
                    eprintln!("river_layer_shell_v1 v{version}, need at least v{VERSION}");
                    std::process::exit(1);
                }
                state.river_layer_shell = Some(registry.bind(name, VERSION, qh, ()));
            }
            // Optional: only used for cursor feedback during pointer ops.
            "wl_seat" => state.wl_seat_version = version,
            "wp_cursor_shape_manager_v1" => {
                state.cursor_shape_manager = Some(registry.bind(name, 1, qh, ()));
            }
            _ => {}
        }
    }
}

impl Dispatch<RiverWindowManagerV1, ()> for AppData {
    fn event(
        state: &mut Self,
        proxy: &RiverWindowManagerV1,
        event: <RiverWindowManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        qh: &QueueHandle<Self>,
    ) {
        use crate::river::river_window_manager_v1::Event;
        match event {
            Event::Unavailable => {
                eprintln!("Error: another WM is already running");
                std::process::exit(1);
            }
            Event::Finished => {
                state.river_wm = None;
                state.wm.should_exit_loop = true;
            }
            Event::Window { id } => {
                state.wm.pending_windows.push(PendingWindow {
                    river_window: id,
                    initialized: false,
                    title: None,
                    app_id: None,
                });
                state.wm.status = Status::Layout;
            }
            Event::Output { id } => {
                crate::output::output_added(state, id, qh);
            }
            Event::Seat { id } => {
                state.river_seat = Some(id);
                crate::seat::seat_added(state, qh);
                // Request binding setup via the flag: output/window events set
                // `status` directly, so assigning Status::SetupBindings here
                // loses the request whenever one of them lands before the next
                // manage sequence (no keybindings at all).
                state.wm.needs_setup_bindings = true;
            }
            Event::ManageStart => {
                manage(state);
                proxy.manage_finish();
            }
            Event::RenderStart => proxy.render_finish(),
            Event::SessionLocked => {
                state.wm.session_locked = true;
                // Save the focused window before lock so it can be restored
                // after unlock (KWM save/restore pattern).
                state.wm.lock_focus = state.wm.focused_window().map(|w| w.river_window.clone());
            }
            Event::SessionUnlocked => {
                state.wm.session_locked = false;
                // Restore focus to the window focused before lock, wherever
                // it currently lives.
                if let Some(locked) = &state.wm.lock_focus
                    && let Some((oi, wi, wi2)) = state.wm.locate_window(locked)
                {
                    state.wm.focused_output_idx = Some(oi);
                    state.wm.outputs[oi].focused_workspace_idx = wi;
                    state.wm.outputs[oi].workspace_list[wi].focused_window_idx = Some(wi2);
                }
                state.wm.lock_focus = None;
                state.wm.layer_shell_focus = crate::wm::LayerShellFocus::None;
                // The lock surface held keyboard focus; invalidate the stale
                // focus cache so focus_window is re-issued on unlock.
                state.wm.last_focused_window = None;
                state.wm.needs_refocus = true;
                state.wm.status = Status::Layout;
                state.manage_dirty();
            }
        }
    }

    wayland_client::event_created_child!(AppData, RiverWindowManagerV1, [
        river_window_manager_v1::EVT_WINDOW_OPCODE => (RiverWindowV1, ()),
        river_window_manager_v1::EVT_OUTPUT_OPCODE => (crate::river::river_output_v1::RiverOutputV1, ()),
        river_window_manager_v1::EVT_SEAT_OPCODE => (RiverSeatV1, ())
    ]);
}

wayland_client::delegate_noop!(AppData: ignore RiverXkbBindingsV1);
wayland_client::delegate_noop!(AppData: ignore crate::river::river_node_v1::RiverNodeV1);
wayland_client::delegate_noop!(AppData: ignore RiverLayerShellV1);
wayland_client::delegate_noop!(AppData: ignore WlPointer);
wayland_client::delegate_noop!(AppData: ignore WpCursorShapeManagerV1);
wayland_client::delegate_noop!(AppData: ignore WpCursorShapeDeviceV1);

impl Dispatch<WlSeat, ()> for AppData {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities { capabilities } = event else {
            return;
        };
        let WEnum::Value(capabilities) = capabilities else {
            return;
        };
        if capabilities.contains(wl_seat::Capability::Pointer) {
            if state.wl_pointer.is_none() {
                let pointer = seat.get_pointer(qh, ());
                state.cursor_shape = state
                    .cursor_shape_manager
                    .as_ref()
                    .map(|manager| manager.get_pointer(&pointer, qh, ()));
                state.wl_pointer = Some(pointer);
            }
        } else if let Some(pointer) = state.wl_pointer.take() {
            if pointer.version() >= 3 {
                pointer.release();
            }
            state.cursor_shape = None;
        }
    }
}

/// The manage-cycle state machine (rill-ed main.zig `manage`), dispatched on
/// river_window_manager_v1.manage_start. No animation: layout passes commit
/// via snap_to_finish immediately.
pub fn manage(state: &mut AppData) {
    use crate::types::Status;

    if state.wm.focused_output_idx.is_none() {
        return;
    }
    let Some(seat) = state.river_seat.clone() else {
        if !state.warned_missing_seat {
            state.warned_missing_seat = true;
            eprintln!("No seat yet; waiting for river_seat_v1");
        }
        return;
    };

    // Handled only once the prerequisites exist: the seat event sets this flag,
    // but manage returns early (dropping the request) while no output is
    // focused yet.
    if state.wm.needs_setup_bindings {
        state.wm.status = Status::SetupBindings;
        state.wm.needs_setup_bindings = false;
    }

    match state.wm.status.clone() {
        Status::Layout => {
            crate::layout::apply(&mut state.wm, &seat);
            if state.wm.needs_refocus {
                // focus_window may have been ignored during exclusive focus;
                // stay in Layout so the next manage sequence retries focus.
                state.wm.needs_refocus = false;
                state.manage_dirty();
            } else {
                crate::layout::snap_to_finish(&mut state.wm);
                state.wm.status = Status::None;
            }
        }
        Status::Overview => {
            crate::overview::apply_borders(&mut state.wm, &seat);
            crate::layout::snap_to_finish(&mut state.wm);
            state.wm.status = Status::None;
        }
        Status::PointerAction(_action) => {
            seat.op_start_pointer();
            crate::seat::pointer_action(&mut state.wm);
        }
        Status::SetupBindings => {
            crate::keybinding::setup_keybindings(state);
            crate::seat::setup_pointer_bindings(state);
            state.wm.status = Status::Layout;
            state.manage_dirty();
        }
        Status::Exit => {
            if let Some(river_wm) = &state.river_wm {
                river_wm.exit_session();
            }
        }
        Status::None => seat.op_end(),
    }
}
