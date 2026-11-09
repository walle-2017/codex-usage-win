//! Native editor child layout, font measurement and focus-safe visibility.
//! Preserve Win32 focus notification order and release STATE before child calls.

use super::*;

// Read the edit's actual selected font, not an assumed 96-DPI line height.
// This also handles a popup moved between monitors with different DPI and
// Windows font metrics without relying on EM_SETRECT (multiline-only).
pub(super) unsafe fn single_line_edit_font_height(edit: HWND, fallback: i32) -> i32 {
    let dc = GetDC(edit);
    if dc.0.is_null() {
        return fallback.max(1);
    }
    let selected = SendMessageW(edit, WM_GETFONT, WPARAM(0), LPARAM(0));
    let old_font = if selected.0 != 0 {
        Some(SelectObject(dc, HGDIOBJ(selected.0 as *mut _)))
    } else {
        None
    };
    let mut metrics = TEXTMETRICW::default();
    let measured = GetTextMetricsW(dc, &mut metrics).as_bool();
    if let Some(old_font) = old_font {
        let _ = SelectObject(dc, old_font);
    }
    let _ = ReleaseDC(edit, dc);
    if measured && metrics.tmHeight > 0 {
        metrics.tmHeight.saturating_add(metrics.tmExternalLeading.max(0))
    } else {
        fallback.max(1)
    }
}

#[allow(clippy::type_complexity)]
pub(super) fn editor_layout_snapshot(
) -> Option<([SendHwnd; 4], SendHwnd, SendHwnd, Section, bool, bool, bool)> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = state.as_ref()?;
    Some((
        s.numeric_edits,
        s.blur_edit,
        s.corner_edit,
        s.section,
        matches!(
            s.section,
            Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
        ) && matches!(s.editor, EditorSelection::Color(_)),
        matches!(s.section, Section::Panel | Section::Tooltip),
        matches!(s.section, Section::Panel | Section::Tooltip | Section::Progress),
    ))
}

pub(super) fn release_editor_focus_before_layout(
    hwnd: HWND,
    numeric_edits: &[SendHwnd; 4],
    blur_edit: SendHwnd,
    corner_edit: SendHwnd,
    show_color: bool,
    show_blur: bool,
    show_corner: bool,
) {
    unsafe {
        let focused = GetFocus();
        let hiding_focused_color =
            !show_color && numeric_edits.iter().any(|edit| edit.to_hwnd() == focused);
        let hiding_focused_blur = !show_blur && blur_edit.to_hwnd() == focused;
        let hiding_focused_corner = !show_corner && corner_edit.to_hwnd() == focused;

        if hiding_focused_color || hiding_focused_blur || hiding_focused_corner {
            // SetFocus synchronously sends EN_KILLFOCUS to the old EDIT control.
            // This must run with STATE unlocked or WM_COMMAND would re-enter
            // STATE.lock() on the same UI thread and deadlock.
            let _ = SetFocus(hwnd);
        }
    }
}

pub(super) fn layout_numeric_edits(hwnd: HWND) {
    // Copy every HWND and visibility decision while STATE is locked, then release
    // the mutex before calling Win32. ShowWindow/SetFocus can synchronously send
    // WM_COMMAND focus notifications back into this window procedure.
    let Some((numeric_edits, blur_edit, corner_edit, section, show_color, show_blur, show_corner)) =
        editor_layout_snapshot()
    else {
        return;
    };

    release_editor_focus_before_layout(
        hwnd,
        &numeric_edits,
        blur_edit,
        corner_edit,
        show_color,
        show_blur,
        show_corner,
    );

    unsafe {
        // All numeric/blur/corner children use the same UI font. Measure once
        // per relayout, then center each native single-line EDIT in its frame.
        let font_height = single_line_edit_font_height(numeric_edits[0].to_hwnd(), scale(hwnd, 14));
        let text_padding = scale(hwnd, 4);
        for (index, edit) in numeric_edits.iter().enumerate() {
            let r = centered_single_line_edit_rect(
                numeric_edit_rect(hwnd, section, index), font_height, text_padding,
            );
            let _ = SetWindowPos(
                edit.to_hwnd(),
                HWND::default(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
            );
            let _ = ShowWindow(
                edit.to_hwnd(),
                if show_color { SW_SHOW } else { SW_HIDE },
            );
        }

        let blur_rect = centered_single_line_edit_rect(
            blur_edit_rect(hwnd, section), font_height, text_padding,
        );
        let _ = SetWindowPos(
            blur_edit.to_hwnd(),
            HWND::default(),
            blur_rect.left,
            blur_rect.top,
            blur_rect.right - blur_rect.left,
            blur_rect.bottom - blur_rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        );
        let _ = ShowWindow(
            blur_edit.to_hwnd(),
            if show_blur { SW_SHOW } else { SW_HIDE },
        );

        let corner_rect = centered_single_line_edit_rect(
            corner_edit_rect(hwnd), font_height, text_padding,
        );
        let _ = SetWindowPos(
            corner_edit.to_hwnd(),
            HWND::default(),
            corner_rect.left,
            corner_rect.top,
            corner_rect.right - corner_rect.left,
            corner_rect.bottom - corner_rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        );
        let _ = ShowWindow(
            corner_edit.to_hwnd(),
            if show_corner { SW_SHOW } else { SW_HIDE },
        );
    }
}

pub(super) fn hex_layout_snapshot() -> Option<([SendHwnd; HEX_EDIT_COUNT], Section, Option<StyleColorTarget>)> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = state.as_ref()?;
    Some((s.hex_edits, s.section, s.focused_hex_edit))
}

pub(super) fn layout_hex_edits(hwnd: HWND) {
    let Some((hex_edits, section, focused_target)) = hex_layout_snapshot() else {
        return;
    };

    unsafe {
        let font_height = single_line_edit_font_height(hex_edits[0].to_hwnd(), scale(hwnd, 14));
        let text_padding = scale(hwnd, 4);
        if let Some(target) = focused_target {
            if color_row_index(section, target).is_none() {
                let index = encode_color_target(target);
                if hex_edits[index].to_hwnd() == GetFocus() {
                    let _ = SetFocus(hwnd);
                }
            }
        }
        for (index, target) in COLOR_TARGETS.iter().copied().enumerate() {
            let edit = hex_edits[index].to_hwnd();
            if let Some(row_index) = color_row_index(section, target) {
                let r = centered_single_line_edit_rect(
                    hex_edit_rect(hwnd, row_index), font_height, text_padding,
                );
                let _ = SetWindowPos(
                    edit,
                    HWND::default(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
                );
                let _ = ShowWindow(edit, SW_SHOW);
            } else {
                let _ = ShowWindow(edit, SW_HIDE);
            }
        }
    }
}
