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

const USAGE: &str = "\
tailrace - tiny scrolling window manager for river

usage: tailrace [-c <path>]

options:
  -c, --config <path>  read this file instead of searching
                       $XDG_CONFIG_HOME/tailrace/config.toml and ~/.config/tailrace/config.toml
  -h, --help           print this help and exit
  -V, --version        print the version and exit
";

struct Args {
    config: Option<std::path::PathBuf>,
}

/// Prints help/version and returns None when the process should exit early.
fn parse_args() -> Result<Option<Args>, String> {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I: Iterator<Item = String>>(mut args: I) -> Result<Option<Args>, String> {
    let mut config = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" => {
                let Some(path) = args.next() else {
                    return Err(format!("{arg} needs a path"));
                };
                config = Some(std::path::PathBuf::from(path));
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("tailrace {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(Some(Args { config }))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => return Ok(()),
        Err(message) => {
            eprintln!("tailrace: {message}\n{USAGE}");
            std::process::exit(2);
        }
    };
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
    // Die when our parent (typically river -c tailrace) dies, so we don't get
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

    let config = config::load(args.config.as_deref())?;

    let mut state = AppData {
        registry,
        river_wm: None,
        river_xkb: None,
        river_layer_shell: None,
        river_seat: None,
        layer_shell_seat: None,
        config_path: args.config,
        warned_missing_seat: false,
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
    // Required: without them tailrace cannot manage windows or bind keys. Exit
    // non-zero so a failed session start is visible to whoever launched it.
    if state.river_wm.is_none() {
        return Err("river_window_manager_v1 global not found".into());
    }
    if state.river_xkb.is_none() {
        return Err("river_xkb_bindings_v1 global not found".into());
    }
    if state.river_layer_shell.is_none() {
        // Optional (rill-ed behaves the same): without it layer-shell focus is
        // not tracked, so exclusive keyboard focus from a bar is not honored.
        eprintln!("tailrace: river_layer_shell_v1 missing, layer-shell focus tracking disabled");
    }

    // Don't pass WAYLAND_DEBUG on to children; the added noise makes
    // debugging spawned programs impractical (tailrace itself may be debugged
    // with it).
    for command in state.wm.config.spawn_at_startup.clone() {
        if let Err(e) = spawn::spawn_detached(&command) {
            eprintln!("tailrace: cannot run spawn_at_startup {command:?}: {e}");
        }
    }

    loop {
        event_queue.blocking_dispatch(&mut state)?;
        if state.wm.should_exit_loop {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Option<Args>, String> {
        parse_args_from(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn config_path_is_parsed() {
        let parsed = args(&["-c", "/tmp/x.toml"]).unwrap().unwrap();
        assert_eq!(parsed.config, Some(std::path::PathBuf::from("/tmp/x.toml")));
        let parsed = args(&["--config", "/tmp/y.toml"]).unwrap().unwrap();
        assert_eq!(parsed.config, Some(std::path::PathBuf::from("/tmp/y.toml")));
        assert!(args(&[]).unwrap().unwrap().config.is_none());
    }

    #[test]
    fn help_and_version_exit_early() {
        assert!(args(&["--help"]).unwrap().is_none());
        assert!(args(&["-V"]).unwrap().is_none());
    }

    #[test]
    fn bad_arguments_are_rejected() {
        assert!(args(&["--config"]).is_err(), "missing path");
        assert!(args(&["--nope"]).is_err(), "unknown flag");
    }
}
