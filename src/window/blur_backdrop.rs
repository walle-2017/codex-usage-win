//! Taskbar-child DirectComposition Gaussian blur with per-taskbar HWND/context reuse.
use super::*;

pub(super) fn blur_backdrop_hwnd() -> Option<HWND> {
    let state = BLUR_BACKDROP_HWND
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    state.as_ref().map(|h| h.to_hwnd())
}

fn blur_backdrop_context() -> Option<usize> {
    let state = BLUR_BACKDROP_CONTEXT
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *state
}

/// Snapshot the layout under STATE and release that mutex *before* calling
/// WinRT/DirectComposition. A synchronous COM callback may re-enter the HWND
/// procedure, so holding STATE across any native call can deadlock the UI when
/// horizontal/vertical desktop layout is toggled repeatedly.
#[derive(Clone, Copy)]
enum BlurBoundsSnapshot {
    Embedded { inset: f32, radius: f32 },
    Desktop {
        panel: RECT, grip: RECT, inset: f32, radius: f32,
        grip_radius: f32, grip_alpha: u8, panel_visible: bool,
    },
}

fn blur_bounds_snapshot() -> Option<BlurBoundsSnapshot> {
    let state = lock_state();
    let s = state.as_ref()?;
    let outer_radius = sc(i32::from(s.styles.active(s.is_dark).panel_corner_radius));
    let inset = (sc(1).max(1) + PANEL_BORDER_WIDTH_PX) as f32;
    let radius = (outer_radius - PANEL_BORDER_WIDTH_PX).max(0) as f32;
    if !s.floating_mode {
        return Some(BlurBoundsSnapshot::Embedded { inset, radius });
    }
    let (canvas_w, canvas_h, panel) = floating_canvas_geometry(
        total_widget_width_for_state(s), widget_height_for_state(s), s.floating_edge,
    );
    let grip = floating_grip_rect(
        canvas_w, canvas_h, s.floating_edge, sc(FLOAT_EDGE_PEEK),
    );
    let grip_radius = outer_radius.min(
        ((grip.right - grip.left).min(grip.bottom - grip.top) / 2).max(0),
    );
    Some(BlurBoundsSnapshot::Desktop {
        panel,
        grip,
        inset,
        radius,
        grip_radius: (grip_radius - PANEL_BORDER_WIDTH_PX).max(0) as f32,
        grip_alpha: if s.floating_edge.is_some() || s.dragging {
            100
        } else {
            s.floating_grip_alpha
        },
        panel_visible: !(s.floating_collapsed && s.floating_reveal == 0),
    })
}

fn sync_composition_blur_bounds(width: i32, height: i32) -> bool {
    let Some(context) = blur_backdrop_context() else { return false };
    let Some(snapshot) = blur_bounds_snapshot() else { return false };
    // Do not hold STATE or any backdrop mutex while calling the native COM
    // helper: resize and layout changes must remain re-entrancy safe.
    match snapshot {
        BlurBoundsSnapshot::Embedded { inset, radius } => {
            native_interop::set_composition_blur_bounds(
                context, width, height, inset, radius,
            )
        }
        BlurBoundsSnapshot::Desktop {
            panel, grip, inset, radius, grip_radius, grip_alpha, panel_visible,
        } => {
            native_interop::set_composition_blur_desktop_bounds(
                context, width, height, panel, grip,
                inset, radius, inset, grip_radius, grip_alpha, panel_visible,
            )
        }
    }
}

pub(super) fn destroy_blur_backdrop() {
    let context = {
        let mut state = BLUR_BACKDROP_CONTEXT
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.take()
    };
    if let Some(context) = context {
        native_interop::destroy_composition_blur(context);
    }

    {
        let mut params = BLUR_BACKDROP_PARAMS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *params = None;
    }

    let hwnd = {
        let mut state = BLUR_BACKDROP_HWND
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.take().map(|h| h.to_hwnd())
    };
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        diagnose::log("composition gaussian backdrop destroyed");
    }
}

// Only the UI thread parks and restores the active backdrop. Keep the HWND
// and its Composition target intact: reparenting either is not allowed.
fn park_embedded_blur_backdrop() {
    let (Some(hwnd), Some(_context)) = (blur_backdrop_hwnd(), blur_backdrop_context()) else {
        destroy_blur_backdrop();
        return;
    };
    let parent = unsafe { GetAncestor(hwnd, GA_PARENT) };
    if parent == HWND::default() || !unsafe { IsWindow(parent).as_bool() } {
        diagnose::log("taskbar embedded blur cache rejected stale parent");
        destroy_blur_backdrop();
        return;
    }
    let mut cache = PARKED_BLUR_BACKDROPS.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= TASKBAR_REPLICA_MAX_WINDOWS.saturating_sub(1) ||
        cache.iter().any(|entry| entry.parent.to_hwnd() == parent)
    {
        drop(cache);
        diagnose::log("taskbar embedded blur cache full or duplicate; destroying old backdrop");
        destroy_blur_backdrop();
        return;
    }
    unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
    let held_hwnd = BLUR_BACKDROP_HWND.lock().unwrap_or_else(|e| e.into_inner()).take();
    let held_context = BLUR_BACKDROP_CONTEXT.lock().unwrap_or_else(|e| e.into_inner()).take();
    let params = BLUR_BACKDROP_PARAMS.lock().unwrap_or_else(|e| e.into_inner()).take();
    match (held_hwnd, held_context) {
        (Some(held_hwnd), Some(held_context)) => {
            cache.push(ParkedBlurBackdrop {
                parent: SendHwnd::from_hwnd(parent),
                hwnd: held_hwnd,
                context: held_context,
                params,
            });
            diagnose::log(format!(
                "taskbar embedded blur cache parked parent={:?} backdrop={:?} entries={}",
                parent, hwnd, cache.len()
            ));
        }
        (held_hwnd, held_context) => {
            drop(cache);
            if let Some(context) = held_context {
                native_interop::destroy_composition_blur(context);
            }
            if let Some(hwnd) = held_hwnd {
                unsafe { let _ = DestroyWindow(hwnd.to_hwnd()); }
            }
        }
    }
}

fn restore_parked_embedded_blur_backdrop(parent: HWND) -> bool {
    let entry = {
        let mut cache = PARKED_BLUR_BACKDROPS.lock().unwrap_or_else(|e| e.into_inner());
        cache.iter().position(|s| s.parent.to_hwnd() == parent)
            .map(|index| cache.swap_remove(index))
    };
    let Some(entry) = entry else { return false };
    let hwnd = entry.hwnd.to_hwnd();
    if !unsafe { IsWindow(parent).as_bool() && IsWindow(hwnd).as_bool()
        && GetAncestor(hwnd, GA_PARENT) == parent }
    {
        native_interop::destroy_composition_blur(entry.context);
        if unsafe { IsWindow(hwnd).as_bool() } {
            unsafe { let _ = DestroyWindow(hwnd); }
        }
        diagnose::log(format!("taskbar embedded blur cache invalid target={:?}", parent));
        return false;
    }
    *BLUR_BACKDROP_HWND.lock().unwrap_or_else(|e| e.into_inner()) = Some(entry.hwnd);
    *BLUR_BACKDROP_CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(entry.context);
    *BLUR_BACKDROP_PARAMS.lock().unwrap_or_else(|e| e.into_inner()) = entry.params;
    unsafe { let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE); }
    diagnose::log(format!(
        "taskbar embedded blur cache restored target={:?} backdrop={:?}",
        parent, hwnd
    ));
    true
}

pub(super) fn destroy_parked_blur_backdrops() {
    let entries = {
        let mut cache = PARKED_BLUR_BACKDROPS.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *cache)
    };
    for entry in entries {
        native_interop::destroy_composition_blur(entry.context);
        let hwnd = entry.hwnd.to_hwnd();
        if unsafe { IsWindow(hwnd).as_bool() } {
            unsafe { let _ = DestroyWindow(hwnd); }
        }
    }
}

pub(super) fn blur_amount_for_strength(strength: u8) -> f32 {
    FROSTED_MAX_BLUR_PX * f32::from(strength.min(FROSTED_STRENGTH_MAX))
        / f32::from(FROSTED_STRENGTH_MAX)
}

pub(super) fn ensure_blur_backdrop(blur_amount: f32, tint: Color) -> Option<HWND> {
    let params = BlurBackdropParams {
        blur_bits: blur_amount.to_bits(),
        tint,
    };

    // Cache one Composition target for each taskbar and restore on return.
    let target = lock_state().as_ref().filter(|s| s.embedded).and_then(|s| s.taskbar_hwnd);
    let desktop = lock_state().as_ref().is_some_and(|s| s.floating_mode);
    if let Some(backdrop) = blur_backdrop_hwnd() {
        let child = unsafe {
            (GetWindowLongW(backdrop, GWL_STYLE) as u32) & WS_CHILD.0 != 0
        };
        if child {
            if desktop || target.is_some_and(|taskbar|
                unsafe { GetAncestor(backdrop, GA_PARENT) != taskbar }) {
                park_embedded_blur_backdrop();
            }
        } else if target.is_some() {
            destroy_blur_backdrop();
        }
    }
    if let Some(target) = target {
        if blur_backdrop_hwnd().is_none() {
            let _ = restore_parked_embedded_blur_backdrop(target);
        }
    }

    let existing_hwnd = blur_backdrop_hwnd();
    let existing_context = blur_backdrop_context();

    if let (Some(hwnd), Some(context)) = (existing_hwnd, existing_context) {
        let unchanged = {
            let cached = BLUR_BACKDROP_PARAMS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *cached == Some(params)
        };
        if unchanged {
            return Some(hwnd);
        }

        let amount_ok = native_interop::set_composition_blur_amount(context, blur_amount);
        let tint_ok = native_interop::set_composition_blur_tint(context, tint);
        if amount_ok && tint_ok {
            let mut cached = BLUR_BACKDROP_PARAMS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *cached = Some(params);
            return Some(hwnd);
        }

        destroy_blur_backdrop();
    } else if existing_hwnd.is_some() || existing_context.is_some() {
        // A partial backend state is not reusable. Recreate it atomically.
        destroy_blur_backdrop();
    }

    register_blur_backdrop_class();
    unsafe {
        let class_name = native_interop::wide_str("CodexUsageBlurBackdrop");
        let title = native_interop::wide_str("");
        let taskbar_parent = lock_state().as_ref()
            .filter(|s| s.embedded).and_then(|s| s.taskbar_hwnd);
        if taskbar_parent.is_none() && !desktop { return None; }
        let child_mode = taskbar_parent.is_some();
        let ex_style = WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP
            | if child_mode { WINDOW_EX_STYLE(0) } else { WS_EX_TOOLWINDOW };
        let window_style = if child_mode { WS_CHILD | WS_CLIPSIBLINGS } else { WS_POPUP };
        let hwnd = match CreateWindowExW(
            ex_style,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            window_style,
            0,
            0,
            1,
            1,
            taskbar_parent.unwrap_or_default(),
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                diagnose::log(format!(
                    "taskbar embedded blur CreateWindowExW failed child={} parent={:?} error={:?}",
                    child_mode, taskbar_parent, error
                ));
                return None;
            }
        };

        let Some(context) =
            native_interop::create_composition_blur(hwnd, blur_amount, tint)
        else {
            diagnose::log(format!(
                "taskbar embedded blur DirectComposition create failed child={} parent={:?}",
                child_mode, taskbar_parent
            ));
            let _ = DestroyWindow(hwnd);
            return None;
        };

        {
            let mut state = BLUR_BACKDROP_CONTEXT
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *state = Some(context);
        }
        {
            let mut cached = BLUR_BACKDROP_PARAMS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *cached = Some(params);
        }
        {
            let mut state = BLUR_BACKDROP_HWND
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *state = Some(SendHwnd::from_hwnd(hwnd));
        }

        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        diagnose::log(format!(
            "composition gaussian backdrop created taskbar_parent={:?}",
            taskbar_parent
        ));
        Some(hwnd)
    }
}

pub(super) fn sync_blur_backdrop_zorder(foreground_hwnd: HWND) {
    if widget_hidden_for_taskbar_space() {
        return;
    }
    let Some(backdrop_hwnd) = blur_backdrop_hwnd() else {
        return;
    };
    let Some(rect) = native_interop::get_window_rect_safe(foreground_hwnd) else {
        return;
    };
    let (panel_x, panel_y, width, height) = blur_panel_bounds_for_foreground(rect);
    if width <= 0 || height <= 0 {
        return;
    }

    if !sync_composition_blur_bounds(width, height) {
        diagnose::log("composition blur bounds sync failed during z-order update");
    }

    let embedded_target = lock_state().as_ref().filter(|s| s.embedded).and_then(|s| s.taskbar_hwnd);
    if let Some(target) = embedded_target {
        if let Some(tr) = native_interop::get_window_rect_safe(target) {
            unsafe {
                // The HWNDs share the taskbar parent. Put the blur directly
                // behind the layered foreground in child-window z-order.
                if let Err(error) = SetWindowPos(backdrop_hwnd, foreground_hwnd,
                    panel_x - tr.left, panel_y - tr.top, width, height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW) {
                    diagnose::log(format!("taskbar embedded blur sibling position failed parent={:?} error={:?}", target, error));
                }
            }
        }
    } else if lock_state().as_ref().is_some_and(|s| s.floating_mode) {
        unsafe {
            // Desktop blur belongs directly behind the TOPMOST foreground.
            // The desktop geometry never overlaps a reserved taskbar edge,
            // so neither popup needs to be owned by Explorer.
            // If Explorer hasn't disturbed the pair, don't change Z-order:
            // needless promotion is visible as a flash when the mouse is
            // released and the detached grip changes side.
            let fg_topmost = (GetWindowLongW(foreground_hwnd, GWL_EXSTYLE) as u32)
                & WS_EX_TOPMOST.0 != 0;
            let bg_topmost = (GetWindowLongW(backdrop_hwnd, GWL_EXSTYLE) as u32)
                & WS_EX_TOPMOST.0 != 0;
            let directly_behind = GetWindow(foreground_hwnd, GW_HWNDNEXT).ok()
                == Some(backdrop_hwnd);
            if fg_topmost && bg_topmost && directly_behind {
                return;
            }
            // Keep the foreground above its backdrop without reordering on
            // every drag frame.
            let _ = SetWindowPos(
                backdrop_hwnd, HWND_TOPMOST,
                panel_x, panel_y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = SetWindowPos(
                foreground_hwnd, HWND_TOPMOST,
                0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

fn blur_panel_bounds_for_foreground(rect: RECT) -> (i32, i32, i32, i32) {
    // Desktop mode now clips two rounded glass regions within one transparent
    // Composition surface: the original taskbar-sized body and its small tab.
    // Embedded taskbar mode continues to use its unchanged original bounds.
    (rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top)
}

pub(super) fn sync_blur_backdrop_geometry(foreground_hwnd: HWND) {
    let Some(backdrop_hwnd) = blur_backdrop_hwnd() else {
        return;
    };
    let Some(rect) = native_interop::get_window_rect_safe(foreground_hwnd) else {
        return;
    };
    let (panel_x, panel_y, width, height) = blur_panel_bounds_for_foreground(rect);
    if width <= 0 || height <= 0 {
        return;
    }
    if !sync_composition_blur_bounds(width, height) {
        diagnose::log("composition blur bounds sync failed during geometry update");
    }
    let parent_rect = lock_state().as_ref().filter(|s| s.embedded)
        .and_then(|s| s.taskbar_hwnd)
        .and_then(native_interop::get_window_rect_safe);
    let (x, y) = parent_rect.map(|tr| (panel_x - tr.left, panel_y - tr.top))
        .unwrap_or((panel_x, panel_y));
    unsafe {
        let _ = SetWindowPos(
            backdrop_hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub(super) fn move_window_without_repaint(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub(super) fn move_frosted_pair(foreground_hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    let Some(backdrop_hwnd) = blur_backdrop_hwnd() else {
        move_window_without_repaint(foreground_hwnd, x, y, width, height);
        return;
    };
    let (panel_x, panel_y, panel_w, panel_h) = blur_panel_bounds_for_foreground(
        RECT { left: x, top: y, right: x + width, bottom: y + height },
    );
    // Shrink/expand the visual tree first. The current HWND bounds clip an
    // expansion, while an early visual shrink prevents a stale-DPI blur tail.
    if !sync_composition_blur_bounds(panel_w, panel_h) {
        diagnose::log("composition blur bounds sync failed during drag");
    }
    unsafe {
        // Preserve existing owner/z-order. Re-ordering two top-level windows on
        // every mouse move causes visible DWM flicker.
        let flags = SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(
            backdrop_hwnd,
            HWND::default(),
            panel_x,
            panel_y,
            panel_w,
            panel_h,
            flags,
        );
        let _ = SetWindowPos(
            foreground_hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            flags,
        );
    }
}

pub(super) fn activate_blur_backdrop(hwnd: HWND, blur_amount: f32, tint: Color) -> bool {
    if !lock_state().as_ref().is_some_and(|s| s.embedded || s.floating_mode) {
        return false;
    }
    if ensure_blur_backdrop(blur_amount, tint).is_none() {
        diagnose::log("taskbar embedded blur creation failed");
        return false;
    }
    if let Some(s) = lock_state().as_mut() {
        s.composition_blur_active = true;
    }
    sync_blur_backdrop_zorder(hwnd);
    true
}

pub(super) fn restore_layered_taskbar_mode(hwnd: HWND) {
    destroy_blur_backdrop();
    destroy_parked_blur_backdrops();
    native_interop::set_layered_style(hwnd, true);
    if let Some(s) = lock_state().as_mut() {
        s.composition_blur_active = false;
    }
    if lock_state().as_ref().is_some_and(|s| s.embedded) {
        position_at_taskbar();
    }
}

