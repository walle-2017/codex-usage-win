//! Input-to-model updates for Hex, RGBA, blur and corner radius native EDIT controls.
//! Preserve focused caret, preview messages and Win32 state-lock release ordering.

use super::*;

pub(super) fn update_color_from_hex_edit(target: StyleColorTarget) {
    let index = encode_color_target(target);
    let (edit, syncing) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.hex_edits[index].to_hwnd(), s.syncing_hex_edits)
    };
    if syncing {
        return;
    }

    let parsed = parse_hex_input(&read_edit_text(edit));
    let mut applied = None;
    let mut selected = false;
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        match parsed {
            Ok(Some(color)) => {
                s.invalid_hex_edits[index] = false;
                s.snapshot.active_style.set_color(target, color);
                sync_active_style_into_editable(s);
                selected = s.editor == EditorSelection::Color(target);
                applied = Some(color);
            }
            Ok(None) => s.invalid_hex_edits[index] = false,
            Err(()) => s.invalid_hex_edits[index] = true,
        }
    }
    if let Some(color) = applied {
        if selected {
            sync_numeric_edits();
        }
        send_parent(
            WM_STYLE_COLOR_PREVIEW,
            encode_color_target(target),
            pack_color(color),
        );
        send_parent(WM_STYLE_SAVE, 0, 0);
    }
    unsafe {
        let hwnd = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or_default()
        };
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn update_color_from_numeric_edit(channel_index: usize) {
    let (edit, target, syncing) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        let EditorSelection::Color(target) = s.editor else {
            return;
        };
        (s.numeric_edits[channel_index].to_hwnd(), target, s.syncing_numeric_edits)
    };
    if syncing {
        return;
    }
    let Some(raw_value) = read_edit_value(edit) else {
        return;
    };
    let value = raw_value.min(u16::from(u8::MAX)) as u8;

    let color = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        let current = s.snapshot.active_style.color(target);
        let color = match channel_index {
            0 => Color::rgba(value, current.g, current.b, current.a),
            1 => Color::rgba(current.r, value, current.b, current.a),
            2 => Color::rgba(current.r, current.g, value, current.a),
            _ => Color::rgba(current.r, current.g, current.b, value),
        };
        s.snapshot.active_style.set_color(target, color);
        sync_active_style_into_editable(s);
        color
    };

    if raw_value > u16::from(u8::MAX) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_numeric_edits = true;
        }
        drop(state);
        set_edit_text(edit, value);
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_numeric_edits = false;
        }
    }

    sync_hex_edits();
    send_parent(
        WM_STYLE_COLOR_PREVIEW,
        encode_color_target(target),
        pack_color(color),
    );
    send_parent(WM_STYLE_SAVE, 0, 0);
    unsafe {
        let hwnd = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or_default()
        };
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn update_blur_from_numeric_edit() {
    let (edit, syncing, section) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.blur_edit.to_hwnd(), s.syncing_blur_edit, s.section)
    };
    if syncing {
        return;
    }

    let Some(raw_value) = read_edit_value(edit) else {
        return;
    };
    let value = raw_value.min(u16::from(FROSTED_STRENGTH_MAX)) as u8;

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if section == Section::Tooltip {
            s.snapshot.active_style.set_tooltip_frosted_strength(value);
        } else {
            s.snapshot.active_style.panel_frosted_strength = value;
        }
        sync_active_style_into_editable(s);
    }

    if raw_value > u16::from(FROSTED_STRENGTH_MAX) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_blur_edit = true;
        }
        drop(state);
        set_edit_text(edit, value);
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_blur_edit = false;
        }
    }

    send_parent(
        if section == Section::Tooltip {
            WM_STYLE_TOOLTIP_BLUR_PREVIEW
        } else {
            WM_STYLE_BLUR_PREVIEW
        },
        value as usize,
        0,
    );
    send_parent(WM_STYLE_SAVE, 0, 0);
    unsafe {
        let hwnd = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or_default()
        };
        // Panel blur previews invoke render_layered() in the parent window,
        // unlike Tooltip blur previews. The native EDIT has already delivered
        // EN_CHANGE and the model is correct, but a normal invalidation can
        // leave both the focused glyphs and slider visually stale while the
        // composited Settings parent continues handling updates.
        //
        // Flush the EDIT separately and post a coalesced Settings presentation
        // after this command finishes. Do NOT call sync_blur_edit() here: that
        // would rewrite partially typed text and reset the caret/selection.
        // No STATE mutex may be held during these synchronous Win32 calls.
        let _ = RedrawWindow(
            edit, None, HRGN::default(),
            RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
        );
        let _ = InvalidateRect(hwnd, None, false);
        queue_settings_interactive_frame(hwnd, false);
    }
}

pub(super) fn update_corner_from_numeric_edit() {
    let (edit, syncing, section, radius_max) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        let Some(radius_max) =
            corner_radius_max_for_section(s.snapshot.appearance_preset, s.section)
        else {
            return;
        };
        (
            s.corner_edit.to_hwnd(),
            s.syncing_corner_edit,
            s.section,
            radius_max,
        )
    };
    if syncing {
        return;
    }

    let Some(raw_value) = read_edit_value(edit) else {
        return;
    };
    let value = raw_value.min(u16::from(radius_max)) as u8;

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        match section {
            Section::Panel => s.snapshot.active_style.panel_corner_radius = value,
            Section::Tooltip => s.snapshot.active_style.tooltip_corner_radius = value,
            Section::Progress => s.snapshot.active_style.progress_corner_radius = value,
            _ => return,
        }
        sync_active_style_into_editable(s);
    }

    if raw_value > u16::from(radius_max) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_corner_edit = true;
        }
        drop(state);
        set_edit_text(edit, value);
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_corner_edit = false;
        }
    }

    let section_code = match section {
        Section::Panel => 0usize,
        Section::Tooltip => 1,
        Section::Progress => 2,
        _ => return,
    };
    send_parent(
        WM_STYLE_CORNER_PREVIEW,
        section_code | (usize::from(value) << 8),
        0,
    );
    send_parent(WM_STYLE_SAVE, 0, 0);
    unsafe {
        let hwnd = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or_default()
        };
        let _ = InvalidateRect(hwnd, None, false);
    }
}
