//! Desktop floating-widget canvas geometry and slide composition.
//!
//! All measurements preserve the original logical DPI scaling and edge
//! coordinate conventions. Keeping these transforms together prevents the
//! hit testing, floating layout and layer animation paths from diverging.

use super::*;

/// Desktop tabs occupy an independent, transparent gutter. The panel dimensions
/// remain identical to the taskbar renderer, including progress and ring layout.
pub(super) fn floating_canvas_geometry(
    body_width: i32, body_height: i32, edge: Option<FloatingEdge>,
) -> (i32, i32, RECT) {
    let tab = sc(FLOAT_EDGE_PEEK);
    match edge {
        Some(FloatingEdge::Left) => {
            let glow = sc(FLOAT_FREE_GLOW_MARGIN);
            (body_width + tab + glow, body_height + 2 * glow,
                RECT { left: 0, top: glow, right: body_width, bottom: glow + body_height })
        },
        Some(FloatingEdge::Right) => {
            let glow = sc(FLOAT_FREE_GLOW_MARGIN);
            (body_width + tab + glow, body_height + 2 * glow,
                RECT { left: tab + glow, top: glow,
                    right: body_width + tab + glow, bottom: glow + body_height })
        },
        Some(FloatingEdge::Top) => {
            let glow = sc(FLOAT_FREE_GLOW_MARGIN);
            (body_width + 2 * glow, body_height + tab + glow,
                RECT { left: glow, top: 0, right: glow + body_width, bottom: body_height })
        },
        Some(FloatingEdge::Bottom) => {
            let glow = sc(FLOAT_FREE_GLOW_MARGIN);
            (body_width + 2 * glow, body_height + tab + glow,
                RECT { left: glow, top: tab + glow,
                    right: glow + body_width, bottom: tab + glow + body_height })
        },
        None => {
            let glow = sc(FLOAT_FREE_GLOW_MARGIN);
            (
                body_width + glow * 2,
                body_height + tab + glow * 2,
                RECT {
                    left: glow, top: tab + glow,
                    right: body_width + glow, bottom: body_height + tab + glow,
                },
            )
        },
    }
}

// Switching the free tab to below keeps the canvas size unchanged and
// shifts only the panel pixels, never the visible panel's screen position.
pub(super) fn floating_free_canvas_geometry(
    body_width: i32, body_height: i32, below: bool,
) -> (i32, i32, RECT) {
    let (width, height, mut body) =
        floating_canvas_geometry(body_width, body_height, None);
    if below {
        let shift = sc(FLOAT_EDGE_PEEK);
        body.top -= shift;
        body.bottom -= shift;
    }
    (width, height, body)
}

pub(super) fn floating_layout_grip_rect(
    width: i32, height: i32, edge: Option<FloatingEdge>, free_below: bool,
) -> RECT {
    floating_grip_rect(width, height,
        if edge.is_none() && free_below { Some(FloatingEdge::Top) } else { edge },
        sc(FLOAT_EDGE_PEEK))
}

// This is a layout choice, NOT docking. Prefer the top tab when it fits;
// otherwise use the empty space below. Preserve the body and tab on-screen.
pub(super) fn floating_choose_free_grip_below(bounds: RECT, body_top: i32, body_height: i32) -> bool {
    let above = body_top.saturating_sub(bounds.top);
    let below = bounds.bottom.saturating_sub(body_top.saturating_add(body_height));
    above < sc(FLOAT_EDGE_PEEK) && below >= sc(FLOAT_EDGE_PEEK)
}

/// During dragging, the existing edge orientation is preserved for a stable
/// grip. Once the panel has cleared the monitor edge, add transparent pixels
/// only on the formerly screen-clipped side. The unchanged body dimensions
/// and original tab orientation then gain an external halo on all sides.
pub(super) fn floating_drag_canvas_geometry(
    body_width: i32, body_height: i32, edge: Option<FloatingEdge>,
    drag_glow_detached: bool,
) -> (i32, i32, RECT) {
    let (width, height, mut body) =
        floating_canvas_geometry(body_width, body_height, edge);
    if !drag_glow_detached { return (width, height, body); }
    let glow = sc(FLOAT_FREE_GLOW_MARGIN);
    match edge {
        Some(FloatingEdge::Left) => {
            body.left += glow;
            body.right += glow;
            (width + glow, height, body)
        }
        Some(FloatingEdge::Right) => (width + glow, height, body),
        Some(FloatingEdge::Top) => {
            body.top += glow;
            body.bottom += glow;
            (width, height + glow, body)
        }
        Some(FloatingEdge::Bottom) => (width, height + glow, body),
        None => (width, height, body),
    }
}

/// Shared slide distance for physical screen edges and Explorer taskbar edges.
/// At reveal=0 the grip is flush with the boundary; at reveal=100 the
/// original content and tab occupy their fully expanded coordinates.
pub(super) fn floating_slide_offset(
    width: i32, height: i32, edge: FloatingEdge, reveal: u8,
) -> (i32, i32) {
    let grip = floating_grip_rect(width, height, Some(edge), sc(FLOAT_EDGE_PEEK));
    let (dx, dy) = match edge {
        FloatingEdge::Left => (-grip.left, 0),
        FloatingEdge::Right => (width - grip.right, 0),
        FloatingEdge::Top => (0, -grip.top),
        FloatingEdge::Bottom => (0, height - grip.bottom),
    };
    // Use the physical-edge interpolation's original integer rounding
    // (including halfway frames). This avoids one-pixel differences between
    // the taskbar slide and the ordinary popup slide.
    let progress = f64::from(reveal.min(100)) / 100.0;
    (
        dx + (f64::from(-dx) * progress).round() as i32,
        dy + (f64::from(-dy) * progress).round() as i32,
    )
}

/// Clip moving desktop pixels to the taskbar-free canvas. The panel and grip
/// use the exact same offset as physical-edge docking, not a second
/// shortening/stretching animation invented for reserved work-area edges.
pub(super) fn floating_slid_canvas_rect(
    rect: RECT, width: i32, height: i32, edge: FloatingEdge, reveal: u8,
) -> RECT {
    let (dx, dy) = floating_slide_offset(width, height, edge, reveal);
    RECT {
        left: (rect.left + dx).clamp(0, width),
        right: (rect.right + dx).clamp(0, width),
        top: (rect.top + dy).clamp(0, height),
        bottom: (rect.bottom + dy).clamp(0, height),
    }
}

/// Shift the *completed* premultiplied foreground bitmap as one unit. It
/// preserves text, rounded corners, double-sided halo, and the six-dot tab,
/// while the taskbar-facing pixels outside the desktop canvas stay alpha=0.
pub(super) fn floating_slide_layered_bitmap(
    canvas: &mut [u32], width: i32, height: i32,
    edge: FloatingEdge, reveal: u8,
) {
    let (dx, dy) = floating_slide_offset(width, height, edge, reveal);
    if dx == 0 && dy == 0 { return; }
    let src = canvas.to_vec();
    canvas.fill(0);
    let copy_width = width - dx.abs();
    if copy_width <= 0 { return; }
    let dst_left = dx.max(0);
    let src_left = (-dx).max(0);
    for dst_y in dy.max(0)..(height + dy).min(height) {
        let src_y = dst_y - dy;
        let dst_index = (dst_y * width + dst_left) as usize;
        let src_index = (src_y * width + src_left) as usize;
        let count = copy_width as usize;
        canvas[dst_index..dst_index + count]
            .copy_from_slice(&src[src_index..src_index + count]);
    }
}
