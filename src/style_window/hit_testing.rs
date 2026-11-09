//! Shared settings hit testing for mouse, wheel and cursor interactions.
//! Preserve original option priority and state-lock scope.

use super::*;

pub(super) fn hit_target_at(hwnd: HWND, x: i32, y: i32) -> Option<HitTarget> {
    let discard_pending = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref()?.pending_discard_action.is_some()
    };
    if discard_pending {
        if pt_in_rect(discard_dialog_button_rect(hwnd, true), x, y) {
            return Some(HitTarget::DiscardChanges);
        }
        if pt_in_rect(discard_dialog_button_rect(hwnd, false), x, y) {
            return Some(HitTarget::KeepEditing);
        }
        return None;
    }

    let (section, json_dirty, json_import_pending_apply, json_status_path, language, json_action_pending) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.section, s.json_dirty, s.json_import_pending_apply, s.json_status_path.is_some(),
            s.snapshot.language, s.pending_json_action.is_some())
    };

    for item in [
        Section::General,
        Section::Preset,
        Section::Panel,
        Section::Tooltip,
        Section::Text,
        Section::Progress,
        Section::Interaction,
        Section::Json,
    ] {
        if pt_in_rect(section_rect(hwnd, item), x, y) {
            return Some(HitTarget::Section(item));
        }
    }

    if is_appearance_section(section) {
        for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
            if pt_in_rect(theme_rect(hwnd, mode), x, y) {
                return Some(HitTarget::Theme(mode));
            }
        }
        for preset in [AppearancePreset::Default, AppearancePreset::Minimal] {
            if pt_in_rect(layout_rect(hwnd, preset), x, y) {
                return Some(HitTarget::Layout(preset));
            }
        }
        for position in [DisplayPosition::Taskbar, DisplayPosition::Desktop] {
            if pt_in_rect(position_rect(hwnd, position), x, y) {
                return Some(HitTarget::Position(position));
            }
        }
        let submenu = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().and_then(|s| (
                s.snapshot.display_position == DisplayPosition::Desktop
                    && s.desktop_submenu.hit_enabled()
            ).then_some(s.snapshot.desktop_auto_collapse))
        };
        if let Some(collapsed) = submenu {
            if pt_in_rect(desktop_auto_collapse_rect(hwnd), x, y) {
                return Some(HitTarget::DesktopAutoCollapse(!collapsed));
            }
            for direction in [DesktopDirection::Horizontal, DesktopDirection::Vertical] {
                if pt_in_rect(desktop_direction_rect(hwnd, direction), x, y) {
                    return Some(HitTarget::DesktopDirection(direction));
                }
            }
        }
        if section == Section::Preset {
            for preset in ThemePreset::ALL {
                if pt_in_rect(preset_card_rect(hwnd, preset), x, y) {
                    return Some(HitTarget::Preset(preset));
                }
            }
        }
        for (index, editor) in rows(section).iter().copied().enumerate() {
            if pt_in_rect(row_rect(hwnd, index), x, y) {
                return Some(HitTarget::Row(editor));
            }
        }
    }

    if section == Section::General {
        for index in 0..language_option_count() {
            if pt_in_rect(language_option_rect(hwnd, index), x, y) {
                return Some(HitTarget::LanguageOption(index));
            }
        }
        for interval in [60_000u32, 300_000, 900_000, 3_600_000] {
            if pt_in_rect(general_refresh_rect(hwnd, interval), x, y) {
                return Some(HitTarget::Refresh(interval));
            }
        }
        if pt_in_rect(general_usage_rect(hwnd, false), x, y) {
            return Some(HitTarget::UsageSession);
        }
        if pt_in_rect(general_usage_rect(hwnd, true), x, y) {
            return Some(HitTarget::UsageWeekly);
        }
        for threshold in [0u8, 10, 20, 30] {
            if pt_in_rect(general_alert_rect(hwnd, threshold), x, y) {
                return Some(HitTarget::Alert(threshold));
            }
        }
        if pt_in_rect(general_startup_rect(hwnd), x, y) {
            return Some(HitTarget::Startup);
        }
    }

    if section == Section::Json {
        if json_status_path && pt_in_rect(json_status_path_rect(hwnd, language), x, y) {
            return Some(HitTarget::JsonStatusPath);
        }
        for action in [
            JsonAction::Reload,
            JsonAction::Format,
            JsonAction::Import,
            JsonAction::Export,
            JsonAction::Apply,
        ] {
            if json_action_pending || !json_action_enabled(action, json_dirty, json_import_pending_apply) {
                continue;
            }
            if pt_in_rect(json_action_rect(hwnd, action), x, y) {
                return Some(HitTarget::Json(action));
            }
        }
    }

    None
}

pub(super) fn slider_kind_at(hwnd: HWND, x: i32, y: i32) -> Option<SliderKind> {
    let (section, editor) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.section, s.editor)
    };

    if matches!(section, Section::Panel | Section::Tooltip) && pt_in_rect(blur_slider_hit_rect(hwnd, section), x, y) {
        return Some(SliderKind::Blur);
    }

    if !matches!(
        section,
        Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
    ) {
        return None;
    }

    let EditorSelection::Color(_) = editor else {
        return None;
    };
    for index in 0..4 {
        if pt_in_rect(color_slider_hit_rect(hwnd, section, index), x, y) {
            return Some(match index {
                0 => SliderKind::Red,
                1 => SliderKind::Green,
                2 => SliderKind::Blue,
                _ => SliderKind::Alpha,
            });
        }
    }
    None
}

pub(super) fn wheel_numeric_target_at(
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Option<WheelNumericTarget> {
    let (section, editor) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.section, s.editor)
    };

    if matches!(
        section,
        Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
    ) && matches!(editor, EditorSelection::Color(_))
    {
        for index in 0..4 {
            if pt_in_rect(numeric_edit_frame_rect(hwnd, section, index), x, y) {
                return Some(WheelNumericTarget::Color(index));
            }
        }
    }

    if matches!(section, Section::Panel | Section::Tooltip)
        && pt_in_rect(blur_edit_frame_rect(hwnd, section), x, y)
    {
        return Some(WheelNumericTarget::Blur);
    }

    if matches!(section, Section::Panel | Section::Tooltip | Section::Progress)
        && pt_in_rect(corner_edit_frame_rect(hwnd), x, y)
    {
        return Some(WheelNumericTarget::Corner);
    }

    None
}
