//! Settings window mouse input routing.
//!
//! Preserve the original input message branches and their Win32 call order.
//! In particular, drop state locks before any synchronous scroll, focus,
//! capture, painting, or parent notification operations.

use super::*;

fn handle_settings_mouse_wheel(hwnd: HWND, wparam: WPARAM) -> bool {
    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    if delta == 0 {
        return false;
    }

    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
        let _ = ScreenToClient(hwnd, &mut point);
    }

    if let Some(target) = wheel_numeric_target_at(hwnd, point.x, point.y) {
        adjust_numeric_by_wheel(target, delta);
        return true;
    }
    if let Some(kind) = slider_kind_at(hwnd, point.x, point.y) {
        adjust_slider_by_wheel(hwnd, kind, delta);
        return true;
    }
    if let Some(edit) = json_scrollbar_wheel_target(hwnd, point.x, point.y) {
        unsafe {
            // A WM_MOUSEWHEEL reaches the RichEdit subclass exactly as if
            // the pointer were above the JSON text itself. Do not change
            // focus or trigger an entire settings-page repaint.
            let _ = SendMessageW(edit, WM_MOUSEWHEEL, wparam, LPARAM(0));
        }
        return true;
    }
    false
}

pub(super) fn is_settings_input_message(msg: u32) -> bool {
    matches!(
        msg,
        WM_MOUSEWHEEL
            | WM_SETCURSOR
            | WM_LBUTTONDOWN
            | WM_MOUSEMOVE
            | WM_MOUSELEAVE_MSG
            | WM_LBUTTONUP
            | WM_CANCELMODE
            | WM_CAPTURECHANGED
    )
}

pub(super) unsafe fn handle_settings_input_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_MOUSEWHEEL => {
            if handle_settings_mouse_wheel(hwnd, wparam) {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_SETCURSOR => {
            let cursor_hwnd = HWND(wparam.0 as *mut _);
            let is_numeric_edit = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                match state.as_ref() {
                    Some(s) => {
                        s.numeric_edits
                            .iter()
                            .any(|edit| edit.to_hwnd() == cursor_hwnd)
                            || s.blur_edit.to_hwnd() == cursor_hwnd
                            || s.corner_edit.to_hwnd() == cursor_hwnd
                            || s.hex_edits.iter().any(|edit| edit.to_hwnd() == cursor_hwnd)
                    }
                    None => false,
                }
            };
            if is_numeric_edit {
                let cursor = LoadCursorW(HINSTANCE::default(), IDC_IBEAM).unwrap_or_default();
                SetCursor(cursor);
                return LRESULT(1);
            }

            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = ScreenToClient(hwnd, &mut point);

            let cursor_id = if json_scroll_thumb_hit_rect(hwnd)
                .is_some_and(|r| pt_in_rect(r, point.x, point.y))
            {
                IDC_HAND
            } else if slider_kind_at(hwnd, point.x, point.y).is_some() {
                IDC_SIZEWE
            } else if hit_target_at(hwnd, point.x, point.y).is_some() {
                IDC_HAND
            } else {
                IDC_ARROW
            };
            let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
            SetCursor(cursor);
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_from_lparam(lparam);

            let json_scroll_hit = json_editor_active()
                && json_scroll_thumb_hit_rect(hwnd)
                    .is_some_and(|r| pt_in_rect(r, x, y));
            if json_scroll_hit {
                let thumb = json_scroll_thumb_rect(hwnd).unwrap_or_default();
                {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        s.json_scroll_dragging = true;
                        s.json_scroll_hovered = true;
                        s.json_scroll_drag_offset = (y - thumb.top).clamp(0, thumb.bottom - thumb.top);
                        s.pressed = None;
                    }
                }
                start_json_scroll_drag_pacing(hwnd);
                let _ = SetCapture(hwnd);
                invalidate_json_scrollbar(hwnd);
                return LRESULT(0);
            }

            if json_editor_active() {
                let track = json_scrollbar_track_rect(hwnd);
                if pt_in_rect(track, x, y) {
                    if let (Some(thumb), Some((edit, _, visible_lines, _))) =
                        (json_scroll_thumb_rect(hwnd), json_scroll_line_metrics())
                    {
                        let direction = if y < thumb.top {
                            -1
                        } else if y >= thumb.bottom {
                            1
                        } else {
                            0
                        };
                        if direction != 0 {
                            let page_lines = visible_lines.saturating_sub(1).max(1);
                            let _ = SetFocus(edit);
                            scroll_json_editor_lines(edit, hwnd, direction * page_lines);
                            return LRESULT(0);
                        }
                    }
                }
            }

            if let Some(kind) = slider_kind_at(hwnd, x, y) {
                if kind == SliderKind::Blur {
                    let _ = SetFocus(hwnd);
                    let editor = {
                        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        state.as_ref().map(|s| if s.section == Section::Tooltip { EditorSelection::TooltipBlur } else { EditorSelection::Blur })
                    };
                    if let Some(editor) = editor { select_editor(editor); }
                }
                {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        s.dragging_slider = Some(kind);
                        s.pressed = None;
                    }
                }
                let _ = SetCapture(hwnd);
                update_slider(hwnd, kind, x);
                return LRESULT(0);
            }

            if let Some(target) = hit_target_at(hwnd, x, y) {
                mark_settings_paint_cause(SettingsPaintCause::MousePressed);
                let section = {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        s.pressed = Some(target);
                        s.hovered = Some(target);
                        s.section
                    } else { Section::General }
                };
                let _ = SetCapture(hwnd);
                invalidate_appearance_card_or_window(
                    hwnd, section, is_appearance_card_target(target),
                );
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = point_from_lparam(lparam);
            let (scroll_dragging, old_scroll_hover) = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| (s.json_scroll_dragging, s.json_scroll_hovered))
                    .unwrap_or((false, false))
            };
            if scroll_dragging {
                update_json_scroll_drag_paced(hwnd, Some(y), false);
                return LRESULT(0);
            }
            let scroll_hover = json_editor_active()
                && pt_in_rect(json_scrollbar_track_rect(hwnd), x, y)
                && json_scroll_thumb_rect(hwnd).is_some();
            if scroll_hover != old_scroll_hover {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.json_scroll_hovered = scroll_hover;
                }
                drop(state);
                invalidate_json_scrollbar(hwnd);
            }

            let kind = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                if !s.tracking_mouse_leave {
                    let mut tracking = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tracking);
                    s.tracking_mouse_leave = true;
                }
                s.dragging_slider
            };

            if let Some(kind) = kind {
                update_slider(hwnd, kind, x);
            } else {
                let (_, y) = point_from_lparam(lparam);
                let hovered = hit_target_at(hwnd, x, y);
                let (changed, card_only, section) = {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(s) = state.as_mut() else {
                        return LRESULT(0);
                    };
                    let changed = s.hovered != hovered;
                    let card_only = card_only_hover_change(s.section, s.hovered, hovered);
                    s.hovered = hovered;
                    (changed, card_only, s.section)
                };
                if changed {
                    mark_settings_paint_cause(SettingsPaintCause::MouseHover);
                    invalidate_appearance_card_or_window(hwnd, section, card_only);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            let (changed, card_only, section) = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                s.tracking_mouse_leave = false;
                let changed = s.hovered.is_some() || s.json_scroll_hovered;
                let card_only = card_only_hover_change(s.section, s.hovered, None)
                    && !s.json_scroll_hovered;
                s.hovered = None;
                s.json_scroll_hovered = false;
                (changed, card_only, s.section)
            };
            if changed {
                mark_settings_paint_cause(SettingsPaintCause::MouseHover);
                invalidate_appearance_card_or_window(hwnd, section, card_only);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_from_lparam(lparam);
            let was_scroll_dragging = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                let value = s.json_scroll_dragging;
                s.json_scroll_dragging = false;
                value
            };
            if was_scroll_dragging {
                // Commit the exact release position even if it arrives inside
                // the previous 16.7 ms frame; never leave a deferred thumb.
                update_json_scroll_drag_paced(hwnd, Some(y), true);
                stop_json_scroll_drag_pacing(hwnd);
                let _ = ReleaseCapture();
                invalidate_json_scrollbar(hwnd);
                return LRESULT(0);
            }

            let (was_dragging, pressed) = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                (s.dragging_slider.take().is_some(), s.pressed.take())
            };

            let _ = ReleaseCapture();

            if was_dragging {
                send_parent(WM_STYLE_SAVE, 0, 0);
            } else if let Some(target) = pressed {
                if hit_target_at(hwnd, x, y) == Some(target) {
                    activate_target(hwnd, target);
                }
            }
            let section = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.section).unwrap_or(Section::General)
            };
            mark_settings_paint_cause(SettingsPaintCause::MouseReleased);
            invalidate_appearance_card_or_window(
                hwnd, section, pressed.is_some_and(is_appearance_card_target),
            );
            LRESULT(0)
        }
        WM_CANCELMODE | WM_CAPTURECHANGED => {
            stop_json_scroll_drag_pacing(hwnd);
            let (was_dragging, pressed_target, section) = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                (
                    {
                        s.json_scroll_dragging = false;
                        s.dragging_slider.take().is_some()
                    },
                    s.pressed.take(),
                    s.section,
                )
            };
            if was_dragging {
                send_parent(WM_STYLE_SAVE, 0, 0);
            }
            if let Some(target) = pressed_target {
                // Releasing mouse capture for an appearance choice must not
                // invalidate the entire, expensive Preset gallery.
                mark_settings_paint_cause(SettingsPaintCause::MouseCaptureChanged);
                invalidate_appearance_card_or_window(
                    hwnd, section, is_appearance_card_target(target),
                );
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
