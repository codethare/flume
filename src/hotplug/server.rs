// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Mini Wayland server: advertises the river globals, answers requests and
//! records what the client asked for.

use super::*;

/// One client request observed by the mini-server: (object, opcode, args).
pub(super) type RequestLogEntry = (ObjectId, u16, String);

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
pub(super) struct BindLog(pub(super) Arc<Mutex<Vec<(&'static str, ObjectId)>>>);

struct GenericGlobal {
    log: BindLog,
    label: &'static str,
    children: Arc<Mutex<Vec<ObjectId>>>,
    requests: Arc<Mutex<Vec<RequestLogEntry>>>,
}

impl GenericGlobal {
    pub(super) fn labeled(
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

pub(super) struct MiniServer {
    pub(super) backend: ServerBackend<()>,
    pub(super) bind_log: BindLog,
    pub(super) next_global: u32,
    pub(super) client: ClientId,
    pub(super) wm: Option<ObjectId>,
    pub(super) xkb: Option<ObjectId>,
    pub(super) layershell: Option<ObjectId>,
    /// river_output_v1 object id per output name, for removal events.
    pub(super) output_ids: Vec<(String, ObjectId)>,
    /// Every `new_id` child the client requested (layer-shell outputs, nodes).
    pub(super) children: Arc<Mutex<Vec<ObjectId>>>,
    /// Every request the client made, keyed by target object.
    pub(super) request_log: Arc<Mutex<Vec<RequestLogEntry>>>,
}

impl MiniServer {
    pub(super) fn new(stream: UnixStream) -> Self {
        let backend = ServerBackend::<()>::new().unwrap();
        let bind_log = BindLog::default();
        let children = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        // Globals advertised to every registry the client creates, in bind order.
        // The wm and xkb versions must match the VERSION constants in app.rs:
        // if flume's floor moves past them, the client exits and the tests fail.
        let globals: [(
            &'static wayland_backend::protocol::Interface,
            u32,
            &'static str,
        ); 5] = [
            (&RIVER_WINDOW_MANAGER_V1_INTERFACE, 5, "wm"),
            (&RIVER_XKB_BINDINGS_V1_INTERFACE, 3, "xkb"),
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
    pub(super) fn requests_for(&self, object: &ObjectId) -> Vec<RequestLogEntry> {
        self.request_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(o, _, _)| o == object)
            .cloned()
            .collect()
    }

    pub(super) fn clear_request_log(&self) {
        self.request_log.lock().unwrap().clear();
    }

    pub(super) fn output_id(&self, name: &str) -> ObjectId {
        self.output_ids
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, o)| o.clone())
            .unwrap_or_else(|| panic!("no river_output_v1 for {name:?}"))
    }

    pub(super) fn core_ids(&mut self) {
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

    pub(super) fn add_wl_output_global(&mut self) -> u32 {
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

    pub(super) fn create_object(
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

    pub(super) fn send(
        &mut self,
        sender: ObjectId,
        opcode: u16,
        args: Vec<Argument<ObjectId, RawFd>>,
    ) {
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

    pub(super) fn process_and_flush(&mut self) -> usize {
        let n = self.backend.dispatch_all_clients(&mut ()).unwrap();
        self.backend.flush(None).unwrap();
        n
    }

    pub(super) fn wl_output_object(&self, gname: u32) -> ObjectId {
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

/// Requests with the given opcode seen on `object`.
pub(super) fn count_requests(server: &MiniServer, object: &ObjectId, opcode: u16) -> usize {
    server
        .requests_for(object)
        .into_iter()
        .filter(|(_, op, _)| *op == opcode)
        .count()
}

/// Client-created objects of a given interface, in creation order.
pub(super) fn children_with_interface(server: &MiniServer, interface: &str) -> Vec<ObjectId> {
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
pub(super) fn pointer_binding_objects(server: &MiniServer) -> Vec<ObjectId> {
    let bindings = children_with_interface(server, "river_pointer_binding_v1");
    assert_eq!(
        bindings.len(),
        2,
        "expected the two default pointer bindings"
    );
    bindings
}
/// Node `set_position` requests seen in the log at the given coordinates.
pub(super) fn node_positions(server: &MiniServer, x: i32, y: i32) -> usize {
    let want = format!("i{x},i{y}");
    server
        .request_log
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, op, args)| *op == 1 && *args == want)
        .count()
}
