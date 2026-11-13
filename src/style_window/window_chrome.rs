use super::*;

// Hide only the very first presentation from DWM. A WS_EX_COMPOSITED
// settings window can otherwise expose its caption before any client pixels
// have been committed, even though all child controls were created hidden.
pub(super) fn set_settings_initial_dwm_cloak(hwnd: HWND, cloaked: bool) -> bool {
    let value: i32 = i32::from(cloaked); // DWM expects a Win32 BOOL.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK,
            &value as *const i32 as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        ).is_ok()
    }
}

pub(super) fn apply_fixed_titlebar(hwnd: HWND) {
    let caption_color = COLORREF(FIXED_CAPTION_COLORREF);
    let text_color = COLORREF(FIXED_CAPTION_TEXT_COLORREF);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption_color as *const COLORREF as *const std::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text_color as *const COLORREF as *const std::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        );
    }
}

pub(super) fn settings_window_ex_style() -> WINDOW_EX_STYLE {
    WS_EX_APPWINDOW | WS_EX_COMPOSITED
}

// Keep other settings pages buffered. JSON has its own opaque child surface,
// so compositing its RichEdit subtree adds latency without visual benefit.
pub(super) fn settings_composition_for_section(section: Section) -> bool {
    section != Section::Json
}

pub(super) fn sync_settings_window_composition(hwnd: HWND, section: Section) {
    let enabled = settings_composition_for_section(section);
    unsafe {
        let current = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let desired = if enabled {
            current | WS_EX_COMPOSITED.0
        } else {
            current & !WS_EX_COMPOSITED.0
        };
        if desired != current {
            let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, desired as i32);
            // Apply the style change without moving or resizing the top-level
            // window. The section transition explicitly commits its own paint.
            let _ = SetWindowPos(
                hwnd, HWND::default(), 0, 0, 0, 0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE
                    | SWP_FRAMECHANGED | SWP_NOREDRAW,
            );
        }
    }
}
