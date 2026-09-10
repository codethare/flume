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
        ); 3] = [
            (&RIVER_WINDOW_MANAGER_V1_INTERFACE, 4, "wm"),
            (&RIVER_XKB_BINDINGS_V1_INTERFACE, 1, "xkb"),
            (&RIVER_LAYER_SHELL_V1_INTERFACE, 1, "ls"),
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
            next_global: 3,
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
// Scenarios
// ---------------------------------------------------------------------------

/// eDP-1 and HDMI both active; HDMI is unplugged. Focus is on HDMI.
#[test]
fn unplug_hdmi_while_edp_present() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.manage(&mut sv); // bindings setup consumed one manage_start

    s.add_output(&mut sv, "eDP-1", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv); // focused: eDP-1
    s.manage(&mut sv);

    let hdmi = s.add_output(&mut sv, "HDMI-A-1", (1920, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv); // focused: HDMI-A-1
    s.manage(&mut sv);

    // Unplug HDMI.
    s.send(&mut sv, hdmi, EVT_OUT_REMOVED, vec![]);
    s.manage(&mut sv);
    s.check_consistent();

    let wm = &s.state.wm;
    assert_eq!(wm.outputs.len(), 1, "only eDP-1 should survive");
    let edp = &wm.outputs[0];
    assert_eq!(edp.name.as_deref(), Some("eDP-1"));
    assert_eq!(wm.focused_output_idx, Some(0));
    let edp_ws0 = &edp.workspace_list[0];
    assert_eq!(
        edp_ws0.window_list.len(),
        2,
        "HDMI windows migrate to eDP ws0"
    );
    assert_eq!(
        edp_ws0.window_list[1].geom.former_output_name.as_deref(),
        Some("HDMI-A-1"),
        "migrated window remembers its source output"
    );
    layout::update(&mut s.state.wm);
    s.check_consistent();
}

/// External-only setup: plugging HDMI removes eDP-1 (`detach`), unplugging
/// HDMI leaves zero outputs (`detach`), then eDP-1 reappears and windows are
/// restored. This is the reported crash: "back on eDP-1 after HDMI unplug".
#[test]
fn external_only_hdmi_unplug_returns_to_edp() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.manage(&mut sv);

    s.add_output(&mut sv, "eDP-1", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    // HDMI plugged, eDP-1 disabled (external-only mode).
    let edp = sv.output_id("eDP-1");
    s.send(&mut sv, edp, EVT_OUT_REMOVED, vec![]);
    s.manage(&mut sv);
    s.check_consistent();
    assert_eq!(s.state.wm.outputs.len(), 0, "no outputs while HDMI-only");

    let hdmi = s.add_output(&mut sv, "HDMI-A-1", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);
    assert!(!s.state.wm.detached_outputs.is_empty() || s.state.wm.outputs.len() == 1);

    // Unplug HDMI: zero outputs left, workspaces detached and preserved.
    s.send(&mut sv, hdmi, EVT_OUT_REMOVED, vec![]);
    s.manage(&mut sv);
    s.check_consistent();
    assert_eq!(s.state.wm.outputs.len(), 0);
    assert!(
        !s.state.wm.detached_outputs.is_empty(),
        "windows must survive the HDMI unplug"
    );

    // eDP-1 comes back: the "returning to eDP-1" moment.
    s.add_output(&mut sv, "eDP-1", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.check_consistent();

    let wm = &s.state.wm;
    assert_eq!(wm.outputs.len(), 1, "eDP-1 back");
    assert_eq!(wm.outputs[0].name.as_deref(), Some("eDP-1"));
    assert_eq!(wm.focused_output_idx, Some(0));
    let total: usize = wm.outputs[0]
        .workspace_list
        .iter()
        .map(|ws| ws.window_list.len())
        .sum();
    assert_eq!(total, 2, "both windows restored to eDP-1");
    assert!(
        wm.detached_outputs.is_empty(),
        "restored workspaces are no longer detached"
    );
}

/// Detach (last output removed) then re-add by the same name: the
/// name-keyed restore path in layout::apply.
#[test]
fn detach_then_readd_same_name_restores_windows() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.manage(&mut sv);

    s.add_output(&mut sv, "eDP-1", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    // eDP-1 disabled, then re-enabled.
    let edp = sv.output_id("eDP-1");
    s.send(&mut sv, edp, EVT_OUT_REMOVED, vec![]);
    s.manage(&mut sv);
    s.check_consistent();

    // eDP-1 back as the only output.
    s.add_output(&mut sv, "eDP-1", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.check_consistent();
    let wm = &s.state.wm;
    assert_eq!(wm.outputs.len(), 1);
    let total: usize = wm.outputs[0]
        .workspace_list
        .iter()
        .map(|ws| ws.window_list.len())
        .sum();
    assert!(total >= 1, "window survives the cycle");
    assert_eq!(wm.focused_output_idx, Some(0));
}
/// The reported crash: laptop display (1360x768) OFF while a 2K HDMI
/// (2560x1440) is the only active screen; HDMI is unplugged and the laptop
/// comes back. Every window must land back on the laptop AND fit within its
/// smaller bounds — a floating window centered on the 2K screen must not
/// overhang the 1360x768 screen (the "does not fit" symptom).
#[test]
fn hdmi_2k_unplug_restores_small_laptop_screen() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.manage(&mut sv);

    s.add_output(&mut sv, "eDP-1", (0, 0), (1360, 768));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    // HDMI plugged, laptop screen disabled: tiled window migrates to 2K.
    let hdmi = s.add_output(&mut sv, "HDMI-A-1", (0, 0), (2560, 1440));
    s.manage(&mut sv);
    let edp = sv.output_id("eDP-1");
    s.send(&mut sv, edp, EVT_OUT_REMOVED, vec![]);
    s.manage(&mut sv);
    s.check_consistent();
    assert_eq!(s.state.wm.outputs.len(), 1, "only HDMI while laptop is off");
    assert_eq!(s.state.wm.outputs[0].name.as_deref(), Some("HDMI-A-1"));

    // A second window, toggled floating on the 2K screen (centered rect,
    // mirroring ToggleWorkspaceFloating).
    s.add_window(&mut sv);
    s.manage(&mut sv);
    {
        let wm = &mut s.state.wm;
        let oi = wm.focused_output_idx.unwrap();
        let ws = wm.outputs[oi].focused_workspace_idx;
        let rect = wm.outputs[oi].rectangle;
        let window = wm.outputs[oi].workspace_list[ws]
            .window_list
            .last_mut()
            .unwrap();
        window.geom.is_floating = true;
        window.geom.floating = crate::layout::common::center_rectangle(rect, &wm.config);
    }
    s.manage(&mut sv);
    s.check_consistent();

    // Unplug HDMI: zero outputs, workspaces detached and preserved.
    s.send(&mut sv, hdmi, EVT_OUT_REMOVED, vec![]);
    s.manage(&mut sv);
    assert_eq!(s.state.wm.outputs.len(), 0, "nothing left while laptop off");
    assert!(!s.state.wm.detached_outputs.is_empty(), "windows preserved");

    // Laptop screen comes back: the "back to the laptop panel" moment.
    s.add_output(&mut sv, "eDP-1", (0, 0), (1360, 768));
    s.manage(&mut sv);
    s.check_consistent();

    let wm = &s.state.wm;
    assert_eq!(wm.outputs.len(), 1, "eDP-1 back");
    assert_eq!(wm.outputs[0].name.as_deref(), Some("eDP-1"));
    assert_eq!(wm.focused_output_idx, Some(0));
    let out = wm.outputs[0].rectangle;
    assert_eq!((out.width, out.height), (1360, 768));

    let mut total = 0;
    for ws in &wm.outputs[0].workspace_list {
        for w in &ws.window_list {
            total += 1;
            let rect = w.geom.finish.unwrap_or(w.geom.current);
            assert!(
                rect.x >= out.x
                    && rect.y >= out.y
                    && rect.x + rect.width <= out.x + out.width
                    && rect.y + rect.height <= out.y + out.height,
                "window {total} {rect:?} overhangs the laptop {out:?}"
            );
        }
    }
    assert_eq!(total, 2, "both windows transferred back to the laptop");
    assert!(
        wm.detached_outputs.is_empty(),
        "no detached workspaces left"
    );
}

/// HDMI unplug and laptop re-enable in the SAME batch, before one manage:
/// the race the auto-detect path hits. No panic; windows come back on the
/// laptop.
#[test]
fn unplug_and_readd_in_same_batch_returns_to_laptop() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.manage(&mut sv);

    s.add_output(&mut sv, "eDP-1", (0, 0), (1360, 768));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    // Laptop disabled while HDMI active; work on the 2K screen.
    let hdmi = s.add_output(&mut sv, "HDMI-A-1", (0, 0), (2560, 1440));
    s.manage(&mut sv);
    let edp = sv.output_id("eDP-1");
    s.send(&mut sv, edp, EVT_OUT_REMOVED, vec![]);
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);

    // Both events before the manage: HDMI disappears, laptop returns.
    s.send(&mut sv, hdmi, EVT_OUT_REMOVED, vec![]);
    s.add_output(&mut sv, "eDP-1", (0, 0), (1360, 768));
    s.manage(&mut sv);
    s.check_consistent();

    let wm = &s.state.wm;
    assert_eq!(wm.outputs.len(), 1, "eDP-1 back");
    assert_eq!(wm.outputs[0].name.as_deref(), Some("eDP-1"));
    let total: usize = wm.outputs[0]
        .workspace_list
        .iter()
        .map(|ws| ws.window_list.len())
        .sum();
    assert_eq!(total, 2, "both windows transferred back to the laptop");
    assert!(wm.detached_outputs.is_empty());
}

/// Regression: overview on a multi-display setup with different resolutions.
/// The grid is drawn on the focused output (global coordinates), but
/// place_window measured visibility/clip against each window's HOME output,
/// so every window from another display was hidden or clipped off the grid
/// (the "misplaced / cannot adapt" report). Both windows must land visibly
/// inside the focused output.
#[test]
fn overview_keeps_foreign_display_windows_in_grid() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.manage(&mut sv);

    // Two displays, different resolutions: eDP-1 1360x768 at origin,
    // HDMI-A-1 2560x1440 to its right. Focus lands on HDMI (last added).
    s.add_output(&mut sv, "eDP-1", (0, 0), (1360, 768));
    s.manage(&mut sv);
    s.add_window(&mut sv); // window homed on eDP-1
    s.manage(&mut sv);

    s.add_output(&mut sv, "HDMI-A-1", (1360, 0), (2560, 1440));
    s.manage(&mut sv);
    s.add_window(&mut sv); // window homed on HDMI
    s.manage(&mut sv);
    s.check_consistent();
    assert_eq!(s.state.wm.focused_output_idx, Some(1));

    // Enter overview: grid on the focused output (HDMI), global coords.
    crate::overview::enter(&mut s.state);
    s.state.wm.status = crate::types::Status::Overview;
    s.manage(&mut sv);
    s.check_consistent();

    let wm = &s.state.wm;
    let grid = wm.outputs[1].rectangle;
    for (oi, label) in [(0usize, "eDP-1"), (1, "HDMI-A-1")] {
        let w = &wm.outputs[oi].workspace_list[0].window_list[0];
        let r = w.geom.current;
        assert!(
            r.x >= grid.x
                && r.y >= grid.y
                && r.x + r.width <= grid.x + grid.width
                && r.y + r.height <= grid.y + grid.height,
            "{label} window {r:?} outside the focused grid {grid:?}"
        );
        assert_eq!(
            w.geom.sent_visible,
            Some(true),
            "{label} window was hidden from the overview grid"
        );
    }
}

/// Focus-follows-pointer: hover refocuses exactly like clicking (sloppy
/// focus) when `focus_follows_pointer` is enabled; with the toggle off,
/// hovering must not move focus.
#[test]
fn pointer_enter_focus_follows_toggle() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "eDP-1", (0, 0), (1360, 768));
    s.manage(&mut sv);
    let a = s.add_window(&mut sv);
    s.manage(&mut sv);
    let b = s.add_window(&mut sv);
    s.manage(&mut sv);
    s.check_consistent();

    let ws = &s.state.wm.outputs[0].workspace_list[0];
    let initial = ws.focused_window_idx.expect("window has focus");
    // Server-side id for the event payload, client-side proxy for the check.
    let (hover, hover_proxy) = if initial == 0 {
        (b, ws.window_list[1].river_window.clone())
    } else {
        (a, ws.window_list[0].river_window.clone())
    };

    // Toggle off: hover changes nothing.
    s.state.wm.config.focus_follows_pointer = false;
    s.send(
        &mut sv,
        seat.clone(),
        EVT_SEAT_POINTER_ENTER,
        vec![Argument::Object(hover.clone())],
    );
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(initial),
        "hover must not move focus when focus_follows_pointer is off"
    );

    // Toggle on: hovering the other window refocuses it.
    s.state.wm.config.focus_follows_pointer = true;
    s.send(
        &mut sv,
        seat,
        EVT_SEAT_POINTER_ENTER,
        vec![Argument::Object(hover)],
    );
    s.manage(&mut sv);
    let ws = &s.state.wm.outputs[0].workspace_list[0];
    let now = ws.focused_window_idx.expect("window has focus");
    assert_ne!(now, initial, "hover must move focus when enabled");
    assert_eq!(
        ws.window_list[now].river_window, hover_proxy,
        "focused window must be the hovered one"
    );
}

/// Regression: a fullscreen window fills its output rect exactly, so any
/// window border (and the -border clip offset) would be drawn 3px beyond the
/// output onto a neighboring monitor's adjoining edge ("the edge of A facing
/// B shows the edge of B's fullscreen window"). Fullscreen must send border
/// width 0 and a
/// zero-origin clip box.
#[test]
fn fullscreen_window_sends_no_border_and_zero_origin_clip() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    // A at origin, B to the right (adjoining edge = A's right / B's left).
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_output(&mut sv, "B", (1920, 0), (1920, 1080));
    s.manage(&mut sv);
    s.state.wm.focused_output_idx = Some(0);
    s.manage(&mut sv);
    s.add_window(&mut sv); // window on A
    s.manage(&mut sv);
    s.state.wm.focused_output_idx = Some(1);
    s.manage(&mut sv);
    let b_win = s.add_window(&mut sv); // window to be fullscreened
    s.manage(&mut sv);
    s.state.wm.outputs[1].workspace_list[0].focused_window_idx = Some(0);
    s.manage(&mut sv);

    sv.clear_request_log();
    crate::keybinding::dispatch_action(
        &mut s.state,
        &crate::actions::KeybindingAction::ToggleFullscreen,
    );
    s.manage(&mut sv);

    for (_obj, op, args) in sv.requests_for(&b_win) {
        if op == 8 {
            // set_borders(edges, width, r, g, b, a)
            let width: i32 = args
                .split(',')
                .nth(1)
                .unwrap()
                .trim_start_matches('i')
                .parse()
                .unwrap();
            assert_eq!(
                width, 0,
                "fullscreen window must not set a border, got {args}"
            );
        }
        if op == 21 {
            // set_clip_box(x, y, w, h)
            let v: Vec<i32> = args
                .split(',')
                .map(|a| a.trim_start_matches('i').parse().unwrap())
                .collect();
            assert_eq!(
                (v[0], v[1]),
                (0, 0),
                "fullscreen clip box must be zero-origin, got {args}"
            );
        }
    }
}

/// The seat's pointer binding objects in config order (left, right), created
/// by setup_pointer_bindings during the first manage after `add_seat`.
fn pointer_binding_objects(server: &MiniServer) -> Vec<ObjectId> {
    let children = server.children.lock().unwrap();
    assert!(children.len() >= 2, "pointer bindings not created yet");
    children[children.len() - 2..].to_vec()
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

/// seat.rs: a pointer binding press starts a drag, op_delta follows the
/// pointer (clamped to the output), op_release ends it. This is the
/// `move_window` path behind drag-to-move and side-button bindings.
#[test]
fn pointer_binding_drag_moves_floating_window() {
    let (mut s, mut sv) = build();
    let seat = s.add_seat(&mut sv);
    s.manage(&mut sv);
    // Bindings are only created once a focused output exists (manage returns
    // early before that), so capture them after the first output manage.
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    let binds = pointer_binding_objects(&sv);
    let move_binding = binds[0].clone();

    s.add_window(&mut sv);
    s.manage(&mut sv);
    s.check_consistent();

    // Drag only applies to floating windows (seat.rs press handler).
    crate::keybinding::dispatch_action(
        &mut s.state,
        &crate::actions::KeybindingAction::ToggleWorkspaceFloating,
    );
    s.manage(&mut sv);
    let origin = s.state.wm.outputs[0].workspace_list[0].window_list[0]
        .geom
        .current;
    assert!(
        s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .is_floating,
        "window must be floating for the drag binding to engage"
    );

    s.send(&mut sv, move_binding, EVT_PTR_BINDING_PRESSED, vec![]);
    s.manage(&mut sv);
    assert!(
        matches!(
            s.state.wm.status,
            crate::types::Status::PointerAction(crate::actions::PointerAction::MoveWindow)
        ),
        "press must enter the move drag state, got {:?}",
        s.state.wm.status
    );

    // Compose the op delta: the window follows the pointer, clamped to the
    // output and pushed to the compositor.
    sv.clear_request_log();
    s.send(
        &mut sv,
        seat.clone(),
        EVT_SEAT_OP_DELTA,
        vec![Argument::Int(100), Argument::Int(50)],
    );
    s.manage(&mut sv);
    let moved = s.state.wm.outputs[0].workspace_list[0].window_list[0]
        .geom
        .current;
    assert_eq!(
        (moved.x, moved.y),
        (origin.x + 100, origin.y + 50),
        "window must follow the pointer delta"
    );
    assert!(
        node_positions(&sv, moved.x + 3, moved.y + 3) > 0,
        "the dragged position must reach the compositor (border inset)"
    );

    s.send(&mut sv, seat, EVT_SEAT_OP_RELEASE, vec![]);
    assert_eq!(
        s.state.wm.status,
        crate::types::Status::None,
        "release ends the drag"
    );
    assert!(
        s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .drag_origin
            .is_none(),
        "drag origin must be cleared on release"
    );
    s.manage(&mut sv);
}

/// window.rs: a client-side fullscreen request fills the window's output and
/// the inverse request restores the tiled layout.
#[test]
fn window_fullscreen_requests_toggle_and_fill_output() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    let out = s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    let win = s.add_window(&mut sv);
    s.manage(&mut sv);

    s.send(
        &mut sv,
        win.clone(),
        EVT_WIN_FULLSCREEN_REQUESTED,
        vec![Argument::Object(out)],
    );
    s.manage(&mut sv);
    let output_rect = s.state.wm.outputs[0].rectangle;
    let geom = s.state.wm.outputs[0].workspace_list[0].window_list[0]
        .geom
        .current;
    assert!(
        s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .is_fullscreen,
        "fullscreen_requested must set the flag"
    );
    assert!(
        geom.eql(output_rect),
        "fullscreen window must fill its output: {geom:?} vs {output_rect:?}"
    );

    s.send(&mut sv, win, EVT_WIN_EXIT_FULLSCREEN_REQUESTED, vec![]);
    s.manage(&mut sv);
    assert!(
        !s.state.wm.outputs[0].workspace_list[0].window_list[0]
            .geom
            .is_fullscreen,
        "exit_fullscreen_requested must clear the flag"
    );
}

/// window.rs: closing windows keeps the workspace focus index valid — the
/// off-by-one class that used to panic in window moves.
#[test]
fn closing_windows_keeps_focus_index_valid() {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.add_window(&mut sv);
    s.manage(&mut sv);
    let w1 = s.add_window(&mut sv);
    s.manage(&mut sv);
    let w2 = s.add_window(&mut sv);
    s.manage(&mut sv);
    s.check_consistent();

    // Focus the middle window, close it: focus shifts to the one before it.
    s.state.wm.outputs[0].workspace_list[0].focused_window_idx = Some(1);
    s.manage(&mut sv);
    s.send(&mut sv, w1, EVT_WIN_CLOSED, vec![]);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(0),
        "closing the focused middle window moves focus down"
    );
    assert_eq!(s.state.wm.outputs[0].workspace_list[0].window_list.len(), 2);
    s.check_consistent();

    // Closing a window after the focused one leaves focus alone.
    s.send(&mut sv, w2, EVT_WIN_CLOSED, vec![]);
    s.manage(&mut sv);
    assert_eq!(
        s.state.wm.outputs[0].workspace_list[0].focused_window_idx,
        Some(0),
        "closing a later window must not move focus"
    );
    s.check_consistent();
}
