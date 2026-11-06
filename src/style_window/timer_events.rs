//! Timer and queued settings refresh events.
//!
//! Preserve the original WM_TIMER fallback, JSON polling and validation,
//! preset-gallery warmup, animation ticks, and frame presentation order.
//! Keep state snapshots/lock-release points before synchronous Win32 calls.

use super::*;

pub(super) fn is_settings_scheduled_message(msg: u32) -> bool {
    matches!(
        msg,
        WM_SETTINGS_PREWARM_GALLERY
            | WM_SETTINGS_PRESET_GALLERY_REFRESH
            | WM_SETTINGS_MULTIMEDIA_ANIMATION_TICK
            | WM_SETTINGS_PRESENT_INTERACTIVE_FRAME
            | WM_TIMER
    )
}

pub(super) unsafe fn handle_settings_scheduled_message(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SETTINGS_PREWARM_GALLERY => {
            SETTINGS_GALLERY_WARM_REQUESTED.with(|cell| cell.set(false));
            let to_warm = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().and_then(|s| {
                    (s.section == Section::Preset && !s.desktop_submenu.active()
                        && !s.desktop_submenu.final_frame_pending
                        && s.pending_discard_action.is_none())
                    .then(|| (s.snapshot.clone(), s.font))
                })
            };
            if let Some((snapshot, font)) = to_warm {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let width = (client.right - client.left).max(1);
                let height = (client.bottom - client.top).max(1);
                if !preset_gallery_cache_ready(hwnd, width, height, &snapshot) {
                    let screen = GetDC(hwnd);
                    if !screen.0.is_null() {
                        let _ = build_preset_gallery_cache(
                            screen, hwnd, width, height, &snapshot, font,
                        );
                        let _ = ReleaseDC(hwnd, screen);
                    }
                }
            }
            LRESULT(0)
        }
        WM_SETTINGS_PRESET_GALLERY_REFRESH => {
            let should_refresh = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    if s.section != Section::Preset {
                        s.preset_gallery_refresh_pending = false;
                        false
                    } else if s.preset_gallery_refresh_pending
                        && !s.desktop_submenu.active()
                        && !s.desktop_submenu.final_frame_pending
                    {
                        s.preset_gallery_refresh_pending = false;
                        true
                    } else {
                        false
                    }
                } else { false }
            };
            if should_refresh {
                // Refresh the area below the top card, not the entire window.
                // While animated, this message is deferred until after the
                // cached terminal card frame is presented.
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let rect = RECT {
                    left: scale(hwnd, SETTINGS_CONTENT_LEFT),
                    top: scale(hwnd, 206),
                    right: client.right,
                    bottom: client.bottom,
                };
                if rect.right > rect.left && rect.bottom > rect.top {
                    SETTINGS_GALLERY_REFRESH.with(|cell| cell.set(true));
                    mark_settings_paint_cause(SettingsPaintCause::GalleryRefresh);
                    let _ = InvalidateRect(hwnd, Some(&rect), false);
                    queue_settings_interactive_frame(hwnd, false);
                }
            }
            LRESULT(0)
        }
        WM_SETTINGS_MULTIMEDIA_ANIMATION_TICK => {
            // Consume the single coalesced worker callback before processing.
            SUBMENU_MEDIA_TICK_QUEUED.store(false, Ordering::Release);
            if SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| cell.get() != 0) {
                drive_submenu_frame_tick(hwnd, true);
            }
            LRESULT(0)
        }
        WM_SETTINGS_PRESENT_INTERACTIVE_FRAME => {
            SETTINGS_INTERACTIVE_FRAME_QUEUED.with(|pending| pending.set(false));
            // Unlike generated WM_PAINT, UpdateWindow bypasses the message
            // queue; the original invalid region and cached renderer remain
            // responsible for what is drawn.
            let _ = UpdateWindow(hwnd);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == DESKTOP_SUBMENU_TIMER_ID {
                // Fallback when multimedia timer creation was not available.
                if SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| cell.get() == 0) {
                    drive_submenu_frame_tick(hwnd, false);
                }
                return LRESULT(0);
            }
            if wparam.0 == JSON_ACTION_TIMER_ID {
                finish_pending_json_action(hwnd);
                return LRESULT(0);
            }
            if wparam.0 == JSON_VALIDATION_TIMER_ID {
                let _ = KillTimer(hwnd, JSON_VALIDATION_TIMER_ID);
                update_json_validation_status(false);
                return LRESULT(0);
            }
            if wparam.0 == JSON_ERROR_SHAKE_TIMER_ID {
                advance_json_error_shake(hwnd);
                return LRESULT(0);
            }
            if wparam.0 == JSON_SCROLL_DRAG_FRAME_TIMER_ID {
                if json_editor_active() {
                    let dragging = {
                        let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        guard.as_ref().is_some_and(|s| s.json_scroll_dragging)
                    };
                    if dragging {
                        update_json_scroll_drag_paced(hwnd, None, false);
                    }
                }
                return LRESULT(0);
            }
            if wparam.0 == JSON_SCROLLBAR_TIMER_ID {
                // Keyboard navigation and caret auto-scroll do not pass through
                // our custom wheel/drag handlers. Poll only for actual thumb
                // movement; never repaint the parent when nothing changed.
                if json_editor_active() {
                    refresh_json_scrollbar_if_changed(hwnd);
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
