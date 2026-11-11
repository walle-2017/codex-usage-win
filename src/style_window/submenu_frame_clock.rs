// High-resolution Desktop submenu frame clock and WinMM fallback.
// The multimedia callback only posts to the UI thread; state and GDI remain on that thread.
use super::*;

type MultimediaTimerCallback = unsafe extern "system" fn(u32, u32, usize, usize, usize);

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
    fn timeSetEvent(
        delay: u32, resolution: u32, callback: Option<MultimediaTimerCallback>,
        user: usize, flags: u32,
    ) -> u32;
    fn timeKillEvent(timer_id: u32) -> u32;
}

// The multimedia callback is NOT on the UI thread. Do not touch STATE,
// window DCs, or thread-local rendering caches here; only post a single
// pending animation message. A queued flag prevents unbounded message buildup.
unsafe extern "system" fn submenu_multimedia_timer_callback(
    _timer_id: u32, _message: u32, user: usize, _dw1: usize, _dw2: usize,
) {
    if SUBMENU_MEDIA_TICK_QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    if PostMessageW(
        HWND(user as *mut _), WM_SETTINGS_MULTIMEDIA_ANIMATION_TICK, WPARAM(0), LPARAM(0),
    ).is_err() {
        SUBMENU_MEDIA_TICK_QUEUED.store(false, Ordering::Release);
    }
}

fn begin_precise_animation_timer() {
    PRECISE_ANIMATION_TIMER.with(|slot| {
        let mut enabled = slot.borrow_mut();
        if !*enabled {
            // Temporarily request a finer timer resolution; always release
            // it after the animation to avoid affecting idle power usage.
            *enabled = unsafe { timeBeginPeriod(1) == 0 };
        }
    });
}

fn end_precise_animation_timer() {
    PRECISE_ANIMATION_TIMER.with(|slot| {
        let mut enabled = slot.borrow_mut();
        if *enabled {
            unsafe { let _ = timeEndPeriod(1); }
            *enabled = false;
        }
    });
}

pub(super) fn begin_submenu_frame_clock(hwnd: HWND) {
    begin_precise_animation_timer();
    let active = SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| cell.get() != 0);
    if active { return; }
    // Windows multimedia periodic timers are more suitable for frame cadence
    // than synthesized low-priority WM_TIMER messages.
    let id = unsafe {
        timeSetEvent(
            DESKTOP_SUBMENU_FRAME_MS, 1, Some(submenu_multimedia_timer_callback),
            hwnd.0 as usize, TIME_PERIODIC_FLAG | TIME_KILL_SYNCHRONOUS_FLAG,
        )
    };
    if id != 0 {
        SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| cell.set(id));
        unsafe { let _ = KillTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID); }
    } else {
        // On constrained or older systems, retain the standard Win32 timer.
        unsafe {
            let _ = SetTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID, DESKTOP_SUBMENU_FRAME_MS, None);
        }
    }
}

pub(super) fn end_submenu_frame_clock(hwnd: HWND) {
    SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| {
        let id = cell.replace(0);
        if id != 0 {
            // The synchronous flag guarantees the callback has finished.
            unsafe { let _ = timeKillEvent(id); }
        }
    });
    SUBMENU_MEDIA_TICK_QUEUED.store(false, Ordering::Release);
    unsafe { let _ = KillTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID); }
    end_precise_animation_timer();
}

unsafe pub(super) fn drive_submenu_frame_tick(hwnd: HWND, multimedia: bool) {
    let now = Instant::now();
    let (active, rendered) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            if multimedia {
                s.desktop_submenu.tick_for_paint_with_gate(
                    now, DESKTOP_SUBMENU_MEDIA_MIN_FRAME_MS,
                )
            } else {
                s.desktop_submenu.tick_for_paint(now)
            }
        } else { (false, false) }
    };
    if !active {
        end_submenu_frame_clock(hwnd);
    }
    if rendered {
        mark_settings_paint_cause(SettingsPaintCause::AnimationTimer);
        let _ = InvalidateRect(hwnd, Some(&settings_card_rect(hwnd, 58, 206)), false);
        queue_settings_interactive_frame(hwnd, active);
    }
}
