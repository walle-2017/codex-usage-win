use std::sync::atomic::{AtomicIsize, Ordering};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::native_interop::{self, Color};

const WINDOW_CLASS: &str = "CodexUsagePopupMenuV1";
const ROOT_WIDTH: i32 = 300;
const SUBMENU_WIDTH: i32 = 238;
const OUTER_PADDING: i32 = 8;
const ITEM_HEIGHT: i32 = 40;
const SEPARATOR_HEIGHT: i32 = 14;
const ITEM_RADIUS: i32 = 6;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const CS_DROPSHADOW_VALUE: u32 = 0x0002_0000;
const DWMWA_WINDOW_CORNER_PREFERENCE_VALUE: i32 = 33;
const DWMWCP_ROUND_VALUE: u32 = 2;

static ROOT_POPUP: AtomicIsize = AtomicIsize::new(0);

#[derive(Clone)]
pub enum PopupAction {
    Command(u16),
    Separator,
    Submenu(Vec<PopupItem>),
}

#[derive(Clone)]
pub struct PopupItem {
    pub text: String,
    pub enabled: bool,
    pub action: PopupAction,
}

impl PopupItem {
    pub fn command(text: impl Into<String>, command: u16) -> Self {
        Self {
            text: text.into(),
            enabled: true,
            action: PopupAction::Command(command),
        }
    }

    pub fn disabled_command(text: impl Into<String>, command: u16) -> Self {
        Self {
            text: text.into(),
            enabled: false,
            action: PopupAction::Command(command),
        }
    }

    pub fn separator() -> Self {
        Self {
            text: String::new(),
            enabled: false,
            action: PopupAction::Separator,
        }
    }

    pub fn submenu(text: impl Into<String>, items: Vec<PopupItem>) -> Self {
        Self {
            text: text.into(),
            enabled: true,
            action: PopupAction::Submenu(items),
        }
    }
}

#[derive(Clone, Copy)]
struct Palette {
    background: Color,
    hover: Color,
    border: Color,
    text: Color,
    disabled: Color,
    separator: Color,
}

impl Palette {
    fn for_windows_theme(dark: bool) -> Self {
        if dark {
            Self {
                background: Color::from_hex("#1F1F1FFF"),
                hover: Color::from_hex("#2D2D2FFF"),
                border: Color::from_hex("#343434FF"),
                text: Color::from_hex("#F4F4F4FF"),
                disabled: Color::from_hex("#858585FF"),
                separator: Color::from_hex("#4A4A4AFF"),
            }
        } else {
            Self {
                background: Color::from_hex("#FAFAFAFF"),
                hover: Color::from_hex("#EEEEF0FF"),
                border: Color::from_hex("#D8D8D8FF"),
                text: Color::from_hex("#202020FF"),
                disabled: Color::from_hex("#929292FF"),
                separator: Color::from_hex("#DEDEDEFF"),
            }
        }
    }
}

struct PopupState {
    command_target: HWND,
    root_hwnd: HWND,
    items: Vec<PopupItem>,
    hover: Option<usize>,
    submenu_hwnd: Option<HWND>,
    dark: bool,
    font_face: String,
    font: isize,
    is_root: bool,
}

fn scale_for_dpi(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

fn dpi_for_target(target: HWND) -> u32 {
    unsafe {
        let dpi = GetDpiForWindow(target);
        if dpi == 0 { 96 } else { dpi }
    }
}

fn item_height(item: &PopupItem, dpi: u32) -> i32 {
    scale_for_dpi(
        if matches!(item.action, PopupAction::Separator) {
            SEPARATOR_HEIGHT
        } else {
            ITEM_HEIGHT
        },
        dpi,
    )
}

fn menu_height(items: &[PopupItem], dpi: u32) -> i32 {
    let padding = scale_for_dpi(OUTER_PADDING, dpi);
    padding * 2 + items.iter().map(|item| item_height(item, dpi)).sum::<i32>()
}

fn item_rect(items: &[PopupItem], index: usize, width: i32, dpi: u32) -> RECT {
    let padding = scale_for_dpi(OUTER_PADDING, dpi);
    let mut top = padding;
    for item in items.iter().take(index) {
        top += item_height(item, dpi);
    }
    RECT {
        left: padding,
        top,
        right: width - padding,
        bottom: top + item_height(&items[index], dpi),
    }
}

fn hit_item(items: &[PopupItem], y: i32, dpi: u32) -> Option<usize> {
    let padding = scale_for_dpi(OUTER_PADDING, dpi);
    let mut top = padding;
    for (index, item) in items.iter().enumerate() {
        let bottom = top + item_height(item, dpi);
        if y >= top && y < bottom {
            return (!matches!(item.action, PopupAction::Separator)).then_some(index);
        }
        top = bottom;
    }
    None
}

unsafe fn register_window_class() {
    let class_name = native_interop::wide_str(WINDOW_CLASS);
    let instance = GetModuleHandleW(PCWSTR::null()).unwrap();
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CLASS_STYLE(CS_HREDRAW.0 | CS_VREDRAW.0 | CS_DROPSHADOW_VALUE),
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance.into(),
        hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
        lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
        ..Default::default()
    };
    let _ = RegisterClassExW(&class);
}

unsafe fn apply_rounded_corners(hwnd: HWND) {
    let preference = DWMWCP_ROUND_VALUE;
    let _ = DwmSetWindowAttribute(
        hwnd,
        DWMWINDOWATTRIBUTE(DWMWA_WINDOW_CORNER_PREFERENCE_VALUE),
        (&preference as *const u32).cast(),
        std::mem::size_of::<u32>() as u32,
    );
}

unsafe fn create_font(hwnd: HWND, face: &str) -> isize {
    let dpi = GetDpiForWindow(hwnd).max(96);
    let face = native_interop::wide_str(face);
    let font = CreateFontW(
        -scale_for_dpi(14, dpi),
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
        PCWSTR::from_raw(face.as_ptr()),
    );
    font.0 as isize
}

unsafe fn set_state(hwnd: HWND, state: *mut PopupState) {
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
}

unsafe fn state_ptr(hwnd: HWND) -> *mut PopupState {
    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PopupState
}

unsafe fn close_submenu(state: &mut PopupState) {
    if let Some(submenu) = state.submenu_hwnd.take() {
        let _ = DestroyWindow(submenu);
    }
}

unsafe fn open_submenu(hwnd: HWND, index: usize) {
    let ptr = state_ptr(hwnd);
    if ptr.is_null() {
        return;
    }
    let state = &mut *ptr;
    let PopupAction::Submenu(items) = state.items[index].action.clone() else {
        close_submenu(state);
        return;
    };

    close_submenu(state);

    let dpi = GetDpiForWindow(hwnd).max(96);
    let width = scale_for_dpi(ROOT_WIDTH, dpi);
    let row = item_rect(&state.items, index, width, dpi);
    let mut origin = POINT {
        x: row.right,
        y: row.top,
    };
    let _ = ClientToScreen(hwnd, &mut origin);

    let submenu = create_window(
        state.command_target,
        state.root_hwnd,
        items,
        state.dark,
        state.font_face.clone(),
        false,
        POINT {
            x: origin.x - scale_for_dpi(2, dpi),
            y: origin.y - scale_for_dpi(OUTER_PADDING, dpi),
        },
    );
    if !submenu.0.is_null() {
        state.submenu_hwnd = Some(submenu);
    }
}

unsafe fn menu_work_area(point: POINT) -> RECT {
    let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(monitor, &mut info).as_bool() {
        info.rcWork
    } else {
        RECT {
            left: 0,
            top: 0,
            right: GetSystemMetrics(SM_CXSCREEN),
            bottom: GetSystemMetrics(SM_CYSCREEN),
        }
    }
}

unsafe fn create_window(
    command_target: HWND,
    root_hwnd: HWND,
    items: Vec<PopupItem>,
    dark: bool,
    font_face: String,
    is_root: bool,
    desired: POINT,
) -> HWND {
    register_window_class();

    let dpi = dpi_for_target(command_target);
    let logical_width = if is_root { ROOT_WIDTH } else { SUBMENU_WIDTH };
    let width = scale_for_dpi(logical_width, dpi);
    let height = menu_height(&items, dpi);
    let work = menu_work_area(desired);

    let mut x = desired.x;
    let mut y = desired.y;
    if x + width > work.right {
        x = (desired.x - width).max(work.left);
    }
    if y + height > work.bottom {
        y = (work.bottom - height).max(work.top);
    }
    x = x.clamp(work.left, (work.right - width).max(work.left));
    y = y.clamp(work.top, (work.bottom - height).max(work.top));

    let state = Box::new(PopupState {
        command_target,
        root_hwnd,
        items,
        hover: None,
        submenu_hwnd: None,
        dark,
        font_face,
        font: 0,
        is_root,
    });
    let raw = Box::into_raw(state);

    let class_name = native_interop::wide_str(WINDOW_CLASS);
    let empty = native_interop::wide_str("");
    let ex_style = if is_root {
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST
    } else {
        WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE
    };
    let hwnd = match CreateWindowExW(
        ex_style,
        PCWSTR::from_raw(class_name.as_ptr()),
        PCWSTR::from_raw(empty.as_ptr()),
        WS_POPUP,
        x,
        y,
        width,
        height,
        if is_root { command_target } else { root_hwnd },
        HMENU::default(),
        GetModuleHandleW(PCWSTR::null()).unwrap(),
        Some(raw.cast()),
    ) {
        Ok(hwnd) => hwnd,
        Err(_) => {
            drop(Box::from_raw(raw));
            return HWND::default();
        }
    };

    let state = &mut *raw;
    if is_root {
        state.root_hwnd = hwnd;
    }
    state.font = create_font(hwnd, &state.font_face);
    apply_rounded_corners(hwnd);

    let _ = SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        width,
        height,
        SWP_SHOWWINDOW | SWP_NOACTIVATE,
    );
    let _ = UpdateWindow(hwnd);
    hwnd
}

pub fn show(
    command_target: HWND,
    items: Vec<PopupItem>,
    dark: bool,
    font_face: impl Into<String>,
) {
    unsafe {
        close();
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let hwnd = create_window(
            command_target,
            HWND::default(),
            items,
            dark,
            font_face.into(),
            true,
            point,
        );
        if hwnd.0.is_null() {
            return;
        }

        ROOT_POPUP.store(hwnd.0 as isize, Ordering::Release);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(hwnd);
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub fn close() {
    let raw = ROOT_POPUP.swap(0, Ordering::AcqRel);
    if raw != 0 {
        unsafe {
            let _ = DestroyWindow(HWND(raw as *mut _));
        }
    }
}

unsafe fn paint(hwnd: HWND, state: &PopupState) {
    let mut ps = PAINTSTRUCT::default();
    let screen = BeginPaint(hwnd, &mut ps);

    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let width = client.right.max(1);
    let height = client.bottom.max(1);
    let dpi = GetDpiForWindow(hwnd).max(96);
    let palette = Palette::for_windows_theme(state.dark);

    let mem = CreateCompatibleDC(screen);
    let bitmap = CreateCompatibleBitmap(screen, width, height);
    let old_bitmap = SelectObject(mem, HGDIOBJ(bitmap.0));

    let background = CreateSolidBrush(COLORREF(palette.background.to_colorref()));
    let _ = FillRect(mem, &client, background);
    let _ = DeleteObject(background);

    let border = CreateSolidBrush(COLORREF(palette.border.to_colorref()));
    let _ = FrameRect(mem, &client, border);
    let _ = DeleteObject(border);

    let old_font = if state.font != 0 {
        SelectObject(mem, HGDIOBJ(state.font as *mut _))
    } else {
        HGDIOBJ::default()
    };
    let _ = SetBkMode(mem, TRANSPARENT);

    for (index, item) in state.items.iter().enumerate() {
        let row = item_rect(&state.items, index, width, dpi);
        if matches!(item.action, PopupAction::Separator) {
            let y = (row.top + row.bottom) / 2;
            let separator = CreateSolidBrush(COLORREF(palette.separator.to_colorref()));
            let line = RECT {
                left: row.left + scale_for_dpi(8, dpi),
                top: y,
                right: row.right - scale_for_dpi(8, dpi),
                bottom: y + 1,
            };
            let _ = FillRect(mem, &line, separator);
            let _ = DeleteObject(separator);
            continue;
        }

        if state.hover == Some(index) && item.enabled {
            let brush = CreateSolidBrush(COLORREF(palette.hover.to_colorref()));
            let region = CreateRoundRectRgn(
                row.left,
                row.top + scale_for_dpi(2, dpi),
                row.right + 1,
                row.bottom - scale_for_dpi(2, dpi) + 1,
                scale_for_dpi(ITEM_RADIUS * 2, dpi),
                scale_for_dpi(ITEM_RADIUS * 2, dpi),
            );
            let _ = FillRgn(mem, region, brush);
            let _ = DeleteObject(region);
            let _ = DeleteObject(brush);
        }

        let foreground = if item.enabled {
            palette.text
        } else {
            palette.disabled
        };
        let _ = SetTextColor(mem, COLORREF(foreground.to_colorref()));
        let mut text = native_interop::wide_str(&item.text);
        let text_len = text.len().saturating_sub(1);
        let mut text_rect = RECT {
            left: row.left + scale_for_dpi(14, dpi),
            top: row.top,
            right: row.right - scale_for_dpi(40, dpi),
            bottom: row.bottom,
        };
        let _ = DrawTextW(
            mem,
            &mut text[..text_len],
            &mut text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );

        if matches!(item.action, PopupAction::Submenu(_)) {
            let mut arrow = native_interop::wide_str("›");
            let arrow_len = arrow.len().saturating_sub(1);
            let mut arrow_rect = RECT {
                left: row.right - scale_for_dpi(32, dpi),
                top: row.top,
                right: row.right - scale_for_dpi(10, dpi),
                bottom: row.bottom,
            };
            let _ = DrawTextW(
                mem,
                &mut arrow[..arrow_len],
                &mut arrow_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }

    if state.font != 0 {
        SelectObject(mem, old_font);
    }
    let _ = BitBlt(screen, 0, 0, width, height, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_bitmap);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            let raw = create.lpCreateParams as *mut PopupState;
            set_state(hwnd, raw);
            LRESULT(1)
        }
        WM_PAINT => {
            let raw = state_ptr(hwnd);
            if !raw.is_null() {
                paint(hwnd, &*raw);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            let raw = state_ptr(hwnd);
            if raw.is_null() {
                return LRESULT(0);
            }
            let state = &mut *raw;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let dpi = GetDpiForWindow(hwnd).max(96);
            let hover = hit_item(&state.items, y, dpi);
            if hover != state.hover {
                state.hover = hover;
                if let Some(index) = hover {
                    if matches!(state.items[index].action, PopupAction::Submenu(_)) {
                        open_submenu(hwnd, index);
                    } else {
                        close_submenu(state);
                    }
                }
                let _ = InvalidateRect(hwnd, None, false);
            }
            let mut track = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: hwnd,
                ..Default::default()
            };
            let _ = TrackMouseEvent(&mut track);
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            let raw = state_ptr(hwnd);
            if !raw.is_null() {
                let state = &mut *raw;
                state.hover = None;
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let raw = state_ptr(hwnd);
            if raw.is_null() {
                return LRESULT(0);
            }
            let state = &mut *raw;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let dpi = GetDpiForWindow(hwnd).max(96);
            let Some(index) = hit_item(&state.items, y, dpi) else {
                return LRESULT(0);
            };
            if !state.items[index].enabled {
                return LRESULT(0);
            }
            match &state.items[index].action {
                PopupAction::Command(command) => {
                    let _ = PostMessageW(
                        state.command_target,
                        WM_COMMAND,
                        WPARAM(*command as usize),
                        LPARAM(0),
                    );
                    let root = state.root_hwnd;
                    if !root.0.is_null() {
                        let _ = DestroyWindow(root);
                    }
                }
                PopupAction::Submenu(_) => open_submenu(hwnd, index),
                PopupAction::Separator => {}
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if wparam.0 as u32 == VK_ESCAPE.0 {
                close();
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_ACTIVATE => {
            let raw = state_ptr(hwnd);
            if !raw.is_null() {
                let state = &*raw;
                if state.is_root && (wparam.0 & 0xFFFF) == WA_INACTIVE.0 as usize {
                    close();
                    return LRESULT(0);
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_NCDESTROY => {
            let raw = state_ptr(hwnd);
            if !raw.is_null() {
                set_state(hwnd, std::ptr::null_mut());
                let mut state = Box::from_raw(raw);
                close_submenu(&mut state);
                if state.font != 0 {
                    let _ = DeleteObject(HGDIOBJ(state.font as *mut _));
                }
                if state.is_root {
                    let _ = ROOT_POPUP.compare_exchange(
                        hwnd.0 as isize,
                        0,
                        Ordering::AcqRel,
                        Ordering::Relaxed,
                    );
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
