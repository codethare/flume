// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Shared rectangle/geometry helpers, ported from rill-ed layout/common.zig.

use crate::river::river_node_v1::RiverNodeV1;
use crate::river::river_window_v1::RiverWindowV1;
use crate::types::{Config, Rectangle, WindowGeom};

/// All four edges (tiled on all sides).
pub fn edges_all() -> crate::river::river_window_v1::Edges {
    use crate::river::river_window_v1::Edges;
    Edges::Top
        .union(Edges::Bottom)
        .union(Edges::Left)
        .union(Edges::Right)
}

/// Initial (off-screen right) rectangle for a newly added tiled window.
pub fn initial_rectangle(non_exclusive: Rectangle, config: &Config) -> Rectangle {
    let base_width = (non_exclusive.width - config.horizontal_gap) as f32;
    let width_with_gap = (base_width * config.default_window_width) as i32;
    Rectangle {
        width: width_with_gap - config.horizontal_gap,
        height: non_exclusive.height - 2 * config.vertical_gap,
        x: non_exclusive.x + non_exclusive.width - width_with_gap,
        y: non_exclusive.y + config.vertical_gap,
    }
}

/// Centered floating rectangle; 16:9 height derives from the width proportion.
pub fn center_rectangle(non_exclusive: Rectangle, config: &Config) -> Rectangle {
    let base_width = (non_exclusive.width - config.horizontal_gap) as f32;
    let width_with_gap = (base_width * config.default_window_width) as i32;
    let w = width_with_gap - config.horizontal_gap;
    let h = w * 9 / 16;
    Rectangle {
        width: w,
        height: h,
        x: non_exclusive.x + (non_exclusive.width - w) / 2,
        y: non_exclusive.y + (non_exclusive.height - h) / 2,
    }
}

/// Keyboard move step for floating windows, logical pixels (matches niri's
/// DIRECTIONAL_MOVE_PX; niri also hardcodes it rather than exposing config).
pub const FLOATING_MOVE_STEP: i32 = 50;

/// Clear `finish` when the window is already at rest at it.
pub fn skip_if_at_rest(window: &mut WindowGeom) {
    if let Some(finish) = window.finish
        && finish.eql(window.current)
        && window
            .sent_current
            .is_some_and(|sent| sent.eql(window.current))
    {
        window.finish = None;
    }
}

fn clamp_i32(v: i32, min: i32, max: i32) -> i32 {
    v.clamp(min, max)
}

/// Translate a floating window by (dx, dy), clamped so it stays fully on the output.
pub fn move_floating(geom: &mut WindowGeom, output_rect: Rectangle, dx: i32, dy: i32) {
    let left = output_rect.x;
    let right = output_rect.x + output_rect.width;
    let top = output_rect.y;
    let bottom = output_rect.y + output_rect.height;
    let width = geom.floating.width;
    let height = geom.floating.height;
    geom.floating.x = clamp_i32(geom.floating.x + dx, left, right - width);
    geom.floating.y = clamp_i32(geom.floating.y + dy, top, bottom - height);
}

/// Resize a floating window by (dw, dh), top-left anchored, clamped to output and min_size.
pub fn resize_floating(
    geom: &mut WindowGeom,
    output_rect: Rectangle,
    dw: i32,
    dh: i32,
    min_size: i32,
) {
    let right = output_rect.x + output_rect.width;
    let bottom = output_rect.y + output_rect.height;
    geom.floating.width = clamp_i32(geom.floating.width + dw, min_size, right - geom.floating.x);
    geom.floating.height = clamp_i32(
        geom.floating.height + dh,
        min_size,
        bottom - geom.floating.y,
    );
}

/// Expand/shrink a floating window on all four sides, keeping its center fixed,
/// clamped to the output and min_size.
pub fn scale_floating(
    geom: &mut WindowGeom,
    output_rect: Rectangle,
    dw: i32,
    dh: i32,
    min_size: i32,
) {
    let left = output_rect.x;
    let right = output_rect.x + output_rect.width;
    let top = output_rect.y;
    let bottom = output_rect.y + output_rect.height;

    let new_width = clamp_i32(geom.floating.width + dw, min_size, right - left);
    let new_height = clamp_i32(geom.floating.height + dh, min_size, bottom - top);

    let cx = geom.floating.x + geom.floating.width / 2;
    let cy = geom.floating.y + geom.floating.height / 2;

    geom.floating.width = new_width;
    geom.floating.height = new_height;
    geom.floating.x = clamp_i32(cx - new_width / 2, left, right - new_width);
    geom.floating.y = clamp_i32(cy - new_height / 2, top, bottom - new_height);
}

/// Clamp a floating window's rest rect fully inside the output, shrinking
/// it if the output is smaller than the window. Used when a hotplug replaces
/// the output with a smaller one (HDMI unplug): the old larger-screen rect
/// would otherwise overhang the new screen with no way to reach it.
pub fn clamp_floating_into_output(geom: &mut WindowGeom, output_rect: Rectangle) {
    let left = output_rect.x;
    let top = output_rect.y;
    let right = output_rect.x + output_rect.width;
    let bottom = output_rect.y + output_rect.height;
    geom.floating.width = geom.floating.width.min(right - left);
    geom.floating.height = geom.floating.height.min(bottom - top);
    geom.floating.x = clamp_i32(geom.floating.x, left, right - geom.floating.width);
    geom.floating.y = clamp_i32(geom.floating.y, top, bottom - geom.floating.height);
}

pub fn compute_clip_box(
    window_rect: Rectangle,
    output_rect: Rectangle,
    border_width: u8,
) -> Rectangle {
    let window_left = window_rect.x;
    let window_right = window_rect.x + window_rect.width;
    let window_top = window_rect.y;
    let window_bottom = window_rect.y + window_rect.height;

    let output_left = output_rect.x;
    let output_right = output_rect.x + output_rect.width;
    let output_top = output_rect.y;
    let output_bottom = output_rect.y + output_rect.height;

    let mut clip_width = window_rect.width;
    let mut clip_height = window_rect.height;
    let mut clip_x = 0;
    let mut clip_y = 0;

    if output_left < window_right && output_left > window_left {
        clip_x = output_left - window_left;
        clip_width = (window_right - output_left).min(output_rect.width);
    } else if output_right > window_left && output_right < window_right {
        clip_width = output_right - window_left;
    }

    if output_top < window_bottom && output_top > window_top {
        clip_y = output_top - window_top;
        clip_height = (window_bottom - output_top).min(output_rect.height);
    } else if output_bottom > window_top && output_bottom < window_bottom {
        clip_height = output_bottom - window_top;
    }

    Rectangle {
        x: clip_x - border_width as i32,
        y: clip_y - border_width as i32,
        width: clip_width,
        height: clip_height,
    }
}

/// Send geometry/show/clip requests for a window, skipping redundant ones.
/// Ported from rill-ed `placeWindow` (protocol IO; not unit-tested).
pub fn place_window(
    river_window: &RiverWindowV1,
    river_node: &RiverNodeV1,
    geom: &mut WindowGeom,
    output_rect: Rectangle,
    config: &Config,
) {
    let border_width = if geom.is_fullscreen {
        0
    } else {
        config.border.width as i32
    };

    if geom.sent_current.is_none_or(|sent| !sent.eql(geom.current)) {
        river_window.propose_dimensions(
            (geom.current.width - 2 * border_width).max(0),
            (geom.current.height - 2 * border_width).max(0),
        );
        river_node.set_position(geom.current.x + border_width, geom.current.y + border_width);
        geom.sent_current = Some(geom.current);
    }

    let window_left = geom.current.x;
    let window_right = geom.current.x + geom.current.width;
    let window_top = geom.current.y;
    let window_bottom = geom.current.y + geom.current.height;

    let output_left = output_rect.x;
    let output_right = output_rect.x + output_rect.width;
    let output_top = output_rect.y;
    let output_bottom = output_rect.y + output_rect.height;

    let visible = !(output_left >= window_right
        || output_right <= window_left
        || output_top >= window_bottom
        || output_bottom <= window_top);
    if geom.sent_visible != Some(visible) {
        if visible {
            river_window.show();
        } else {
            river_window.hide();
        }
        geom.sent_visible = Some(visible);
    }

    let clip = compute_clip_box(geom.current, output_rect, border_width as u8);
    if geom.sent_clip.is_none_or(|sent| !sent.eql(clip)) {
        river_window.set_clip_box(clip.x, clip.y, clip.width, clip.height);
        geom.sent_clip = Some(clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CenterFocused;

    fn config(default_window_width: f32, center: CenterFocused) -> Config {
        Config {
            default_window_width,
            center_focused_window: center,
            ..Config::default()
        }
    }

    fn geom(floating: Rectangle) -> WindowGeom {
        WindowGeom::new(0.5, floating)
    }

    const OUTPUT: Rectangle = Rectangle {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn floating_move_and_resize_clamp_to_output_bounds() {
        let mut g = geom(Rectangle {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        });

        move_floating(&mut g, OUTPUT, -FLOATING_MOVE_STEP, FLOATING_MOVE_STEP);
        assert_eq!(g.floating.x, 50);
        assert_eq!(g.floating.y, 150);

        // Clamp at left/top edge.
        move_floating(&mut g, OUTPUT, -1000, -1000);
        assert_eq!(g.floating.x, 0);
        assert_eq!(g.floating.y, 0);

        // Clamp at right/bottom edge.
        move_floating(&mut g, OUTPUT, 100000, 100000);
        assert_eq!(g.floating.x, 1920 - 800);
        assert_eq!(g.floating.y, 1080 - 600);

        // Grow past output edge: clamp to remaining space.
        resize_floating(&mut g, OUTPUT, 100000, 100000, 6);
        assert_eq!(g.floating.width, 1920 - g.floating.x);
        assert_eq!(g.floating.height, 1080 - g.floating.y);

        // Shrink past min_size: clamp to min_size.
        resize_floating(&mut g, OUTPUT, -100000, -100000, 6);
        assert_eq!(g.floating.width, 6);
        assert_eq!(g.floating.height, 6);
    }

    #[test]
    fn floating_scale_expands_and_shrinks_on_all_four_sides() {
        let mut g = geom(Rectangle {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        });

        // Expand: center stays fixed, all four edges move outward.
        scale_floating(&mut g, OUTPUT, 200, 100, 6);
        assert_eq!(g.floating.width, 1000);
        assert_eq!(g.floating.height, 700);
        assert_eq!(g.floating.x, 0);
        assert_eq!(g.floating.y, 50);

        // Shrink: center stays fixed, all four edges move inward.
        g.floating = Rectangle {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        };
        scale_floating(&mut g, OUTPUT, -200, -100, 6);
        assert_eq!(g.floating.width, 600);
        assert_eq!(g.floating.height, 500);
        assert_eq!(g.floating.x, 200);
        assert_eq!(g.floating.y, 150);

        // Grow past output: clamp to output size.
        g.floating = Rectangle {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        };
        scale_floating(&mut g, OUTPUT, 100000, 100000, 6);
        assert_eq!(g.floating.width, 1920);
        assert_eq!(g.floating.height, 1080);
        assert_eq!(g.floating.x, 0);
        assert_eq!(g.floating.y, 0);

        // Shrink past min_size: clamp to min_size.
        g.floating = Rectangle {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        };
        scale_floating(&mut g, OUTPUT, -100000, -100000, 6);
        assert_eq!(g.floating.width, 6);
        assert_eq!(g.floating.height, 6);
        assert_eq!(g.floating.x, 497);
        assert_eq!(g.floating.y, 397);
    }

    #[test]
    fn center_rectangle_is_centered_and_16_9() {
        let rect = center_rectangle(OUTPUT, &config(0.5, CenterFocused::Never));

        // 16:9 within integer truncation.
        assert!((rect.width * 9 - rect.height * 16).abs() <= 15);
        // Centered and inside the output.
        assert_eq!(rect.x, (1920 - rect.width) / 2);
        assert_eq!(rect.y, (1080 - rect.height) / 2);
        assert!(rect.x >= 0 && rect.y >= 0);
        assert!(rect.x + rect.width <= 1920 && rect.y + rect.height <= 1080);
    }

    #[test]
    fn compute_clip_box_clips_window_hanging_off_bottom_edge() {
        // Window top is inside output, bottom extends below output bottom.
        let window = Rectangle {
            x: 100,
            y: 800,
            width: 800,
            height: 400,
        };
        let clip = compute_clip_box(window, OUTPUT, 3);

        // Visible height is from window top to output bottom.
        assert_eq!(clip.height, 280);
        assert_eq!(clip.width, 800);
        assert_eq!(clip.x, -3);
        assert_eq!(clip.y, -3);
    }

    #[test]
    fn compute_clip_box_clips_window_hanging_off_right_edge() {
        // Window left is inside output, right extends past output right.
        let window = Rectangle {
            x: 1500,
            y: 100,
            width: 600,
            height: 400,
        };
        let clip = compute_clip_box(window, OUTPUT, 3);

        assert_eq!(clip.width, 420); // 1920 - 1500
        assert_eq!(clip.height, 400);
        assert_eq!(clip.x, -3);
        assert_eq!(clip.y, -3);
    }

    #[test]
    fn initial_rectangle_starts_offscreen_right() {
        let rect = initial_rectangle(OUTPUT, &config(0.5, CenterFocused::Never));
        assert_eq!(rect.x + rect.width + 9, 1920);
        assert_eq!(rect.width, 946); // trunc(1911 * 0.5) - 9
        assert_eq!(rect.height, 1080 - 2 * 9);
        assert_eq!(rect.y, 9);
    }
}
