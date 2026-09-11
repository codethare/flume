// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Entry point: connection, initial setup, event loop. The manage cycle
//! lives in `app::manage` (dispatched from river_window_manager_v1 events).

mod actions;
mod app;
mod config;
mod keybinding;
mod layout;
mod output;
mod overview;
mod river;
mod seat;
mod spawn;
mod types;
mod window;
mod wm;

#[cfg(test)]
mod hotplug;

use wayland_client::{Connection, QueueHandle};

use crate::app::AppData;
use crate::wm::WindowManager;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Auto-reap children without breaking waitpid() in spawned programs.
    // SA_NOCLDWAIT alone prevents zombies while preserving waitpid()
    // semantics for children that use fork()+waitpid() internally
    // (wmenu, shells, etc.). Using SIG_IGN would be inherited by children
    // and break their waitpid().
    unsafe {
        let sa: libc::sigaction = libc::sigaction {
            sa_sigaction: libc::SIG_DFL,
            sa_mask: std::mem::zeroed(),
            sa_flags: libc::SA_NOCLDWAIT | libc::SA_RESTART,
            sa_restorer: None,
        };
        libc::sigaction(libc::SIGCHLD, &sa, std::ptr::null_mut());
    }
    // Die when our parent (typically river -c flume) dies, so we don't get
    // reparented to init and outlive the session if river crashes or is
    // killed.
    unsafe {
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0);
    }

    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue = conn.new_event_queue();
    let qh: QueueHandle<AppData> = event_queue.handle();
    let registry = display.get_registry(&qh, ());

    let config = match config::load() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Failed to load config: {e}");
            return Err(e.into());
        }
    };

    let mut state = AppData {
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
        wm: WindowManager::new(config),
        xkb_bindings: Vec::new(),
        pointer_bindings: Vec::new(),
        qh: qh.clone(),
    };

    // Roundtrip to process the registry globals and bind river interfaces.
    event_queue.roundtrip(&mut state)?;
    if state.river_wm.is_none() {
        eprintln!("Failed to find river_window_manager_v1 global");
        return Ok(());
    }
    if state.river_xkb.is_none() {
        eprintln!("Failed to find river_xkb_bindings_v1 global");
        return Ok(());
    }

    // Don't pass WAYLAND_DEBUG on to children; the added noise makes
    // debugging spawned programs impractical (flume itself may be debugged
    // with it).
    for command in state.wm.config.spawn_at_startup.clone() {
        let _ = spawn::spawn_detached(&command);
    }

    loop {
        event_queue.blocking_dispatch(&mut state)?;
        if state.wm.should_exit_loop {
            break;
        }
    }
    Ok(())
}
