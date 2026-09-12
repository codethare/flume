// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Client side of the harness: drives the real AppData/dispatch code over a
//! socketpair against the mini server.

use super::*;

pub(super) struct Session {
    pub(super) queue: EventQueue<AppData>,
    pub(super) conn: Connection,
    pub(super) state: AppData,
}

impl Session {
    pub(super) fn new(server: &mut MiniServer, client_stream: UnixStream) -> Self {
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
            config_path: None,
            warned_missing_seat: false,
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
    pub(super) fn pump(&mut self, server: &mut MiniServer) {
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

    pub(super) fn send(
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
    pub(super) fn add_layershell_area(
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
    pub(super) fn add_wl_seat(&mut self, server: &mut MiniServer, seat: ObjectId) {
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

    /// Test-only config override: `Rc::make_mut` copies only while shared.
    pub(super) fn config_mut(&mut self) -> &mut crate::types::Config {
        std::rc::Rc::make_mut(&mut self.state.wm.config)
    }

    pub(super) fn manage(&mut self, server: &mut MiniServer) {
        self.send(server, server.wm.clone().unwrap(), EVT_MANAGE_START, vec![]);
    }

    pub(super) fn add_seat(&mut self, server: &mut MiniServer) -> ObjectId {
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
    pub(super) fn add_output(
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

    /// Create a window object without announcing dimensions: it stays
    /// pending until `map_window`, which is where river sends app_id/title
    /// (before dimensions), and where flume evaluates window rules.
    pub(super) fn add_pending_window(&mut self, server: &mut MiniServer) -> ObjectId {
        let win = server.create_object(&RIVER_WINDOW_V1_INTERFACE);
        self.send(
            server,
            server.wm.clone().unwrap(),
            EVT_WINDOW,
            vec![Argument::NewId(win.clone())],
        );
        win
    }

    /// Announce dimensions, which maps the window (window.rs adds it on this
    /// event, not during a manage sequence).
    pub(super) fn map_window(&mut self, server: &mut MiniServer, win: &ObjectId) {
        self.send(
            server,
            win.clone(),
            EVT_WIN_DIMENSIONS,
            vec![Argument::Int(800), Argument::Int(600)],
        );
    }

    /// Map a window on the currently focused output/workspace.
    pub(super) fn add_window(&mut self, server: &mut MiniServer) -> ObjectId {
        let win = self.add_pending_window(server);
        self.map_window(server, &win);
        win
    }

    pub(super) fn check_consistent(&mut self) {
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

pub(super) fn build() -> (Session, MiniServer) {
    let (ca, cb) = UnixStream::pair().unwrap();
    let mut server = MiniServer::new(cb);
    let session = Session::new(&mut server, ca);
    (session, server)
}

// ---------------------------------------------------------------------------
// Scenarios live in submodules so the harness stays navigable.
// ---------------------------------------------------------------------------
