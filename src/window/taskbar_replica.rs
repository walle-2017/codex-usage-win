//! Taskbar-local foreground replicas. Existing children are never moved between Explorer taskbars.
use super::*;

// Bounded, lazy, per-taskbar child-window cache. Each window is created with
// its final Explorer parent, avoiding SetParent between taskbars entirely.
// A single AppState, poller, timer owner, tray registration and renderer remain.
pub(super) const TASKBAR_REPLICA_MAX_WINDOWS: usize = 4;
const TASKBAR_REPLICA_MAX_RESOURCE_DELTA: u32 = 8;
const TASKBAR_REPLICA_MAX_PRIVATE_DELTA: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct ReplicaSlot {
    pub(super) parent: SendHwnd,
    pub(super) child: SendHwnd,
}

#[derive(Clone, Copy)]
struct ReplicaResources {
    user: u32,
    gdi: u32,
    private: usize,
    working: usize,
}

thread_local! {
    pub(super) static TASKBAR_PRIMARY_WINDOW: std::cell::Cell<isize> =
        const { std::cell::Cell::new(0) };
    pub(super) static TASKBAR_REPLICA_WINDOWS: std::cell::RefCell<Vec<ReplicaSlot>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[repr(C)]
#[derive(Default)]
struct ReplicaProcessMemory {
    cb: u32,
    faults: u32,
    peak_working: usize,
    working: usize,
    quota_peak_paged: usize,
    quota_paged: usize,
    quota_peak_nonpaged: usize,
    quota_nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private_bytes: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetCurrentProcess"]
    fn replica_current_process() -> *mut std::ffi::c_void;
}
#[link(name = "user32")]
unsafe extern "system" {
    #[link_name = "GetGuiResources"]
    fn replica_gui_resources(process: *mut std::ffi::c_void, flags: u32) -> u32;
}
#[link(name = "psapi")]
unsafe extern "system" {
    #[link_name = "GetProcessMemoryInfo"]
    fn replica_process_memory(
        process: *mut std::ffi::c_void, counters: *mut ReplicaProcessMemory, cb: u32,
    ) -> i32;
}

pub(super) fn taskbar_replica_enabled() -> bool {
    // Stable taskbar placement is a normal product feature, not diagnostic mode.
    true
}

/// Stable message, timer, and notification owner. Extra taskbar HWNDs handle
/// interaction and painting; the primary HWND alone runs polling, timers,
/// settings dialogs, updater callbacks and the tray icon.
pub(super) fn taskbar_replica_service_hwnd(fallback: HWND) -> HWND {
    if !taskbar_replica_enabled() { return fallback; }
    TASKBAR_PRIMARY_WINDOW.with(|cell| {
        let primary = HWND(cell.get() as *mut _);
        if primary != HWND::default() && unsafe { IsWindow(primary).as_bool() } {
            primary
        } else {
            fallback
        }
    })
}

fn replica_resources() -> Option<ReplicaResources> {
    unsafe {
        let process = replica_current_process();
        let mut counters = ReplicaProcessMemory {
            cb: std::mem::size_of::<ReplicaProcessMemory>() as u32,
            ..Default::default()
        };
        let cb = counters.cb;
        if replica_process_memory(process, &mut counters, cb) == 0 {
            return None;
        }
        Some(ReplicaResources {
            user: replica_gui_resources(process, 1),
            gdi: replica_gui_resources(process, 0),
            private: counters.private_bytes,
            working: counters.working,
        })
    }
}

fn replica_resources_acceptable(before: Option<ReplicaResources>, after: Option<ReplicaResources>) -> bool {
    let (Some(a), Some(b)) = (before, after) else {
        diagnose::log("taskbar replica resource measurement unavailable; refusing new window");
        return false;
    };
    let user_delta = b.user.saturating_sub(a.user);
    let gdi_delta = b.gdi.saturating_sub(a.gdi);
    let private_delta = b.private.saturating_sub(a.private);
    let working_delta = b.working.saturating_sub(a.working);
    let acceptable = user_delta <= TASKBAR_REPLICA_MAX_RESOURCE_DELTA
        && gdi_delta <= TASKBAR_REPLICA_MAX_RESOURCE_DELTA
        && private_delta <= TASKBAR_REPLICA_MAX_PRIVATE_DELTA;
    diagnose::log(format!(
        "taskbar replica resource delta user={} gdi={} private_bytes={} working_set_bytes={} acceptable={} before_private={} after_private={}",
        user_delta, gdi_delta, private_delta, working_delta,
        acceptable, a.private, b.private
    ));
    acceptable
}

unsafe extern "system" fn replica_wnd_proc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        TASKBAR_REPLICA_WINDOWS.with(|cache|
            cache.borrow_mut().retain(|slot| slot.child.to_hwnd() != hwnd));
    }
    if msg == WM_DESTROY {
        if lock_state().as_ref().is_some_and(|s| s.hwnd.to_hwnd() == hwnd) {
            diagnose::log("taskbar replica active window destroyed; ending process message loop");
            PostQuitMessage(0);
        }
        return LRESULT(0);
    }
    if lock_state().as_ref().is_some_and(|s| s.hwnd.to_hwnd() == hwnd) {
        wnd_proc(hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

pub(super) fn register_taskbar_replica_class() -> bool {
    unsafe {
        let class = native_interop::wide_str("CodexUsageWinReplica");
        let hinstance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(h) => h, Err(error) => {
                diagnose::log(format!("taskbar replica class module failed={error:?}"));
                return false;
            },
        };
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(replica_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class.as_ptr()),
            ..Default::default()
        };
        let result = RegisterClassExW(&wc);
        diagnose::log(format!("taskbar replica class registered={}", result != 0));
        result != 0
    }
}

/// Returns Some(success) for a cross-taskbar replica handoff, or None
/// when no handoff is required.
pub(super) fn try_taskbar_replica_handoff(
    source_hwnd: HWND,
    index: usize,
    taskbar: &native_interop::TaskbarWindow,
) -> Option<bool> {
    if !taskbar_replica_enabled() { return None; }
    let (parent, embedded, frosted, dragging, anchor, width, height,
         old_index, old_monitor, old_tray_notify) = {
        let state = lock_state();
        let s = state.as_ref()?;
        (
            s.taskbar_hwnd?, s.embedded,
            s.composition_blur_active ||
                s.styles.active(s.is_dark).panel_frosted_strength > 0,
            s.dragging, s.drag_anchor_logical_x,
            total_widget_width_for_state(s), widget_height_for_state(s),
            s.taskbar_index, s.taskbar_monitor.clone(), s.tray_notify_hwnd
        )
    };
    if !embedded || !dragging || parent == taskbar.hwnd {
        return None;
    }
    // Each replica is parented only once, at creation. A frosted handoff
    // reuses the target foreground and that taskbar's cached Composition blur.
    let embedded_blur_handoff = frosted
        && lock_state().as_ref().is_some_and(|s| s.composition_blur_active);
    if frosted && !embedded_blur_handoff {
        diagnose::log("taskbar replica skipped frosted mode; no cross-screen mutation");
        return Some(false);
    }
    if embedded_blur_handoff {
        diagnose::log(format!(
            "taskbar embedded frosted replica handoff begin source={:?} source_parent={:?} target_parent={:?}",
            source_hwnd, parent, taskbar.hwnd
        ));
    }
    unsafe {
        if !IsWindow(source_hwnd).as_bool()
            || !IsWindow(taskbar.hwnd).as_bool()
            || GetAncestor(source_hwnd, GA_PARENT) != parent
        {
            diagnose::log("taskbar replica rejected invalid window/parent topology");
            return Some(false);
        }
    }
    // Explorer can recreate a secondary taskbar while its parked replica
    // remains cached. Drop invalid HWND/parent pairs before choosing a target
    // or counting against the bounded cache.
    let target_hwnd = TASKBAR_REPLICA_WINDOWS.with(|cache| {
        let mut slots = cache.borrow_mut();
        let before = slots.len();
        slots.retain(|slot| unsafe {
            IsWindow(slot.parent.to_hwnd()).as_bool()
                && IsWindow(slot.child.to_hwnd()).as_bool()
                && GetAncestor(slot.child.to_hwnd(), GA_PARENT) == slot.parent.to_hwnd()
        });
        if slots.len() != before {
            diagnose::log(format!(
                "taskbar replica cache pruned stale_slots={}",
                before - slots.len()
            ));
        }
        slots.iter().find(|slot| slot.parent.to_hwnd() == taskbar.hwnd)
            .map(|slot| slot.child.to_hwnd())
    }).or_else(|| {
        let primary = TASKBAR_PRIMARY_WINDOW.with(|cell| HWND(cell.get() as *mut _));
        if primary != HWND::default()
            && unsafe { IsWindow(primary).as_bool() && GetAncestor(primary, GA_PARENT) == taskbar.hwnd }
        { Some(primary) } else { None }
    });
    let mut newly_created = false;
    let mut resource_baseline = None;
    let next = if let Some(existing) = target_hwnd {
        existing
    } else {
        let count = TASKBAR_REPLICA_WINDOWS.with(|cache| cache.borrow().len()) + 1;
        if count >= TASKBAR_REPLICA_MAX_WINDOWS {
            diagnose::log("taskbar replica window limit reached; leaving source unchanged");
            return Some(false);
        }
        let before = replica_resources();
        resource_baseline = before;
        let class = native_interop::wide_str("CodexUsageWinReplica");
        let title = native_interop::wide_str("Codex usage replica");
        // A direct WS_CHILD CreateWindowExW against Explorer's HWND failed
        // repeatedly with ERROR_SUCCESS in the first field test. Instead use
        // exactly the startup path of the primary window: create a *new* hidden
        // top-level popup in our process and attach only that new HWND to its
        // target taskbar once. Never detach or reparent the source widget.
        let created = unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).ok()?;
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
                PCWSTR::from_raw(class.as_ptr()), PCWSTR::from_raw(title.as_ptr()),
                WS_POPUP,
                0, 0, width.max(1), height.max(1),
                HWND::default(), HMENU::default(), module, None
            )
        };
        let created = match created {
            Ok(hwnd) => {
                diagnose::log(format!(
                    "taskbar replica popup created hwnd={:?} parent={:?}",
                    hwnd, unsafe { GetAncestor(hwnd, GA_PARENT) }
                ));
                hwnd
            }
            Err(error) => {
                diagnose::log(format!(
                    "taskbar replica popup CreateWindowExW failed={error:?}"
                ));
                return Some(false);
            },
        };
        // This is a one-time initial embed of a NEW popup, not a transfer of
        // an existing child between two Explorer taskbar HWNDs.
        let embedded = native_interop::embed_in_taskbar(created, taskbar.hwnd);
        let actual_parent = unsafe { GetAncestor(created, GA_PARENT) };
        diagnose::log(format!(
            "taskbar replica initial-embed target={:?} result={} actual_parent={:?}",
            taskbar.hwnd, embedded, actual_parent
        ));
        if !embedded || actual_parent != taskbar.hwnd {
            unsafe { let _ = DestroyWindow(created); }
            diagnose::log("taskbar replica initial embed refused, source window unchanged");
            return Some(false);
        }
        if !replica_resources_acceptable(before, replica_resources()) {
            unsafe { let _ = DestroyWindow(created); }
            diagnose::log("taskbar replica resource guard rejected new window");
            return Some(false);
        }
        TASKBAR_REPLICA_WINDOWS.with(|cache| cache.borrow_mut().push(ReplicaSlot {
            parent: SendHwnd::from_hwnd(taskbar.hwnd),
            child: SendHwnd::from_hwnd(created)
        }));
        newly_created = true;
        created
    };
    let mut cursor = POINT::default();
    unsafe { let _ = GetCursorPos(&mut cursor); }
    let target_dpi = unsafe { GetDpiForWindow(taskbar.hwnd) }.max(1);
    let anchor_px = drag_anchor_px_for_dpi(anchor, target_dpi);
    let width = width.max(1);
    let height = height.max(1);
    let left = drag_left_from_cursor(taskbar.rect, cursor, anchor_px);
    let height_of_tray = taskbar.rect.bottom - taskbar.rect.top;
    let y = compute_anchor_y(taskbar.rect.top, height_of_tray, height) - taskbar.rect.top;
    unsafe {
        let _ = SetWindowPos(next, HWND::default(), left, y, width, height,
            SWP_NOACTIVATE | SWP_NOZORDER);
    }
    // Only the active HWND changes. No additional poller/timer, data worker,
    // renderer, tray icon, input hook or global AppState is created.
    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.hwnd = SendHwnd::from_hwnd(next);
            s.taskbar_hwnd = Some(taskbar.hwnd);
            s.taskbar_index = index;
            s.taskbar_monitor = taskbar.monitor_device.clone();
            s.tray_notify_hwnd = native_interop::find_child_window(taskbar.hwnd, "TrayNotifyWnd");
        }
    }
    unsafe {
        let _ = ShowWindow(next, SW_SHOWNOACTIVATE);
    }
    // render_layered() parks the old backdrop under its original taskbar;
    // the target restores its cached backdrop or creates one on first visit.
    // Never reparent an existing blur HWND.
    render_layered();
    if embedded_blur_handoff {
        sync_blur_backdrop_zorder(next);
    }
    // Capture the layered bitmap and USER/GDI allocations as well as the bare
    // HWND. A cheap window alone does not imply cheap compositing resources.
    let after_render_ok = !newly_created ||
        replica_resources_acceptable(resource_baseline, replica_resources());
    if !after_render_ok {
        diagnose::log("taskbar replica post-render resource guard rejected new window");
    }
    let backdrop_ready = !embedded_blur_handoff || blur_backdrop_hwnd().is_some_and(|blur| unsafe {
        IsWindow(blur).as_bool() && IsWindowVisible(blur).as_bool()
            && GetAncestor(blur, GA_PARENT) == taskbar.hwnd
    });
    if embedded_blur_handoff {
        diagnose::log(format!(
            "taskbar embedded frosted replica backdrop ready={} target_parent={:?} backdrop={:?}",
            backdrop_ready, taskbar.hwnd, blur_backdrop_hwnd()
        ));
    }
    let shown = after_render_ok && backdrop_ready && unsafe {
        IsWindow(next).as_bool() && IsWindowVisible(next).as_bool()
            && GetAncestor(next, GA_PARENT) == taskbar.hwnd
    };
    if !shown {
        // No visual switch and no state drift if a dormant target cannot show.
        {
            let mut state = lock_state();
            if let Some(s) = state.as_mut() {
                s.hwnd = SendHwnd::from_hwnd(source_hwnd);
                s.taskbar_hwnd = Some(parent);
                s.taskbar_index = old_index;
                s.taskbar_monitor = old_monitor;
                s.tray_notify_hwnd = old_tray_notify;
            }
        }
        unsafe { let _ = ShowWindow(next, SW_HIDE); }
        if newly_created {
            TASKBAR_REPLICA_WINDOWS.with(|cache|
                cache.borrow_mut().retain(|slot| slot.child.to_hwnd() != next));
            unsafe { let _ = DestroyWindow(next); }
        }
        if embedded_blur_handoff {
            // Restore both the original foreground bitmap and its original
            // taskbar-owned blur sibling after a rejected target handoff.
            render_layered();
            sync_blur_backdrop_zorder(source_hwnd);
            diagnose::log("taskbar embedded frosted replica rollback: original backdrop restored");
        }
        diagnose::log("taskbar replica show failed, original HWND restored");
        return Some(false);
    }
    unsafe { let _ = ShowWindow(source_hwnd, SW_HIDE); }
    let old_hook = lock_state().as_mut().and_then(|s| s.win_event_hook.take());
    if let Some(hook) = old_hook { native_interop::unhook_win_event(hook); }
    let tray_notify = native_interop::find_child_window(taskbar.hwnd, "TrayNotifyWnd");
    let hook = tray_notify.and_then(|tray_hwnd| {
        let thread_id = native_interop::get_window_thread_id(tray_hwnd);
        native_interop::set_tray_event_hook(thread_id, on_tray_location_changed)
    });
    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.tray_notify_hwnd = tray_notify;
            s.win_event_hook = hook;
        }
    }
    let count = TASKBAR_REPLICA_WINDOWS.with(|cache| cache.borrow().len()) + 1;
    diagnose::log(format!(
        "taskbar replica switch success source={:?} target={:?} parent={:?} created={} total_windows={} old_hidden=true",
        source_hwnd, next, taskbar.hwnd, newly_created, count
    ));
    if let Some(resources) = replica_resources() {
        diagnose::log(format!(
            "taskbar replica health active={:?} total_windows={} private_bytes={} working_set_bytes={} user_objects={} gdi_objects={}",
            next, count, resources.private, resources.working,
            resources.user, resources.gdi
        ));
    }
    if embedded_blur_handoff {
        diagnose::log(format!(
            "taskbar embedded frosted replica handoff success active={:?} parent={:?} backdrop={:?}",
            next, taskbar.hwnd, blur_backdrop_hwnd()
        ));
    }
    Some(true)
}

