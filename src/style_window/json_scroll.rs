//! JSON editor scroll geometry, RichEdit line scrolling, frame pacing and thumb drag.
//!
//! Keep synchronous RichEdit calls and scrollbar invalidation behavior intact.
//! Window subclass/message dispatch and page painting stay in style_window.rs.

use super::*;

pub(super) fn json_scrollbar_track_rect(hwnd: HWND) -> RECT {
    let outer = json_edit_rect(hwnd);
    RECT {
        left: outer.right - scale(hwnd, 12),
        top: outer.top + scale(hwnd, 8),
        right: outer.right - scale(hwnd, 4),
        bottom: outer.bottom - scale(hwnd, 8),
    }
}

// Line count and font height are stable while the JSON text and viewport
// dimensions remain unchanged. WM_MOUSEMOVE previously measured a font DC
// and asked RichEdit to lay out the whole document multiple times per event.
#[derive(Clone, Copy)]
pub(super) struct JsonScrollLayoutCache {
    pub(super) edit: isize,
    pub(super) font: isize,
    pub(super) viewport_height: i32,
    pub(super) viewport_width: i32,
    pub(super) line_height: i32,
    pub(super) total_lines: i32,
}

impl JsonScrollLayoutCache {
    pub(super) fn matches(self, edit: isize, font: isize, width: i32, height: i32) -> bool {
        self.edit == edit && self.font == font
            && self.viewport_width == width && self.viewport_height == height
    }
}

thread_local! {
    static JSON_SCROLL_LAYOUT_CACHE: RefCell<Option<JsonScrollLayoutCache>> =
        const { RefCell::new(None) };
    static JSON_SCROLL_LAST_THUMB: RefCell<Option<(i32, i32)>> =
        const { RefCell::new(None) };
    // The scrollbar is narrow; reuse its GDI memory surface across drag frames.
    // The existing whole-page buffer cannot be used without painting siblings.
    pub(super) static JSON_SCROLLBAR_PAINT_BUFFER: RefCell<Option<SettingsPaintBuffer>> =
        const { RefCell::new(None) };
    static JSON_SCROLL_FRAME_CLOCK: RefCell<JsonScrollFrameClock> =
        RefCell::new(JsonScrollFrameClock::default());
}

#[derive(Default)]
struct JsonScrollFrameClock {
    last_sync: Option<Instant>,
    last_refresh_probe: Option<Instant>,
    refresh_hz: i32,
    last_drag_update: Option<Instant>,
    pending_drag_y: Option<i32>,
}

// Match the Windows display DC, but never issue unbounded synchronous WM_PAINT
// messages (high-refresh monitors can report up to 240 Hz here). Unknown 0/1
// values use a conservative 60 Hz fallback.
pub(super) fn json_scroll_target_hz(device_hz: i32) -> i32 {
    // Windows GDI frequently reports 59 for the normal 59.94/60 Hz
    // desktop mode. Both use a nominal 60 Hz input/paint budget.
    if device_hz <= 1 || (58..=61).contains(&device_hz) {
        60
    } else {
        device_hz.clamp(30, 240)
    }
}

pub(super) fn json_scroll_frame_interval(refresh_hz: i32) -> Duration {
    Duration::from_micros(1_000_000 / json_scroll_target_hz(refresh_hz) as u64)
}

pub(super) fn json_scroll_frame_due(last: Option<Instant>, now: Instant, refresh_hz: i32) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= json_scroll_frame_interval(refresh_hz))
}

// A drag can generate pointer events at 120+ Hz on a 60 Hz monitor.
// Updating RichEdit on every WM_MOUSEMOVE performs redundant native paints.
// Preserve the last pointer rather than queueing intermediate text frames.
pub(super) fn json_scroll_drag_should_apply(
    last: Option<Instant>, now: Instant, refresh_hz: i32, force: bool,
) -> bool {
    // WM_MOUSEMOVE is usually sampled every ~8 ms. A strict 16.67 ms
    // cutoff would skip the 16 ms sample and paint only at 24 ms (~42 Hz).
    // A 2 ms scheduling allowance keeps updates close to 60 Hz without
    // trying to render every high-frequency input sample.
    let interval = json_scroll_frame_interval(refresh_hz)
        .saturating_sub(Duration::from_millis(2));
    force || last.is_none_or(|at| now.saturating_duration_since(at) >= interval)
}

pub(super) fn start_json_scroll_drag_pacing(hwnd: HWND) {
    JSON_SCROLL_FRAME_CLOCK.with(|cell| {
        let mut clock = cell.borrow_mut();
        clock.last_drag_update = None;
        clock.pending_drag_y = None;
    });
    unsafe {
        // Timer messages are a fallback for the final pending pointer
        // when the mouse stops between display frames. Native timers have
        // coarse resolution; active input itself drives most frames.
        let _ = SetTimer(hwnd, JSON_SCROLL_DRAG_FRAME_TIMER_ID, 10, None);
    }
}

pub(super) fn stop_json_scroll_drag_pacing(hwnd: HWND) {
    unsafe { let _ = KillTimer(hwnd, JSON_SCROLL_DRAG_FRAME_TIMER_ID); }
    JSON_SCROLL_FRAME_CLOCK.with(|cell| {
        let mut clock = cell.borrow_mut();
        clock.last_drag_update = None;
        clock.pending_drag_y = None;
    });
}

pub(super) fn update_json_scroll_drag_paced(hwnd: HWND, y: Option<i32>, force: bool) {
    let now = Instant::now();
    let next = JSON_SCROLL_FRAME_CLOCK.with(|cell| {
        let mut frame = cell.borrow_mut();
        if let Some(y) = y { frame.pending_drag_y = Some(y); }
        frame.pending_drag_y?;
        let due = json_scroll_drag_should_apply(
            frame.last_drag_update, now, frame.refresh_hz, force,
        );
        if due {
            frame.last_drag_update = Some(now);
            frame.pending_drag_y.take()
        } else {
            None
        }
    });
    if let Some(y) = next {
        update_json_scroll_drag(hwnd, y);
    }
}

// WM_PAINT is delivered only when the queue is idle. A continuous mouse-drag
// can keep the queue busy while the edit's update region grows, causing a low
// apparent visual frame rate even though EM_LINESCROLL takes under 1 ms.
// At most once per display period, flush *already invalidated* RichEdit and
// narrow scrollbar regions without RDW_INVALIDATE / RDW_ALLCHILDREN.
fn present_json_scroll_frame_if_due(edit: HWND, panel: HWND) {
    let now = Instant::now();
    let due = JSON_SCROLL_FRAME_CLOCK.with(|cell| {
        let mut frame = cell.borrow_mut();
        if frame.last_refresh_probe.is_none_or(|probe| {
            now.saturating_duration_since(probe) >= Duration::from_secs(2)
        }) {
            let hz = unsafe {
                let dc = GetDC(edit);
                if dc.0.is_null() {
                    0
                } else {
                    let hz = GetDeviceCaps(dc, VREFRESH);
                    let _ = ReleaseDC(edit, dc);
                    hz
                }
            };
            frame.refresh_hz = json_scroll_target_hz(hz);
            frame.last_refresh_probe = Some(now);
        }
        let due = json_scroll_frame_due(frame.last_sync, now, frame.refresh_hz);
        if due {
            frame.last_sync = Some(now);
        }
        due
    });
    if !due { return; }
    let page = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().and_then(|s| {
            (s.section == Section::Json && s.pending_discard_action.is_none()
                && s.hwnd.to_hwnd() == panel && s.json_edit.to_hwnd() == edit)
                .then_some(s.json_page_surface.to_hwnd())
        })
    };
    let Some(page) = page else { return; };
    unsafe {
        let _ = UpdateWindow(edit);
        let _ = UpdateWindow(page);
    }
}

pub(super) fn invalidate_json_scroll_layout_cache() {
    JSON_SCROLL_LAYOUT_CACHE.with(|cache| *cache.borrow_mut() = None);
    JSON_SCROLL_LAST_THUMB.with(|last| *last.borrow_mut() = None);
}

pub(super) fn json_scroll_line_metrics() -> Option<(HWND, i32, i32, i32)> {
    let (edit, json_font) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.json_edit.to_hwnd(), s.json_font)
    };
    unsafe {
        let mut client = RECT::default();
        let _ = GetClientRect(edit, &mut client);
        let viewport_height = (client.bottom - client.top).max(1);
        let viewport_width = (client.right - client.left).max(1);
        let cached = JSON_SCROLL_LAYOUT_CACHE.with(|cache| {
            cache.borrow().as_ref().copied().filter(|metrics| {
                metrics.matches(edit.0 as isize, json_font, viewport_width, viewport_height)
            })
        });
        let metrics = if let Some(cached) = cached {
            cached
        } else {
            let hdc = GetDC(edit);
            if hdc.0.is_null() {
                return None;
            }
            let old_font = if json_font != 0 {
                Some(SelectObject(hdc, HGDIOBJ(json_font as *mut _)))
            } else {
                None
            };
            let mut font_metrics = TEXTMETRICW::default();
            let _ = GetTextMetricsW(hdc, &mut font_metrics);
            if let Some(old_font) = old_font {
                SelectObject(hdc, old_font);
            }
            let _ = ReleaseDC(edit, hdc);
            let metrics = JsonScrollLayoutCache {
                edit: edit.0 as isize,
                font: json_font,
                viewport_height,
                viewport_width,
                line_height: font_metrics.tmHeight.max(1),
                total_lines: (SendMessageW(
                    edit, EM_GETLINECOUNT_MSG, WPARAM(0), LPARAM(0),
                ).0 as i32).max(1),
            };
            JSON_SCROLL_LAYOUT_CACHE.with(|cache| *cache.borrow_mut() = Some(metrics));
            metrics
        };
        let first_visible =
            SendMessageW(edit, EM_GETFIRSTVISIBLELINE_MSG, WPARAM(0), LPARAM(0)).0 as i32;
        let visible_lines = (viewport_height / metrics.line_height).max(1);
        Some((edit, metrics.total_lines, visible_lines, first_visible.max(0)))
    }
}

fn json_scroll_thumb_from_lines(
    hwnd: HWND, total_lines: i32, visible_lines: i32, first_visible: i32,
) -> Option<RECT> {
    if visible_lines >= total_lines {
        return None;
    }
    let track = json_scrollbar_track_rect(hwnd);
    let track_h = (track.bottom - track.top).max(1);
    let thumb_h = (track_h * visible_lines / total_lines)
        .max(scale(hwnd, 28))
        .min(track_h);
    let max_first = (total_lines - visible_lines).max(1);
    let available = (track_h - thumb_h).max(0);
    let offset = available * first_visible.clamp(0, max_first) / max_first;
    Some(RECT {
        left: track.left + scale(hwnd, 1),
        top: track.top + offset,
        right: track.right - scale(hwnd, 1),
        bottom: track.top + offset + thumb_h,
    })
}

pub(super) fn json_scroll_thumb_rect(hwnd: HWND) -> Option<RECT> {
    let (_, total, visible, first) = json_scroll_line_metrics()?;
    json_scroll_thumb_from_lines(hwnd, total, visible, first)
}

// The scrollbar is owned visually by the opaque JSON page sibling, not by
// the parent. Invalidating only this child avoids repainting toolbars, text,
// and the entire settings tree on every wheel or drag event.
pub(super) fn invalidate_json_scrollbar(hwnd: HWND) {
    let page = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().and_then(|s| {
            (s.section == Section::Json && s.pending_discard_action.is_none())
                .then_some(s.json_page_surface.to_hwnd())
        })
    };
    if let Some(page) = page {
        let offset = scale(hwnd, 180);
        let track = json_scrollbar_track_rect(hwnd);
        let local = RECT {
            left: track.left - offset,
            top: track.top,
            right: track.right - offset,
            bottom: track.bottom,
        };
        unsafe { let _ = InvalidateRect(page, Some(&local), false); }
    }
}

pub(super) fn refresh_json_scrollbar_if_changed(hwnd: HWND) {
    let current = json_scroll_thumb_rect(hwnd).map(|thumb| (thumb.top, thumb.bottom));
    let changed = JSON_SCROLL_LAST_THUMB.with(|last| {
        let mut previous = last.borrow_mut();
        if *previous == current { false } else {
            *previous = current;
            true
        }
    });
    if changed { invalidate_json_scrollbar(hwnd); }
}

pub(super) fn json_scroll_thumb_hit_rect(hwnd: HWND) -> Option<RECT> {
    let thumb = json_scroll_thumb_rect(hwnd)?;
    Some(RECT {
        left: thumb.left - scale(hwnd, 4),
        top: thumb.top - scale(hwnd, 3),
        right: thumb.right + scale(hwnd, 4),
        bottom: thumb.bottom + scale(hwnd, 3),
    })
}

pub(super) fn json_editor_active() -> bool {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state
        .as_ref()
        .is_some_and(|s| s.section == Section::Json && s.pending_discard_action.is_none())
}

// The custom scrollbar lives in the opaque JSON page sibling, outside the
// RichEdit HWND. Wheel messages over that strip can therefore arrive at the
// parent instead of the editor. Route them to the existing RichEdit subclass
// to preserve line steps, save-mask blocking, and low-latency scroll painting.
pub(super) fn json_scrollbar_wheel_target(hwnd: HWND, x: i32, y: i32) -> Option<HWND> {
    let track = json_scrollbar_track_rect(hwnd);
    if !pt_in_rect(track, x, y)
        && !json_scroll_thumb_hit_rect(hwnd).is_some_and(|thumb| pt_in_rect(thumb, x, y))
    {
        return None;
    }
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.as_ref().and_then(|s| {
        (s.section == Section::Json && s.pending_discard_action.is_none())
            .then_some(s.json_edit.to_hwnd())
    })
}

pub(super) fn scroll_json_editor_lines(edit: HWND, panel: HWND, lines: i32) {
    if lines == 0 { return; }
    unsafe {
        // RichEdit invalidates its own changed text lines. Forcing
        // RDW_UPDATENOW also paints the entire WS_EX_COMPOSITED hierarchy
        // synchronously on every wheel message; let the message loop coalesce.
        let _ = SendMessageW(edit, EM_LINESCROLL_MSG, WPARAM(0), LPARAM(lines as isize));
    }
    refresh_json_scrollbar_if_changed(panel);
    present_json_scroll_frame_if_due(edit, panel);
}

// EM_GET/SETSCROLLPOS expose 16-bit pixel coordinates even though POINT
// contains i32 fields. Restrict absolute scrolling to a representable
// document range; long documents continue to use native relative scrolling.
const JSON_RICHEDIT_SCROLL_PIXEL_LIMIT: i64 = 32767;
const JSON_DRAG_DIRECT_MIN_LINES: i32 = 8;

pub(super) fn json_direct_drag_pixel_target(
    current_pixel_y: i32, first_visible: i32, target_first: i32,
    line_height: i32, total_lines: i32, visible_lines: i32,
) -> Option<i32> {
    if line_height <= 0 || current_pixel_y < 0 || total_lines <= visible_lines {
        return None;
    }
    let max_first = i64::from(total_lines - visible_lines);
    let max_pixel = max_first * i64::from(line_height);
    if max_pixel > JSON_RICHEDIT_SCROLL_PIXEL_LIMIT {
        return None;
    }
    let delta = i64::from(target_first) - i64::from(first_visible);
    let destination = (i64::from(current_pixel_y) + delta * i64::from(line_height))
        .clamp(0, max_pixel);
    i32::try_from(destination).ok()
}

fn json_drag_line_height() -> Option<i32> {
    JSON_SCROLL_LAYOUT_CACHE.with(|slot| slot.borrow().as_ref().map(|c| c.line_height))
}

fn update_json_scroll_drag(hwnd: HWND, y: i32) {
    let drag_offset = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else { return; };
        s.json_scroll_drag_offset
    };
    let Some((edit, total_lines, visible_lines, first_visible)) = json_scroll_line_metrics() else {
        return;
    };
    let Some(thumb) = json_scroll_thumb_from_lines(
        hwnd, total_lines, visible_lines, first_visible,
    ) else { return; };
    let track = json_scrollbar_track_rect(hwnd);
    let thumb_h = thumb.bottom - thumb.top;
    let available = (track.bottom - track.top - thumb_h).max(1);
    let top = (y - drag_offset).clamp(track.top, track.bottom - thumb_h);
    let max_first = (total_lines - visible_lines).max(1);
    let target_first =
        ((top - track.top) * max_first + available / 2) / available;
    let delta = target_first - first_visible;
    if delta != 0 {
        let mut direct_drag = false;
        unsafe {
            // For long thumb jumps avoid RichEdit's relative per-line scroll
            // path. A pixel target is faster for normal-sized JSON documents;
            // preserve the native line path if the pixel range exceeds the
            // documented 16-bit coordinate limit.
            if delta.abs() >= JSON_DRAG_DIRECT_MIN_LINES {
                if let Some(line_height) = json_drag_line_height() {
                    let mut current = POINT::default();
                    let _ = SendMessageW(edit, EM_GETSCROLLPOS_MSG, WPARAM(0),
                        LPARAM((&mut current as *mut POINT) as isize));
                    if let Some(target_y) = json_direct_drag_pixel_target(
                        current.y, first_visible, target_first, line_height,
                        total_lines, visible_lines,
                    ) {
                        let desired = POINT { x: current.x, y: target_y };
                        let _ = SendMessageW(edit, EM_SETSCROLLPOS_MSG, WPARAM(0),
                            LPARAM((&desired as *const POINT) as isize));
                        direct_drag = true;
                    }
                }
            }
            if !direct_drag {
                let _ = SendMessageW(edit, EM_LINESCROLL_MSG, WPARAM(0), LPARAM(delta as isize));
            }
        }
        refresh_json_scrollbar_if_changed(hwnd);
        present_json_scroll_frame_if_due(edit, hwnd);
    }
}

pub(super) fn reset_json_scroll_frame_clock() {
    JSON_SCROLL_FRAME_CLOCK.with(|cell| *cell.borrow_mut() = JsonScrollFrameClock::default());
}
