use std::time::{SystemTime, UNIX_EPOCH};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, FILETIME, HWND, LPARAM, RECT, SYSTEMTIME};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETTASKBARPOS, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::*;

// Window style constants
pub const WS_POPUP_STYLE: u32 = 0x80000000;
pub const WS_CHILD_STYLE: u32 = 0x40000000;
pub const WS_CLIPSIBLINGS_STYLE: u32 = 0x04000000;

unsafe extern "C" {
    fn codex_composition_blur_create(
        hwnd_raw: isize,
        blur_amount: f32,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    ) -> *mut std::ffi::c_void;
    fn codex_composition_blur_set_bounds(
        context: *mut std::ffi::c_void,
        width: f32,
        height: f32,
    ) -> i32;
    fn codex_composition_blur_set_amount(
        context: *mut std::ffi::c_void,
        blur_amount: f32,
    ) -> i32;
    fn codex_composition_blur_set_tint(
        context: *mut std::ffi::c_void,
        r: u8,
        g: u8,
        b: u8,
        a: u8,
    ) -> i32;
    fn codex_composition_blur_destroy(context: *mut std::ffi::c_void);
    fn codex_directwrite_text_mask(
        text: *const u16,
        text_len: u32,
        font_family: *const u16,
        font_size_px: f32,
        font_weight: i32,
        width: i32,
        height: i32,
        coverage: *mut u8,
        coverage_len: usize,
    ) -> i32;
}

// Win event constants
pub const EVENT_OBJECT_LOCATIONCHANGE: u32 = 0x800B;
pub const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;

// Timer IDs
pub const TIMER_POLL: usize = 1;
pub const TIMER_COUNTDOWN: usize = 2;
pub const TIMER_RESET_POLL: usize = 3;

// Custom messages
pub const WM_APP: u32 = 0x8000;
pub const WM_APP_USAGE_UPDATED: u32 = WM_APP + 1;
pub const WM_APP_TRAY: u32 = WM_APP + 3;

const WINDOWS_TO_UNIX_EPOCH_SECONDS: u64 = 11_644_473_600;

pub fn system_time_to_local(value: SystemTime) -> Option<SYSTEMTIME> {
    let unix_seconds = value.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let file_ticks = unix_seconds
        .checked_add(WINDOWS_TO_UNIX_EPOCH_SECONDS)?
        .checked_mul(10_000_000)?;
    let file_time = FILETIME {
        dwLowDateTime: file_ticks as u32,
        dwHighDateTime: (file_ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    unsafe {
        FileTimeToSystemTime(&file_time, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
}

#[derive(Clone, Debug)]
pub struct TaskbarWindow {
    pub hwnd: HWND,
    pub rect: RECT,
    pub monitor_device: Option<String>,
}

pub fn monitor_device_name(hwnd: HWND) -> Option<String> {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut _,
        )
        .as_bool()
        {
            return None;
        }
        let len = info
            .szDevice
            .iter()
            .position(|ch| *ch == 0)
            .unwrap_or(info.szDevice.len());
        String::from_utf16(&info.szDevice[..len]).ok()
    }
}

pub fn find_taskbars() -> Vec<TaskbarWindow> {
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let taskbars = &mut *(lparam.0 as *mut Vec<TaskbarWindow>);
        let mut class_name = [0u16; 64];
        let len = unsafe { GetClassNameW(hwnd, &mut class_name) };
        if len > 0 {
            let class_name = String::from_utf16_lossy(&class_name[..len as usize]);
            if class_name == "Shell_TrayWnd" || class_name == "Shell_SecondaryTrayWnd" {
                if let Some(rect) = get_taskbar_rect(hwnd).or_else(|| get_window_rect_safe(hwnd)) {
                    taskbars.push(TaskbarWindow {
                        hwnd,
                        rect,
                        monitor_device: monitor_device_name(hwnd),
                    });
                }
            }
        }
        BOOL(1)
    }

    let mut taskbars: Vec<TaskbarWindow> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut taskbars as *mut _ as isize));
    }
    taskbars.sort_by_key(|taskbar| {
        (
            taskbar.rect.top,
            taskbar.rect.left,
            taskbar.rect.bottom,
            taskbar.rect.right,
        )
    });
    taskbars
}

/// Find a child window by class name
pub fn find_child_window(parent: HWND, class_name: &str) -> Option<HWND> {
    unsafe {
        let class = wide_str(class_name);
        match FindWindowExW(
            parent,
            HWND::default(),
            PCWSTR::from_raw(class.as_ptr()),
            PCWSTR::null(),
        ) {
            Ok(h) if h != HWND::default() => Some(h),
            _ => None,
        }
    }
}

/// Get taskbar position via SHAppBarMessage
pub fn get_taskbar_rect(taskbar_hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut class_name = [0u16; 64];
        let len = GetClassNameW(taskbar_hwnd, &mut class_name);
        if len > 0 {
            let class_name = String::from_utf16_lossy(&class_name[..len as usize]);
            if class_name == "Shell_SecondaryTrayWnd" {
                return get_window_rect_safe(taskbar_hwnd);
            }
        }

        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: taskbar_hwnd,
            ..Default::default()
        };
        let result = SHAppBarMessage(ABM_GETTASKBARPOS, &mut abd);
        if result == 0 {
            return None;
        }
        Some(abd.rc)
    }
}

/// Get the bounding rectangle of a window
pub fn get_window_rect_safe(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            Some(rect)
        } else {
            None
        }
    }
}

/// Embed our window as a child of the taskbar
pub fn embed_in_taskbar(hwnd: HWND, taskbar_hwnd: HWND) {
    unsafe {
        // Preserve existing extended style, add tool window + no activate
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongW(
            hwnd,
            GWL_EXSTYLE,
            ex_style | WS_EX_TOOLWINDOW.0 as i32 | WS_EX_NOACTIVATE.0 as i32,
        );

        // Change from popup to child
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let new_style = (style & !WS_POPUP_STYLE) | WS_CHILD_STYLE | WS_CLIPSIBLINGS_STYLE;
        let _ = SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);

        let _ = SetParent(hwnd, taskbar_hwnd);
        let _ = SetWindowPos(
            hwnd,
            HWND_NOTOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Detach the widget from Explorer and turn it back into a top-level popup.
/// The independent Windows Composition backdrop requires a top-level target;
/// the foreground remains layered and interactive above that target.
pub fn detach_from_taskbar_as_popup(hwnd: HWND) {
    unsafe {
        let _ = SetParent(hwnd, HWND::default());

        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let new_style =
            (style & !(WS_CHILD_STYLE | WS_CLIPSIBLINGS_STYLE)) | WS_POPUP_STYLE;
        let _ = SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let _ = SetWindowLongW(
            hwnd,
            GWL_EXSTYLE,
            ex_style | WS_EX_TOOLWINDOW.0 as i32 | WS_EX_NOACTIVATE.0 as i32,
        );

        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Assign an owner to a top-level popup without turning it into a child window.
/// Owned popups remain above their owner in z-order, which keeps taskbar overlays
/// visible when Explorer re-activates the taskbar.
pub fn set_popup_owner(hwnd: HWND, owner: Option<HWND>) {
    unsafe {
        let owner_value = owner.map(|h| h.0 as isize).unwrap_or(0);
        let _ = SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner_value);
        let _ = SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW,
        );
    }
}

/// Toggle WS_EX_LAYERED without changing the other extended window styles.
pub fn set_layered_style(hwnd: HWND, enabled: bool) {
    unsafe {
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let next = if enabled {
            ex_style | WS_EX_LAYERED.0 as i32
        } else {
            ex_style & !(WS_EX_LAYERED.0 as i32)
        };
        if next != ex_style {
            let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, next);
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }
}

/// Create a Windows Composition backdrop using a real Direct2D Gaussian blur.
/// The returned opaque handle must be destroyed on the same UI thread.
pub fn create_composition_blur(
    hwnd: HWND,
    blur_amount: f32,
    tint: Color,
) -> Option<usize> {
    let raw = unsafe {
        codex_composition_blur_create(
            hwnd.0 as isize,
            blur_amount,
            tint.r,
            tint.g,
            tint.b,
            tint.a,
        )
    };
    (!raw.is_null()).then_some(raw as usize)
}

pub fn set_composition_blur_bounds(context: usize, width: i32, height: i32) -> bool {
    if context == 0 || width <= 0 || height <= 0 {
        return false;
    }
    unsafe {
        codex_composition_blur_set_bounds(
            context as *mut std::ffi::c_void,
            width as f32,
            height as f32,
        ) != 0
    }
}

pub fn set_composition_blur_amount(context: usize, blur_amount: f32) -> bool {
    if context == 0 {
        return false;
    }
    unsafe {
        codex_composition_blur_set_amount(context as *mut std::ffi::c_void, blur_amount) != 0
    }
}

pub fn set_composition_blur_tint(context: usize, tint: Color) -> bool {
    if context == 0 {
        return false;
    }
    unsafe {
        codex_composition_blur_set_tint(
            context as *mut std::ffi::c_void,
            tint.r,
            tint.g,
            tint.b,
            tint.a,
        ) != 0
    }
}

pub fn destroy_composition_blur(context: usize) {
    if context != 0 {
        unsafe {
            codex_composition_blur_destroy(context as *mut std::ffi::c_void);
        }
    }
}

pub fn directwrite_text_mask(
    text: &str,
    font_family: &str,
    font_size_px: f32,
    font_weight: i32,
    width: i32,
    height: i32,
) -> Option<Vec<u8>> {
    if text.is_empty() || width <= 0 || height <= 0 || font_size_px <= 0.0 {
        return None;
    }

    let text_wide: Vec<u16> = text.encode_utf16().collect();
    let family_wide = wide_str(font_family);
    let pixel_count = (width as usize).checked_mul(height as usize)?;
    let mut coverage = vec![0u8; pixel_count];
    let ok = unsafe {
        codex_directwrite_text_mask(
            text_wide.as_ptr(),
            text_wide.len() as u32,
            family_wide.as_ptr(),
            font_size_px,
            font_weight,
            width,
            height,
            coverage.as_mut_ptr(),
            coverage.len(),
        )
    };
    (ok != 0).then_some(coverage)
}

/// Move the window
pub fn move_window(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let _ = MoveWindow(hwnd, x, y, w, h, true);
    }
}

/// Set up a WinEvent hook for tray location changes
pub fn set_tray_event_hook(
    thread_id: u32,
    callback: unsafe extern "system" fn(HWINEVENTHOOK, u32, HWND, i32, i32, u32, u32),
) -> Option<HWINEVENTHOOK> {
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_LOCATIONCHANGE,
            EVENT_OBJECT_LOCATIONCHANGE,
            None,
            Some(callback),
            0,
            thread_id,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_invalid() {
            None
        } else {
            Some(hook)
        }
    }
}

/// Get the thread ID that owns a window
pub fn get_window_thread_id(hwnd: HWND) -> u32 {
    unsafe { GetWindowThreadProcessId(hwnd, None) }
}

/// Unhook a WinEvent hook
pub fn unhook_win_event(hook: HWINEVENTHOOK) {
    unsafe {
        let _ = UnhookWinEvent(hook);
    }
}

/// Convert a Rust string to a null-terminated wide string
pub fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// COLORREF wrapper (RGB packed into u32)
pub fn colorref(r: u8, g: u8, b: u8) -> u32 {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

/// Color helper
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    #[allow(dead_code)]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub fn try_from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim().trim_start_matches('#');
        if hex.len() != 6 && hex.len() != 8 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        let a = if hex.len() == 8 {
            u8::from_str_radix(&hex[6..8], 16).ok()?
        } else {
            255
        };
        Some(Self { r, g, b, a })
    }

    pub fn from_hex(hex: &str) -> Self {
        Self::try_from_hex(hex).unwrap_or(Self::rgba(0, 0, 0, 255))
    }

    pub fn to_hex_rgba(self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
    }

    pub fn to_colorref(self) -> u32 {
        colorref(self.r, self.g, self.b)
    }

    pub fn blend_over(self, background: Self) -> Self {
        if self.a == 255 {
            return Self::rgba(self.r, self.g, self.b, 255);
        }
        let alpha = self.a as u16;
        let inv = 255u16 - alpha;
        let blend = |fg: u8, bg: u8| -> u8 {
            ((fg as u16 * alpha + bg as u16 * inv + 127) / 255) as u8
        };
        Self::rgba(
            blend(self.r, background.r),
            blend(self.g, background.g),
            blend(self.b, background.b),
            255,
        )
    }
}
