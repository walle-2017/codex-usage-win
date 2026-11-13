//! JSON action scheduling, validation, formatting and apply operations.
//! Preserve timer order, state locking, unsaved feedback and RichEdit behavior.

use super::*;

pub(super) fn begin_json_apply(hwnd: HWND) {
    let (edit, language, already_running) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.json_edit.to_hwnd(),
            s.snapshot.language,
            s.pending_json_action.is_some(),
        )
    };
    if already_running {
        return;
    }

    let raw = read_large_edit_text(edit);
    match parse_jsonc(&raw) {
        Ok(_) => begin_json_action(hwnd, JsonAction::Apply),
        Err(error) => show_json_apply_validation_error(hwnd, edit, language, &error),
    }
}

pub(super) fn begin_json_action(hwnd: HWND, action: JsonAction) {
    let (already_pending, dirty, import_pending_apply) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.pending_json_action.is_some() || s.pending_discard_action.is_some(),
            s.json_dirty,
            s.json_import_pending_apply,
        )
    };
    if already_pending || !json_action_enabled(action, dirty, import_pending_apply) {
        return;
    }
    if json_action_needs_confirmation(action, dirty, import_pending_apply) {
        request_discard_confirmation(hwnd, PendingDiscardAction::Json(action));
        return;
    }
    begin_json_action_confirmed(hwnd, action);
}

// Called only after confirming destructive Reload/Import, or for actions
// that do not discard editor contents. Do not hold STATE across modal dialogs.
pub(super) fn begin_json_action_confirmed(hwnd: HWND, action: JsonAction) {
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.pending_json_action.is_some() {
            return;
        }
        s.pending_json_action = Some(action);
        if action == JsonAction::Apply {
            s.json_save_feedback = Some(JsonSaveFeedback::Saving);
            s.json_error_shake_step = 0;
        } else {
            s.json_save_feedback = None;
            s.json_status = json_action_running_text(action, s.snapshot.language).to_string();
            s.json_status_path = None;
        }
    }
    if action == JsonAction::Apply {
        unsafe {
            let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        }
        let _ = ensure_json_save_mask(hwnd);
        layout_settings_children(hwnd);
    }
    unsafe {
        let _ = SetTimer(hwnd, JSON_ACTION_TIMER_ID, JSON_ACTION_DELAY_MS, None);
        let _ = InvalidateRect(hwnd, None, false);
        let _ = UpdateWindow(hwnd);
    }
}

pub(super) fn finish_pending_json_action(hwnd: HWND) {
    let action = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().and_then(|s| s.pending_json_action)
    };
    let Some(action) = action else {
        return;
    };
    unsafe {
        let _ = KillTimer(hwnd, JSON_ACTION_TIMER_ID);
    }

    if action == JsonAction::Apply {
        // Keep Apply marked pending while the synchronous parent handler updates
        // application state and calls style_window::sync(). That sync must update
        // the snapshot without touching/reloading the JSON RichEdit contents.
        apply_json_editor();
        {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = state.as_mut() {
                if s.pending_json_action == Some(JsonAction::Apply) {
                    s.pending_json_action = None;
                }
            }
        }
        layout_settings_children(hwnd);
        redraw_settings_window(hwnd);
        return;
    }

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.pending_json_action = None;
        }
    }
    match action {
        JsonAction::Reload => reload_json_editor_from_snapshot(),
        JsonAction::Format => format_json_editor(),
        JsonAction::Import => import_json_file(hwnd),
        JsonAction::Export => export_json_file(hwnd),
        JsonAction::Apply => unreachable!(),
    }
}

pub(super) fn schedule_json_validation(hwnd: HWND) {
    let should_schedule = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.syncing_json_edit {
            false
        } else {
            // Any user edit after a successful save replaces the saved indicator
            // with the normal unsaved-changes state immediately.
            s.json_save_feedback = None;
            // Operation results refer to the previous editor content. Clear
            // successful Reload/Format/Import/Export text and its link as soon
            // as the user makes a real edit, before deferred validation runs.
            if json_success_status_is_stale_on_edit(&s.json_status) {
                s.json_status.clear();
                s.json_status_path = None;
            }
            s.json_error_shake_step = 0;
            // Mark dirty immediately so closing the window before the debounce
            // fires still triggers the unsaved-changes confirmation.
            s.json_dirty = true;
            s.json_import_pending_apply = false;
            true
        }
    };
    if !should_schedule {
        return;
    }

    unsafe {
        let _ = KillTimer(hwnd, JSON_VALIDATION_TIMER_ID);
        let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        let _ = SetTimer(
            hwnd,
            JSON_VALIDATION_TIMER_ID,
            JSON_VALIDATION_DELAY_MS,
            None,
        );
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn update_json_validation_status(_mark_dirty: bool) {
    let (edit, syncing, language, hwnd, applied_settings) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.json_edit.to_hwnd(),
            s.syncing_json_edit,
            s.snapshot.language,
            s.hwnd.to_hwnd(),
            s.snapshot.editable_settings.clone(),
        )
    };
    if syncing {
        return;
    }

    // Keep live validation read-only with respect to the RichEdit control.
    // Reformatting character ranges while handling a user's edit can re-enter
    // RichEdit internals and stall the process when an invalid document becomes
    // valid. Full syntax highlighting is still applied on reload/format/import
    // and when the editor theme is refreshed.
    let text = normalize_to_lf(&read_large_edit_text_raw(edit));
    let parsed = parse_jsonc(&text);
    let dirty = match &parsed {
        Ok(settings) => settings != &applied_settings,
        Err(_) => true,
    };

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.json_dirty = dirty;
            match parsed {
                Err(error) => {
                    let detail = friendly_json_error(&error, language);
                    s.json_status = json_error_status(language, &detail);
                    s.json_status_path = None;
                }
                Ok(_) => {
                    if s.json_status.starts_with('×') {
                        s.json_status.clear();
                        s.json_status_path = None;
                    }
                }
            }
        }
    }
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn format_json_editor() {
    let (edit, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.snapshot.language)
    };
    let raw = read_large_edit_text(edit);
    match parse_jsonc(&raw) {
        Ok(settings) => {
            let dirty = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .is_some_and(|s| settings != s.snapshot.editable_settings)
            };
            let text = settings.to_jsonc(language);
            let status = if language == LanguageId::SimplifiedChinese {
                "✓ 已格式化".to_string()
            } else {
                "✓ Formatted".to_string()
            };
            write_json_editor(&text, status, dirty);
        }
        Err(error) => {
            set_json_status(json_action_error(
                "无法格式化：请先修复配置错误",
                "Cannot format: fix the configuration first",
                &error,
                language,
            ));
            locate_json_error(edit, &error);
        }
    }
}

pub(super) fn apply_json_editor() {
    let (edit, language, hwnd) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.snapshot.language, s.hwnd.to_hwnd())
    };
    let raw = read_large_edit_text(edit);
    let settings = match parse_jsonc(&raw) {
        Ok(value) => value,
        Err(error) => {
            show_json_apply_validation_error(hwnd, edit, language, &error);
            return;
        }
    };
    {
        let mut pending = PENDING_EDITABLE_SETTINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *pending = Some(settings);
    }

    // WM_SETTINGS_JSON_APPLY is handled synchronously by the parent. Keep the
    // editor dirty until it returns so the parent's style_window::sync() cannot
    // reload the pre-apply snapshot into the RichEdit control.
    send_parent(WM_SETTINGS_JSON_APPLY, 0, 0);

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.json_dirty = false;
            s.json_import_pending_apply = false;
            s.json_save_feedback = Some(JsonSaveFeedback::Saved);
            if s.json_status.starts_with('×') {
                s.json_status.clear();
                s.json_status_path = None;
            }
        }
    }

    // The parent sync has already updated the snapshot/theme. Refresh syntax
    // colors in place, without replacing the user's JSON text or caret.
    refresh_json_editor_theme();
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}
