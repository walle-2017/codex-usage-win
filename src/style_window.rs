use std::sync::Mutex;
use std::{fs, path::PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OPENFILENAMEW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::appearance::AppearancePreset;
use crate::localization::LanguageId;
use crate::native_interop::{self, Color, WM_APP};
use crate::settings_model::{parse_jsonc, EditableSettings};
use crate::style::{
    StyleColorTarget, ThemeMode, ThemePreset, ThemeStyle, FROSTED_STRENGTH_MAX,
};

// Keep this block well away from updater.rs (WM_APP + 21..23).
pub const WM_STYLE_COLOR_PREVIEW: u32 = WM_APP + 120;
pub const WM_STYLE_BLUR_PREVIEW: u32 = WM_APP + 121;
pub const WM_STYLE_SAVE: u32 = WM_APP + 122;
pub const WM_STYLE_THEME_CHANGE: u32 = WM_APP + 123;
pub const WM_STYLE_LAYOUT_CHANGE: u32 = WM_APP + 124;
pub const WM_STYLE_RESET_CURRENT: u32 = WM_APP + 125;
pub const WM_STYLE_PRESET_CHANGE: u32 = WM_APP + 126;
pub const WM_SETTINGS_REFRESH_CHANGE: u32 = WM_APP + 127;
pub const WM_SETTINGS_USAGE_CHANGE: u32 = WM_APP + 128;
pub const WM_SETTINGS_ALERT_CHANGE: u32 = WM_APP + 129;
pub const WM_SETTINGS_STARTUP_CHANGE: u32 = WM_APP + 130;
pub const WM_SETTINGS_LANGUAGE_CHANGE: u32 = WM_APP + 131;
pub const WM_SETTINGS_JSON_APPLY: u32 = WM_APP + 132;

const WINDOW_CLASS: &str = "CodexUsageUnifiedSettingsV1";
const WINDOW_WIDTH: i32 = 980;
const WINDOW_HEIGHT: i32 = 700;
const WINDOW_MIN_WIDTH: i32 = 900;
const WINDOW_MIN_HEIGHT: i32 = 620;
const ID_EDIT_R: u16 = 300;
const ID_EDIT_G: u16 = 301;
const ID_EDIT_B: u16 = 302;
const ID_EDIT_A: u16 = 303;
const ID_EDIT_BLUR: u16 = 304;
const ID_EDIT_HEX_BASE: u16 = 320;
const HEX_EDIT_COUNT: usize = 11;
const ID_COMBO_LANGUAGE: u16 = 360;
const ID_EDIT_JSON: u16 = 400;
const CBN_SELCHANGE_CODE: u16 = 1;
const CB_ADDSTRING_MSG: u32 = 0x0143;
const CB_GETCURSEL_MSG: u32 = 0x0147;
const CB_SETCURSEL_MSG: u32 = 0x014E;
const JSON_EDIT_LIMIT: usize = 262_144;
const EN_SETFOCUS_CODE: u16 = 0x0100;
const EN_KILLFOCUS_CODE: u16 = 0x0200;
const EN_CHANGE_CODE: u16 = 0x0300;
const EM_SETLIMITTEXT_MSG: u32 = 0x00C5;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const FIXED_CAPTION_COLORREF: u32 = 0x00524843; // #434852 in COLORREF byte order
const FIXED_CAPTION_TEXT_COLORREF: u32 = 0x00FFFFFF;

#[derive(Clone)]
pub struct StyleWindowSnapshot {
    pub language: LanguageId,
    pub theme_mode: ThemeMode,
    pub is_dark: bool,
    pub appearance_preset: AppearancePreset,
    pub active_style: ThemeStyle,
    pub editable_settings: EditableSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    General,
    Preset,
    Panel,
    Text,
    Progress,
    Interaction,
    Json,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonAction {
    Reload,
    Format,
    Import,
    Export,
    Apply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitTarget {
    Theme(ThemeMode),
    Layout(AppearancePreset),
    Preset(ThemePreset),
    Section(Section),
    Row(EditorSelection),
    Refresh(u32),
    UsageSession,
    UsageWeekly,
    Alert(u8),
    Startup,
    Json(JsonAction),
    Reset,
    Close,
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
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    tracking_mouse_leave: bool,
    numeric_edits: [SendHwnd; 4],
    blur_edit: SendHwnd,
    hex_edits: [SendHwnd; HEX_EDIT_COUNT],
    language_combo: SendHwnd,
    json_edit: SendHwnd,
    focused_numeric_edit: Option<usize>,
    focused_blur_edit: bool,
    focused_hex_edit: Option<StyleColorTarget>,
    invalid_hex_edits: [bool; HEX_EDIT_COUNT],
    syncing_numeric_edits: bool,
    syncing_blur_edit: bool,
    syncing_hex_edits: bool,
    syncing_json_edit: bool,
    json_dirty: bool,
    json_status: String,
    edit_brush: isize,
    font: isize,
    json_font: isize,
}

unsafe impl Send for PanelState {}

static STATE: Mutex<Option<PanelState>> = Mutex::new(None);
static PENDING_EDITABLE_SETTINGS: Mutex<Option<EditableSettings>> = Mutex::new(None);

pub fn take_pending_editable_settings() -> Option<EditableSettings> {
    PENDING_EDITABLE_SETTINGS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take()
}

#[derive(Clone, Copy)]
struct EditorPalette {
    secondary: Color,
    track_background: Color,
    accent: Color,
}

#[derive(Clone, Copy)]
struct ButtonPalette {
    normal: Color,
    hover: Color,
    pressed: Color,
    selected: Color,
    selected_hover: Color,
    selected_pressed: Color,
}

#[derive(Clone, Copy)]
struct EditFramePalette {
    border: Color,
    accent: Color,
}

#[derive(Clone, Copy)]
struct PresetGalleryPalette {
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    border: Color,
    accent: Color,
    primary: Color,
}

const COLOR_TARGETS: [StyleColorTarget; HEX_EDIT_COUNT] = [
    StyleColorTarget::PanelBackground,
    StyleColorTarget::PanelBorder,
    StyleColorTarget::QuotaType,
    StyleColorTarget::Remaining,
    StyleColorTarget::ResetTime,
    StyleColorTarget::Error,
    StyleColorTarget::ProgressHigh,
    StyleColorTarget::ProgressMedium,
    StyleColorTarget::ProgressLow,
    StyleColorTarget::ProgressConsumed,
    StyleColorTarget::DragHandle,
];

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

        let title = native_interop::wide_str("Codex Usage Win");
        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_CLIPCHILDREN,
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

        let edit_class = native_interop::wide_str("EDIT");
        let empty = native_interop::wide_str("");
        let mut numeric_edits_raw = [HWND::default(); 4];
        for (index, id) in [ID_EDIT_R, ID_EDIT_G, ID_EDIT_B, ID_EDIT_A]
            .iter()
            .copied()
            .enumerate()
        {
            let edit = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR::from_raw(edit_class.as_ptr()),
                PCWSTR::from_raw(empty.as_ptr()),
                WINDOW_STYLE(
                    WS_CHILD.0
                        | ES_NUMBER as u32
                        | ES_CENTER as u32
                        | ES_AUTOHSCROLL as u32,
                ),
                0,
                0,
                s(54),
                s(22),
                hwnd,
                HMENU(id as usize as *mut _),
                GetModuleHandleW(PCWSTR::null()).unwrap(),
                None,
            ) {
                Ok(edit) => edit,
                Err(_) => {
                    let _ = DestroyWindow(hwnd);
                    let _ = DeleteObject(font);
                    return;
                }
            };
            let _ = SendMessageW(
                edit,
                WM_SETFONT,
                WPARAM(font.0 as usize),
                LPARAM(1),
            );
            let _ = SendMessageW(
                edit,
                EM_SETLIMITTEXT_MSG,
                WPARAM(3),
                LPARAM(0),
            );
            numeric_edits_raw[index] = edit;
        }
        let numeric_edits = numeric_edits_raw.map(SendHwnd::from_hwnd);

        let blur_edit = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR::from_raw(edit_class.as_ptr()),
            PCWSTR::from_raw(empty.as_ptr()),
            WINDOW_STYLE(
                WS_CHILD.0
                    | ES_NUMBER as u32
                    | ES_CENTER as u32
                    | ES_AUTOHSCROLL as u32,
            ),
            0,
            0,
            s(54),
            s(22),
            hwnd,
            HMENU(ID_EDIT_BLUR as usize as *mut _),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) {
            Ok(edit) => edit,
            Err(_) => {
                let _ = DestroyWindow(hwnd);
                let _ = DeleteObject(font);
                return;
            }
        };
        let _ = SendMessageW(
            blur_edit,
            WM_SETFONT,
            WPARAM(font.0 as usize),
            LPARAM(1),
        );
        let _ = SendMessageW(
            blur_edit,
            EM_SETLIMITTEXT_MSG,
            WPARAM(3),
            LPARAM(0),
        );
        let mut hex_edits_raw = [HWND::default(); HEX_EDIT_COUNT];
        for (index, slot) in hex_edits_raw.iter_mut().enumerate() {
            let edit = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR::from_raw(edit_class.as_ptr()),
                PCWSTR::from_raw(empty.as_ptr()),
                WINDOW_STYLE(WS_CHILD.0 | ES_CENTER as u32 | ES_AUTOHSCROLL as u32),
                0,
                0,
                s(112),
                s(22),
                hwnd,
                HMENU((ID_EDIT_HEX_BASE + index as u16) as usize as *mut _),
                GetModuleHandleW(PCWSTR::null()).unwrap(),
                None,
            ) {
                Ok(edit) => edit,
                Err(_) => {
                    let _ = DestroyWindow(hwnd);
                    let _ = DeleteObject(font);
                    return;
                }
            };
            let _ = SendMessageW(edit, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            let _ = SendMessageW(edit, EM_SETLIMITTEXT_MSG, WPARAM(9), LPARAM(0));
            *slot = edit;
        }
        let hex_edits = hex_edits_raw.map(SendHwnd::from_hwnd);

        let edit_background = if snapshot.is_dark {
            Color::from_hex("#20242AFF")
        } else {
            Color::from_hex("#EEF3F8FF")
        };
        let edit_brush = CreateSolidBrush(COLORREF(edit_background.to_colorref()));

        apply_fixed_titlebar(hwnd);

        {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            *state = Some(PanelState {
                hwnd: SendHwnd::from_hwnd(hwnd),
                parent: SendHwnd::from_hwnd(parent),
                snapshot,
                section: Section::Preset,
                editor: EditorSelection::Color(StyleColorTarget::PanelBackground),
                dragging_slider: None,
                hovered: None,
                pressed: None,
                tracking_mouse_leave: false,
                numeric_edits,
                blur_edit: SendHwnd::from_hwnd(blur_edit),
                hex_edits,
                focused_numeric_edit: None,
                focused_blur_edit: false,
                focused_hex_edit: None,
                invalid_hex_edits: [false; HEX_EDIT_COUNT],
                syncing_numeric_edits: false,
                syncing_blur_edit: false,
                syncing_hex_edits: false,
                edit_brush: edit_brush.0 as isize,
                font: font.0 as isize,
            });
        }
        layout_numeric_edits(hwnd);
        layout_hex_edits(hwnd);
        sync_hex_edits();
        sync_numeric_edits();
        sync_blur_edit();
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
        let background = if s.snapshot.is_dark {
            Color::from_hex("#20242AFF")
        } else {
            Color::from_hex("#EEF3F8FF")
        };
        unsafe {
            if s.edit_brush != 0 {
                let _ = DeleteObject(HGDIOBJ(s.edit_brush as *mut _));
            }
            let brush = CreateSolidBrush(COLORREF(background.to_colorref()));
            s.edit_brush = brush.0 as isize;
        }
        s.hwnd.to_hwnd()
    };
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
        // Commit the client-area theme in this UI turn so it lands together
        // with the now non-animated DWM title-bar update.
        let _ = UpdateWindow(hwnd);
    }
}

fn apply_fixed_titlebar(hwnd: HWND) {
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
        Section::Preset => 0,
        Section::Panel => 1,
        Section::Text => 2,
        Section::Progress => 3,
        Section::Interaction => 4,
    };
    rect(hwnd, 20, 108 + index * 48, 142, 148 + index * 48)
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
        AppearancePreset::Default => 0,
        AppearancePreset::Minimal => 1,
    };
    rect(hwnd, 540 + index * 104, 50, 636 + index * 104, 84)
}

fn preset_card_rect(hwnd: HWND, preset: ThemePreset) -> RECT {
    let index = match preset {
        ThemePreset::Classic => 0,
        ThemePreset::Ocean => 1,
        ThemePreset::Forest => 2,
    };
    let left = 174 + index * 204;
    rect(hwnd, left, 146, left + 192, 350)
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
        Section::Preset => &[],
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
        108 + index as i32 * 48,
        786,
        148 + index as i32 * 48,
    )
}

fn editor_box_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 174, 326, 786, 472)
}

fn color_slider_track_rect(hwnd: HWND, channel_index: usize) -> RECT {
    let top = 348 + channel_index as i32 * 32;
    rect(hwnd, 244, top, 676, top + 4)
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
    // Match the RGBA label-to-track spacing and end at the color-swatch edge.
    rect(hwnd, 292, 224, 646, 228)
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

fn blur_edit_frame_rect(hwnd: HWND) -> RECT {
    // Align the percentage input with the Hex input column above.
    rect(hwnd, 654, 210, 714, 238)
}

fn blur_edit_rect(hwnd: HWND) -> RECT {
    let frame = blur_edit_frame_rect(hwnd);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 3),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

fn color_row_index(section: Section, target: StyleColorTarget) -> Option<usize> {
    rows(section)
        .iter()
        .position(|row| *row == EditorSelection::Color(target))
}

fn hex_edit_frame_rect(hwnd: HWND, row_index: usize) -> RECT {
    let row = row_rect(hwnd, row_index);
    RECT {
        left: row.right - scale(hwnd, 132),
        top: row.top + scale(hwnd, 6),
        right: row.right - scale(hwnd, 12),
        bottom: row.bottom - scale(hwnd, 6),
    }
}

fn hex_edit_rect(hwnd: HWND, row_index: usize) -> RECT {
    let frame = hex_edit_frame_rect(hwnd, row_index);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 3),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

fn numeric_edit_frame_rect(hwnd: HWND, channel_index: usize) -> RECT {
    let top = 334 + channel_index as i32 * 32;
    rect(hwnd, 690, top, 764, top + 26)
}

fn numeric_edit_rect(hwnd: HWND, channel_index: usize) -> RECT {
    let frame = numeric_edit_frame_rect(hwnd, channel_index);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 3),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

fn editor_layout_snapshot() -> Option<([SendHwnd; 4], SendHwnd, bool, bool)> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = state.as_ref()?;
    Some((
        s.numeric_edits,
        s.blur_edit,
        s.section != Section::Preset && matches!(s.editor, EditorSelection::Color(_)),
        s.section == Section::Panel,
    ))
}

fn release_editor_focus_before_layout(
    hwnd: HWND,
    numeric_edits: &[SendHwnd; 4],
    blur_edit: SendHwnd,
    show_color: bool,
    show_blur: bool,
) {
    unsafe {
        let focused = GetFocus();
        let hiding_focused_color =
            !show_color && numeric_edits.iter().any(|edit| edit.to_hwnd() == focused);
        let hiding_focused_blur = !show_blur && blur_edit.to_hwnd() == focused;

        if hiding_focused_color || hiding_focused_blur {
            // SetFocus synchronously sends EN_KILLFOCUS to the old EDIT control.
            // This must run with STATE unlocked or WM_COMMAND would re-enter
            // STATE.lock() on the same UI thread and deadlock.
            let _ = SetFocus(hwnd);
        }
    }
}

fn layout_numeric_edits(hwnd: HWND) {
    // Copy every HWND and visibility decision while STATE is locked, then release
    // the mutex before calling Win32. ShowWindow/SetFocus can synchronously send
    // WM_COMMAND focus notifications back into this window procedure.
    let Some((numeric_edits, blur_edit, show_color, show_blur)) = editor_layout_snapshot() else {
        return;
    };

    release_editor_focus_before_layout(
        hwnd,
        &numeric_edits,
        blur_edit,
        show_color,
        show_blur,
    );

    unsafe {
        for (index, edit) in numeric_edits.iter().enumerate() {
            let r = numeric_edit_rect(hwnd, index);
            let _ = SetWindowPos(
                edit.to_hwnd(),
                HWND::default(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = ShowWindow(
                edit.to_hwnd(),
                if show_color { SW_SHOW } else { SW_HIDE },
            );
        }

        let blur_rect = blur_edit_rect(hwnd);
        let _ = SetWindowPos(
            blur_edit.to_hwnd(),
            HWND::default(),
            blur_rect.left,
            blur_rect.top,
            blur_rect.right - blur_rect.left,
            blur_rect.bottom - blur_rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = ShowWindow(
            blur_edit.to_hwnd(),
            if show_blur { SW_SHOW } else { SW_HIDE },
        );
    }
}

fn hex_layout_snapshot() -> Option<([SendHwnd; HEX_EDIT_COUNT], Section, Option<StyleColorTarget>)> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = state.as_ref()?;
    Some((s.hex_edits, s.section, s.focused_hex_edit))
}

fn layout_hex_edits(hwnd: HWND) {
    let Some((hex_edits, section, focused_target)) = hex_layout_snapshot() else {
        return;
    };

    unsafe {
        if let Some(target) = focused_target {
            if color_row_index(section, target).is_none() {
                let index = encode_color_target(target);
                if hex_edits[index].to_hwnd() == GetFocus() {
                    let _ = SetFocus(hwnd);
                }
            }
        }
        for (index, target) in COLOR_TARGETS.iter().copied().enumerate() {
            let edit = hex_edits[index].to_hwnd();
            if let Some(row_index) = color_row_index(section, target) {
                let r = hex_edit_rect(hwnd, row_index);
                let _ = SetWindowPos(
                    edit,
                    HWND::default(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                let _ = ShowWindow(edit, SW_SHOW);
            } else {
                let _ = ShowWindow(edit, SW_HIDE);
            }
        }
    }
}

fn set_edit_text_string(edit: HWND, value: &str) {
    let text = native_interop::wide_str(value);
    unsafe {
        let _ = SendMessageW(
            edit,
            WM_SETTEXT,
            WPARAM(0),
            LPARAM(text.as_ptr() as isize),
        );
    }
}

fn set_edit_text(edit: HWND, value: u8) {
    let text = native_interop::wide_str(&value.to_string());
    unsafe {
        let _ = SendMessageW(
            edit,
            WM_SETTEXT,
            WPARAM(0),
            LPARAM(text.as_ptr() as isize),
        );
    }
}

fn sync_numeric_edits() {
    let (edits, values) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.section == Section::Preset {
            return;
        }
        let EditorSelection::Color(target) = s.editor else {
            return;
        };
        let color = s.snapshot.active_style.color(target);
        s.syncing_numeric_edits = true;
        (s.numeric_edits, [color.r, color.g, color.b, color.a])
    };

    for (edit, value) in edits.iter().zip(values) {
        set_edit_text(edit.to_hwnd(), value);
    }

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_numeric_edits = false;
    }
}

fn sync_blur_edit() {
    let (edit, value) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.section != Section::Panel {
            return;
        }
        s.syncing_blur_edit = true;
        (
            s.blur_edit.to_hwnd(),
            s.snapshot.active_style.panel_frosted_strength,
        )
    };
    set_edit_text(edit, value);

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_blur_edit = false;
    }
}

fn read_edit_value(edit: HWND) -> Option<u16> {
    let mut buffer = [0u16; 4];
    let len = unsafe {
        SendMessageW(
            edit,
            WM_GETTEXT,
            WPARAM(buffer.len()),
            LPARAM(buffer.as_mut_ptr() as isize),
        )
        .0 as usize
    };
    if len == 0 {
        return None;
    }
    String::from_utf16_lossy(&buffer[..len]).parse::<u16>().ok()
}

fn read_edit_text(edit: HWND) -> String {
    let mut buffer = [0u16; 16];
    let len = unsafe {
        SendMessageW(
            edit,
            WM_GETTEXT,
            WPARAM(buffer.len()),
            LPARAM(buffer.as_mut_ptr() as isize),
        )
        .0 as usize
    };
    String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
}

fn parse_hex_input(value: &str) -> Result<Option<Color>, ()> {
    let trimmed = value.trim();
    let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if digits.len() > 8 || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(());
    }
    match digits.len() {
        6 | 8 => Color::try_from_hex(trimmed).map(Some).ok_or(()),
        0..=5 | 7 => Ok(None),
        _ => Err(()),
    }
}

fn sync_hex_edits() {
    let (edits, values) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.syncing_hex_edits = true;
        let values =
            COLOR_TARGETS.map(|target| s.snapshot.active_style.color(target).to_hex_rgba());
        (s.hex_edits, values)
    };
    for (edit, value) in edits.iter().zip(values.iter()) {
        set_edit_text_string(edit.to_hwnd(), value);
    }
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_hex_edits = false;
        s.invalid_hex_edits = [false; HEX_EDIT_COUNT];
    }
}

fn update_color_from_hex_edit(target: StyleColorTarget) {
    let index = encode_color_target(target);
    let (edit, syncing) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.hex_edits[index].to_hwnd(), s.syncing_hex_edits)
    };
    if syncing {
        return;
    }

    let parsed = parse_hex_input(&read_edit_text(edit));
    let mut applied = None;
    let mut selected = false;
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        match parsed {
            Ok(Some(color)) => {
                s.invalid_hex_edits[index] = false;
                s.snapshot.active_style.set_color(target, color);
                selected = s.editor == EditorSelection::Color(target);
                applied = Some(color);
            }
            Ok(None) => s.invalid_hex_edits[index] = false,
            Err(()) => s.invalid_hex_edits[index] = true,
        }
    }
    if let Some(color) = applied {
        if selected {
            sync_numeric_edits();
        }
        send_parent(
            WM_STYLE_COLOR_PREVIEW,
            encode_color_target(target),
            pack_color(color),
        );
        send_parent(WM_STYLE_SAVE, 0, 0);
    }
    unsafe {
        let hwnd = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or_default()
        };
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn normalize_hex_edit(target: StyleColorTarget) {
    let index = encode_color_target(target);
    let (edit, value) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.syncing_hex_edits = true;
        s.invalid_hex_edits[index] = false;
        (
            s.hex_edits[index].to_hwnd(),
            s.snapshot.active_style.color(target).to_hex_rgba(),
        )
    };
    set_edit_text_string(edit, &value);
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_hex_edits = false;
    }
}

fn target_from_hex_control_id(control_id: u16) -> Option<StyleColorTarget> {
    let index = control_id.checked_sub(ID_EDIT_HEX_BASE)? as usize;
    COLOR_TARGETS.get(index).copied()
}

fn update_color_from_numeric_edit(channel_index: usize) {
    let (edit, target, syncing) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        let EditorSelection::Color(target) = s.editor else {
            return;
        };
        (s.numeric_edits[channel_index].to_hwnd(), target, s.syncing_numeric_edits)
    };
    if syncing {
        return;
    }
    let Some(raw_value) = read_edit_value(edit) else {
        return;
    };
    let value = raw_value.min(u16::from(u8::MAX)) as u8;

    let color = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        let current = s.snapshot.active_style.color(target);
        let color = match channel_index {
            0 => Color::rgba(value, current.g, current.b, current.a),
            1 => Color::rgba(current.r, value, current.b, current.a),
            2 => Color::rgba(current.r, current.g, value, current.a),
            _ => Color::rgba(current.r, current.g, current.b, value),
        };
        s.snapshot.active_style.set_color(target, color);
        color
    };

    if raw_value > u16::from(u8::MAX) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_numeric_edits = true;
        }
        drop(state);
        set_edit_text(edit, value);
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_numeric_edits = false;
        }
    }

    sync_hex_edits();
    send_parent(
        WM_STYLE_COLOR_PREVIEW,
        encode_color_target(target),
        pack_color(color),
    );
    send_parent(WM_STYLE_SAVE, 0, 0);
    unsafe {
        let hwnd = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or_default()
        };
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn update_blur_from_numeric_edit() {
    let (edit, syncing) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.blur_edit.to_hwnd(), s.syncing_blur_edit)
    };
    if syncing {
        return;
    }

    let Some(raw_value) = read_edit_value(edit) else {
        return;
    };
    let value = raw_value.min(u16::from(FROSTED_STRENGTH_MAX)) as u8;

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.snapshot.active_style.panel_frosted_strength = value;
    }

    if raw_value > u16::from(FROSTED_STRENGTH_MAX) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_blur_edit = true;
        }
        drop(state);
        set_edit_text(edit, value);
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_blur_edit = false;
        }
    }

    send_parent(WM_STYLE_BLUR_PREVIEW, value as usize, 0);
    send_parent(WM_STYLE_SAVE, 0, 0);
    unsafe {
        let hwnd = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or_default()
        };
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn hit_target_at(hwnd: HWND, x: i32, y: i32) -> Option<HitTarget> {
    for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
        if pt_in_rect(theme_rect(hwnd, mode), x, y) {
            return Some(HitTarget::Theme(mode));
        }
    }
    for preset in [AppearancePreset::Default, AppearancePreset::Minimal] {
        if pt_in_rect(layout_rect(hwnd, preset), x, y) {
            return Some(HitTarget::Layout(preset));
        }
    }
    for section in [
        Section::Preset,
        Section::Panel,
        Section::Text,
        Section::Progress,
        Section::Interaction,
    ] {
        if pt_in_rect(section_rect(hwnd, section), x, y) {
            return Some(HitTarget::Section(section));
        }
    }
    let section = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.section)?
    };
    if section == Section::Preset {
        for preset in ThemePreset::ALL {
            if pt_in_rect(preset_card_rect(hwnd, preset), x, y) {
                return Some(HitTarget::Preset(preset));
            }
        }
    }
    for (index, editor) in rows(section).iter().copied().enumerate() {
        if pt_in_rect(row_rect(hwnd, index), x, y) {
            return Some(HitTarget::Row(editor));
        }
    }
    if pt_in_rect(reset_rect(hwnd), x, y) {
        return Some(HitTarget::Reset);
    }
    if pt_in_rect(close_rect(hwnd), x, y) {
        return Some(HitTarget::Close);
    }
    None
}

fn activate_target(hwnd: HWND, target: HitTarget) {
    match target {
        HitTarget::Theme(mode) => {
            send_parent(
                WM_STYLE_THEME_CHANGE,
                match mode {
                    ThemeMode::System => 0,
                    ThemeMode::Dark => 1,
                    ThemeMode::Light => 2,
                },
                0,
            );
        }
        HitTarget::Layout(preset) => {
            send_parent(
                WM_STYLE_LAYOUT_CHANGE,
                if preset == AppearancePreset::Default { 0 } else { 1 },
                0,
            );
        }
        HitTarget::Preset(preset) => {
            let index = match preset {
                ThemePreset::Classic => 0,
                ThemePreset::Ocean => 1,
                ThemePreset::Forest => 2,
            };
            send_parent(WM_STYLE_PRESET_CHANGE, index, 0);
        }
        HitTarget::Section(section) => set_section(section),
        HitTarget::Row(editor) => select_editor(editor),
        HitTarget::Reset => send_parent(WM_STYLE_RESET_CURRENT, 0, 0),
        HitTarget::Close => unsafe {
            send_parent(WM_STYLE_SAVE, 0, 0);
            let _ = DestroyWindow(hwnd);
        },
    }
}

fn button_background(
    target: HitTarget,
    selected: bool,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    palette: ButtonPalette,
) -> Color {
    if pressed == Some(target) {
        if selected {
            palette.selected_pressed
        } else {
            palette.pressed
        }
    } else if hovered == Some(target) {
        if selected {
            palette.selected_hover
        } else {
            palette.hover
        }
    } else if selected {
        palette.selected
    } else {
        palette.normal
    }
}

fn set_section(section: Section) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.section = section;
        if let Some(editor) = rows(section).first().copied() {
            s.editor = editor;
        }
        s.dragging_slider = None;
        s.hwnd.to_hwnd()
    };
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
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
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
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
    let (section, editor) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.section, s.editor)
    };

    if section == Section::Panel && pt_in_rect(blur_slider_hit_rect(hwnd), x, y) {
        return Some(SliderKind::Blur);
    }

    let EditorSelection::Color(_) = editor else {
        return None;
    };
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
            (_, SliderKind::Blur) if s.section == Section::Panel => {
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
        sync_numeric_edits();
        sync_hex_edits();
        send_parent(
            WM_STYLE_COLOR_PREVIEW,
            encode_color_target(target),
            pack_color(color),
        );
    }
    if let Some(value) = blur_update {
        sync_blur_edit();
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
        WM_SETCURSOR => {
            let cursor_hwnd = HWND(wparam.0 as *mut _);
            let is_numeric_edit = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                match state.as_ref() {
                    Some(s) => {
                        s.numeric_edits
                            .iter()
                            .any(|edit| edit.to_hwnd() == cursor_hwnd)
                            || s.blur_edit.to_hwnd() == cursor_hwnd
                            || s.hex_edits.iter().any(|edit| edit.to_hwnd() == cursor_hwnd)
                    }
                    None => false,
                }
            };
            if is_numeric_edit {
                let cursor = LoadCursorW(HINSTANCE::default(), IDC_IBEAM).unwrap_or_default();
                SetCursor(cursor);
                return LRESULT(1);
            }

            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = ScreenToClient(hwnd, &mut point);

            let cursor_id = if slider_kind_at(hwnd, point.x, point.y).is_some() {
                IDC_SIZEWE
            } else if hit_target_at(hwnd, point.x, point.y).is_some() {
                IDC_HAND
            } else {
                IDC_ARROW
            };
            let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
            SetCursor(cursor);
            LRESULT(1)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point_from_lparam(lparam);

            if let Some(kind) = slider_kind_at(hwnd, x, y) {
                if kind == SliderKind::Blur {
                    let _ = SetFocus(hwnd);
                    select_editor(EditorSelection::Blur);
                }
                {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        s.dragging_slider = Some(kind);
                        s.pressed = None;
                    }
                }
                let _ = SetCapture(hwnd);
                update_slider(hwnd, kind, x);
                return LRESULT(0);
            }

            if let Some(target) = hit_target_at(hwnd, x, y) {
                {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        s.pressed = Some(target);
                        s.hovered = Some(target);
                    }
                }
                let _ = SetCapture(hwnd);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, _) = point_from_lparam(lparam);
            let kind = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                if !s.tracking_mouse_leave {
                    let mut tracking = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tracking);
                    s.tracking_mouse_leave = true;
                }
                s.dragging_slider
            };

            if let Some(kind) = kind {
                update_slider(hwnd, kind, x);
            } else {
                let (_, y) = point_from_lparam(lparam);
                let hovered = hit_target_at(hwnd, x, y);
                let changed = {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    let Some(s) = state.as_mut() else {
                        return LRESULT(0);
                    };
                    let changed = s.hovered != hovered;
                    s.hovered = hovered;
                    changed
                };
                if changed {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            let changed = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                s.tracking_mouse_leave = false;
                let changed = s.hovered.is_some();
                s.hovered = None;
                changed
            };
            if changed {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_from_lparam(lparam);
            let (was_dragging, pressed) = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                (s.dragging_slider.take().is_some(), s.pressed.take())
            };

            let _ = ReleaseCapture();

            if was_dragging {
                send_parent(WM_STYLE_SAVE, 0, 0);
            } else if let Some(target) = pressed {
                if hit_target_at(hwnd, x, y) == Some(target) {
                    activate_target(hwnd, target);
                }
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_CANCELMODE | WM_CAPTURECHANGED => {
            let (was_dragging, had_pressed) = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                (
                    s.dragging_slider.take().is_some(),
                    s.pressed.take().is_some(),
                )
            };
            if was_dragging {
                send_parent(WM_STYLE_SAVE, 0, 0);
            }
            if had_pressed {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let control_id = (wparam.0 & 0xFFFF) as u16;
            let notification = ((wparam.0 >> 16) & 0xFFFF) as u16;
            if let Some(target) = target_from_hex_control_id(control_id) {
                match notification {
                    EN_CHANGE_CODE => {
                        update_color_from_hex_edit(target);
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_hex_edit = Some(target);
                            }
                        }
                        select_editor(EditorSelection::Color(target));
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        normalize_hex_edit(target);
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                if s.focused_hex_edit == Some(target) {
                                    s.focused_hex_edit = None;
                                }
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            let channel = match control_id {
                ID_EDIT_R => Some(0),
                ID_EDIT_G => Some(1),
                ID_EDIT_B => Some(2),
                ID_EDIT_A => Some(3),
                _ => None,
            };
            if let Some(channel) = channel {
                match notification {
                    EN_CHANGE_CODE => {
                        update_color_from_numeric_edit(channel);
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_numeric_edit = Some(channel);
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                if s.focused_numeric_edit == Some(channel) {
                                    s.focused_numeric_edit = None;
                                }
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            if control_id == ID_EDIT_BLUR {
                match notification {
                    EN_CHANGE_CODE => {
                        update_blur_from_numeric_edit();
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_blur_edit = true;
                            }
                        }
                        select_editor(EditorSelection::Blur);
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_blur_edit = false;
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CTLCOLOREDIT => {
            let hdc = HDC(wparam.0 as *mut _);
            let (is_dark, brush) = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_ref() else {
                    return DefWindowProcW(hwnd, msg, wparam, lparam);
                };
                (s.snapshot.is_dark, s.edit_brush)
            };
            let background = if is_dark {
                Color::from_hex("#20242AFF")
            } else {
                Color::from_hex("#EEF3F8FF")
            };
            let foreground = if is_dark {
                Color::from_hex("#F2F3F5FF")
            } else {
                Color::from_hex("#202124FF")
            };
            let _ = SetBkColor(hdc, COLORREF(background.to_colorref()));
            let _ = SetTextColor(hdc, COLORREF(foreground.to_colorref()));
            LRESULT(brush)
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
            layout_numeric_edits(hwnd);
            layout_hex_edits(hwnd);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_CLOSE => LRESULT(0),
        WM_DESTROY => {
            let resources = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.take().map(|s| (s.font, s.edit_brush))
            };
            if let Some((font, edit_brush)) = resources {
                if font != 0 {
                    let _ = DeleteObject(HGDIOBJ(font as *mut _));
                }
                if edit_brush != 0 {
                    let _ = DeleteObject(HGDIOBJ(edit_brush as *mut _));
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let screen_hdc = BeginPaint(hwnd, &mut ps);

    let mut paint_client = RECT::default();
    let _ = GetClientRect(hwnd, &mut paint_client);
    let width = (paint_client.right - paint_client.left).max(1);
    let height = (paint_client.bottom - paint_client.top).max(1);

    let mem_hdc = CreateCompatibleDC(screen_hdc);
    if mem_hdc.0.is_null() {
        let _ = EndPaint(hwnd, &ps);
        return;
    }
    let bitmap = CreateCompatibleBitmap(screen_hdc, width, height);
    if bitmap.0.is_null() {
        let _ = DeleteDC(mem_hdc);
        let _ = EndPaint(hwnd, &ps);
        return;
    }
    let old_bitmap = SelectObject(mem_hdc, HGDIOBJ(bitmap.0));
    let hdc = mem_hdc;

    let (
        snapshot,
        section,
        editor,
        hovered,
        pressed,
        focused_numeric_edit,
        focused_blur_edit,
        focused_hex_edit,
        invalid_hex_edits,
        font,
    ) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            let _ = EndPaint(hwnd, &ps);
            return;
        };
        (
            s.snapshot.clone(),
            s.section,
            s.editor,
            s.hovered,
            s.pressed,
            s.focused_numeric_edit,
            s.focused_blur_edit,
            s.focused_hex_edit,
            s.invalid_hex_edits,
            s.font,
        )
    };

    let dark = snapshot.is_dark;
    let background = if dark {
        Color::from_hex("#1F2125FF")
    } else {
        Color::from_hex("#E9EEF4FF")
    };
    let card = if dark {
        Color::from_hex("#292C31FF")
    } else {
        Color::from_hex("#FFFFFFFF")
    };
    let card_hover = if dark {
        Color::from_hex("#343840FF")
    } else {
        Color::from_hex("#DCE5EFFF")
    };
    let card_pressed = if dark {
        Color::from_hex("#414751FF")
    } else {
        Color::from_hex("#CBD7E4FF")
    };
    let track_background = if dark {
        Color::from_hex("#454A52FF")
    } else {
        Color::from_hex("#C1CCD8FF")
    };
    let accent = Color::from_hex("#4C8DFFFF");
    let accent_hover = Color::from_hex("#629CFFFF");
    let accent_pressed = Color::from_hex("#3678E6FF");
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
        let target = HitTarget::Theme(mode);
        let button_bg = button_background(
            target,
            selected,
            hovered,
            pressed,
            ButtonPalette {
                normal: card,
                hover: card_hover,
                pressed: card_pressed,
                selected: accent,
                selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        );
        draw_segment(
            hdc,
            theme_rect(hwnd, mode),
            selected,
            button_bg,
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

    for preset in [AppearancePreset::Default, AppearancePreset::Minimal] {
        let selected = snapshot.appearance_preset == preset;
        let target = HitTarget::Layout(preset);
        let button_bg = button_background(
            target,
            selected,
            hovered,
            pressed,
            ButtonPalette {
                normal: card,
                hover: card_hover,
                pressed: card_pressed,
                selected: accent,
                selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        );
        draw_segment(
            hdc,
            layout_rect(hwnd, preset),
            selected,
            button_bg,
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            match (snapshot.language == LanguageId::SimplifiedChinese, preset) {
                (true, AppearancePreset::Default) => "默认",
                (true, AppearancePreset::Minimal) => "极简",
                (false, AppearancePreset::Default) => "Default",
                (false, AppearancePreset::Minimal) => "Minimal",
            },
        );
    }

    for item in [
        Section::Preset,
        Section::Panel,
        Section::Text,
        Section::Progress,
        Section::Interaction,
    ] {
        let selected = item == section;
        let r = section_rect(hwnd, item);
        let target = HitTarget::Section(item);
        let section_bg = button_background(
            target,
            selected,
            hovered,
            pressed,
            ButtonPalette {
                normal: background,
                hover: card_hover,
                pressed: card_pressed,
                selected: card_hover,
                selected_hover: card_hover,
                selected_pressed: card_pressed,
            },
        );
        fill(hdc, r, section_bg);
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

    if section == Section::Preset {
        paint_preset_gallery(
            hdc,
            hwnd,
            &snapshot,
            hovered,
            pressed,
            PresetGalleryPalette {
                card,
                card_hover,
                card_pressed,
                border: track_background,
                accent,
                primary,
            },
        );
    } else {
        for (index, row) in rows(section).iter().copied().enumerate() {
        let r = row_rect(hwnd, index);
        let selected = row == editor;
        let target = HitTarget::Row(row);
        let row_bg = button_background(
            target,
            selected,
            hovered,
            pressed,
            ButtonPalette {
                normal: card,
                hover: card_hover,
                pressed: card_pressed,
                selected: card_hover,
                selected_hover: card_hover,
                selected_pressed: card_pressed,
            },
        );
        fill(hdc, r, row_bg);
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
                    left: r.right - scale(hwnd, 170),
                    top: r.top + scale(hwnd, 9),
                    right: r.right - scale(hwnd, 140),
                    bottom: r.bottom - scale(hwnd, 9),
                };
                fill(hdc, swatch, color);
            }
            EditorSelection::Blur => {
                draw_slider(
                    hdc,
                    hwnd,
                    blur_slider_track_rect(hwnd),
                    snapshot.active_style.panel_frosted_strength,
                    FROSTED_STRENGTH_MAX,
                    track_background,
                    accent,
                );
                let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
                draw_text(
                    hdc,
                    "%",
                    rect(hwnd, 722, 210, 746, 238),
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
    }
    }

    if section != Section::Preset {
        paint_hex_edit_frames(
            hdc,
            hwnd,
            section,
            snapshot.is_dark,
            focused_hex_edit,
            &invalid_hex_edits,
            EditFramePalette {
                border: track_background,
                accent,
            },
        );
    }
    if section == Section::Panel {
        paint_blur_edit_frame(
            hdc,
            hwnd,
            snapshot.is_dark,
            focused_blur_edit,
            track_background,
            accent,
        );
    }
    if matches!(editor, EditorSelection::Color(_)) && section != Section::Preset {
        let editor_box = editor_box_rect(hwnd);
        fill(hdc, editor_box, card);
        paint_numeric_edit_frames(
            hdc,
            hwnd,
            snapshot.is_dark,
            focused_numeric_edit,
            track_background,
            accent,
        );
        paint_editor(
            hdc,
            hwnd,
            &snapshot,
            editor,
            EditorPalette {
                secondary,
                track_background,
                accent,
            },
        );
    }

    draw_segment(
        hdc,
        reset_rect(hwnd),
        false,
        button_background(
            HitTarget::Reset,
            false,
            hovered,
            pressed,
            ButtonPalette {
                normal: card,
                hover: card_hover,
                pressed: card_pressed,
                selected: card,
                selected_hover: card_hover,
                selected_pressed: card_pressed,
            },
        ),
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
        button_background(
            HitTarget::Close,
            true,
            hovered,
            pressed,
            ButtonPalette {
                normal: accent,
                hover: accent_hover,
                pressed: accent_pressed,
                selected: accent,
                selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        ),
        Color::from_hex("#FFFFFFFF"),
        if snapshot.language == LanguageId::SimplifiedChinese {
            "关闭"
        } else {
            "Close"
        },
    );

    SelectObject(hdc, old_font);

    let _ = BitBlt(
        screen_hdc,
        0,
        0,
        width,
        height,
        hdc,
        0,
        0,
        SRCCOPY,
    );
    SelectObject(hdc, old_bitmap);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(hdc);
    let _ = EndPaint(hwnd, &ps);
}

fn preset_group_label(is_dark: bool, language: LanguageId) -> &'static str {
    match (language == LanguageId::SimplifiedChinese, is_dark) {
        (true, true) => "深色预设",
        (true, false) => "浅色预设",
        (false, true) => "Dark presets",
        (false, false) => "Light presets",
    }
}

fn preset_label(preset: ThemePreset, is_dark: bool, language: LanguageId) -> &'static str {
    match (language == LanguageId::SimplifiedChinese, is_dark, preset) {
        (true, true, ThemePreset::Classic) => "石墨",
        (true, true, ThemePreset::Ocean) => "深海",
        (true, true, ThemePreset::Forest) => "松影",
        (false, true, ThemePreset::Classic) => "Graphite",
        (false, true, ThemePreset::Ocean) => "Deep Sea",
        (false, true, ThemePreset::Forest) => "Pine Shade",
        (true, false, ThemePreset::Classic) => "晨霜",
        (true, false, ThemePreset::Ocean) => "雾蓝",
        (true, false, ThemePreset::Forest) => "暖砂",
        (false, false, ThemePreset::Classic) => "Morning Frost",
        (false, false, ThemePreset::Ocean) => "Mist Blue",
        (false, false, ThemePreset::Forest) => "Warm Sand",
    }
}

unsafe fn paint_preset_gallery(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    palette: PresetGalleryPalette,
) {
    let PresetGalleryPalette {
        card,
        card_hover,
        card_pressed,
        border,
        accent,
        primary,
    } = palette;
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        preset_group_label(snapshot.is_dark, snapshot.language),
        rect(hwnd, 174, 108, 786, 136),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    for preset in ThemePreset::ALL {
        let r = preset_card_rect(hwnd, preset);
        let target = HitTarget::Preset(preset);
        let selected = snapshot
            .active_style
            .matches_preset(snapshot.is_dark, preset);
        let surface = if pressed == Some(target) {
            card_pressed
        } else if hovered == Some(target) {
            card_hover
        } else {
            card
        };
        fill(hdc, r, surface);
        draw_outline_rect_width(
            hdc,
            r,
            if selected { accent } else { border },
            if selected { 2 } else { 1 },
        );

        let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
        draw_text(
            hdc,
            preset_label(preset, snapshot.is_dark, snapshot.language),
            RECT {
                left: r.left + scale(hwnd, 12),
                top: r.top + scale(hwnd, 8),
                right: r.right - scale(hwnd, 12),
                bottom: r.top + scale(hwnd, 38),
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let style = ThemeStyle::preset(snapshot.is_dark, preset);
        let preview = RECT {
            left: r.left + scale(hwnd, 12),
            top: r.top + scale(hwnd, 46),
            right: r.right - scale(hwnd, 12),
            bottom: r.top + scale(hwnd, 138),
        };
        fill(
            hdc,
            preview,
            style.color(StyleColorTarget::PanelBackground),
        );
        draw_outline_rect(
            hdc,
            preview,
            style.color(StyleColorTarget::PanelBorder),
        );

        let text_left = preview.left + scale(hwnd, 10);
        let text_right = preview.right - scale(hwnd, 10);
        let _ = SetTextColor(
            hdc,
            COLORREF(style.color(StyleColorTarget::QuotaType).to_colorref()),
        );
        draw_text(
            hdc,
            "5h",
            RECT {
                left: text_left,
                top: preview.top + scale(hwnd, 6),
                right: text_right,
                bottom: preview.top + scale(hwnd, 28),
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let _ = SetTextColor(
            hdc,
            COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()),
        );
        draw_text(
            hdc,
            "82%",
            RECT {
                left: text_left,
                top: preview.top + scale(hwnd, 29),
                right: preview.left + scale(hwnd, 78),
                bottom: preview.top + scale(hwnd, 53),
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let _ = SetTextColor(
            hdc,
            COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()),
        );
        draw_text(
            hdc,
            "21:30",
            RECT {
                left: preview.left + scale(hwnd, 76),
                top: preview.top + scale(hwnd, 29),
                right: text_right,
                bottom: preview.top + scale(hwnd, 53),
            },
            DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
        );

        let progress = RECT {
            left: text_left,
            top: preview.top + scale(hwnd, 65),
            right: text_right,
            bottom: preview.top + scale(hwnd, 72),
        };
        fill(
            hdc,
            progress,
            style.color(StyleColorTarget::ProgressConsumed),
        );
        let filled = RECT {
            right: progress.left + (progress.right - progress.left) * 72 / 100,
            ..progress
        };
        fill(hdc, filled, style.color(StyleColorTarget::ProgressHigh));

        let swatches = [
            StyleColorTarget::ProgressHigh,
            StyleColorTarget::ProgressMedium,
            StyleColorTarget::ProgressLow,
            StyleColorTarget::ProgressConsumed,
        ];
        for (index, target) in swatches.iter().copied().enumerate() {
            let left = r.left + scale(hwnd, 14 + index as i32 * 42);
            let swatch = RECT {
                left,
                top: r.top + scale(hwnd, 158),
                right: left + scale(hwnd, 30),
                bottom: r.top + scale(hwnd, 170),
            };
            fill(hdc, swatch, style.color(target));
        }

        if selected {
            let marker = RECT {
                left: r.right - scale(hwnd, 24),
                top: r.top + scale(hwnd, 12),
                right: r.right - scale(hwnd, 12),
                bottom: r.top + scale(hwnd, 24),
            };
            fill(hdc, marker, accent);
        }
    }
}

unsafe fn paint_hex_edit_frames(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
    is_dark: bool,
    focused: Option<StyleColorTarget>,
    invalid: &[bool; HEX_EDIT_COUNT],
    palette: EditFramePalette,
) {
    let EditFramePalette { border, accent } = palette;
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };
    let error = Color::from_hex("#D95C5CFF");

    for (index, target) in COLOR_TARGETS.iter().copied().enumerate() {
        let Some(row_index) = color_row_index(section, target) else {
            continue;
        };
        let frame = hex_edit_frame_rect(hwnd, row_index);
        fill(hdc, frame, background);
        draw_outline_rect(
            hdc,
            frame,
            if invalid[index] {
                error
            } else if focused == Some(target) {
                accent
            } else {
                border
            },
        );
    }
}

unsafe fn paint_numeric_edit_frames(
    hdc: HDC,
    hwnd: HWND,
    is_dark: bool,
    focused: Option<usize>,
    border: Color,
    accent: Color,
) {
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };

    for index in 0..4 {
        let frame = numeric_edit_frame_rect(hwnd, index);
        fill(hdc, frame, background);
        draw_outline_rect(
            hdc,
            frame,
            if focused == Some(index) { accent } else { border },
        );
    }
}

unsafe fn paint_blur_edit_frame(
    hdc: HDC,
    hwnd: HWND,
    is_dark: bool,
    focused: bool,
    border: Color,
    accent: Color,
) {
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };
    let frame = blur_edit_frame_rect(hwnd);
    fill(hdc, frame, background);
    draw_outline_rect(hdc, frame, if focused { accent } else { border });
}

unsafe fn paint_editor(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    editor: EditorSelection,
    palette: EditorPalette,
) {
    let EditorSelection::Color(target) = editor else {
        return;
    };
    let EditorPalette {
        secondary,
        track_background,
        accent,
    } = palette;
    let color = snapshot.active_style.color(target);
    let values = [color.r, color.g, color.b, color.a];
    for (index, (label, value)) in ["R", "G", "B", "A"].iter().zip(values).enumerate() {
        let top = 336 + index as i32 * 32;
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
        Section::Preset => {
            if zh {
                "预设"
            } else {
                "Presets"
            }
        }
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

unsafe fn draw_outline_rect(hdc: HDC, rect: RECT, color: Color) {
    draw_outline_rect_width(hdc, rect, color, 1);
}

unsafe fn draw_outline_rect_width(hdc: HDC, rect: RECT, color: Color, width: i32) {
    let pen = CreatePen(PS_SOLID, width, COLORREF(color.to_colorref()));
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    let _ = Rectangle(hdc, rect.left, rect.top, rect.right, rect.bottom);
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
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


#[cfg(test)]
mod ui_smoke_tests {
    use super::*;

    #[test]
    fn hex_input_accepts_rgb_rgba_and_transient_partial_values() {
        assert_eq!(
            parse_hex_input("#123456"),
            Ok(Some(Color::from_hex("#123456FF")))
        );
        assert_eq!(
            parse_hex_input("89ABCDEF"),
            Ok(Some(Color::from_hex("#89ABCDEF")))
        );
        assert_eq!(parse_hex_input("#1234567"), Ok(None));
        assert_eq!(parse_hex_input("#12GG56"), Err(()));
    }

    #[test]
    fn preset_page_can_open_paint_and_destroy_without_editor_reentry() {
        unsafe {
            let snapshot = StyleWindowSnapshot {
                language: LanguageId::English,
                theme_mode: ThemeMode::Dark,
                is_dark: true,
                appearance_preset: AppearancePreset::Default,
                active_style: ThemeStyle::dark_default(),
            };

            open_or_focus(HWND::default(), snapshot);

            let hwnd = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| s.hwnd.to_hwnd())
                    .expect("style settings window should be created")
            };

            let _ = UpdateWindow(hwnd);
            assert!(!hwnd.0.is_null(), "style settings HWND must remain valid");

            set_section(Section::Panel);
            let hex_edit = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().unwrap().hex_edits[0].to_hwnd()
            };
            let _ = SetFocus(hex_edit);
            set_edit_text_string(hex_edit, "#12345678");
            let _ = UpdateWindow(hwnd);
            {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let s = state.as_ref().unwrap();
                assert_eq!(
                    s.editor,
                    EditorSelection::Color(StyleColorTarget::PanelBackground)
                );
                assert_eq!(s.snapshot.active_style.panel_background, "#12345678");
            }

            let (_, _, show_rgba, show_blur) = editor_layout_snapshot().unwrap();
            assert!(show_rgba);
            assert!(show_blur);

            select_editor(EditorSelection::Blur);
            let (_, _, show_rgba, show_blur) = editor_layout_snapshot().unwrap();
            assert!(!show_rgba, "blur selection must clear the lower RGBA editor");
            assert!(show_blur, "blur control must remain visible inline");

            let _ = DestroyWindow(hwnd);

            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            assert!(state.is_none(), "style settings state must be released on destroy");
        }
    }
}
