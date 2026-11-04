//! General settings-page painting: connected choices, usage checkboxes and
//! refresh/alert/startup/language controls.
//!
//! The original rendering order, localization, selection backgrounds and
//! hit-target geometry are retained verbatim. Input routing, persistence,
//! window state and HWND ownership remain in the parent settings module.

use super::*;

// Connected single-choice controls: selected/hover backgrounds have square
// inner edges, and the complete group has one rounded exterior and outline.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_general_segment_group(
    hdc: HDC, hwnd: HWND, bounds: RECT, choices: &[(HitTarget, bool, &str)],
    hovered: Option<HitTarget>, pressed: Option<HitTarget>,
    normal: Color, hover: Color, pressed_color: Color,
    accent: Color, accent_hover: Color, accent_pressed: Color, primary: Color,
) {
    if choices.is_empty() { return; }
    let radius = scale(hwnd, 9);
    fill_rounded_rect(hdc, bounds, normal, radius);
    for (index, &(target, selected, label)) in choices.iter().enumerate() {
        let option = general_segment_rect(bounds, index, choices.len());
        let background = button_background(target, selected, hovered, pressed,
            ButtonPalette {
                normal, hover, pressed: pressed_color,
                selected: accent, selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        );
        if selected || hovered == Some(target) || pressed == Some(target) {
            fill_rounded_rect(hdc, option, background, radius);
            if index != 0 {
                fill(hdc, RECT { right: option.left + radius, ..option }, background);
            }
            if index + 1 != choices.len() {
                fill(hdc, RECT { left: option.right - radius, ..option }, background);
            }
        }
        let _ = SetTextColor(hdc, COLORREF(
            if selected { Color::from_hex("#FFFFFFFF") } else { primary }.to_colorref()
        ));
        draw_text(hdc, label, option, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    let separator = subtle_control_border(normal);
    for index in 1..choices.len() {
        let x = general_segment_rect(bounds, index, choices.len()).left;
        fill(hdc, RECT {
            left: x, top: bounds.top + scale(hwnd, 5),
            right: x + scale(hwnd, 1), bottom: bounds.bottom - scale(hwnd, 5),
        }, separator);
    }
    draw_rounded_outline_rect(hdc, bounds, separator, radius, 1);
}

// Each usage type is a checkbox; the existing persistence guard still keeps
// at least one quota window enabled when either box is clicked.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_general_usage_checkbox(
    hdc: HDC, hwnd: HWND, hit: RECT, target: HitTarget,
    enabled: bool, hovered: Option<HitTarget>, pressed: Option<HitTarget>,
    card: Color, card_hover: Color, accent: Color, primary: Color, secondary: Color,
    label: &str,
) {
    if hovered == Some(target) || pressed == Some(target) {
        fill_rounded_rect(hdc, hit, card_hover, scale(hwnd, 7));
    }
    let size = scale(hwnd, 20);
    let left = hit.left + scale(hwnd, 14);
    let top = hit.top + (hit.bottom - hit.top - size) / 2;
    let checkbox = RECT { left, top, right: left + size, bottom: top + size };
    fill_rounded_rect(hdc, checkbox, if enabled { accent } else { card }, scale(hwnd, 5));
    draw_rounded_outline_rect(
        hdc, checkbox, if enabled { accent } else { subtle_control_border(card_hover) },
        scale(hwnd, 5), 1,
    );
    if enabled {
        let _ = SetTextColor(hdc, COLORREF(Color::from_hex("#FFFFFFFF").to_colorref()));
        draw_text(hdc, "✓", checkbox, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
    let _ = SetTextColor(hdc, COLORREF(if enabled { primary } else { secondary }.to_colorref()));
    draw_text(hdc, label, RECT {
        left: checkbox.right + scale(hwnd, 14), ..hit
    }, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_general_page(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    accent: Color,
    accent_hover: Color,
    accent_pressed: Color,
    primary: Color,
    secondary: Color,
) {
    let zh = snapshot.language == LanguageId::SimplifiedChinese;
    let strings = snapshot.language.strings();
    let general = &snapshot.editable_settings.general;
    let current_interval = match general.refresh_interval.as_str() {
        "1m" => 60_000,
        "5m" => 300_000,
        "1h" => 3_600_000,
        _ => 900_000,
    };
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "常规" } else { "General" },
        settings_page_title_rect(hwnd),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    for r in [
        settings_card_rect(hwnd, 58, 112),
        settings_card_rect(hwnd, 128, 182),
        general_usage_card_rect(hwnd),
        general_application_card_rect(hwnd),
    ] {
        fill_rounded_rect(hdc, r, card, scale(hwnd, SETTINGS_CARD_RADIUS));
    }

    // Connected radio groups share button geometry with their hit targets.
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc, strings.update_frequency, settings_card_label_rect(hwnd, 68, 102),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_general_segment_group(
        hdc, hwnd, general_refresh_group_rect(hwnd),
        &[
            (HitTarget::Refresh(60_000), current_interval == 60_000, strings.one_minute),
            (HitTarget::Refresh(300_000), current_interval == 300_000, strings.five_minutes),
            (HitTarget::Refresh(900_000), current_interval == 900_000, strings.fifteen_minutes),
            (HitTarget::Refresh(3_600_000), current_interval == 3_600_000, strings.one_hour),
        ],
        hovered, pressed, card_hover, card_pressed, card_pressed,
        accent, accent_hover, accent_pressed, primary,
    );

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc, if zh { "额度提醒" } else { "Quota alerts" },
        settings_card_label_rect(hwnd, 138, 172),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_general_segment_group(
        hdc, hwnd, general_alert_group_rect(hwnd),
        &[
            (HitTarget::Alert(0), general.quota_alert_percent == 0,
                if zh { "关闭" } else { "Off" }),
            (HitTarget::Alert(10), general.quota_alert_percent == 10, "10%"),
            (HitTarget::Alert(20), general.quota_alert_percent == 20, "20%"),
            (HitTarget::Alert(30), general.quota_alert_percent == 30, "30%"),
        ],
        hovered, pressed, card_hover, card_pressed, card_pressed,
        accent, accent_hover, accent_pressed, primary,
    );

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc, if zh { "显示用量" } else { "Usage display" },
        rect(hwnd, 218, 212, 544, 240),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (target, weekly, enabled, label) in [
        (HitTarget::UsageSession, false, general.show_usage.session_5h, "5H"),
        (HitTarget::UsageWeekly, true, general.show_usage.weekly, "7D"),
    ] {
        draw_general_usage_checkbox(
            hdc, hwnd, general_usage_rect(hwnd, weekly),
            target, enabled, hovered, pressed,
            card, card_hover, accent, primary, secondary, label,
        );
    }

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc, if zh { "应用" } else { "Application" },
        rect(hwnd, 596, 212, 922, 240),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc, strings.start_with_windows,
        rect(hwnd, 596, 246, 854, 286),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let startup = general.start_with_windows;
    draw_switch(
        hdc, general_startup_rect(hwnd), startup,
        button_background(
            HitTarget::Startup, startup, hovered, pressed,
            ButtonPalette {
                normal: card_hover, hover: card_pressed, pressed: card_pressed,
                selected: accent, selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        ),
    );

    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc, strings.language, rect(hwnd, 596, 302, 700, 338),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let selected_language = if general.language == "system" {
        snapshot.language
    } else {
        LanguageId::from_code(&general.language)
            .map(LanguageId::ui_supported)
            .unwrap_or(snapshot.language)
    };
    let language_choices: Vec<(HitTarget, bool, &str)> = LanguageId::SELECTABLE
        .iter().enumerate()
        .map(|(index, language)| (
            HitTarget::LanguageOption(index),
            *language == selected_language,
            language_label_for_index(index, snapshot.language),
        ))
        .collect();
    draw_general_segment_group(
        hdc, hwnd, general_language_group_rect(hwnd), &language_choices,
        hovered, pressed, card_hover, card_pressed, card_pressed,
        accent, accent_hover, accent_pressed, primary,
    );
}

