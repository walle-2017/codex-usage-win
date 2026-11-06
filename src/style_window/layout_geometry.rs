//! Shared geometry for the sidebar, General and appearance choices.
//! Rendering and input hit tests intentionally use these same original rectangles.

use super::*;

// Shared DPI scaling, rectangle construction and signed pointer coordinates.
pub(super) fn window_dpi(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd).max(96) }
}

pub(super) fn scale(hwnd: HWND, value: i32) -> i32 {
    let dpi = window_dpi(hwnd);
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

pub(super) fn rect(hwnd: HWND, left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left: scale(hwnd, left),
        top: scale(hwnd, top),
        right: scale(hwnd, right),
        bottom: scale(hwnd, bottom),
    }
}

pub(super) fn point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

pub(super) fn pt_in_rect(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

pub(super) fn settings_page_title_rect(hwnd: HWND) -> RECT {
    rect(
        hwnd,
        SETTINGS_CONTENT_LEFT,
        18,
        SETTINGS_CONTENT_RIGHT,
        46,
    )
}

pub(super) fn settings_card_rect(hwnd: HWND, top: i32, bottom: i32) -> RECT {
    rect(
        hwnd,
        SETTINGS_CONTENT_LEFT,
        top,
        SETTINGS_CONTENT_RIGHT,
        bottom,
    )
}

pub(super) fn settings_card_label_rect(hwnd: HWND, top: i32, bottom: i32) -> RECT {
    rect(
        hwnd,
        SETTINGS_CONTENT_LEFT + SETTINGS_CARD_INSET_X,
        top,
        SETTINGS_CHOICE_LABEL_RIGHT,
        bottom,
    )
}

// One shared segment formula for painting and hit tests.
pub(super) fn general_segment_rect(bounds: RECT, index: usize, count: usize) -> RECT {
    let width = bounds.right - bounds.left;
    RECT {
        left: bounds.left + width * index as i32 / count as i32,
        right: bounds.left + width * (index as i32 + 1) / count as i32,
        ..bounds
    }
}

pub(super) fn general_refresh_group_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 360, 68, 920, 102)
}

pub(super) fn general_alert_group_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 360, 138, 920, 172)
}

pub(super) fn general_usage_card_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 200, 198, 562, 378)
}

pub(super) fn general_application_card_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 578, 198, 940, 378)
}

pub(super) fn general_language_group_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 708, 302, 922, 338)
}

pub(super) fn section_rect(hwnd: HWND, section: Section) -> RECT {
    let (top, bottom) = match section {
        Section::General => (28, 68),
        Section::Preset => (132, 172),
        Section::Panel => (176, 216),
        Section::Tooltip => (220, 260),
        Section::Text => (264, 304),
        Section::Progress => (308, 348),
        Section::Interaction => (352, 392),
        Section::Json => (456, 496),
    };
    rect(hwnd, 20, top, 166, bottom)
}


// Appearance options use the same adjacent segments for painting and hit
// testing. The left edge and row spacing match the existing settings card.
pub(super) const APPEARANCE_BUTTON_LEFT: i32 = 314;
pub(super) const APPEARANCE_BUTTON_WIDTH: i32 = 108;
pub(super) const APPEARANCE_BUTTON_HEIGHT: i32 = 34;

pub(super) fn appearance_group_rect(hwnd: HWND, top: i32, count: usize) -> RECT {
    rect(hwnd, APPEARANCE_BUTTON_LEFT, top,
        APPEARANCE_BUTTON_LEFT + APPEARANCE_BUTTON_WIDTH * count as i32,
        top + APPEARANCE_BUTTON_HEIGHT)
}

pub(super) fn appearance_button_rect(hwnd: HWND, column: i32, top: i32) -> RECT {
    general_segment_rect(
        appearance_group_rect(hwnd, top, if top == 70 { 3 } else { 2 }),
        column as usize, if top == 70 { 3 } else { 2 },
    )
}

pub(super) fn theme_rect(hwnd: HWND, mode: ThemeMode) -> RECT {
    let column = match mode {
        ThemeMode::System => 0,
        ThemeMode::Dark => 1,
        ThemeMode::Light => 2,
    };
    appearance_button_rect(hwnd, column, 70)
}

pub(super) fn position_rect(hwnd: HWND, position: DisplayPosition) -> RECT {
    let column = if position == DisplayPosition::Desktop { 1 } else { 0 };
    appearance_button_rect(hwnd, column, 162)
}

pub(super) fn desktop_direction_rect(hwnd: HWND, direction: DesktopDirection) -> RECT {
    let layout = desktop_submenu_layout();
    scaled_rect(hwnd, if direction == DesktopDirection::Vertical {
        layout.vertical
    } else {
        layout.horizontal
    })
}

// A single switch hit area; the desired state is derived from the current
// snapshot when clicked, rather than exposing separate On and Off buttons.
pub(super) fn desktop_auto_collapse_rect(hwnd: HWND) -> RECT {
    scaled_rect(hwnd, desktop_submenu_layout().collapse_switch)
}

pub(super) fn layout_rect(hwnd: HWND, preset: AppearancePreset) -> RECT {
    let column = match preset {
        AppearancePreset::Default => 0,
        AppearancePreset::Minimal => 1,
    };
    appearance_button_rect(hwnd, column, 116)
}

pub(super) fn settings_centered_origin(work: RECT, width: i32, height: i32) -> (i32, i32) {
    let work_width = (i64::from(work.right) - i64::from(work.left)).max(0);
    let work_height = (i64::from(work.bottom) - i64::from(work.top)).max(0);
    let x = i64::from(work.left) + (work_width - i64::from(width).max(1)).max(0) / 2;
    let y = i64::from(work.top) + (work_height - i64::from(height).max(1)).max(0) / 2;
    (
        x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
    )
}

pub(super) fn desktop_submenu_layout() -> DesktopSubmenuLayout {
    let row_height = 30;
    let row = |left, right, top| RECT {
        left, top, right, bottom: top + row_height,
    };
    DesktopSubmenuLayout {
        well: RECT { left: 652, top: 66, right: 924, bottom: 198 },
        title: RECT { left: 668, top: 76, right: 904, bottom: 96 },
        direction_label: row(668, 736, 106),
        collapse_label: row(668, 736, 152),
        horizontal: row(742, 823, 106),
        vertical: row(823, 904, 106),
        collapse_switch: row(850, 904, 152),
    }
}

pub(super) fn scaled_rect(hwnd: HWND, bounds: RECT) -> RECT {
    rect(hwnd, bounds.left, bounds.top, bounds.right, bounds.bottom)
}

pub(super) fn settings_work_area_for_widget(widget: HWND) -> Option<RECT> {
    if widget.0.is_null() || !unsafe { IsWindow(widget).as_bool() } {
        return None;
    }
    unsafe {
        let monitor = MonitorFromWindow(widget, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).as_bool().then_some(info.rcWork)
    }
}

pub(super) fn position_is_only_snapshot_change(
    old: &StyleWindowSnapshot, new: &StyleWindowSnapshot,
) -> bool {
    if old.display_position == new.display_position {
        return false;
    }
    let mut normalized = new.editable_settings.clone();
    normalized.appearance.position.clone_from(&old.editable_settings.appearance.position);
    old.language == new.language
        && old.theme_mode == new.theme_mode
        && old.is_dark == new.is_dark
        && old.appearance_preset == new.appearance_preset
        && old.desktop_direction == new.desktop_direction
        && old.desktop_auto_collapse == new.desktop_auto_collapse
        // Vertical taskbar preview orientation only affects the lower Preset
        // gallery, which is refreshed separately after the card animation.
        && old.active_style == new.active_style
        && old.editable_settings == normalized
}
