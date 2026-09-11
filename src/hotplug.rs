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
const EVT_WIN_FULLSCREEN_REQUESTED: u16 = 12;
const EVT_WIN_EXIT_FULLSCREEN_REQUESTED: u16 = 13;
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
const REQ_SEAT_FOCUS_WINDOW: u16 = 1;
const REQ_SEAT_CLEAR_FOCUS: u16 = 3;
const REQ_SEAT_OP_END: u16 = 5;
const REQ_SEAT_GET_POINTER_BINDING: u16 = 6;
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

/// One client request observed by the mini-server: (object, opcode, args).
type RequestLogEntry = (ObjectId, u16, String);

fn fmt_args<F>(args: &[Argument<ObjectId, F>]) -> String {
    args.iter()
        .map(|a| match a {
            Argument::Int(v) => format!("i{v}"),
            Argument::Uint(v) => format!("u{v}"),
            Argument::Fixed(v) => format!("f{v}"),
            Argument::Str(Some(s)) => format!("s({})", s.to_string_lossy()),
            Argument::Str(None) => "s(null)".into(),
            Argument::Object(o) => format!("o{o:?}"),
            Argument::NewId(o) => format!("n{o:?}"),
            Argument::Array(_) => "a[]".into(),
            Argument::Fd(_) => "fd".into(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Generic no-op server object: tolerates every request flume makes, and
/// returns fresh `Obj` data for children it creates via `new_id` requests.
/// Records every `new_id` child it spawns so tests can send events on them
/// (e.g. `non_exclusive_area` on layer-shell outputs), and every request the
/// client makes so tests can inspect what flume sends for a window.
#[derive(Default)]
struct Obj {
    children: Option<Arc<Mutex<Vec<ObjectId>>>>,
    requests: Option<Arc<Mutex<Vec<RequestLogEntry>>>>,
}

impl ObjectData<()> for Obj {
    fn request(
        self: Arc<Self>,
        _handle: &wayland_backend::server::Handle,
        _data: &mut (),
        _client_id: ClientId,
        msg: Message<ObjectId, OwnedFd>,
    ) -> Option<Arc<dyn ObjectData<()>>> {
        if let Some(log) = &self.requests {
            log.lock()
                .unwrap()
                .push((msg.sender_id.clone(), msg.opcode, fmt_args(&msg.args)));
        }
        let mut has_new_id = false;
        for a in &msg.args {
            if let Argument::NewId(id) = a {
                has_new_id = true;
                if let Some(children) = &self.children {
                    children.lock().unwrap().push(id.clone());
                }
            }
        }
        if has_new_id {
            Some(Arc::new(Obj {
                children: self.children.clone(),
                requests: self.requests.clone(),
            }))
        } else {
            None
        }
    }

    fn destroyed(
        self: Arc<Self>,
        _handle: &wayland_backend::server::Handle,
        _data: &mut (),
        _client_id: ClientId,
        _object_id: ObjectId,
    ) {
    }
}

#[derive(Default)]
struct Client;

impl ClientData for Client {}

/// Records every global bind: `(label, client object id)`, in order.
#[derive(Clone, Default)]
struct BindLog(Arc<Mutex<Vec<(&'static str, ObjectId)>>>);

struct GenericGlobal {
    log: BindLog,
    label: &'static str,
    children: Arc<Mutex<Vec<ObjectId>>>,
    requests: Arc<Mutex<Vec<RequestLogEntry>>>,
}

impl GenericGlobal {
    fn labeled(
        log: BindLog,
        label: &'static str,
        children: Arc<Mutex<Vec<ObjectId>>>,
        requests: Arc<Mutex<Vec<RequestLogEntry>>>,
    ) -> Arc<dyn GlobalHandler<()>> {
        Arc::new(GenericGlobal {
            log,
            label,
            children,
            requests,
        })
    }
}

impl GlobalHandler<()> for GenericGlobal {
    fn bind(
        self: Arc<Self>,
        _handle: &wayland_backend::server::Handle,
        _data: &mut (),
        _client_id: ClientId,
        _global_id: GlobalId,
        object_id: ObjectId,
    ) -> Arc<dyn ObjectData<()>> {
        self.log.0.lock().unwrap().push((self.label, object_id));
        Arc::new(Obj {
            children: Some(self.children.clone()),
            requests: Some(self.requests.clone()),
        })
    }
}

struct MiniServer {
    backend: ServerBackend<()>,
    bind_log: BindLog,
    next_global: u32,
    client: ClientId,
    wm: Option<ObjectId>,
    xkb: Option<ObjectId>,
    layershell: Option<ObjectId>,
    /// river_output_v1 object id per output name, for removal events.
    output_ids: Vec<(String, ObjectId)>,
    /// Every `new_id` child the client requested (layer-shell outputs, nodes).
    children: Arc<Mutex<Vec<ObjectId>>>,
    /// Every request the client made, keyed by target object.
    request_log: Arc<Mutex<Vec<RequestLogEntry>>>,
}

impl MiniServer {
    fn new(stream: UnixStream) -> Self {
        let backend = ServerBackend::<()>::new().unwrap();
        let bind_log = BindLog::default();
        let children = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        // Globals advertised to every registry the client creates, in bind order.
        let globals: [(
            &'static wayland_backend::protocol::Interface,
            u32,
            &'static str,
        ); 5] = [
            (&RIVER_WINDOW_MANAGER_V1_INTERFACE, 4, "wm"),
            (&RIVER_XKB_BINDINGS_V1_INTERFACE, 1, "xkb"),
            (&RIVER_LAYER_SHELL_V1_INTERFACE, 1, "ls"),
            // Bound by the client only once river_seat_v1.wl_seat names it.
            (&WL_SEAT_INTERFACE, 9, "wl_seat"),
            // Optional cursor feedback during pointer operations.
            (&WP_CURSOR_SHAPE_MANAGER_V1_INTERFACE, 1, "cursor_shape"),
        ];
        for (interface, version, label) in globals {
            backend.handle().create_global(
                interface,
                version,
                GenericGlobal::labeled(bind_log.clone(), label, children.clone(), requests.clone()),
            );
        }
        let client = backend
            .handle()
            .insert_client(stream, Arc::new(Client))
            .unwrap();
        MiniServer {
            backend,
            bind_log,
            next_global: 5,
            client,
            wm: None,
            xkb: None,
            layershell: None,
            output_ids: Vec::new(),
            children,
            request_log: requests,
        }
    }

    /// Every request the client made on `object`, in arrival order.
    fn requests_for(&self, object: &ObjectId) -> Vec<RequestLogEntry> {
        self.request_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(o, _, _)| o == object)
            .cloned()
            .collect()
    }

    fn clear_request_log(&self) {
        self.request_log.lock().unwrap().clear();
    }

    fn output_id(&self, name: &str) -> ObjectId {
        self.output_ids
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, o)| o.clone())
            .unwrap_or_else(|| panic!("no river_output_v1 for {name:?}"))
    }

    fn core_ids(&mut self) {
        if self.wm.is_some() {
            return;
        }
        let binds = self.bind_log.0.lock().unwrap().clone();
        let get = |label: &str| {
            binds
                .iter()
                .find(|(l, _)| *l == label)
                .map(|(_, o)| o.clone())
        };
        self.wm = get("wm");
        self.xkb = get("xkb");
        self.layershell = get("ls");
        assert!(
            self.wm.is_some() && self.xkb.is_some() && self.layershell.is_some(),
            "river globals not all bound: {binds:?}"
        );
    }

    fn add_wl_output_global(&mut self) -> u32 {
        self.next_global += 1;
        let label: &'static str =
            Box::leak(format!("wl_output:{}", self.next_global).into_boxed_str());
        self.backend.handle().create_global(
            &WL_OUTPUT_INTERFACE,
            4,
            GenericGlobal::labeled(
                self.bind_log.clone(),
                label,
                self.children.clone(),
                self.request_log.clone(),
            ),
        );
        self.next_global
    }

    fn create_object(
        &mut self,
        interface: &'static wayland_backend::protocol::Interface,
    ) -> ObjectId {
        self.backend
            .handle()
            .create_object(
                self.client.clone(),
                interface,
                4,
                Arc::new(Obj {
                    children: Some(self.children.clone()),
                    requests: Some(self.request_log.clone()),
                }),
            )
            .unwrap()
    }

    fn send(&mut self, sender: ObjectId, opcode: u16, args: Vec<Argument<ObjectId, RawFd>>) {
        let mut msg = Message {
            sender_id: sender,
            opcode,
            args: Default::default(),
        };
        for a in args {
            msg.args.push(a);
        }
        self.backend.handle().send_event(msg).unwrap();
    }

    fn process_and_flush(&mut self) -> usize {
        let n = self.backend.dispatch_all_clients(&mut ()).unwrap();
        self.backend.flush(None).unwrap();
        n
    }

    fn wl_output_object(&self, gname: u32) -> ObjectId {
        let label = format!("wl_output:{gname}");
        self.bind_log
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|(l, _)| **l == label)
            .map(|(_, o)| o.clone())
            .unwrap_or_else(|| panic!("wl_output global {gname} never bound"))
    }
}

struct Session {
    queue: EventQueue<AppData>,
    conn: Connection,
    state: AppData,
}

impl Session {
    fn new(server: &mut MiniServer, client_stream: UnixStream) -> Self {
        let cbackend = ClientBackend::connect(client_stream).unwrap();
        let conn = Connection::from_backend(cbackend);
        let queue = conn.new_event_queue();
        let qh: QueueHandle<AppData> = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let state = AppData {
            registry,
            river_wm: None,
            river_xkb: None,
            river_layer_shell: None,
            river_seat: None,
            layer_shell_seat: None,
            wl_seat: None,
            wl_seat_version: 0,
            wl_pointer: None,
            cursor_shape_manager: None,
            cursor_shape: None,
            wm: WindowManager::new(config::default_config()),
            xkb_bindings: Vec::new(),
            pointer_bindings: Vec::new(),
            qh: qh.clone(),
        };
        let mut s = Session { queue, conn, state };
        s.pump(server);
        server.core_ids();
        assert!(
            s.state.river_wm.is_some(),
            "river_window_manager_v1 never bound"
        );
        assert!(
            s.state.river_xkb.is_some(),
            "river_xkb_bindings_v1 never bound"
        );
        s
    }

    /// Client requests out; server processes/replies; server events in;
    /// repeat until both sides are quiescent.
    fn pump(&mut self, server: &mut MiniServer) {
        self.conn.flush().unwrap();
        server.process_and_flush();
        loop {
            let mut progressed = false;
            if let Some(guard) = self.queue.prepare_read()
                && guard.read().is_ok()
            {
                progressed = true;
            }
            if self.queue.dispatch_pending(&mut self.state).unwrap() > 0 {
                progressed = true;
            }
            self.conn.flush().unwrap();
            if server.process_and_flush() > 0 {
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
    }

    fn send(
        &mut self,
        server: &mut MiniServer,
        sender: ObjectId,
        opcode: u16,
        args: Vec<Argument<ObjectId, RawFd>>,
    ) {
        server.send(sender, opcode, args);
        self.pump(server);
    }

    /// Emit the layer-shell non-exclusive area for the newest child the
    /// client created (layer_shell.get_output during the output event).
    fn add_layershell_area(
        &mut self,
        server: &mut MiniServer,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        let lso = server
            .children
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("layer-shell output child created");
        self.send(
            server,
            lso,
            EVT_LSO_NON_EXCLUSIVE_AREA,
            vec![
                Argument::Int(x),
                Argument::Int(y),
                Argument::Int(width),
                Argument::Int(height),
            ],
        );
    }

    /// Announce the seat's wl_seat global and its pointer capability, which
    /// makes the client bind wl_seat and create the wl_pointer plus (when the
    /// compositor offers it) the cursor shape device.
    fn add_wl_seat(&mut self, server: &mut MiniServer, seat: ObjectId) {
        self.send(
            server,
            seat,
            EVT_SEAT_WL_SEAT,
            vec![Argument::Uint(GLOBAL_NAME_WL_SEAT)],
        );
        let wl_seat = server
            .bind_log
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|(label, _)| *label == "wl_seat")
            .map(|(_, object)| object.clone())
            .expect("client bound wl_seat");
        self.send(
            server,
            wl_seat,
            EVT_WL_SEAT_CAPABILITIES,
            vec![Argument::Uint(1)], // wl_seat::Capability::Pointer
        );
    }

    fn manage(&mut self, server: &mut MiniServer) {
        self.send(server, server.wm.clone().unwrap(), EVT_MANAGE_START, vec![]);
    }

    fn add_seat(&mut self, server: &mut MiniServer) -> ObjectId {
        let seat = server.create_object(&RIVER_SEAT_V1_INTERFACE);
        self.send(
            server,
            server.wm.clone().unwrap(),
            EVT_SEAT,
            vec![Argument::NewId(seat.clone())],
        );
        seat
    }

    /// Full output add sequence, modeled on river's manageStart: wl_output
    /// global -> river_window_manager_v1.output -> wl_output bind -> name,
    /// position, dimensions, plus the layer-shell non-exclusive area.
    fn add_output(
        &mut self,
        server: &mut MiniServer,
        name: &str,
        pos: (i32, i32),
        dims: (i32, i32),
    ) -> ObjectId {
        let gname = server.add_wl_output_global();
        let out = server.create_object(&RIVER_OUTPUT_V1_INTERFACE);
        server.output_ids.push((name.to_string(), out.clone()));
        self.send(
            server,
            server.wm.clone().unwrap(),
            EVT_OUTPUT,
            vec![Argument::NewId(out.clone())],
        );
        self.add_layershell_area(server, pos.0, pos.1, dims.0, dims.1);
        self.send(
            server,
            out.clone(),
            EVT_OUT_WL_OUTPUT,
            vec![Argument::Uint(gname)],
        );
        let wl_out = server.wl_output_object(gname);
        self.send(
            server,
            wl_out,
            EVT_WL_OUTPUT_NAME,
            vec![Argument::Str(Some(Box::new(CString::new(name).unwrap())))],
        );
        self.send(
            server,
            out.clone(),
            EVT_OUT_POSITION,
            vec![Argument::Int(pos.0), Argument::Int(pos.1)],
        );
        self.send(
            server,
            out.clone(),
            EVT_OUT_DIMENSIONS,
            vec![Argument::Int(dims.0), Argument::Int(dims.1)],
        );
        out
    }

    /// Map a window on the currently focused output/workspace.
    fn add_window(&mut self, server: &mut MiniServer) -> ObjectId {
        let win = server.create_object(&RIVER_WINDOW_V1_INTERFACE);
        self.send(
            server,
            server.wm.clone().unwrap(),
            EVT_WINDOW,
            vec![Argument::NewId(win.clone())],
        );
        self.send(
            server,
            win.clone(),
            EVT_WIN_DIMENSIONS,
            vec![Argument::Int(800), Argument::Int(600)],
        );
        win
    }

    fn check_consistent(&mut self) {
        let wm = &mut self.state.wm;
        if let Some(foi) = wm.focused_output_idx {
            assert!(
                foi < wm.outputs.len(),
                "focused_output_idx {foi} out of bounds ({} outputs)",
                wm.outputs.len()
            );
            let out = &wm.outputs[foi];
            assert!(!out.is_removed, "focused output is removed");
        }
        for (i, out) in wm.outputs.iter().enumerate() {
            if out.is_removed {
                continue;
            }
            for (wi, ws) in out.workspace_list.iter().enumerate() {
                if let Some(f) = ws.focused_window_idx {
                    assert!(f < ws.window_list.len(), "ws {i}/{wi} focus {f} OOB");
                }
            }
        }
    }
}

fn build() -> (Session, MiniServer) {
    let (ca, cb) = UnixStream::pair().unwrap();
    let mut server = MiniServer::new(cb);
    let session = Session::new(&mut server, ca);
    (session, server)
}

// ---------------------------------------------------------------------------
// Scenarios live in submodules so the harness stays navigable.
// ---------------------------------------------------------------------------

mod cursor;
mod manage;
mod outputs;
mod pointer;
mod windows;

/// Client-created objects of a given interface, in creation order.
fn children_with_interface(server: &MiniServer, interface: &str) -> Vec<ObjectId> {
    server
        .children
        .lock()
        .unwrap()
        .iter()
        .filter(|object| object.interface().name == interface)
        .cloned()
        .collect()
}

/// The seat's pointer binding objects in config order (left, right), created
/// by setup_pointer_bindings during the first manage after `add_seat`.
fn pointer_binding_objects(server: &MiniServer) -> Vec<ObjectId> {
    let bindings = children_with_interface(server, "river_pointer_binding_v1");
    assert_eq!(
        bindings.len(),
        2,
        "expected the two default pointer bindings"
    );
    bindings
}
/// Node `set_position` requests seen in the log at the given coordinates.
fn node_positions(server: &MiniServer, x: i32, y: i32) -> usize {
    let want = format!("i{x},i{y}");
    server
        .request_log
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, op, args)| *op == 1 && *args == want)
        .count()
}
