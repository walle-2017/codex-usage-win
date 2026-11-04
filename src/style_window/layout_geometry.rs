//! Shared geometry for the sidebar, General and appearance choices.
//! Rendering and input hit tests intentionally use these same original rectangles.

use super::*;

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
const APPEARANCE_BUTTON_LEFT: i32 = 314;
const APPEARANCE_BUTTON_WIDTH: i32 = 108;
const APPEARANCE_BUTTON_HEIGHT: i32 = 34;

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


