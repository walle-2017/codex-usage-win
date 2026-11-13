//! Settings preset-gallery widget previews.
//!
//! The preview primitives belong together so the taskbar, desktop, portrait
//! and horizontal gallery cards retain the same geometry and visual tokens.

use super::*;

// Match the vertical widget's 36-degree bottom opening and clockwise
// remaining-quota fill. Angles are measured from 12 o'clock.
pub(super) const PREVIEW_RING_GAP_HALF_RADIANS: f32 = std::f32::consts::PI / 10.0;

pub(super) fn preview_ring_angles(remaining: f32) -> (f32, f32, f32) {
    let start = std::f32::consts::PI + PREVIEW_RING_GAP_HALF_RADIANS;
    let sweep = std::f32::consts::TAU - 2.0 * PREVIEW_RING_GAP_HALF_RADIANS;
    (start, start + sweep, start + sweep * (remaining / 100.0).clamp(0.0, 1.0))
}

pub(super) fn preview_remaining_color(remaining: f32) -> StyleColorTarget {
    if remaining > 50.0 {
        StyleColorTarget::ProgressHigh
    } else if remaining > 20.0 {
        StyleColorTarget::ProgressMedium
    } else {
        StyleColorTarget::ProgressLow
    }
}

// A four-by-four coverage raster instead of the jagged GDI Arc stroke.
// The ring remains clockwise, displays remaining quota and keeps its
// 36-degree bottom opening at 0% through 100%.
pub(super) const PREVIEW_RING_SAMPLES: i32 = 4;

#[allow(clippy::too_many_arguments)]
pub(super) fn preview_ring_pixel(
    px: i32, py: i32, size: i32, radius: f32, stroke: f32,
    remaining: f32, background: Color, track: Color, accent: Color,
) -> u32 {
    let (start, end, _) = preview_ring_angles(remaining);
    let sweep = end - start;
    let fraction = (remaining / 100.0).clamp(0.0, 1.0);
    let track = track.blend_over(background);
    let accent = accent.blend_over(background);
    let count = (PREVIEW_RING_SAMPLES * PREVIEW_RING_SAMPLES) as u32;
    let (mut red, mut green, mut blue) = (0u32, 0u32, 0u32);
    for sy in 0..PREVIEW_RING_SAMPLES {
        for sx in 0..PREVIEW_RING_SAMPLES {
            let dx = px as f32 + (sx as f32 + 0.5) / PREVIEW_RING_SAMPLES as f32
                - size as f32 * 0.5;
            let dy = py as f32 + (sy as f32 + 0.5) / PREVIEW_RING_SAMPLES as f32
                - size as f32 * 0.5;
            let distance = (dx * dx + dy * dy).sqrt();
            let color = if (distance - radius).abs() <= stroke * 0.5 {
                let angle = dx.atan2(-dy).rem_euclid(std::f32::consts::TAU);
                let offset = (angle - start).rem_euclid(std::f32::consts::TAU);
                if offset <= sweep {
                    if offset / sweep <= fraction { accent } else { track }
                } else {
                    background
                }
            } else {
                background
            };
            red += u32::from(color.r);
            green += u32::from(color.g);
            blue += u32::from(color.b);
        }
    }
    let r = (red + count / 2) / count;
    let g = (green + count / 2) / count;
    let b = (blue + count / 2) / count;
    (r << 16) | (g << 8) | b
}

pub(super) unsafe fn paint_preview_remaining_ring(
    hdc: HDC, hwnd: HWND, cx: i32, cy: i32, remaining: f32, style: &ThemeStyle,
) {
    let size = scale(hwnd, 36).max(8);
    let radius = scale(hwnd, 15).max(2) as f32;
    let stroke = scale(hwnd, 3).max(1) as f32;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let dc = CreateCompatibleDC(hdc);
    if dc.0.is_null() { return; }
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        .unwrap_or_default();
    if dib.is_invalid() || bits.is_null() {
        let _ = DeleteDC(dc);
        return;
    }
    let old_bitmap = SelectObject(dc, dib);
    let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
    let background = style.color(StyleColorTarget::PanelBackground);
    let track = style.color(StyleColorTarget::ProgressConsumed);
    let accent = style.color(preview_remaining_color(remaining));
    for y in 0..size {
        for x in 0..size {
            pixels[(y * size + x) as usize] = preview_ring_pixel(
                x, y, size, radius, stroke, remaining, background, track, accent,
            );
        }
    }
    let _ = BitBlt(hdc, cx - size / 2, cy - size / 2, size, size, dc, 0, 0, SRCCOPY);
    let _ = SelectObject(dc, old_bitmap);
    let _ = DeleteObject(dib);
    let _ = DeleteDC(dc);
}

// Use the same textual template as the live usage tooltip. The preview
// deliberately uses sample data and wraps *only* at the final separator
// between the date and time when a theme card cannot fit one line.
pub(super) fn preview_reset_tooltip_lines(minimal: bool) -> (String, String) {
    let preset = if minimal { AppearancePreset::Minimal } else { AppearancePreset::Default };
    let sample = crate::appearance::quota_reset_tooltip_text(
        preset, "5H", "YYYY-MM-DD HH:mm",
    );
    let (date, time) = sample.rsplit_once(' ')
        .expect("sample full local timestamp contains a time separator");
    (date.to_owned(), time.to_owned())
}

pub(super) fn portrait_preview_tooltip_rect(
    canvas: RECT, widget: RECT, logical: impl Fn(i32) -> i32,
) -> RECT {
    let width = logical(128).min(canvas.right - canvas.left - logical(12));
    let height = logical(40);
    let left = canvas.left + (canvas.right - canvas.left - width) / 2;
    let top = widget.bottom + logical(7);
    RECT { left, top, right: left + width, bottom: top + height }
}

pub(super) unsafe fn paint_portrait_preview_tooltip(
    hdc: HDC, hwnd: HWND, canvas: RECT, widget: RECT,
    style: &ThemeStyle, minimal: bool,
) {
    // The preview always includes a sample floating tooltip to demonstrate
    // theme customization. It sits below the widget rather than obscuring it.
    // Runtime hover rules are unchanged (reset only / percent only).
    let tooltip = portrait_preview_tooltip_rect(canvas, widget, |v| scale(hwnd, v));
    let radius = scale(hwnd, i32::from(style.tooltip_corner_radius))
        .min(((tooltip.right - tooltip.left).min(tooltip.bottom - tooltip.top) / 2).max(0));
    if radius > 0 {
        fill_rounded_rect(hdc, tooltip, style.color(StyleColorTarget::TooltipBackground), radius);
        draw_rounded_outline_rect(
            hdc, tooltip, style.color(StyleColorTarget::TooltipBorder), radius, 1,
        );
    } else {
        fill(hdc, tooltip, style.color(StyleColorTarget::TooltipBackground));
        draw_outline_rect(hdc, tooltip, style.color(StyleColorTarget::TooltipBorder));
    }
    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
    let font = CreateFontW(
        -scale(hwnd, 10).max(1), 0, 0, 0, FW_MEDIUM.0 as i32, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32, ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, PCWSTR::from_raw(face.as_ptr()),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, TRANSPARENT);
    let text_color = style.color(StyleColorTarget::ResetTime);
    let _ = SetTextColor(hdc, COLORREF(text_color.to_colorref()));
    let (date, time) = preview_reset_tooltip_lines(minimal);
    draw_text(
        hdc,
        &date,
        RECT {
            left: tooltip.left + scale(hwnd, 3),
            top: tooltip.top + scale(hwnd, 3),
            right: tooltip.right - scale(hwnd, 3),
            bottom: tooltip.top + scale(hwnd, 21),
        },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc, &time,
        RECT {
            left: tooltip.left + scale(hwnd, 3),
            top: tooltip.top + scale(hwnd, 20),
            right: tooltip.right - scale(hwnd, 3),
            bottom: tooltip.bottom - scale(hwnd, 2),
        },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
}

// Preview the actual portrait taskbar widget rather than a stretched card
// illustration. The same 50x168 proportion and 20/94px row anchors are used
// for all four themes; only each theme's colors and corners change.
pub(super) const PORTRAIT_PREVIEW_WIDGET_W: i32 = 50;
pub(super) const PORTRAIT_PREVIEW_WIDGET_H: i32 = 168;
pub(super) const PORTRAIT_PREVIEW_MINIMAL_H: i32 = 82;

pub(super) fn portrait_preview_widget_bounds(
    canvas: RECT, minimal: bool, logical: impl Fn(i32) -> i32,
) -> RECT {
    let width = logical(PORTRAIT_PREVIEW_WIDGET_W);
    let height = logical(if minimal {
        PORTRAIT_PREVIEW_MINIMAL_H
    } else {
        PORTRAIT_PREVIEW_WIDGET_H
    });
    let left = canvas.left + (canvas.right - canvas.left - width) / 2;
    // Align the widget together with its tooltip, rather than centering
    // the widget alone (which left a large dead area above both).
    let group_height = height + logical(7 + 40);
    let available = (canvas.bottom - canvas.top - group_height).max(0);
    let top = canvas.top + available / 3;
    RECT { left, top, right: left + width, bottom: top + height }
}


pub(super) const DESKTOP_PREVIEW_WIDGET_H: i32 = 164; // V22: 2 * 10px padding + 2 * 70px rows + 4px gap
pub(super) const DESKTOP_PREVIEW_MINIMAL_H: i32 = 74; // V22: 2 * 10px padding + 2 * 24px rows + 6px gap
pub(super) const PREVIEW_DRAG_TAB_H: i32 = 16;
pub(super) const PREVIEW_DRAG_TAB_W: i32 = 36;
pub(super) const PREVIEW_DRAG_TAB_GAP: i32 = 4;
pub(super) const PREVIEW_TASKBAR_GRIP_W: i32 = 16;
pub(super) const PREVIEW_HORIZONTAL_TOOLTIP_H: i32 = 36;

pub(super) fn desktop_portrait_preview_bounds(
    canvas: RECT, minimal: bool, logical: impl Fn(i32) -> i32,
) -> (RECT, RECT) {
    let width = logical(PORTRAIT_PREVIEW_WIDGET_W);
    let height = logical(if minimal { DESKTOP_PREVIEW_MINIMAL_H }
        else { DESKTOP_PREVIEW_WIDGET_H });
    let tab_height = logical(PREVIEW_DRAG_TAB_H);
    let tab_gap = logical(PREVIEW_DRAG_TAB_GAP);
    let group_height = tab_height + tab_gap + height + logical(7 + 40);
    let top = canvas.top + (canvas.bottom - canvas.top - group_height).max(0) / 2;
    let left = canvas.left + (canvas.right - canvas.left - width) / 2;
    let body_top = top + tab_height + tab_gap;
    let body = RECT { left, top: body_top, right: left + width,
        bottom: body_top + height };
    let tab_width = logical(PREVIEW_DRAG_TAB_W);
    let tab = RECT { left: (body.left + body.right - tab_width) / 2, top,
        right: (body.left + body.right + tab_width) / 2, bottom: top + tab_height };
    (body, tab)
}

// The desktop tab has its own panel surface and is never painted inside
// the body. A single six-dot renderer is shared by both orientations.
pub(super) unsafe fn paint_preview_drag_tab(
    hdc: HDC, hwnd: HWND, tab: RECT, style: &ThemeStyle,
) {
    let radius = scale(hwnd, i32::from(style.panel_corner_radius))
        .min(((tab.right - tab.left).min(tab.bottom - tab.top) / 2).max(0));
    if radius > 0 {
        fill_rounded_rect(hdc, tab, style.color(StyleColorTarget::PanelBackground), radius);
        draw_rounded_outline_rect(hdc, tab, style.color(StyleColorTarget::PanelBorder), radius, 1);
    } else {
        fill(hdc, tab, style.color(StyleColorTarget::PanelBackground));
        draw_outline_rect(hdc, tab, style.color(StyleColorTarget::PanelBorder));
    }
    let dot = scale(hwnd, 2).max(1);
    let gap_x = scale(hwnd, 2).max(1);
    let gap_y = scale(hwnd, 1).max(1);
    let dots_w = 3 * dot + 2 * gap_x;
    let dots_h = 2 * dot + gap_y;
    let start_x = (tab.left + tab.right - dots_w) / 2;
    let start_y = (tab.top + tab.bottom - dots_h) / 2;
    for row in 0..2 {
        for col in 0..3 {
            let x = start_x + col * (dot + gap_x);
            let y = start_y + row * (dot + gap_y);
            fill(hdc, RECT { left: x, top: y, right: x + dot, bottom: y + dot },
                style.color(StyleColorTarget::DragHandle));
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PortraitPreviewRow {
    pub(super) label: RECT,
    pub(super) center_x: i32,
    pub(super) center_y: i32,
    pub(super) reset: RECT,
}

pub(super) fn portrait_preview_row_for_mode(
    widget: RECT, index: i32, desktop: bool, logical: impl Fn(i32) -> i32,
) -> PortraitPreviewRow {
    let top = widget.top + logical((if desktop { 10 } else { 20 }) + index * 74);
    PortraitPreviewRow {
        label: RECT {
            left: widget.left + logical(2),
            top,
            right: widget.right - logical(2),
            bottom: top + logical(18),
        },
        center_x: (widget.left + widget.right) / 2,
        center_y: top + logical(36),
        reset: RECT {
            left: widget.left + logical(2),
            top: top + logical(54),
            right: widget.right - logical(2),
            bottom: top + logical(72),
        },
    }
}

#[cfg(test)]
pub(super) fn portrait_preview_row(widget: RECT, index: i32, logical: impl Fn(i32) -> i32) -> PortraitPreviewRow {
    portrait_preview_row_for_mode(widget, index, false, logical)
}

pub(super) unsafe fn paint_vertical_style_preview(
    hdc: HDC, hwnd: HWND, preview: RECT, style: &ThemeStyle,
    minimal: bool, desktop: bool,
) {
    let (widget, detached_tab) = if desktop {
        let (widget, tab) = desktop_portrait_preview_bounds(preview, minimal, |v| scale(hwnd, v));
        (widget, Some(tab))
    } else {
        (portrait_preview_widget_bounds(preview, minimal, |v| scale(hwnd, v)), None)
    };
    let radius = scale(hwnd, i32::from(style.panel_corner_radius))
        .min(((widget.right - widget.left).min(widget.bottom - widget.top) / 2).max(0));
    if radius > 0 {
        fill_rounded_rect(hdc, widget, style.color(StyleColorTarget::PanelBackground), radius);
        draw_rounded_outline_rect(hdc, widget, style.color(StyleColorTarget::PanelBorder), radius, 1);
    } else {
        fill(hdc, widget, style.color(StyleColorTarget::PanelBackground));
        draw_outline_rect(hdc, widget, style.color(StyleColorTarget::PanelBorder));
    }

    if let Some(tab) = detached_tab {
        paint_preview_drag_tab(hdc, hwnd, tab, style);
    } else {
        // The embedded taskbar grip remains inside the taskbar body.
        let dot = scale(hwnd, 2).max(1);
        let dot_gap = scale(hwnd, 3);
        let grip_color = style.color(StyleColorTarget::DragHandle);
        for row in 0..2 {
            for col in 0..3 {
                let x = (widget.left + widget.right) / 2
                    - (3 * dot + 2 * dot_gap) / 2 + col * (dot + dot_gap);
                let y = widget.top + scale(hwnd, 5) + row * (dot + dot_gap);
                fill(hdc, RECT { left: x, top: y, right: x + dot, bottom: y + dot }, grip_color);
            }
        }
    }

    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
    let font = |size: i32, weight: i32| CreateFontW(
        -scale(hwnd, size).max(1), 0, 0, 0, weight, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32, ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, PCWSTR::from_raw(face.as_ptr()),
    );
    let label_font = font(10, FW_MEDIUM.0 as i32);
    // At a 50px taskbar width, a three-character percentage needs room
    // around its glyphs; a 9px font inside a 15px ring preserves that gap.
    let percent_font = font(if minimal { 12 } else { 9 }, FW_SEMIBOLD.0 as i32);
    let reset_font = font(9, FW_NORMAL.0 as i32);
    let old_font = SelectObject(hdc, label_font);
    let _ = SetBkMode(hdc, TRANSPARENT);

    for (index, (label, remaining, reset)) in [
        ("5H", 82.0_f32, "03:49"),
        ("7D", 37.0_f32, "10/22"),
    ].iter().enumerate() {
        if minimal {
            let center_y = widget.top + scale(hwnd, if desktop {
                if index == 0 { 22 } else { 52 }
            } else if index == 0 { 34 } else { 63 });
            let _ = SelectObject(hdc, percent_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
            draw_text(hdc, &format!("{remaining:.0}%"), RECT {
                left: widget.left + scale(hwnd, 2),
                top: center_y - scale(hwnd, 12),
                right: widget.right - scale(hwnd, 2),
                bottom: center_y + scale(hwnd, 12),
            }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        } else {
            let row = portrait_preview_row_for_mode(widget, index as i32, desktop, |v| scale(hwnd, v));
            let _ = SelectObject(hdc, label_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::QuotaType).to_colorref()));
            draw_text(hdc, label, row.label, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            paint_preview_remaining_ring(hdc, hwnd, row.center_x, row.center_y, *remaining, style);
            let _ = SelectObject(hdc, percent_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
            draw_text(hdc, &format!("{remaining:.0}%"), RECT {
                left: row.center_x - scale(hwnd, 14),
                top: row.center_y - scale(hwnd, 11),
                right: row.center_x + scale(hwnd, 14),
                bottom: row.center_y + scale(hwnd, 11),
            }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let _ = SelectObject(hdc, reset_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()));
            draw_text(hdc, reset, row.reset, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }

    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(label_font);
    let _ = DeleteObject(percent_font);
    let _ = DeleteObject(reset_font);
    paint_portrait_preview_tooltip(hdc, hwnd, preview, widget, style, minimal);
}



pub(super) unsafe fn paint_taskbar_horizontal_preview_grip(
    hdc: HDC, hwnd: HWND, canvas: RECT, style: &ThemeStyle,
) {
    // The taskbar grip is embedded at the left edge; unlike the separate
    // desktop tab it uses the taskbar's original 2-column, 3-row dot shape.
    let dot = scale(hwnd, 2).max(1);
    let gap_x = scale(hwnd, 1).max(1);
    let gap_y = scale(hwnd, 2).max(1);
    let dots_w = 2 * dot + gap_x;
    let dots_h = 3 * dot + 2 * gap_y;
    let x0 = canvas.left + scale(hwnd, 6) - dots_w / 2;
    let y0 = (canvas.top + canvas.bottom - dots_h) / 2;
    for row in 0..3 {
        for col in 0..2 {
            let x = x0 + col * (dot + gap_x);
            let y = y0 + row * (dot + gap_y);
            fill(hdc, RECT { left: x, top: y, right: x + dot, bottom: y + dot },
                style.color(StyleColorTarget::DragHandle));
        }
    }
}

pub(super) fn horizontal_preview_body(
    canvas: RECT, minimal: bool, desktop: bool, logical: impl Fn(i32) -> i32,
) -> RECT {
    let width = logical(if minimal { 56 } else { 184 }
        + if desktop { 0 } else { PREVIEW_TASKBAR_GRIP_W })
        .min(canvas.right - canvas.left - logical(if desktop { 8 } else { 4 }));
    let height = logical(if minimal { 40 } else { 42 });
    let tab_space = if desktop {
        logical(PREVIEW_DRAG_TAB_H + PREVIEW_DRAG_TAB_GAP)
    } else { 0 };
    let tooltip_space = logical(6 + PREVIEW_HORIZONTAL_TOOLTIP_H);
    let top = canvas.top + (canvas.bottom - canvas.top
        - (tab_space + height + tooltip_space)).max(0) / 2 + tab_space;
    let left = (canvas.left + canvas.right - width) / 2;
    RECT { left, top, right: left + width, bottom: top + height }
}

pub(super) fn horizontal_preview_detached_tab(body: RECT, logical: impl Fn(i32) -> i32) -> RECT {
    let tab_w = logical(PREVIEW_DRAG_TAB_W);
    let tab_bottom = body.top - logical(PREVIEW_DRAG_TAB_GAP);
    RECT {
        left: (body.left + body.right - tab_w) / 2,
        top: tab_bottom - logical(PREVIEW_DRAG_TAB_H),
        right: (body.left + body.right + tab_w) / 2,
        bottom: tab_bottom,
    }
}

pub(super) unsafe fn paint_preview_horizontal_minimal_text(
    hdc: HDC, hwnd: HWND, body: RECT, style: &ThemeStyle,
) {
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
    let row_height = (body.bottom - body.top) / 2;
    for (index, percent) in ["82%", "37%"].iter().enumerate() {
        let top = body.top + index as i32 * row_height;
        draw_text(hdc, percent, RECT {
            left: body.left + scale(hwnd, 2), top,
            right: body.right - scale(hwnd, 2), bottom: top + row_height,
        }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

pub(super) unsafe fn paint_horizontal_style_preview(
    hdc: HDC, hwnd: HWND, canvas: RECT, style: &ThemeStyle,
    minimal: bool, desktop: bool,
) {
    let body = horizontal_preview_body(canvas, minimal, desktop, |v| scale(hwnd, v));
    let radius = scale(hwnd, i32::from(style.panel_corner_radius))
        .min((body.bottom - body.top) / 2);
    if radius > 0 {
        fill_rounded_rect(hdc, body, style.color(StyleColorTarget::PanelBackground), radius);
        draw_rounded_outline_rect(hdc, body, style.color(StyleColorTarget::PanelBorder), radius, 1);
    } else {
        fill(hdc, body, style.color(StyleColorTarget::PanelBackground));
        draw_outline_rect(hdc, body, style.color(StyleColorTarget::PanelBorder));
    }
    if desktop {
        let tab = horizontal_preview_detached_tab(body, |v| scale(hwnd, v));
        paint_preview_drag_tab(hdc, hwnd, tab, style);
    } else {
        paint_taskbar_horizontal_preview_grip(hdc, hwnd, body, style);
    }
    // Both modes share the same data rows, progress palette and tooltip.
    // Taskbar mode alone reserves 16px inside the panel for the grip.
    let content_left = body.left
        + if desktop { 0 } else { scale(hwnd, PREVIEW_TASKBAR_GRIP_W) };
    let content = RECT { left: content_left, ..body };
    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
    let font = CreateFontW(
        -scale(hwnd, 10).max(1), 0, 0, 0, FW_MEDIUM.0 as i32, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32, ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, PCWSTR::from_raw(face.as_ptr()),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, TRANSPARENT);
    if minimal {
        paint_preview_horizontal_minimal_text(hdc, hwnd, content, style);
    } else {
        for (index, (label, percent, reset)) in [
            ("5H", "82%", "21:30"), ("7D", "37%", "10/22"),
        ].iter().enumerate() {
            let y = body.top + scale(hwnd, 3 + index as i32 * 19);
            let text_rect = |left: i32, right: i32| RECT {
                left, top: y, right, bottom: y + scale(hwnd, 18),
            };
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::QuotaType).to_colorref()));
            draw_text(hdc, label, text_rect(content.left + scale(hwnd, 9),
                content.left + scale(hwnd, 31)), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            let progress = RECT {
                left: content.left + scale(hwnd, 34), top: y + scale(hwnd, 6),
                right: content.left + scale(hwnd, 86), bottom: y + scale(hwnd, 12),
            };
            let remaining = if index == 0 { 82 } else { 37 };
            let fill_rect = RECT { right: progress.left + (progress.right - progress.left)
                * remaining / 100, ..progress };
            let progress_radius = scale(hwnd, i32::from(style.progress_corner_radius))
                .min((progress.bottom - progress.top) / 2);
            let progress_color = style.color(if index == 0 {
                StyleColorTarget::ProgressHigh
            } else {
                StyleColorTarget::ProgressMedium
            });
            if progress_radius > 0 {
                fill_rounded_rect(hdc, progress,
                    style.color(StyleColorTarget::ProgressConsumed), progress_radius);
                fill_rounded_rect(hdc, fill_rect, progress_color,
                    progress_radius.min((fill_rect.right - fill_rect.left) / 2));
            } else {
                fill(hdc, progress, style.color(StyleColorTarget::ProgressConsumed));
                fill(hdc, fill_rect, progress_color);
            }
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
            draw_text(hdc, percent, text_rect(content.left + scale(hwnd, 90),
                content.left + scale(hwnd, 124)), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()));
            draw_text(hdc, reset, text_rect(content.left + scale(hwnd, 128),
                body.right - scale(hwnd, 6)), DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
        }
    }

    let tip_width = scale(hwnd, 136).min(
        (canvas.right - canvas.left - scale(hwnd, 8)).max(scale(hwnd, 48)),
    );
    let tip_height = scale(hwnd, PREVIEW_HORIZONTAL_TOOLTIP_H);
    let tip = RECT {
        left: (canvas.left + canvas.right - tip_width) / 2,
        top: body.bottom + scale(hwnd, 6),
        right: (canvas.left + canvas.right + tip_width) / 2,
        bottom: body.bottom + scale(hwnd, 6) + tip_height,
    };
    let tooltip_radius = scale(hwnd, i32::from(style.tooltip_corner_radius))
        .min(tip_height / 2);
    if tooltip_radius > 0 {
        fill_rounded_rect(hdc, tip, style.color(StyleColorTarget::TooltipBackground), tooltip_radius);
        draw_rounded_outline_rect(hdc, tip, style.color(StyleColorTarget::TooltipBorder), tooltip_radius, 1);
    } else {
        fill(hdc, tip, style.color(StyleColorTarget::TooltipBackground));
        draw_outline_rect(hdc, tip, style.color(StyleColorTarget::TooltipBorder));
    }
    let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()));
    let (date, time) = preview_reset_tooltip_lines(minimal);
    let first = RECT { bottom: tip.top + scale(hwnd, 18), ..tip };
    let second = RECT { top: tip.top + scale(hwnd, 18), ..tip };
    draw_text(hdc, &date, first, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    draw_text(hdc, &time, second, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
}
