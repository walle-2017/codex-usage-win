//! Editor row order, slider paint/hit rectangles and inline numeric frames.
//! These original measurements are shared by rendering and input handlers.

use super::*;

pub(super) fn rows(section: Section) -> &'static [EditorSelection] {
    const PANEL: [EditorSelection; 4] = [
        EditorSelection::CornerRadius,
        EditorSelection::Color(StyleColorTarget::PanelBackground),
        EditorSelection::Color(StyleColorTarget::PanelBorder),
        EditorSelection::Blur,
    ];
    const TOOLTIP: [EditorSelection; 4] = [
        EditorSelection::CornerRadius,
        EditorSelection::Color(StyleColorTarget::TooltipBackground),
        EditorSelection::Color(StyleColorTarget::TooltipBorder),
        EditorSelection::TooltipBlur,
    ];
    const TEXT: [EditorSelection; 4] = [
        EditorSelection::Color(StyleColorTarget::QuotaType),
        EditorSelection::Color(StyleColorTarget::Remaining),
        EditorSelection::Color(StyleColorTarget::ResetTime),
        EditorSelection::Color(StyleColorTarget::Error),
    ];
    const PROGRESS: [EditorSelection; 5] = [
        EditorSelection::CornerRadius,
        EditorSelection::Color(StyleColorTarget::ProgressHigh),
        EditorSelection::Color(StyleColorTarget::ProgressMedium),
        EditorSelection::Color(StyleColorTarget::ProgressLow),
        EditorSelection::Color(StyleColorTarget::ProgressConsumed),
    ];
    const INTERACTION: [EditorSelection; 1] =
        [EditorSelection::Color(StyleColorTarget::DragHandle)];

    match section {
        Section::General | Section::Preset | Section::Json => &[],
        Section::Panel => &PANEL,
        Section::Tooltip => &TOOLTIP,
        Section::Text => &TEXT,
        Section::Progress => &PROGRESS,
        Section::Interaction => &INTERACTION,
    }
}

pub(super) fn row_rect(hwnd: HWND, index: usize) -> RECT {
    rect(
        hwnd,
        200,
        218 + index as i32 * 50,
        940,
        260 + index as i32 * 50,
    )
}

pub(super) fn editor_top(section: Section) -> i32 {
    382 + (rows(section).len() as i32 - 3) * 50
}

pub(super) fn editor_box_rect(hwnd: HWND, section: Section) -> RECT {
    let top = editor_top(section);
    let mut editor = settings_card_rect(hwnd, top, top + SETTINGS_EDITOR_HEIGHT);
    let mut client = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client); }
    let safe_bottom = client.bottom - scale(hwnd, 24);
    editor.bottom = editor.bottom.min(safe_bottom);
    editor
}

pub(super) fn color_slider_track_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let top = editor_top(section) + 42 + channel_index as i32 * SETTINGS_EDITOR_CHANNEL_GAP;
    rect(hwnd, 330, top, 790, top + 4)
}

pub(super) fn color_slider_hit_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let track = color_slider_track_rect(hwnd, section, channel_index);
    RECT {
        left: track.left - scale(hwnd, 8),
        top: track.top - scale(hwnd, 10),
        right: track.right + scale(hwnd, 8),
        bottom: track.bottom + scale(hwnd, 10),
    }
}

pub(super) fn blur_row_index(section: Section) -> Option<usize> {
    rows(section).iter().position(|row| {
        matches!(row, EditorSelection::Blur | EditorSelection::TooltipBlur)
    })
}

pub(super) fn blur_slider_track_rect(hwnd: HWND, section: Section) -> RECT {
    let row = row_rect(hwnd, blur_row_index(section).unwrap_or(0));
    let center_y = (row.top + row.bottom) / 2;
    RECT {
        left: scale(hwnd, 396),
        top: center_y - scale(hwnd, 2),
        right: scale(hwnd, 770),
        bottom: center_y + scale(hwnd, 2),
    }
}

pub(super) fn blur_slider_hit_rect(hwnd: HWND, section: Section) -> RECT {
    let track = blur_slider_track_rect(hwnd, section);
    RECT {
        left: track.left - scale(hwnd, 8),
        top: track.top - scale(hwnd, 12),
        right: track.right + scale(hwnd, 8),
        bottom: track.bottom + scale(hwnd, 12),
    }
}

pub(super) fn inline_numeric_frame_rect(hwnd: HWND, row_index: usize) -> RECT {
    let row = row_rect(hwnd, row_index);
    RECT {
        left: row.right - scale(hwnd, 162),
        top: row.top + scale(hwnd, 6),
        right: row.right - scale(hwnd, 12),
        bottom: row.bottom - scale(hwnd, 6),
    }
}

pub(super) fn blur_edit_frame_rect(hwnd: HWND, section: Section) -> RECT {
    inline_numeric_frame_rect(hwnd, blur_row_index(section).unwrap_or(0))
}

pub(super) fn blur_edit_rect(hwnd: HWND, section: Section) -> RECT {
    let frame = blur_edit_frame_rect(hwnd, section);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 28),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

pub(super) fn blur_suffix_rect(hwnd: HWND, section: Section) -> RECT {
    let frame = blur_edit_frame_rect(hwnd, section);
    RECT {
        left: frame.right - scale(hwnd, 28),
        top: frame.top,
        right: frame.right - scale(hwnd, 8),
        bottom: frame.bottom,
    }
}

pub(super) fn corner_edit_frame_rect(hwnd: HWND) -> RECT {
    inline_numeric_frame_rect(hwnd, 0)
}

pub(super) fn corner_edit_rect(hwnd: HWND) -> RECT {
    let frame = corner_edit_frame_rect(hwnd);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 32),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

pub(super) fn corner_suffix_rect(hwnd: HWND) -> RECT {
    let frame = corner_edit_frame_rect(hwnd);
    RECT {
        left: frame.right - scale(hwnd, 32),
        top: frame.top,
        right: frame.right - scale(hwnd, 8),
        bottom: frame.bottom,
    }
}

pub(super) fn color_row_index(section: Section, target: StyleColorTarget) -> Option<usize> {
    rows(section)
        .iter()
        .position(|row| *row == EditorSelection::Color(target))
}

pub(super) fn hex_edit_frame_rect(hwnd: HWND, row_index: usize) -> RECT {
    let row = row_rect(hwnd, row_index);
    RECT {
        left: row.right - scale(hwnd, 162),
        top: row.top + scale(hwnd, 6),
        right: row.right - scale(hwnd, 12),
        bottom: row.bottom - scale(hwnd, 6),
    }
}

pub(super) fn hex_edit_rect(hwnd: HWND, row_index: usize) -> RECT {
    let frame = hex_edit_frame_rect(hwnd, row_index);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 3),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

pub(super) fn numeric_edit_frame_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let top = editor_top(section) + 29 + channel_index as i32 * SETTINGS_EDITOR_CHANNEL_GAP;
    rect(hwnd, 812, top, 920, top + 26)
}

// Shared helpers relocated verbatim from the settings parent module.

pub(super) fn numeric_edit_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let frame = numeric_edit_frame_rect(hwnd, section, channel_index);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 3),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

// A standard single-line EDIT only supports horizontal ES_CENTER; EM_SETRECT
// does not change its vertical text position. Center the native child window
// within our larger, separately painted rounded input background instead.
// Keep full horizontal bounds so caret/selection and horizontal scrolling work.
pub(super) fn centered_single_line_edit_rect(bounds: RECT, font_height: i32, padding: i32) -> RECT {
    let available = (bounds.bottom - bounds.top).max(0);
    if available == 0 {
        return bounds;
    }
    let height = font_height.saturating_add(padding).clamp(1, available);
    let top = bounds.top + (available - height) / 2;
    RECT { top, bottom: top + height, ..bounds }
}

pub(super) fn slider_value_from_x(track: RECT, x: i32, max: u8) -> u8 {
    let width = (track.right - track.left).max(1);
    let pos = (x.clamp(track.left, track.right) - track.left) as i64;
    ((pos * i64::from(max) + i64::from(width / 2)) / i64::from(width))
        .clamp(0, i64::from(max)) as u8
}

pub(super) fn corner_radius_max_for_section(
    preset: AppearancePreset,
    section: Section,
) -> Option<u8> {
    match section {
        Section::Panel => Some(panel_corner_radius_max(preset)),
        Section::Tooltip => Some(tooltip_corner_radius_max()),
        Section::Progress => Some(progress_corner_radius_max(preset)),
        _ => None,
    }
}
