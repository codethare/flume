// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! In-process compositor harness: replays real river output hotplug event
//! streams over a socketpair against the actual dispatch code, so removal /
//! re-add sequences run the true `output_event` -> `layout::apply` manage
//! cycle. A panic or index-out-of-bounds here is the crash users hit on
//! HDMI unplug: the test fails instead.

use std::ffi::CString;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use wayland_backend::protocol::{Argument, Message};
use wayland_backend::rs::client::Backend as ClientBackend;
use wayland_backend::rs::server::Backend as ServerBackend;
use wayland_backend::server::{
    ClientData, ClientId, GlobalHandler, GlobalId, ObjectData, ObjectId,
};

use wayland_client::{Connection, EventQueue, QueueHandle};

use crate::app::AppData;
use crate::wm::WindowManager;
use crate::{config, layout};

// Regenerate the interface descriptors locally: river.rs generates them in a
// private module, so the harness gets its own copies (wire format is defined
// by interface NAME, so separate statics are fine). Nested like river.rs so
// cross-XML references (river_output_v1 -> wl_surface etc.) resolve.
mod ifaces {
    pub mod rwm {
        pub use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/river-window-management-v1.xml");
    }
    pub mod rxkb {
        use super::rwm::*;
        wayland_scanner::generate_interfaces!("./protocol/river-xkb-bindings-v1.xml");
    }
    pub mod rls {
        use super::rwm::*;
        wayland_scanner::generate_interfaces!("./protocol/river-layer-shell-v1.xml");
    }
}
use ifaces::rls::*;
use ifaces::rwm::*;
use ifaces::rxkb::*;
use wayland_protocols::wp::cursor_shape::v1::client::__interfaces::WP_CURSOR_SHAPE_MANAGER_V1_INTERFACE;

// ---------------------------------------------------------------------------
// Event opcodes (declaration order in the protocol XMLs; the client's
// Dispatch impls are generated from the same files).
// ---------------------------------------------------------------------------

// river_window_manager_v1 events
const EVT_MANAGE_START: u16 = 2;
const EVT_WINDOW: u16 = 6;
const EVT_OUTPUT: u16 = 7;
const EVT_SEAT: u16 = 8;
// river_output_v1 events
const EVT_OUT_REMOVED: u16 = 0;
const EVT_OUT_WL_OUTPUT: u16 = 1;
const EVT_OUT_POSITION: u16 = 2;
const EVT_OUT_DIMENSIONS: u16 = 3;
// river_window_v1 events (declaration order: closed=0, dimensions_hint=1,
// dimensions=2, app_id=3, ... fullscreen_requested=12, exit_fullscreen_requested=13)
const EVT_WIN_CLOSED: u16 = 0;
const EVT_WIN_DIMENSIONS: u16 = 2;
const EVT_WIN_APP_ID: u16 = 3;
const EVT_WIN_TITLE: u16 = 4;
const EVT_WIN_FULLSCREEN_REQUESTED: u16 = 12;
const EVT_WIN_EXIT_FULLSCREEN_REQUESTED: u16 = 13;
// river_xkb_binding_v1 events (pressed=0, released=1) and requests
// (destroy=0, set_layout_override=1, enable=2, disable=3)
const EVT_XKB_BINDING_PRESSED: u16 = 0;
const REQ_XKB_DISABLE: u16 = 3;
// both river_xkb_binding_v1 and river_pointer_binding_v1 have destroy = 0
const REQ_BINDING_DESTROY: u16 = 0;
// river_node_v1 requests (destroy=0, set_position=1, place_top=2)
const REQ_NODE_PLACE_TOP: u16 = 2;
// river_seat_v1 events (removed=0, wl_seat=1, …)
const EVT_SEAT_REMOVED: u16 = 0;
// river_seat_v1 events (…, wl_seat=1)
const EVT_SEAT_WL_SEAT: u16 = 1;
// wl_seat events (capabilities=0, name=1)
const EVT_WL_SEAT_CAPABILITIES: u16 = 0;
// wp_cursor_shape_device_v1 requests (destroy=0, set_shape=1)
const REQ_CURSOR_SHAPE_SET_SHAPE: u16 = 1;
// Name the server backend assigns to the wl_seat global (creation order:
// wm=1, xkb=2, ls=3, wl_seat=4, cursor shape manager=5).
const GLOBAL_NAME_WL_SEAT: u32 = 4;
// river_window_manager_v1 events (…, session_locked=4, session_unlocked=5)
const EVT_SESSION_LOCKED: u16 = 4;
const EVT_SESSION_UNLOCKED: u16 = 5;
// river_window_manager_v1 requests (…, exit_session=6)
const REQ_WM_EXIT_SESSION: u16 = 6;
// river_seat_v1 requests (focus_window=1, clear_focus=3, op_start_pointer=4,
// op_end=5, get_pointer_binding=6)
const REQ_SEAT_DESTROY: u16 = 0;
const REQ_SEAT_FOCUS_WINDOW: u16 = 1;
const REQ_SEAT_CLEAR_FOCUS: u16 = 3;
const REQ_SEAT_OP_END: u16 = 5;
const REQ_SEAT_GET_POINTER_BINDING: u16 = 6;
const REQ_SEAT_POINTER_WARP: u16 = 8;
// river_seat_v1 events (…, op_delta=6, op_release=7)
const EVT_SEAT_OP_DELTA: u16 = 6;
const EVT_SEAT_OP_RELEASE: u16 = 7;
// river_pointer_binding_v1 events (pressed=0, released=1)
const EVT_PTR_BINDING_PRESSED: u16 = 0;
// river_seat_v1 events (declaration order: removed=0, wl_seat=1,
// pointer_enter=2, pointer_leave=3, window_interaction=4)
const EVT_SEAT_POINTER_ENTER: u16 = 2;
// river_layer_shell_output_v1 events
const EVT_LSO_NON_EXCLUSIVE_AREA: u16 = 0;

// wl_output events (geometry=0, mode=1, done=2, scale=3)
const EVT_WL_OUTPUT_NAME: u16 = 4;
mod server;
mod session;

use server::{
    MiniServer, children_with_interface, count_requests, node_positions, pointer_binding_objects,
};
use session::{Session, build};

mod cursor;
mod keybindings;
mod manage;
mod outputs;
mod overview;
mod pointer;
mod rules;
mod windows;
