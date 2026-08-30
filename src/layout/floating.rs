// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Floating workspace layout, ported from rill-ed layout/floating.zig.

use crate::layout::common;
use crate::types::{Rectangle, WindowGeom};

pub fn apply(windows: &mut [WindowGeom], output_rect: Rectangle, y_offset: i32) {
    for window in windows.iter_mut() {
        let mut finish = if window.is_fullscreen {
            output_rect
        } else {
            window.floating
        };
        finish.y += y_offset;
        window.finish = Some(finish);
    }

    for window in windows.iter_mut() {
        common::skip_if_at_rest(window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: Rectangle = Rectangle {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    fn floating_geom(floating: Rectangle, current: Rectangle) -> WindowGeom {
        let mut g = WindowGeom::new(0.5, current);
        g.is_floating = true;
        g.floating = floating;
        g
    }

    #[test]
    fn new_floating_window_snaps_to_centered_rect() {
        // Mirrors window.add(): current starts at initialRectangle (right
        // edge), floating is the centered target rect.
        let mut windows = vec![floating_geom(
            Rectangle {
                x: 460,
                y: 90,
                width: 1000,
                height: 900,
            },
            Rectangle {
                x: 920,
                y: 90,
                width: 1000,
                height: 900,
            },
        )];

        apply(&mut windows, OUTPUT, 0);

        let finish = windows[0].finish.expect("finish computed");
        assert!(finish.eql(windows[0].floating));
    }

    #[test]
    fn fullscreen_floating_window_fills_output() {
        let mut windows = vec![floating_geom(
            Rectangle {
                x: 460,
                y: 90,
                width: 1000,
                height: 900,
            },
            Rectangle {
                x: 460,
                y: 90,
                width: 1000,
                height: 900,
            },
        )];
        windows[0].is_fullscreen = true;

        apply(&mut windows, OUTPUT, 0);
        assert!(windows[0].finish.unwrap().eql(OUTPUT));
    }
}
