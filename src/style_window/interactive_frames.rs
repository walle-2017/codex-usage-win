// Interactive settings frames: coalesced UI-thread posts and 16ms pacing.
// Keep thread-local queue state in the parent and retain the existing paint path.
use super::*;

pub(super) fn mark_settings_paint_cause(cause: SettingsPaintCause) {
    SETTINGS_PAINT_CAUSE.with(|cell| cell.set(cause));
}

// WM_PAINT is generated only when the thread's queue has no higher-priority
// messages. Persistent widget updates can leave the Settings parent dirty
// without presenting it, while its hit testing continues to work. An
// explicitly posted, coalesced frame calls UpdateWindow on the UI thread.
// Only interactive changes use this path; JSON's own child scroll rendering
// and full-page synchronous redraws retain their existing behavior.
pub(super) fn settings_interactive_frame_due(last: Option<Instant>, now: Instant, throttle: bool) -> bool {
    !throttle || last.is_none_or(|at| {
        now.saturating_duration_since(at) >= Duration::from_millis(16)
    })
}

pub(super) fn queue_settings_interactive_frame(hwnd: HWND, throttle: bool) {
    let now = Instant::now();
    let queued = SETTINGS_INTERACTIVE_FRAME_QUEUED.with(|pending| {
        if pending.get() {
            return false;
        }
        let due = SETTINGS_LAST_INTERACTIVE_FRAME.with(|last| {
            settings_interactive_frame_due(last.get(), now, throttle)
        });
        if !due {
            return false;
        }
        pending.set(true);
        true
    });
    if !queued {
        return;
    }
    if unsafe {
        PostMessageW(hwnd, WM_SETTINGS_PRESENT_INTERACTIVE_FRAME, WPARAM(0), LPARAM(0))
    }.is_ok() {
        SETTINGS_LAST_INTERACTIVE_FRAME.with(|last| last.set(Some(now)));
    } else {
        SETTINGS_INTERACTIVE_FRAME_QUEUED.with(|pending| pending.set(false));
    }
}
