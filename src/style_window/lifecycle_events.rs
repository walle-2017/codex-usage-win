//! Settings window size, DPI, close and destroy message handling.
//!
//! Keep WM_DPICHANGED sizing before child layout, and preserve the JSON
//! discard confirmation, timer shutdown and resource-release ordering.
//! Never hold STATE while calling synchronous native window destruction.

use super::*;

pub(super) fn is_settings_lifecycle_message(msg: u32) -> bool {
    matches!(
        msg,
        WM_DPICHANGED | WM_SIZE | WM_GETMINMAXINFO | WM_CLOSE | WM_DESTROY
    )
}

pub(super) unsafe fn handle_settings_lifecycle_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DPICHANGED => {
            let suggested = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            layout_numeric_edits(hwnd);
            layout_hex_edits(hwnd);
            layout_settings_children(hwnd);
            redraw_settings_window(hwnd);
            LRESULT(0)
        }
        WM_SIZE => {
            layout_settings_children(hwnd);
            redraw_settings_window(hwnd);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            info.ptMinTrackSize.x = scale(hwnd, WINDOW_MIN_WIDTH);
            info.ptMinTrackSize.y = scale(hwnd, WINDOW_MIN_HEIGHT);
            LRESULT(0)
        }
        WM_CLOSE => {
            let (dirty, pending) = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| (s.json_dirty, s.pending_discard_action.is_some()))
                    .unwrap_or((false, false))
            };
            if pending {
                return LRESULT(0);
            }
            if dirty {
                request_discard_confirmation(hwnd, PendingDiscardAction::Close);
                return LRESULT(0);
            }
            send_parent(WM_STYLE_SAVE, 0, 0);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        },
        WM_DESTROY => {
            invalidate_json_scroll_layout_cache();
            end_submenu_frame_clock(hwnd);
            PRESET_GALLERY_FRAMES.with(|cell| *cell.borrow_mut() = None);
            SETTINGS_GALLERY_REFRESH.with(|cell| cell.set(false));
            SETTINGS_GALLERY_WARM_REQUESTED.with(|cell| cell.set(false));
            SUBMENU_CARD_FRAMES.with(|cell| *cell.borrow_mut() = None);
            SETTINGS_PAINT_BUFFER.with(|cell| *cell.borrow_mut() = None);
            JSON_SCROLLBAR_PAINT_BUFFER.with(|cell| *cell.borrow_mut() = None);
            reset_json_scroll_frame_clock();
            let _ = KillTimer(hwnd, JSON_SCROLL_DRAG_FRAME_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_ACTION_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_SCROLLBAR_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_VALIDATION_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
            let resources = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.take().map(|s| (s.font, s.json_font, s.edit_brush, s.json_save_mask))
            };
            if let Some((font, json_font, edit_brush, json_save_mask)) = resources {
                if font != 0 {
                    let _ = DeleteObject(HGDIOBJ(font as *mut _));
                }
                if json_font != 0 {
                    let _ = DeleteObject(HGDIOBJ(json_font as *mut _));
                }
                if edit_brush != 0 {
                    let _ = DeleteObject(HGDIOBJ(edit_brush as *mut _));
                }
                let mask = json_save_mask.to_hwnd();
                if !mask.0.is_null() {
                    let _ = DestroyWindow(mask);
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
