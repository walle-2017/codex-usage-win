//! JSON editor/pane/mask positioning, Z-order, focus and scroll timer.
use super::*;

pub(super) fn layout_settings_children(hwnd: HWND) {
    let Some((json_edit, json_page_surface, json_save_mask, section, discard_pending, save_mask_visible)) = ({
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| {
            (
                s.json_edit,
                s.json_page_surface,
                s.json_save_mask,
                s.section,
                s.pending_discard_action.is_some(),
                s.section == Section::Json
                    && s.pending_discard_action.is_none()
                    && s.json_save_feedback == Some(JsonSaveFeedback::Saving),
            )
        })
    }) else {
        return;
    };

    unsafe {
        let focused = GetFocus();
        let visible = json_editor_should_be_visible(section, discard_pending);
        if !visible && focused == json_edit.to_hwnd() {
            let _ = SetFocus(hwnd);
        }

        // Reveal the full opaque JSON pane before revealing RichEdit. Even if
        // the parent still displays a cached preset frame, the entire right
        // content region is now covered by the correctly painted JSON page.
        let page = json_page_surface.to_hwnd();
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let page_left = scale(hwnd, 180);
        if !visible {
            let _ = ShowWindow(json_edit.to_hwnd(), SW_HIDE);
        }
        let _ = SetWindowPos(
            page,
            HWND::default(),
            page_left, 0,
            (client.right - page_left).max(1),
            client.bottom.max(1),
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS
                | if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW },
        );
        if visible {
            let _ = RedrawWindow(
                page, None, HRGN::default(),
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );
        }

        let json_rect = json_edit_text_rect(hwnd);
        let visibility = if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        // Both windows are siblings. WS_EX_COMPOSITED uses their *actual*
        // Z-order, not the order in which SetWindowPos was called. The opaque
        // JSON page must stay BEHIND RichEdit or its text is fully covered.
        // SWP_NOZORDER here left the pane in front on some Windows systems.
        let _ = SetWindowPos(
            json_edit.to_hwnd(),
            HWND_TOP,
            json_rect.left,
            json_rect.top,
            (json_rect.right - json_rect.left).max(1),
            (json_rect.bottom - json_rect.top).max(1),
            SWP_NOACTIVATE | SWP_NOCOPYBITS | visibility,
        );

        let mask_hwnd = json_save_mask.to_hwnd();
        if !mask_hwnd.0.is_null() {
            if save_mask_visible {
                let mask_rect = json_save_mask_rect(hwnd);
                let mut mask_origin = POINT {
                    x: mask_rect.left,
                    y: mask_rect.top,
                };
                let _ = ClientToScreen(hwnd, &mut mask_origin);
                let _ = SetWindowPos(
                    mask_hwnd,
                    HWND_TOP,
                    mask_origin.x,
                    mask_origin.y,
                    (mask_rect.right - mask_rect.left).max(1),
                    (mask_rect.bottom - mask_rect.top).max(1),
                    SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_SHOWWINDOW,
                );
                // The saving state only lasts 200 ms. Paint the layered popup
                // synchronously so the mask is visible for that full interval.
                let _ = InvalidateRect(mask_hwnd, None, true);
                let _ = UpdateWindow(mask_hwnd);
            } else {
                let _ = ShowWindow(mask_hwnd, SW_HIDE);
            }
        }

        if visible {
            invalidate_json_scroll_layout_cache();
            let _ = SetTimer(hwnd, JSON_SCROLLBAR_TIMER_ID, 120, None);
        } else {
            let _ = KillTimer(hwnd, JSON_SCROLLBAR_TIMER_ID);
            invalidate_json_scroll_layout_cache();
        }
    }
}
