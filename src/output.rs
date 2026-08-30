// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Output management, ported from rill-ed output.zig.
//!
//! All window-management state modifications (migration, detachment,
//! destroy) are deferred to `layout::apply`, which runs inside the manage
//! sequence — protocol requirement of river-window-management-v1.

use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::{Dispatch, Proxy, QueueHandle};

use crate::app::AppData;
use crate::layout;
use crate::river::river_layer_shell_output_v1::RiverLayerShellOutputV1;
use crate::river::river_output_v1::{self, RiverOutputV1};
use crate::types::{Output, Rectangle, Status, Workspace};
use crate::wm::WindowManager;

/// A new river output appeared (river_window_manager_v1.output event).
pub fn output_added(state: &mut AppData, river_output: RiverOutputV1, qh: &QueueHandle<AppData>) {
    add(
        &mut state.wm,
        state.river_layer_shell.as_ref(),
        river_output,
        qh,
    );
}

/// Create and register the output record. A newly-connected output becomes
/// focused automatically; the pointer is warped to it on the next layout
/// pass (niri/hyprland behavior).
pub fn add(
    wm: &mut WindowManager,
    layer_shell: Option<&crate::river::river_layer_shell_v1::RiverLayerShellV1>,
    river_output: RiverOutputV1,
    qh: &QueueHandle<AppData>,
) {
    let river_layer_shell_output =
        layer_shell.map(|layer_shell| layer_shell.get_output(&river_output, qh, ()));

    let had_previous_output = !wm.outputs.is_empty();
    wm.outputs.push(Output {
        river_output,
        river_layer_shell_output,
        wl_output: None,
        name: None,
        workspace_list: std::array::from_fn(|_| Workspace::new()),
        focused_workspace_idx: 0,
        rectangle: Rectangle::default(),
        non_exclusive: Rectangle::default(),
        is_removed: false,
    });
    let new_idx = wm.outputs.len() - 1;
    wm.focused_output_idx = Some(new_idx);
    if had_previous_output {
        wm.needs_pointer_warp = true;
    }

    // Workspace/window restoration for reappearing outputs is handled by
    // layout::apply inside the manage sequence, keyed by output name.
}

pub fn output_event(
    wm: &mut WindowManager,
    river_output: &RiverOutputV1,
    event: river_output_v1::Event,
    registry: &wayland_client::protocol::wl_registry::WlRegistry,
    qh: &QueueHandle<AppData>,
) -> bool {
    let Some(output_idx) = wm
        .outputs
        .iter()
        .position(|o| &o.river_output == river_output)
    else {
        return false;
    };

    match event {
        river_output_v1::Event::Dimensions { width, height } => {
            let output = &mut wm.outputs[output_idx];
            output.rectangle.width = width;
            output.rectangle.height = height;
            layout::update(wm);
            wm.status = Status::Layout;
            false
        }
        river_output_v1::Event::Position { x, y } => {
            let output = &mut wm.outputs[output_idx];
            output.rectangle.x = x;
            output.rectangle.y = y;
            false
        }
        river_output_v1::Event::WlOutput { name } => {
            wm.outputs[output_idx].wl_output =
                Some(registry.bind::<WlOutput, _, _>(name, 4, qh, ()));
            false
        }
        river_output_v1::Event::Removed => {
            let output = &mut wm.outputs[output_idx];
            output.is_removed = true;
            wm.status = Status::Layout;

            // Adjust focus if the removed output was focused and surviving
            // outputs exist. When it was the last output, leave
            // focused_output_idx alone so manage() doesn't return early —
            // layout::apply nullifies it during cleanup inside the manage
            // sequence.
            if wm.focused_output_idx == Some(output_idx)
                && let Some(active) = wm.outputs.iter().position(|o| !o.is_removed)
            {
                wm.focused_output_idx = Some(active);
                wm.needs_pointer_warp = true;
            }

            // Clear previous_workspace if it pointed at the removed output.
            if wm.previous_workspace.map(|pw| pw.output_idx) == Some(output_idx) {
                wm.previous_workspace = None;
            }

            true // manage_dirty: migrate/detach inside the manage sequence
        }
    }
}

pub fn layer_shell_output_event(
    wm: &mut WindowManager,
    layer_shell_output: &RiverLayerShellOutputV1,
    event: <RiverLayerShellOutputV1 as Proxy>::Event,
) {
    let crate::river::river_layer_shell_output_v1::Event::NonExclusiveArea {
        x,
        y,
        width,
        height,
    } = event;
    let Some(output) = wm
        .outputs
        .iter_mut()
        .find(|o| o.river_layer_shell_output.as_ref() == Some(layer_shell_output))
    else {
        return;
    };
    output.non_exclusive = Rectangle {
        x,
        y,
        width,
        height,
    };
    layout::update(wm);
    wm.status = Status::Layout;
}

pub fn wl_output_event(
    wm: &mut WindowManager,
    wl_output: &WlOutput,
    event: wl_output::Event,
) -> bool {
    let wl_output::Event::Name { name } = event else {
        return false;
    };
    let Some(output) = wm
        .outputs
        .iter_mut()
        .find(|o| o.wl_output.as_ref() == Some(wl_output))
    else {
        return false;
    };
    output.name = Some(name);
    // Ensure the next manage sequence runs so layout::apply can restore any
    // workspaces or windows keyed to this output name.
    wm.status = Status::Layout;
    true
}

impl Dispatch<RiverOutputV1, ()> for AppData {
    fn event(
        state: &mut Self,
        proxy: &RiverOutputV1,
        event: <RiverOutputV1 as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        qh: &QueueHandle<Self>,
    ) {
        let needs_manage = output_event(&mut state.wm, proxy, event, &state.registry, qh);
        if needs_manage {
            state.manage_dirty();
        }
    }
}

impl Dispatch<RiverLayerShellOutputV1, ()> for AppData {
    fn event(
        state: &mut Self,
        proxy: &RiverLayerShellOutputV1,
        event: <RiverLayerShellOutputV1 as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        layer_shell_output_event(&mut state.wm, proxy, event);
    }
}

impl Dispatch<WlOutput, ()> for AppData {
    fn event(
        state: &mut Self,
        proxy: &WlOutput,
        event: <WlOutput as Proxy>::Event,
        _data: &(),
        _conn: &wayland_client::Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let needs_manage = wl_output_event(&mut state.wm, proxy, event);
        if needs_manage {
            state.manage_dirty();
        }
    }
}
