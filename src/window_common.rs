//! Small Win32 helpers shared by the taskbar widget and settings window.
//! Window handles are transferable only as opaque values; operations on the
//! underlying HWND must still use the appropriate Windows/UI-thread rules.

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::UI::Shell::ExtractIconExW;
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageW, HICON, ICON_BIG, ICON_SMALL, WM_SETICON,
};
use windows::Win32::Foundation::{LPARAM, WPARAM};

/// Transport a HWND handle value between threads; this is not a guarantee
/// that manipulating its window from another thread is safe.
#[derive(Clone, Copy)]
pub(crate) struct SendHwnd(isize);

// Only the integer handle value is moved; callers keep native UI operations
// on the owning thread or use asynchronous window messages.
unsafe impl Send for SendHwnd {}

impl SendHwnd {
    pub(crate) fn from_hwnd(hwnd: HWND) -> Self {
        Self(hwnd.0 as isize)
    }

    pub(crate) fn to_hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

/// Load both caption icons from the resource embedded in the running binary.
pub(crate) fn load_embedded_app_icons() -> (HICON, HICON) {
    unsafe {
        let mut exe_buf = [0u16; 260];
        let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
        if len == 0 {
            return (HICON::default(), HICON::default());
        }

        let mut large_icon = HICON::default();
        let mut small_icon = HICON::default();
        let extracted = ExtractIconExW(
            PCWSTR::from_raw(exe_buf.as_ptr()),
            0,
            Some(&mut large_icon),
            Some(&mut small_icon),
            1,
        );

        if extracted == 0 {
            (HICON::default(), HICON::default())
        } else {
            (large_icon, small_icon)
        }
    }
}

/// Set the caption icons after window creation. Do not call while holding
/// application-state locks: SendMessageW is synchronous and can re-enter.
pub(crate) fn set_window_caption_icons(hwnd: HWND, large_icon: HICON, small_icon: HICON) {
    unsafe {
        if !large_icon.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_BIG as usize),
                LPARAM(large_icon.0 as isize),
            );
        }
        if !small_icon.is_invalid() {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                WPARAM(ICON_SMALL as usize),
                LPARAM(small_icon.0 as isize),
            );
        }
    }
}
