// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

pub extern crate wayland_client;
pub use wayland_client::protocol::*;

mod interfaces {
    pub(super) mod tailrace {
        pub use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("./protocol/river-window-management-v1.xml");
    }

    pub(super) mod rxkb {
        use super::tailrace::*;
        wayland_scanner::generate_interfaces!("./protocol/river-xkb-bindings-v1.xml");
    }

    pub(super) mod rls {
        use super::tailrace::*;
        wayland_scanner::generate_interfaces!("./protocol/river-layer-shell-v1.xml");
    }
}

use self::interfaces::rls::*;
use self::interfaces::rxkb::*;
use self::interfaces::tailrace::*;
wayland_scanner::generate_client_code!("./protocol/river-window-management-v1.xml");
wayland_scanner::generate_client_code!("./protocol/river-xkb-bindings-v1.xml");
wayland_scanner::generate_client_code!("./protocol/river-layer-shell-v1.xml");
