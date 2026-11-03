//! Color, numeric, and slider painting in the settings editor.
//!
//! Only visual drawing is extracted: all Win32 EDIT creation, measured-font
//! vertical centering, visibility, input focus, and change handling stay in
//! the parent settings module. Keep the original theme, validation, suffix,
//! GDI brush, and slider draw order unchanged.

use super::*;

pub(super) unsafe fn paint_hex_edit_frames(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
    is_dark: bool,
    focused: Option<StyleColorTarget>,
    invalid: &[bool; HEX_EDIT_COUNT],
    palette: EditFramePalette,
) {
    let EditFramePalette { border, accent } = palette;
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };
    let error = Color::from_hex("#D95C5CFF");

    for (index, target) in COLOR_TARGETS.iter().copied().enumerate() {
        let Some(row_index) = color_row_index(section, target) else {
            continue;
        };
        let frame = hex_edit_frame_rect(hwnd, row_index);
        fill_rounded_rect(hdc, frame, background, scale(hwnd, 6));
        draw_rounded_outline_rect(
            hdc,
            frame,
            if invalid[index] {
                error
            } else if focused == Some(target) {
                accent
            } else {
                border
            },
            scale(hwnd, 6),
            1,
        );
    }
}

pub(super) unsafe fn paint_numeric_edit_frames(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
    is_dark: bool,
    focused: Option<usize>,
    border: Color,
    accent: Color,
) {
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };

    for index in 0..4 {
        let frame = numeric_edit_frame_rect(hwnd, section, index);
        fill_rounded_rect(hdc, frame, background, scale(hwnd, 6));
        draw_rounded_outline_rect(
            hdc,
            frame,
            if focused == Some(index) { accent } else { border },
            scale(hwnd, 6),
            1,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_blur_edit_frame(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
    is_dark: bool,
    focused: bool,
    border: Color,
    accent: Color,
    suffix_color: Color,
) {
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };
    let frame = blur_edit_frame_rect(hwnd, section);
    fill_rounded_rect(hdc, frame, background, scale(hwnd, 6));
    draw_rounded_outline_rect(
        hdc,
        frame,
        if focused { accent } else { border },
        scale(hwnd, 6),
        1,
    );

    let _ = SetTextColor(hdc, COLORREF(suffix_color.to_colorref()));
    draw_text(
        hdc,
        "%",
        blur_suffix_rect(hwnd, section),
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

pub(super) unsafe fn paint_corner_edit_frame(
    hdc: HDC,
    hwnd: HWND,
    is_dark: bool,
    focused: bool,
    border: Color,
    accent: Color,
    suffix_color: Color,
) {
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };
    let frame = corner_edit_frame_rect(hwnd);
    fill_rounded_rect(hdc, frame, background, scale(hwnd, 6));
    draw_rounded_outline_rect(
        hdc,
        frame,
        if focused { accent } else { border },
        scale(hwnd, 6),
        1,
    );

    let _ = SetTextColor(hdc, COLORREF(suffix_color.to_colorref()));
    draw_text(
        hdc,
        "px",
        corner_suffix_rect(hwnd),
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

pub(super) unsafe fn paint_editor(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
    snapshot: &StyleWindowSnapshot,
    editor: EditorSelection,
    palette: EditorPalette,
) {
    let EditorSelection::Color(target) = editor else {
        return;
    };
    let EditorPalette {
        secondary,
        track_background,
        accent,
    } = palette;
    let color = snapshot.active_style.color(target);
    let values = [color.r, color.g, color.b, color.a];
    let editor_top = editor_top(section);
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "颜色通道"
        } else {
            "Color channels"
        },
        rect(hwnd, 218, editor_top + 4, 420, editor_top + 28),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (index, (label, value)) in ["R", "G", "B", "A"].iter().zip(values).enumerate() {
        let center =
            editor_top + 44 + index as i32 * SETTINGS_EDITOR_CHANNEL_GAP;
        draw_text(
            hdc,
            label,
            rect(hwnd, 224, center - 14, 272, center + 14),
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        draw_slider(
            hdc,
            hwnd,
            color_slider_track_rect(hwnd, section, index),
            value,
            u8::MAX,
            track_background,
            accent,
        );
    }
}

pub(super) unsafe fn draw_slider(
    hdc: HDC,
    hwnd: HWND,
    track: RECT,
    value: u8,
    max: u8,
    track_background: Color,
    accent: Color,
) {
    let track_height = (track.bottom - track.top).max(1);
    fill_rounded_rect(hdc, track, track_background, (track_height / 2).max(1));

    let width = (track.right - track.left).max(1);
    let thumb_x = track.left + width * i32::from(value) / i32::from(max.max(1));
    let filled = RECT {
        right: thumb_x.max(track.left),
        ..track
    };
    if filled.right > filled.left {
        fill_rounded_rect(hdc, filled, accent, (track_height / 2).max(1));
    }

    let radius = scale(hwnd, 6);
    let center_y = (track.top + track.bottom) / 2;
    let thumb = RECT {
        left: thumb_x - radius,
        top: center_y - radius,
        right: thumb_x + radius,
        bottom: center_y + radius,
    };
    fill_rounded_rect(hdc, thumb, accent, radius);
}

