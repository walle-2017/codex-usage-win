//! Shared settings-panel drawing primitives.
//!
//! Preserve the original native SVG icon rendering, antialiased rounded-rect
//! path and GDI fallback, including object selection and deletion order.
//! These are painting helpers only; window state and frame scheduling remain
//! owned by the parent settings module.

use super::*;

pub(super) unsafe fn draw_settings_icon(
    hdc: HDC,
    _hwnd: HWND,
    icon: SettingsIcon,
    rect: RECT,
    color: Color,
    _background: Color,
) {
    // All 32 glyphs are embedded SVG resources. The physical-pixel raster
    // cache preserves theme tint and DPI while Win32 performs alpha compositing.
    let _ = crate::svg_icons::draw(hdc, rect, icon as usize, color);
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_segment_with_icon(
    hdc: HDC,
    hwnd: HWND,
    rect: RECT,
    selected: bool,
    background: Color,
    foreground: Color,
    icon: SettingsIcon,
    text: &str,
) {
    let height = (rect.bottom - rect.top).max(1);
    let radius = (height / 4).clamp(5, 10);
    fill_rounded_rect(hdc, rect, background, radius);

    let border = if selected {
        Color::from_hex("#76A7FFFF")
    } else {
        subtle_control_border(background)
    };
    draw_rounded_outline_rect(hdc, rect, border, radius, 1);

    let important_small_icon = matches!(
        icon,
        SettingsIcon::ThemeSystem | SettingsIcon::PositionTaskbar | SettingsIcon::PositionDesktop,
    );
    // The theme's gear needs a few more pixels than the much simpler
    // rectangular placement icons to keep its teeth distinguishable.
    let preferred_icon_size = if icon == SettingsIcon::ThemeSystem {
        22
    } else if important_small_icon {
        20
    } else {
        18
    };
    let icon_size = scale(hwnd, preferred_icon_size)
        .min((height - scale(hwnd, 8)).max(scale(hwnd, 12)));
    let gap = scale(hwnd, if important_small_icon { 5 } else { 7 });
    let text_width = text_width_px(hwnd, text).max(scale(hwnd, 12));
    let group_width = icon_size + gap + text_width;
    let group_left = rect.left + ((rect.right - rect.left - group_width) / 2).max(scale(hwnd, 6));
    let icon_rect = RECT {
        left: group_left,
        top: rect.top + (height - icon_size) / 2,
        right: group_left + icon_size,
        bottom: rect.top + (height - icon_size) / 2 + icon_size,
    };
    draw_settings_icon(hdc, hwnd, icon, icon_rect, foreground, background);

    let _ = SetTextColor(hdc, COLORREF(foreground.to_colorref()));
    draw_text(
        hdc,
        text,
        RECT {
            left: icon_rect.right + gap,
            top: rect.top,
            right: rect.right - scale(hwnd, 6),
            bottom: rect.bottom,
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
}

pub(super) unsafe fn draw_segment(
    hdc: HDC,
    rect: RECT,
    selected: bool,
    background: Color,
    foreground: Color,
    text: &str,
) {
    let height = (rect.bottom - rect.top).max(1);
    let radius = (height / 4).clamp(5, 10);
    fill_rounded_rect(hdc, rect, background, radius);

    let border = if selected {
        Color::from_hex("#76A7FFFF")
    } else {
        subtle_control_border(background)
    };
    draw_rounded_outline_rect(hdc, rect, border, radius, 1);

    let _ = SetTextColor(hdc, COLORREF(foreground.to_colorref()));
    draw_text(hdc, text, rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
}

pub(super) fn subtle_control_border(background: Color) -> Color {
    let brightness = background.r as u16 + background.g as u16 + background.b as u16;
    if brightness < 384 {
        Color::from_hex("#4B5360FF")
    } else {
        Color::from_hex("#B8C3CFFF")
    }
}

pub(super) unsafe fn fill_rounded_rect(hdc: HDC, rect: RECT, color: Color, radius: i32) {
    if native_interop::draw_antialiased_rounded_rect(
        hdc,
        rect,
        radius.max(1) as f32,
        Some(color),
        None,
    ) {
        return;
    }

    // Conservative fallback for systems where Direct2D DC rendering is unavailable.
    let brush = CreateSolidBrush(COLORREF(color.to_colorref()));
    let pen = CreatePen(PS_SOLID, 1, COLORREF(color.to_colorref()));
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen);
    let diameter = (radius.max(1) * 2).max(2);
    let _ = RoundRect(
        hdc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        diameter,
        diameter,
    );
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(pen);
    let _ = DeleteObject(brush);
}

pub(super) unsafe fn draw_rounded_outline_rect(
    hdc: HDC,
    rect: RECT,
    color: Color,
    radius: i32,
    width: i32,
) {
    if native_interop::draw_antialiased_rounded_rect(
        hdc,
        rect,
        radius.max(1) as f32,
        None,
        Some((color, width.max(1) as f32)),
    ) {
        return;
    }

    // Conservative fallback for systems where Direct2D DC rendering is unavailable.
    let pen = CreatePen(PS_SOLID, width.max(1), COLORREF(color.to_colorref()));
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    let diameter = (radius.max(1) * 2).max(2);
    let _ = RoundRect(
        hdc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        diameter,
        diameter,
    );
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

pub(super) unsafe fn draw_switch(hdc: HDC, hit_rect: RECT, enabled: bool, track_color: Color) {
    let hit_width = (hit_rect.right - hit_rect.left).max(1);
    let hit_height = (hit_rect.bottom - hit_rect.top).max(1);
    let track_height = (hit_height * 11 / 16).max(12);
    let track_width = (track_height * 20 / 11).min(hit_width);
    let track = RECT {
        left: hit_rect.right - track_width,
        top: hit_rect.top + (hit_height - track_height) / 2,
        right: hit_rect.right,
        bottom: hit_rect.top + (hit_height - track_height) / 2 + track_height,
    };

    fill_rounded_rect(hdc, track, track_color, track_height / 2);
    draw_rounded_outline_rect(
        hdc,
        track,
        subtle_control_border(track_color),
        track_height / 2,
        1,
    );

    let knob_size = (track_height - 4).max(8);
    let knob_top = track.top + (track_height - knob_size) / 2;
    let knob_left = if enabled {
        track.right - knob_size - 2
    } else {
        track.left + 2
    };
    let knob_rect = RECT {
        left: knob_left,
        top: knob_top,
        right: knob_left + knob_size,
        bottom: knob_top + knob_size,
    };
    fill_rounded_rect(
        hdc,
        knob_rect,
        Color::from_hex("#FFFFFFFF"),
        knob_size / 2,
    );
    draw_rounded_outline_rect(
        hdc,
        knob_rect,
        Color::from_hex("#E7ECF2FF"),
        knob_size / 2,
        1,
    );
}

pub(super) unsafe fn draw_outline_rect(hdc: HDC, rect: RECT, color: Color) {
    draw_outline_rect_width(hdc, rect, color, 1);
}

pub(super) unsafe fn draw_outline_rect_width(hdc: HDC, rect: RECT, color: Color, width: i32) {
    let pen = CreatePen(PS_SOLID, width, COLORREF(color.to_colorref()));
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    let _ = Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

pub(super) unsafe fn fill(hdc: HDC, rect: RECT, color: Color) {
    let brush = CreateSolidBrush(COLORREF(color.to_colorref()));
    FillRect(hdc, &rect, brush);
    let _ = DeleteObject(brush);
}

pub(super) unsafe fn draw_text(hdc: HDC, text: &str, mut rect: RECT, format: DRAW_TEXT_FORMAT) {
    // windows-rs exposes DrawTextW through a mutable UTF-16 slice. Passing an
    // empty Vec leaves the native call with a zero-length slice backed by a
    // dangling Vec pointer. The JSON validation status is intentionally cleared
    // when an invalid document becomes valid, so that exact transition can reach
    // this helper with an empty string. Skip the native call entirely for empty
    // labels/status text.
    if text.is_empty() {
        return;
    }
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let _ = DrawTextW(hdc, &mut wide, &mut rect, format);
}


