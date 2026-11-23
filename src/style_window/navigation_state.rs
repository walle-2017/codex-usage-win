//! Settings navigation transitions and editor selection.
//! Preserve the JSON discard guard and compositing order across page switches.
use super::*;

pub(super) fn set_section(section: Section) {
    let (current_section, dirty, hwnd, discard_pending) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.section,
            json_has_pending_changes(s.json_dirty, s.json_import_pending_apply),
            s.hwnd.to_hwnd(),
            s.pending_discard_action.is_some(),
        )
    };
    if discard_pending || current_section == section {
        return;
    }
    if current_section == Section::Json && section != Section::Json && dirty {
        request_discard_confirmation(hwnd, PendingDiscardAction::SwitchSection(section));
        return;
    }

    let (hwnd, json_dirty) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.section = section;
        if let Some(editor) = rows(section).first().copied() {
            s.editor = editor;
        } else {
            s.focused_numeric_edit = None;
            s.focused_blur_edit = false;
            s.focused_corner_edit = false;
            s.focused_hex_edit = None;
        }
        s.dragging_slider = None;
        s.pressed = None;
        s.hovered = None;
        (s.hwnd.to_hwnd(), json_has_pending_changes(s.json_dirty, s.json_import_pending_apply))
    };
    // Restore buffering before hiding JSON and drawing a non-JSON page.
    // Entering JSON takes the inverse path: the opaque pane and RichEdit
    // are first shown together, then composition is disabled after painting.
    if current_section == Section::Json && section != Section::Json {
        sync_settings_window_composition(hwnd, section);
    }
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    layout_settings_children(hwnd);
    if section == Section::Json {
        if json_dirty {
            refresh_json_editor_theme();
        } else {
            reload_json_editor_from_snapshot();
        }
    }
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
    sync_corner_edit();
    // Finish the one-time page transition while composited, preventing
    // remnants from the previous page beneath the RichEdit sibling.
    redraw_settings_window(hwnd);
    if section == Section::Json {
        sync_settings_window_composition(hwnd, section);
    }
}

pub(super) fn select_editor(editor: EditorSelection) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.editor = editor;
        s.dragging_slider = None;
        s.hwnd.to_hwnd()
    };
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
    sync_corner_edit();
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}
