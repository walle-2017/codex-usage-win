//! Settings hit-target actions, including JSON discard confirmation ordering.
use super::*;

pub(super) fn activate_target(hwnd: HWND, target: HitTarget) {
    match target {
        HitTarget::Theme(mode) => {
            send_parent(
                WM_STYLE_THEME_CHANGE,
                match mode {
                    ThemeMode::System => 0,
                    ThemeMode::Dark => 1,
                    ThemeMode::Light => 2,
                },
                0,
            );
        }
        HitTarget::Layout(preset) => {
            send_parent(
                WM_STYLE_LAYOUT_CHANGE,
                if preset == AppearancePreset::Minimal { 1 } else { 0 },
                0,
            );
        }
        HitTarget::Position(position) => {
            send_parent(
                WM_STYLE_POSITION_CHANGE,
                if position == DisplayPosition::Desktop { 1 } else { 0 },
                0,
            );
        }
        HitTarget::DesktopDirection(direction) => {
            send_parent(
                WM_STYLE_DESKTOP_DIRECTION_CHANGE,
                if direction == DesktopDirection::Vertical { 1 } else { 0 }, 0,
            );
        }
        HitTarget::DesktopAutoCollapse(enabled) => {
            send_parent(WM_STYLE_DESKTOP_AUTO_COLLAPSE_CHANGE, usize::from(enabled), 0);
        }
        HitTarget::Preset(preset) => {
            let index = match preset {
                ThemePreset::Classic => 0,
                ThemePreset::Ocean => 1,
                ThemePreset::Forest => 2,
            };
            send_parent(WM_STYLE_PRESET_CHANGE, index, 0);
        }
        HitTarget::Section(section) => set_section(section),
        HitTarget::Row(editor) => select_editor(editor),
        HitTarget::Refresh(interval) => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.snapshot.editable_settings.general.refresh_interval =
                        match interval {
                            60_000 => "1m",
                            300_000 => "5m",
                            900_000 => "15m",
                            _ => "1h",
                        }
                        .to_string();
                }
            }
            send_parent(WM_SETTINGS_REFRESH_CHANGE, interval as usize, 0);
        }
        HitTarget::UsageSession | HitTarget::UsageWeekly => {
            let mask = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return;
                };
                let usage = &mut s.snapshot.editable_settings.general.show_usage;
                match target {
                    HitTarget::UsageSession if usage.weekly || !usage.session_5h => {
                        usage.session_5h = !usage.session_5h;
                    }
                    HitTarget::UsageWeekly if usage.session_5h || !usage.weekly => {
                        usage.weekly = !usage.weekly;
                    }
                    _ => {}
                }
                usize::from(usage.session_5h) | (usize::from(usage.weekly) << 1)
            };
            send_parent(WM_SETTINGS_USAGE_CHANGE, mask, 0);
        }
        HitTarget::Alert(threshold) => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.snapshot.editable_settings.general.quota_alert_percent = threshold;
                }
            }
            send_parent(WM_SETTINGS_ALERT_CHANGE, threshold as usize, 0);
        }
        HitTarget::Startup => {
            let enabled = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return;
                };
                let value = !s.snapshot.editable_settings.general.start_with_windows;
                s.snapshot.editable_settings.general.start_with_windows = value;
                value
            };
            send_parent(WM_SETTINGS_STARTUP_CHANGE, usize::from(enabled), 0);
        }
        HitTarget::DiscardChanges => {
            let action = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return;
                };
                let action = s.pending_discard_action.take();
                // JSON reload/import does not discard edits until the operation
                // actually succeeds. Cancelling the file chooser keeps the draft.
                if !matches!(action, Some(PendingDiscardAction::Json(_))) {
                    s.json_dirty = false;
                    s.json_import_pending_apply = false;
                    s.json_status.clear();
                    s.json_status_path = None;
                }
                action
            };
            match action {
                Some(PendingDiscardAction::SwitchSection(section)) => set_section(section),
                Some(PendingDiscardAction::Close) => unsafe {
                    send_parent(WM_STYLE_SAVE, 0, 0);
                    let _ = DestroyWindow(hwnd);
                },
                Some(PendingDiscardAction::Json(action)) => {
                    // The confirmation overlay hid RichEdit. Restore the page
                    // before scheduling Reload/Import so cancelling the file
                    // picker returns to an editable, unchanged draft.
                    layout_settings_children(hwnd);
                    redraw_settings_window(hwnd);
                    begin_json_action_confirmed(hwnd, action);
                }
                None => {}
            }
        }
        HitTarget::KeepEditing => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.pending_discard_action = None;
                    s.pressed = None;
                    s.hovered = None;
                }
            }
            layout_settings_children(hwnd);
            redraw_settings_window(hwnd);
        }
        HitTarget::JsonStatusPath => open_json_status_path(),
        HitTarget::Json(action) => {
            if action == JsonAction::Apply {
                let enabled = STATE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .is_some_and(|s| json_action_enabled(action, s.json_dirty, s.json_import_pending_apply));
                if enabled {
                    begin_json_apply(hwnd);
                }
            } else {
                begin_json_action(hwnd, action);
            }
        },
        HitTarget::LanguageOption(index) => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.snapshot.editable_settings.general.language = language_code_for_index(index);
                }
            }
            send_parent(WM_SETTINGS_LANGUAGE_CHANGE, index, 0);
        }
    }
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}
