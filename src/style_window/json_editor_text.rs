//! JSON RichEdit text I/O, syntax highlighting and editor refresh operations.
//! Preserve selection, scroll position, notification mask and Win32 message ordering.

use super::*;

pub(super) fn read_large_edit_text_raw(edit: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(edit);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(edit, &mut buffer) as usize;
        String::from_utf16_lossy(&buffer[..copied])
    }
}

pub(super) fn read_large_edit_text(edit: HWND) -> String {
    normalize_to_lf(&read_large_edit_text_raw(edit))
}

pub(super) fn syntax_highlight_json_editor(edit: HWND, text: &str, is_dark: bool) {
    unsafe {
        let mut previous_selection = RichCharRange::default();
        let _ = SendMessageW(
            edit,
            EM_EXGETSEL_MSG,
            WPARAM(0),
            LPARAM((&mut previous_selection as *mut RichCharRange) as isize),
        );

        let mut previous_scroll = POINT::default();
        let _ = SendMessageW(
            edit,
            EM_GETSCROLLPOS_MSG,
            WPARAM(0),
            LPARAM((&mut previous_scroll as *mut POINT) as isize),
        );

        // Formatting changes the RichEdit selection repeatedly. Suspend change
        // notifications and painting so this cannot re-enter EN_CHANGE or expose
        // intermediate selection/scroll states while the user is typing.
        let previous_event_mask = SendMessageW(
            edit,
            EM_SETEVENTMASK_MSG,
            WPARAM(0),
            LPARAM(0),
        );
        let _ = SendMessageW(edit, WM_SETREDRAW, WPARAM(0), LPARAM(0));

        rich_set_selection(edit, 0, -1);
        rich_set_selected_color(edit, json_default_text_color(is_dark));

        for (start, end, kind) in jsonc_highlight_ranges(text) {
            rich_set_selection(edit, start, end);
            rich_set_selected_color(edit, json_token_color(kind, is_dark));
        }

        let _ = SendMessageW(
            edit,
            EM_SETBKGNDCOLOR_MSG,
            WPARAM(0),
            LPARAM(json_editor_background(is_dark).to_colorref() as isize),
        );
        let _ = SendMessageW(
            edit,
            EM_EXSETSEL_MSG,
            WPARAM(0),
            LPARAM((&mut previous_selection as *mut RichCharRange) as isize),
        );
        // Restoring the selection may scroll the caret into view, so restore
        // the viewport after the selection and before repainting.
        let _ = SendMessageW(
            edit,
            EM_SETSCROLLPOS_MSG,
            WPARAM(0),
            LPARAM((&previous_scroll as *const POINT) as isize),
        );
        let _ = SendMessageW(
            edit,
            EM_SETEVENTMASK_MSG,
            WPARAM(0),
            LPARAM(previous_event_mask.0),
        );
        let _ = SendMessageW(edit, WM_SETREDRAW, WPARAM(1), LPARAM(0));

        // Erase stale line pixels before repainting. A non-erasing invalidate can
        // leave duplicated/offset glyph fragments after an insertion shifts rows.
        let _ = InvalidateRect(edit, None, true);
        let _ = UpdateWindow(edit);
    }
}

pub(super) fn write_json_editor(text: &str, status: String, dirty: bool) {
    let (edit, hwnd, is_dark) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.syncing_json_edit = true;
        s.json_status = status;
        s.json_status_path = None;
        s.json_save_feedback = None;
        s.json_dirty = dirty;
        (s.json_edit.to_hwnd(), s.hwnd.to_hwnd(), s.snapshot.is_dark)
    };

    let normalized = normalize_to_lf(text);
    let windows_text = to_windows_newlines(&normalized);
    set_edit_text_string(edit, &windows_text);
    // RichEdit exposes CRLF through WM_GETTEXT, but its selection character
    // positions count each paragraph break as one character. Highlight against
    // LF-normalized text so token offsets match EM_EXSETSEL coordinates.
    syntax_highlight_json_editor(edit, &normalized, is_dark);

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_json_edit = false;
        }
    }
    layout_settings_children(hwnd);
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn refresh_json_editor_theme() {
    let (edit, hwnd, is_dark) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.hwnd.to_hwnd(), s.snapshot.is_dark)
    };
    let text = normalize_to_lf(&read_large_edit_text_raw(edit));
    syntax_highlight_json_editor(edit, &text, is_dark);
    layout_settings_children(hwnd);
}

pub(super) fn reload_json_editor_from_snapshot() {
    let (settings, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.snapshot.editable_settings.clone(), s.snapshot.language)
    };
    let text = settings.to_jsonc(language);
    let status = if language == LanguageId::SimplifiedChinese {
        "✓ 已重新载入".to_string()
    } else {
        "✓ Reloaded".to_string()
    };
    write_json_editor(&text, status, false);
}

pub(super) fn locate_json_error(edit: HWND, error: &str) {
    let Some((line, column)) = json_error_location(error) else {
        return;
    };
    unsafe {
        let line_index = SendMessageW(
            edit,
            EM_LINEINDEX_MSG,
            WPARAM(line.saturating_sub(1)),
            LPARAM(0),
        )
        .0 as i32;
        if line_index >= 0 {
            let start = line_index.saturating_add(column.saturating_sub(1) as i32);
            rich_set_selection(edit, start, start.saturating_add(1));
            let _ = SetFocus(edit);
        }
    }
}
