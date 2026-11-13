//! Settings window placement before child controls and the first visible frame.
//! Preserve both monitor-anchor and negotiated-DPI positioning passes.
use super::*;

pub(super) unsafe fn position_new_settings_window(hwnd: HWND, widget: HWND, widget_work_area: Option<RECT>) -> u32 {
    // Keep the original ownerless CW_USEDEFAULT creation contract. Move
    // this still-hidden window to the active widget's monitor before
    // creating child controls, so Windows can negotiate Per-Monitor DPI.
    if let Some(work) = widget_work_area {
        let anchor_dpi = GetDpiForWindow(widget).max(96);
        let scaled = |v: i32| ((v as i64 * anchor_dpi as i64 + 48) / 96) as i32;
        let width = scaled(WINDOW_WIDTH);
        let height = scaled(WINDOW_HEIGHT);
        let (x, y) = settings_centered_origin(work, width, height);
        let _ = SetWindowPos(
            hwnd, HWND::default(), x, y, width, height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    let dpi = GetDpiForWindow(hwnd).max(96);
    let s = |v: i32| ((v as i64 * dpi as i64 + 48) / 96) as i32;
    // Final resize/center is based on the settings HWND's negotiated DPI.
    // Both placement steps occur before the first ShowWindow; no jump.
    let final_width = s(WINDOW_WIDTH);
    let final_height = s(WINDOW_HEIGHT);
    let final_origin = widget_work_area.map(|work| {
        settings_centered_origin(work, final_width, final_height)
    });
    let (final_x, final_y) = final_origin.unwrap_or((0, 0));
    let position_flags = if final_origin.is_some() {
        SWP_NOZORDER | SWP_NOACTIVATE
    } else {
        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE
    };
    let _ = SetWindowPos(
        hwnd,
        HWND::default(),
        final_x,
        final_y,
        final_width,
        final_height,
        position_flags,
    );
    dpi
}
