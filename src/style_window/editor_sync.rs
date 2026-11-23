//! Editor value synchronization between native inputs, the active theme, and Hex values.
//! Keep STATE unlocked while WM_SETTEXT sends synchronous edit notifications.

use super::*;

pub(super) fn sync_numeric_edits() {
    let (edits, values) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if !matches!(
            s.section,
            Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
        ) {
            return;
        }
        let EditorSelection::Color(target) = s.editor else {
            return;
        };
        let color = s.snapshot.active_style.color(target);
        s.syncing_numeric_edits = true;
        (s.numeric_edits, [color.r, color.g, color.b, color.a])
    };

    for (edit, value) in edits.iter().zip(values) {
        set_edit_text(edit.to_hwnd(), value);
    }

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_numeric_edits = false;
    }
}

pub(super) fn sync_blur_edit() {
    let (edit, value) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        let value = match s.section {
            Section::Panel => s.snapshot.active_style.panel_frosted_strength,
            Section::Tooltip => s.snapshot.active_style.tooltip_frosted_strength(),
            _ => return,
        };
        s.syncing_blur_edit = true;
        (s.blur_edit.to_hwnd(), value)
    };
    set_edit_text(edit, value);

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_blur_edit = false;
    }
}

pub(super) fn sync_corner_edit() {
    let (edit, value) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        let value = match s.section {
            Section::Panel => s.snapshot.active_style.panel_corner_radius,
            Section::Tooltip => s.snapshot.active_style.tooltip_corner_radius,
            Section::Progress => s.snapshot.active_style.progress_corner_radius,
            _ => return,
        };
        s.syncing_corner_edit = true;
        (s.corner_edit.to_hwnd(), value)
    };
    set_edit_text(edit, value);

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_corner_edit = false;
    }
}

pub(super) fn sync_active_style_into_editable(state: &mut PanelState) {
    let editable = EditableThemeStyle::from_theme_style(&state.snapshot.active_style);
    if state.snapshot.is_dark {
        state.snapshot.editable_settings.appearance.dark = editable;
    } else {
        state.snapshot.editable_settings.appearance.light = editable;
    }
}

pub(super) fn sync_hex_edits() {
    let (edits, values) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if !matches!(
            s.section,
            Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
        ) {
            return;
        }
        s.syncing_hex_edits = true;
        let values =
            COLOR_TARGETS.map(|target| s.snapshot.active_style.color(target).to_hex_rgba());
        (s.hex_edits, values)
    };
    for (edit, value) in edits.iter().zip(values.iter()) {
        set_edit_text_string(edit.to_hwnd(), value);
    }
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_hex_edits = false;
        s.invalid_hex_edits = [false; HEX_EDIT_COUNT];
    }
}

pub(super) fn normalize_hex_edit(target: StyleColorTarget) {
    let index = encode_color_target(target);
    let (edit, value) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.syncing_hex_edits = true;
        s.invalid_hex_edits[index] = false;
        (
            s.hex_edits[index].to_hwnd(),
            s.snapshot.active_style.color(target).to_hex_rgba(),
        )
    };
    set_edit_text_string(edit, &value);
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_hex_edits = false;
    }
}
