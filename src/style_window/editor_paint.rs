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

// Localized editor row labels and frosted guidance; event code remains in parent.
/// The final blur row ends at logical Y=410; the editor begins at Y=432.
/// Render a small non-interactive hint inside that existing 22px gap.
pub(super) unsafe fn paint_frosted_alpha_hint(
    hdc: HDC,
    hwnd: HWND,
    language: LanguageId,
    secondary: Color,
) {
    let label = if language == LanguageId::SimplifiedChinese {
        "仅当背景颜色含透明度（Alpha < 255）时，磨砂效果才会生效。"
    } else {
        "Frosted effect requires background transparency (Alpha < 255)."
    };
    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Ui, Some(language)));
    let hint_font = CreateFontW(
        -scale(hwnd, 11),
        0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32,
        OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
        PCWSTR::from_raw(face.as_ptr()),
    );
    let original = if !hint_font.0.is_null() {
        Some(SelectObject(hdc, hint_font))
    } else {
        None
    };
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        label,
        rect(hwnd, 218, 410, 940, 431),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
    if let Some(old_font) = original {
        let _ = SelectObject(hdc, old_font);
        let _ = DeleteObject(hint_font);
    }
}

pub(super) fn row_label(row: EditorSelection, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match row {
        EditorSelection::CornerRadius => {
            if zh { "圆角半径" } else { "Corner radius" }
        }
        EditorSelection::Color(StyleColorTarget::PanelBackground) => {
            if zh { "背景颜色" } else { "Background" }
        }
        EditorSelection::Color(StyleColorTarget::PanelBorder) => {
            if zh { "边框颜色" } else { "Border" }
        }
        EditorSelection::Blur | EditorSelection::TooltipBlur => {
            if zh { "磨砂强度" } else { "Frosted intensity" }
        }
        EditorSelection::Color(StyleColorTarget::TooltipBackground) => {
            if zh { "背景颜色" } else { "Background" }
        }
        EditorSelection::Color(StyleColorTarget::TooltipBorder) => {
            if zh { "边框颜色" } else { "Border" }
        }
        EditorSelection::Color(StyleColorTarget::QuotaType) => {
            if zh { "额度类型" } else { "Quota type" }
        }
        EditorSelection::Color(StyleColorTarget::Remaining) => {
            if zh { "剩余额度" } else { "Remaining quota" }
        }
        EditorSelection::Color(StyleColorTarget::ResetTime) => {
            if zh { "重置时间" } else { "Reset time" }
        }
        EditorSelection::Color(StyleColorTarget::Error) => {
            if zh { "异常状态" } else { "Error state" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressHigh) => {
            if zh { "充足额度" } else { "High quota" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressMedium) => {
            if zh { "中等额度" } else { "Medium quota" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressLow) => {
            if zh { "低额度" } else { "Low quota" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressConsumed) => {
            if zh { "已消耗部分" } else { "Consumed" }
        }
        EditorSelection::Color(StyleColorTarget::DragHandle) => {
            if zh { "拖拽点" } else { "Drag handle" }
        }
    }
}

