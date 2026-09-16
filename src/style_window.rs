use std::sync::Mutex;

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::appearance::AppearancePreset;
use crate::localization::LanguageId;
use crate::native_interop::{self, Color, WM_APP};
use crate::style::{StyleColorTarget, ThemeMode, ThemeStyle, FROSTED_STRENGTH_MAX};

// Keep this block well away from updater.rs (WM_APP + 21..23).
pub const WM_STYLE_COLOR_PREVIEW: u32 = WM_APP + 120;
pub const WM_STYLE_BLUR_PREVIEW: u32 = WM_APP + 121;
pub const WM_STYLE_SAVE: u32 = WM_APP + 122;
pub const WM_STYLE_THEME_CHANGE: u32 = WM_APP + 123;
pub const WM_STYLE_LAYOUT_CHANGE: u32 = WM_APP + 124;
pub const WM_STYLE_RESET_CURRENT: u32 = WM_APP + 125;

const WINDOW_CLASS: &str = "CodexUsageStyleSettingsV1";
const WINDOW_WIDTH: i32 = 820;
const WINDOW_HEIGHT: i32 = 570;

#[derive(Clone)]
pub struct StyleWindowSnapshot {
    pub language: LanguageId,
    pub theme_mode: ThemeMode,
    pub is_dark: bool,
    pub appearance_preset: AppearancePreset,
    pub active_style: ThemeStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Panel,
    Text,
    Progress,
    Interaction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorSelection {
    Color(StyleColorTarget),
    Blur,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SliderKind {
    Red,
    Green,
    Blue,
    Alpha,
    Blur,
}

#[derive(Clone, Copy)]
struct SendHwnd(isize);

unsafe impl Send for SendHwnd {}

impl SendHwnd {
    fn from_hwnd(hwnd: HWND) -> Self {
        Self(hwnd.0 as isize)
    }

    fn to_hwnd(self) -> HWND {
        HWND(self.0 as *mut _)
    }
}

struct PanelState {
    hwnd: SendHwnd,
    parent: SendHwnd,
    snapshot: StyleWindowSnapshot,
    section: Section,
    editor: EditorSelection,
    dragging_slider: Option<SliderKind>,
    font: isize,
}

unsafe impl Send for PanelState {}

static STATE: Mutex<Option<PanelState>> = Mutex::new(None);

pub fn open_or_focus(parent: HWND, snapshot: StyleWindowSnapshot) {
    let existing = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.hwnd.to_hwnd())
    };
    if let Some(hwnd) = existing {
        sync(snapshot);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }

    unsafe {
        let class_name = native_interop::wide_str(WINDOW_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: GetModuleHandleW(PCWSTR::null()).unwrap().into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);

        let title = native_interop::wide_str(if snapshot.language == LanguageId::SimplifiedChinese {
            "样式设置"
        } else {
            "Style settings"
        });
        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            parent,
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => return,
        };

        let dpi = GetDpiForWindow(hwnd).max(96);
        let s = |v: i32| ((v as i64 * dpi as i64 + 48) / 96) as i32;
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            0,
            0,
            s(WINDOW_WIDTH),
            s(WINDOW_HEIGHT),
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let font_name = native_interop::wide_str("Segoe UI");
        let font = CreateFontW(
            -s(14),
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
            PCWSTR::from_raw(font_name.as_ptr()),
        );

        {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            *state = Some(PanelState {
                hwnd: SendHwnd::from_hwnd(hwnd),
                parent: SendHwnd::from_hwnd(parent),
                snapshot,
                section: Section::Panel,
                editor: EditorSelection::Color(StyleColorTarget::PanelBackground),
                dragging_slider: None,
                font: font.0 as isize,
            });
        }

        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn sync(snapshot: StyleWindowSnapshot) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.snapshot = snapshot;
        s.hwnd.to_hwnd()
    };
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub fn decode_color_target(value: usize) -> Option<StyleColorTarget> {
    Some(match value {
        0 => StyleColorTarget::PanelBackground,
        1 => StyleColorTarget::PanelBorder,
        2 => StyleColorTarget::QuotaType,
        3 => StyleColorTarget::Remaining,
        4 => StyleColorTarget::ResetTime,
        5 => StyleColorTarget::Error,
        6 => StyleColorTarget::ProgressHigh,
        7 => StyleColorTarget::ProgressMedium,
        8 => StyleColorTarget::ProgressLow,
        9 => StyleColorTarget::ProgressConsumed,
        10 => StyleColorTarget::DragHandle,
        _ => return None,
    })
}

pub fn unpack_color(value: isize) -> Color {
    let value = value as u32;
    Color::rgba(
        (value & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        ((value >> 16) & 0xFF) as u8,
        ((value >> 24) & 0xFF) as u8,
    )
}

fn encode_color_target(target: StyleColorTarget) -> usize {
    match target {
        StyleColorTarget::PanelBackground => 0,
        StyleColorTarget::PanelBorder => 1,
        StyleColorTarget::QuotaType => 2,
        StyleColorTarget::Remaining => 3,
        StyleColorTarget::ResetTime => 4,
        StyleColorTarget::Error => 5,
        StyleColorTarget::ProgressHigh => 6,
        StyleColorTarget::ProgressMedium => 7,
        StyleColorTarget::ProgressLow => 8,
        StyleColorTarget::ProgressConsumed => 9,
        StyleColorTarget::DragHandle => 10,
    }
}

fn pack_color(color: Color) -> isize {
    (u32::from(color.r)
        | (u32::from(color.g) << 8)
        | (u32::from(color.b) << 16)
        | (u32::from(color.a) << 24)) as isize
}

fn window_dpi(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd).max(96) }
}

fn scale(hwnd: HWND, value: i32) -> i32 {
    let dpi = window_dpi(hwnd);
    ((value as i64 * dpi as i64 + 48) / 96) as i32
}

fn rect(hwnd: HWND, left: i32, top: i32, right: i32, bottom: i32) -> RECT {
    RECT {
        left: scale(hwnd, left),
        top: scale(hwnd, top),
        right: scale(hwnd, right),
        bottom: scale(hwnd, bottom),
    }
}

fn point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    (
        (lparam.0 & 0xFFFF) as i16 as i32,
        ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
    )
}

fn pt_in_rect(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn section_rect(hwnd: HWND, section: Section) -> RECT {
    let index = match section {
        Section::Panel => 0,
        Section::Text => 1,
        Section::Progress => 2,
        Section::Interaction => 3,
    };
    rect(hwnd, 20, 126 + index * 48, 142, 166 + index * 48)
}

fn theme_rect(hwnd: HWND, mode: ThemeMode) -> RECT {
    let index = match mode {
        ThemeMode::System => 0,
        ThemeMode::Dark => 1,
        ThemeMode::Light => 2,
    };
    rect(hwnd, 170 + index * 112, 50, 274 + index * 112, 84)
}

fn layout_rect(hwnd: HWND, preset: AppearancePreset) -> RECT {
    let index = match preset {
        AppearancePreset::Compact => 0,
        AppearancePreset::Minimal => 1,
    };
    rect(hwnd, 540 + index * 104, 50, 636 + index * 104, 84)
}

fn reset_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 20, 488, 176, 528)
}

fn close_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 690, 488, 786, 528)
}

fn rows(section: Section) -> &'static [EditorSelection] {
    const PANEL: [EditorSelection; 3] = [
        EditorSelection::Color(StyleColorTarget::PanelBackground),
        EditorSelection::Color(StyleColorTarget::PanelBorder),
        EditorSelection::Blur,
    ];
    const TEXT: [EditorSelection; 4] = [
        EditorSelection::Color(StyleColorTarget::QuotaType),
        EditorSelection::Color(StyleColorTarget::Remaining),
        EditorSelection::Color(StyleColorTarget::ResetTime),
        EditorSelection::Color(StyleColorTarget::Error),
    ];
    const PROGRESS: [EditorSelection; 4] = [
        EditorSelection::Color(StyleColorTarget::ProgressHigh),
        EditorSelection::Color(StyleColorTarget::ProgressMedium),
        EditorSelection::Color(StyleColorTarget::ProgressLow),
        EditorSelection::Color(StyleColorTarget::ProgressConsumed),
    ];
    const INTERACTION: [EditorSelection; 1] =
        [EditorSelection::Color(StyleColorTarget::DragHandle)];

    match section {
        Section::Panel => &PANEL,
        Section::Text => &TEXT,
        Section::Progress => &PROGRESS,
        Section::Interaction => &INTERACTION,
    }
}

fn row_rect(hwnd: HWND, index: usize) -> RECT {
    rect(
        hwnd,
        174,
        126 + index as i32 * 48,
        786,
        166 + index as i32 * 48,
    )
}

fn editor_box_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 174, 326, 786, 472)
}

fn color_slider_track_rect(hwnd: HWND, channel_index: usize) -> RECT {
    let top = 358 + channel_index as i32 * 26;
    rect(hwnd, 310, top, 700, top + 4)
}

fn color_slider_hit_rect(hwnd: HWND, channel_index: usize) -> RECT {
    let track = color_slider_track_rect(hwnd, channel_index);
    RECT {
        left: track.left - scale(hwnd, 8),
        top: track.top - scale(hwnd, 10),
        right: track.right + scale(hwnd, 8),
        bottom: track.bottom + scale(hwnd, 10),
    }
}

fn blur_slider_track_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 310, 390, 700, 394)
}

fn blur_slider_hit_rect(hwnd: HWND) -> RECT {
    let track = blur_slider_track_rect(hwnd);
    RECT {
        left: track.left - scale(hwnd, 8),
        top: track.top - scale(hwnd, 12),
        right: track.right + scale(hwnd, 8),
        bottom: track.bottom + scale(hwnd, 12),
    }
}

fn set_section(section: Section) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.section = section;
        s.editor = rows(section)[0];
        s.dragging_slider = None;
        s.hwnd.to_hwnd()
    };
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn select_editor(editor: EditorSelection) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.editor = editor;
        s.dragging_slider = None;
        s.hwnd.to_hwnd()
    };
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn send_parent(message: u32, wparam: usize, lparam: isize) {
    let parent = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.parent.to_hwnd())
    };
    if let Some(parent) = parent {
        unsafe {
            let _ = PostMessageW(parent, message, WPARAM(wparam), LPARAM(lparam));
        }
    }
}

fn slider_kind_at(hwnd: HWND, x: i32, y: i32) -> Option<SliderKind> {
    let editor = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.editor)?
    };
    match editor {
        EditorSelection::Color(_) => {
            for index in 0..4 {
                if pt_in_rect(color_slider_hit_rect(hwnd, index), x, y) {
                    return Some(match index {
                        0 => SliderKind::Red,
                        1 => SliderKind::Green,
                        2 => SliderKind::Blue,
                        _ => SliderKind::Alpha,
                    });
                }
            }
            None
        }
        EditorSelection::Blur => {
            if pt_in_rect(blur_slider_hit_rect(hwnd), x, y) {
                Some(SliderKind::Blur)
            } else {
                None
            }
        }
    }
}

fn slider_value_from_x(track: RECT, x: i32, max: u8) -> u8 {
    let width = (track.right - track.left).max(1);
    let pos = (x.clamp(track.left, track.right) - track.left) as i64;
    ((pos * i64::from(max) + i64::from(width / 2)) / i64::from(width))
        .clamp(0, i64::from(max)) as u8
}

fn update_slider(hwnd: HWND, kind: SliderKind, x: i32) {
    let mut color_update: Option<(StyleColorTarget, Color)> = None;
    let mut blur_update: Option<u8> = None;

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };

        match (s.editor, kind) {
            (EditorSelection::Color(target), SliderKind::Red)
            | (EditorSelection::Color(target), SliderKind::Green)
            | (EditorSelection::Color(target), SliderKind::Blue)
            | (EditorSelection::Color(target), SliderKind::Alpha) => {
                let index = match kind {
                    SliderKind::Red => 0,
                    SliderKind::Green => 1,
                    SliderKind::Blue => 2,
                    SliderKind::Alpha => 3,
                    SliderKind::Blur => return,
                };
                let value = slider_value_from_x(color_slider_track_rect(hwnd, index), x, u8::MAX);
                let current = s.snapshot.active_style.color(target);
                let color = match kind {
                    SliderKind::Red => Color::rgba(value, current.g, current.b, current.a),
                    SliderKind::Green => Color::rgba(current.r, value, current.b, current.a),
                    SliderKind::Blue => Color::rgba(current.r, current.g, value, current.a),
                    SliderKind::Alpha => Color::rgba(current.r, current.g, current.b, value),
                    SliderKind::Blur => current,
                };
                s.snapshot.active_style.set_color(target, color);
                color_update = Some((target, color));
            }
            (EditorSelection::Blur, SliderKind::Blur) => {
                let value = slider_value_from_x(
                    blur_slider_track_rect(hwnd),
                    x,
                    FROSTED_STRENGTH_MAX,
                );
                s.snapshot.active_style.panel_frosted_strength = value;
                blur_update = Some(value);
            }
            _ => {}
        }
    }

    if let Some((target, color)) = color_update {
        send_parent(
            WM_STYLE_COLOR_PREVIEW,
            encode_color_target(target),
            pack_color(color),
        );
    }
    if let Some(value) = blur_update {
        send_parent(WM_STYLE_BLUR_PREVIEW, value as usize, 0);
    }

    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_from_lparam(lparam);

            for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
                if pt_in_rect(theme_rect(hwnd, mode), x, y) {
                    send_parent(
                        WM_STYLE_THEME_CHANGE,
                        match mode {
                            ThemeMode::System => 0,
                            ThemeMode::Dark => 1,
                            ThemeMode::Light => 2,
                        },
                        0,
                    );
                    return LRESULT(0);
                }
            }

            for preset in [AppearancePreset::Compact, AppearancePreset::Minimal] {
                if pt_in_rect(layout_rect(hwnd, preset), x, y) {
                    send_parent(
                        WM_STYLE_LAYOUT_CHANGE,
                        if preset == AppearancePreset::Compact { 0 } else { 1 },
                        0,
                    );
                    return LRESULT(0);
                }
            }

            for section in [
                Section::Panel,
                Section::Text,
                Section::Progress,
                Section::Interaction,
            ] {
                if pt_in_rect(section_rect(hwnd, section), x, y) {
                    set_section(section);
                    return LRESULT(0);
                }
            }

            let section = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.section)
            };
            if let Some(section) = section {
                for (index, editor) in rows(section).iter().copied().enumerate() {
                    if pt_in_rect(row_rect(hwnd, index), x, y) {
                        select_editor(editor);
                        return LRESULT(0);
                    }
                }
            }

            if let Some(kind) = slider_kind_at(hwnd, x, y) {
                {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        s.dragging_slider = Some(kind);
                    }
                }
                let _ = SetCapture(hwnd);
                update_slider(hwnd, kind, x);
                return LRESULT(0);
            }

            if pt_in_rect(reset_rect(hwnd), x, y) {
                send_parent(WM_STYLE_RESET_CURRENT, 0, 0);
                return LRESULT(0);
            }
            if pt_in_rect(close_rect(hwnd), x, y) {
                send_parent(WM_STYLE_SAVE, 0, 0);
                let _ = DestroyWindow(hwnd);
                return LRESULT(0);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let kind = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().and_then(|s| s.dragging_slider)
            };
            if let Some(kind) = kind {
                let (x, _) = point_from_lparam(lparam);
                update_slider(hwnd, kind, x);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let was_dragging = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                s.dragging_slider.take().is_some()
            };
            if was_dragging {
                let _ = ReleaseCapture();
                send_parent(WM_STYLE_SAVE, 0, 0);
            }
            LRESULT(0)
        }
        WM_CANCELMODE | WM_CAPTURECHANGED => {
            let was_dragging = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_mut()
                    .and_then(|s| s.dragging_slider.take())
                    .is_some()
            };
            if was_dragging {
                send_parent(WM_STYLE_SAVE, 0, 0);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let suggested = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                HWND::default(),
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_CLOSE => {
            send_parent(WM_STYLE_SAVE, 0, 0);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            let font = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.take().map(|s| s.font)
            };
            if let Some(font) = font {
                if font != 0 {
                    let _ = DeleteObject(HGDIOBJ(font as *mut _));
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);

    let (snapshot, section, editor, font) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            let _ = EndPaint(hwnd, &ps);
            return;
        };
        (s.snapshot.clone(), s.section, s.editor, s.font)
    };

    let dark = snapshot.is_dark;
    let background = if dark {
        Color::from_hex("#1F2125FF")
    } else {
        Color::from_hex("#F5F6F8FF")
    };
    let card = if dark {
        Color::from_hex("#292C31FF")
    } else {
        Color::from_hex("#FFFFFFFF")
    };
    let card_hover = if dark {
        Color::from_hex("#343840FF")
    } else {
        Color::from_hex("#E9EDF2FF")
    };
    let track_background = if dark {
        Color::from_hex("#454A52FF")
    } else {
        Color::from_hex("#D8DCE2FF")
    };
    let accent = Color::from_hex("#4C8DFFFF");
    let primary = if dark {
        Color::from_hex("#F2F3F5FF")
    } else {
        Color::from_hex("#202124FF")
    };
    let secondary = if dark {
        Color::from_hex("#A8ADB5FF")
    } else {
        Color::from_hex("#666B73FF")
    };

    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    fill(hdc, client, background);

    let old_font = SelectObject(hdc, HGDIOBJ(font as *mut _));
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));

    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "样式设置"
        } else {
            "Style settings"
        },
        rect(hwnd, 20, 14, 150, 42),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese { "主题" } else { "Theme" },
        rect(hwnd, 170, 18, 230, 42),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese { "排版" } else { "Layout" },
        rect(hwnd, 540, 18, 600, 42),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
        let selected = snapshot.theme_mode == mode;
        draw_segment(
            hdc,
            theme_rect(hwnd, mode),
            selected,
            if selected { accent } else { card },
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            match (snapshot.language == LanguageId::SimplifiedChinese, mode) {
                (true, ThemeMode::System) => "跟随系统",
                (true, ThemeMode::Dark) => "深色",
                (true, ThemeMode::Light) => "浅色",
                (false, ThemeMode::System) => "System",
                (false, ThemeMode::Dark) => "Dark",
                (false, ThemeMode::Light) => "Light",
            },
        );
    }

    for preset in [AppearancePreset::Compact, AppearancePreset::Minimal] {
        let selected = snapshot.appearance_preset == preset;
        draw_segment(
            hdc,
            layout_rect(hwnd, preset),
            selected,
            if selected { accent } else { card },
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            match (snapshot.language == LanguageId::SimplifiedChinese, preset) {
                (true, AppearancePreset::Compact) => "紧凑",
                (true, AppearancePreset::Minimal) => "极简",
                (false, AppearancePreset::Compact) => "Compact",
                (false, AppearancePreset::Minimal) => "Minimal",
            },
        );
    }

    for item in [
        Section::Panel,
        Section::Text,
        Section::Progress,
        Section::Interaction,
    ] {
        let selected = item == section;
        let r = section_rect(hwnd, item);
        fill(hdc, r, if selected { card_hover } else { background });
        if selected {
            let bar = RECT {
                right: r.left + scale(hwnd, 3),
                ..r
            };
            fill(hdc, bar, accent);
        }
        let _ = SetTextColor(
            hdc,
            COLORREF(if selected { primary } else { secondary }.to_colorref()),
        );
        draw_text(
            hdc,
            section_label(item, snapshot.language),
            RECT {
                left: r.left + scale(hwnd, 14),
                ..r
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }

    for (index, row) in rows(section).iter().copied().enumerate() {
        let r = row_rect(hwnd, index);
        fill(hdc, r, if row == editor { card_hover } else { card });
        let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
        draw_text(
            hdc,
            row_label(row, snapshot.language),
            RECT {
                left: r.left + scale(hwnd, 14),
                right: r.left + scale(hwnd, 230),
                ..r
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        match row {
            EditorSelection::Color(target) => {
                let color = snapshot.active_style.color(target);
                let swatch = RECT {
                    left: r.right - scale(hwnd, 148),
                    top: r.top + scale(hwnd, 9),
                    right: r.right - scale(hwnd, 118),
                    bottom: r.bottom - scale(hwnd, 9),
                };
                fill(hdc, swatch, color);
                let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
                draw_text(
                    hdc,
                    &color.to_hex_rgba(),
                    RECT {
                        left: r.right - scale(hwnd, 108),
                        ..r
                    },
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
            EditorSelection::Blur => {
                let value = format!("{}%", snapshot.active_style.panel_frosted_strength);
                let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
                draw_text(
                    hdc,
                    &value,
                    RECT {
                        left: r.right - scale(hwnd, 82),
                        ..r
                    },
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
    }

    let editor_box = editor_box_rect(hwnd);
    fill(hdc, editor_box, card);
    paint_editor(
        hdc,
        hwnd,
        &snapshot,
        editor,
        primary,
        secondary,
        track_background,
        accent,
    );

    draw_segment(
        hdc,
        reset_rect(hwnd),
        false,
        card,
        primary,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "恢复当前主题默认"
        } else {
            "Reset theme"
        },
    );
    draw_segment(
        hdc,
        close_rect(hwnd),
        true,
        accent,
        Color::from_hex("#FFFFFFFF"),
        if snapshot.language == LanguageId::SimplifiedChinese {
            "关闭"
        } else {
            "Close"
        },
    );

    SelectObject(hdc, old_font);
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn paint_editor(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    editor: EditorSelection,
    primary: Color,
    secondary: Color,
    track_background: Color,
    accent: Color,
) {
    match editor {
        EditorSelection::Color(target) => {
            let color = snapshot.active_style.color(target);
            let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
            draw_text(
                hdc,
                "RGBA",
                rect(hwnd, 194, 334, 260, 354),
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );

            let values = [color.r, color.g, color.b, color.a];
            for (index, (label, value)) in ["R", "G", "B", "A"].iter().zip(values).enumerate() {
                let top = 348 + index as i32 * 26;
                let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
                draw_text(
                    hdc,
                    label,
                    rect(hwnd, 196, top, 220, top + 22),
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
                draw_slider(
                    hdc,
                    hwnd,
                    color_slider_track_rect(hwnd, index),
                    value,
                    u8::MAX,
                    track_background,
                    accent,
                );
                let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
                draw_text(
                    hdc,
                    &value.to_string(),
                    rect(hwnd, 716, top, 770, top + 22),
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
        EditorSelection::Blur => {
            let value = snapshot.active_style.panel_frosted_strength;
            let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
            draw_text(
                hdc,
                if snapshot.language == LanguageId::SimplifiedChinese {
                    "强度调节"
                } else {
                    "Intensity"
                },
                rect(hwnd, 194, 344, 300, 368),
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
            draw_slider(
                hdc,
                hwnd,
                blur_slider_track_rect(hwnd),
                value,
                FROSTED_STRENGTH_MAX,
                track_background,
                accent,
            );
            let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
            draw_text(
                hdc,
                &format!("{}%", value),
                rect(hwnd, 716, 378, 770, 406),
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
            let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
            draw_text(
                hdc,
                if snapshot.language == LanguageId::SimplifiedChinese {
                    "0%=关闭；拖动时任务栏组件实时预览，释放后自动保存"
                } else {
                    "0%=Off; taskbar widget previews live while dragging; saves on release"
                },
                rect(hwnd, 196, 420, 760, 450),
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
        }
    }
}

unsafe fn draw_slider(
    hdc: HDC,
    hwnd: HWND,
    track: RECT,
    value: u8,
    max: u8,
    track_background: Color,
    accent: Color,
) {
    fill(hdc, track, track_background);

    let width = (track.right - track.left).max(1);
    let thumb_x = track.left + width * i32::from(value) / i32::from(max.max(1));
    let filled = RECT {
        right: thumb_x.max(track.left),
        ..track
    };
    if filled.right > filled.left {
        fill(hdc, filled, accent);
    }

    let radius = scale(hwnd, 6);
    let brush = CreateSolidBrush(COLORREF(accent.to_colorref()));
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
    let center_y = (track.top + track.bottom) / 2;
    let _ = Ellipse(
        hdc,
        thumb_x - radius,
        center_y - radius,
        thumb_x + radius,
        center_y + radius,
    );
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(brush);
}

fn section_label(section: Section, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match section {
        Section::Panel => {
            if zh {
                "面板"
            } else {
                "Panel"
            }
        }
        Section::Text => {
            if zh {
                "文字"
            } else {
                "Text"
            }
        }
        Section::Progress => {
            if zh {
                "进度条"
            } else {
                "Progress"
            }
        }
        Section::Interaction => {
            if zh {
                "交互"
            } else {
                "Interaction"
            }
        }
    }
}

fn row_label(row: EditorSelection, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match row {
        EditorSelection::Color(StyleColorTarget::PanelBackground) => {
            if zh { "背景颜色" } else { "Background" }
        }
        EditorSelection::Color(StyleColorTarget::PanelBorder) => {
            if zh { "边框颜色" } else { "Border" }
        }
        EditorSelection::Blur => {
            if zh { "磨砂强度" } else { "Frosted intensity" }
        }
        EditorSelection::Color(StyleColorTarget::QuotaType) => {
            if zh { "额度类型" } else { "Quota type" }
        }
        EditorSelection::Color(StyleColorTarget::Remaining) => {
            if zh { "剩余额度" } else { "Remaining quota" }
        }
        EditorSelection::Color(StyleColorTarget::ResetTime) => {
            if zh { "重置时间" } else { "Reset time" }
        }
        EditorSelection::Color(StyleColorTarget::Error) => {
            if zh { "异常状态" } else { "Error state" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressHigh) => {
            if zh { "充足额度" } else { "High quota" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressMedium) => {
            if zh { "中等额度" } else { "Medium quota" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressLow) => {
            if zh { "低额度" } else { "Low quota" }
        }
        EditorSelection::Color(StyleColorTarget::ProgressConsumed) => {
            if zh { "已消耗部分" } else { "Consumed" }
        }
        EditorSelection::Color(StyleColorTarget::DragHandle) => {
            if zh { "拖拽点" } else { "Drag handle" }
        }
    }
}

unsafe fn draw_segment(
    hdc: HDC,
    rect: RECT,
    selected: bool,
    background: Color,
    foreground: Color,
    text: &str,
) {
    fill(hdc, rect, background);
    if selected {
        let border = CreatePen(
            PS_SOLID,
            1,
            COLORREF(Color::from_hex("#76A7FFFF").to_colorref()),
        );
        let old_pen = SelectObject(hdc, border);
        let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
        let _ = Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(border);
    }
    let _ = SetTextColor(hdc, COLORREF(foreground.to_colorref()));
    draw_text(hdc, text, rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
}

unsafe fn fill(hdc: HDC, rect: RECT, color: Color) {
    let brush = CreateSolidBrush(COLORREF(color.to_colorref()));
    FillRect(hdc, &rect, brush);
    let _ = DeleteObject(brush);
}

unsafe fn draw_text(hdc: HDC, text: &str, mut rect: RECT, format: DRAW_TEXT_FORMAT) {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let _ = DrawTextW(hdc, &mut wide, &mut rect, format);
}
