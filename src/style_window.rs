use std::sync::Mutex;
use std::{fs, path::PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW, LoadLibraryW};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OPENFILENAMEW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT,
    OFN_PATHMUSTEXIST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE,
};
use windows::Win32::UI::Shell::{ExtractIconExW, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::appearance::AppearancePreset;
use crate::fonts;
use crate::localization::LanguageId;
use crate::native_interop::{self, Color, WM_APP};
use crate::settings_model::{parse_jsonc, EditableSettings, EditableThemeStyle};
use crate::style::{
    StyleColorTarget, ThemeMode, ThemePreset, ThemeStyle, CORNER_RADIUS_MAX,
    FROSTED_STRENGTH_MAX,
};

#[link(name = "user32")]
unsafe extern "system" {
    #[link_name = "ShowScrollBar"]
    fn show_scroll_bar_native(hwnd: HWND, bar: i32, show: BOOL) -> BOOL;
}

unsafe fn hide_json_native_scrollbars(hwnd: HWND) {
    let _ = show_scroll_bar_native(hwnd, SB_VERT.0, BOOL(0));
    let _ = show_scroll_bar_native(hwnd, SB_HORZ.0, BOOL(0));
}

// Keep this block well away from updater.rs (WM_APP + 21..23).
pub const WM_STYLE_COLOR_PREVIEW: u32 = WM_APP + 120;
pub const WM_STYLE_BLUR_PREVIEW: u32 = WM_APP + 121;
pub const WM_STYLE_SAVE: u32 = WM_APP + 122;
pub const WM_STYLE_THEME_CHANGE: u32 = WM_APP + 123;
pub const WM_STYLE_LAYOUT_CHANGE: u32 = WM_APP + 124;
pub const WM_STYLE_PRESET_CHANGE: u32 = WM_APP + 126;
pub const WM_SETTINGS_REFRESH_CHANGE: u32 = WM_APP + 127;
pub const WM_SETTINGS_USAGE_CHANGE: u32 = WM_APP + 128;
pub const WM_SETTINGS_ALERT_CHANGE: u32 = WM_APP + 129;
pub const WM_SETTINGS_STARTUP_CHANGE: u32 = WM_APP + 130;
pub const WM_SETTINGS_LANGUAGE_CHANGE: u32 = WM_APP + 131;
pub const WM_SETTINGS_JSON_APPLY: u32 = WM_APP + 132;
pub const WM_STYLE_TOOLTIP_BLUR_PREVIEW: u32 = WM_APP + 133;
pub const WM_STYLE_CORNER_PREVIEW: u32 = WM_APP + 134;

const WINDOW_CLASS: &str = "CodexUsageUnifiedSettingsV1";
const WINDOW_WIDTH: i32 = 980;
const WINDOW_HEIGHT: i32 = 700;
const WINDOW_MIN_WIDTH: i32 = 980;
const WINDOW_MIN_HEIGHT: i32 = 700;
const ID_EDIT_R: u16 = 300;
const ID_EDIT_G: u16 = 301;
const ID_EDIT_B: u16 = 302;
const ID_EDIT_A: u16 = 303;
const ID_EDIT_BLUR: u16 = 304;
const ID_EDIT_CORNER: u16 = 305;
const ID_EDIT_HEX_BASE: u16 = 320;
const HEX_EDIT_COUNT: usize = 13;
const ID_EDIT_JSON: u16 = 400;
const JSON_EDIT_LIMIT: usize = 262_144;
const RICH_EDIT_CLASS: &str = "RICHEDIT50W";
const EM_SETBKGNDCOLOR_MSG: u32 = WM_USER + 67;
const EM_SETCHARFORMAT_MSG: u32 = WM_USER + 68;
const EM_SETEVENTMASK_MSG: u32 = WM_USER + 69;
const EM_EXGETSEL_MSG: u32 = WM_USER + 52;
const EM_EXSETSEL_MSG: u32 = WM_USER + 55;
const EM_LINEINDEX_MSG: u32 = 0x00BB;
const EM_GETSCROLLPOS_MSG: u32 = WM_USER + 221;
const EM_SETSCROLLPOS_MSG: u32 = WM_USER + 222;
const SCF_SELECTION_FLAG: usize = 0x0001;
const CFM_COLOR_MASK: u32 = 0x40000000;
const ENM_CHANGE_MASK: isize = 0x00000001;
const JSON_ACTION_TIMER_ID: usize = 0x4A53;
const JSON_SCROLLBAR_TIMER_ID: usize = 0x4A54;
const JSON_ACTION_DELAY_MS: u32 = 200;

#[repr(C)]
#[derive(Default)]
struct RichCharFormatW {
    cb_size: u32,
    dw_mask: u32,
    dw_effects: u32,
    y_height: i32,
    y_offset: i32,
    cr_text_color: COLORREF,
    b_char_set: u8,
    b_pitch_and_family: u8,
    sz_face_name: [u16; 32],
    w_weight: u16,
    s_spacing: i16,
    cr_back_color: COLORREF,
    lcid: u32,
    reserved: u32,
    s_style: i16,
    w_kerning: u16,
    b_underline_type: u8,
    b_animation: u8,
    b_rev_author: u8,
    b_underline_color: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RichCharRange {
    cp_min: i32,
    cp_max: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonTokenKind {
    Comment,
    Key,
    String,
    Number,
    Keyword,
}
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
    Tooltip,
    Text,
    Progress,
    Interaction,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorSelection {
    CornerRadius,
    Color(StyleColorTarget),
    Blur,
    TooltipBlur,
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
enum PendingDiscardAction {
    SwitchSection(Section),
    Close,
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
    JsonStatusPath,
    DiscardChanges,
    KeepEditing,
    LanguageToggle,
    LanguageOption(usize),
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
    corner_edit: SendHwnd,
    hex_edits: [SendHwnd; HEX_EDIT_COUNT],
    json_edit: SendHwnd,
    focused_numeric_edit: Option<usize>,
    focused_blur_edit: bool,
    focused_corner_edit: bool,
    focused_hex_edit: Option<StyleColorTarget>,
    invalid_hex_edits: [bool; HEX_EDIT_COUNT],
    syncing_numeric_edits: bool,
    syncing_blur_edit: bool,
    syncing_corner_edit: bool,
    syncing_hex_edits: bool,
    syncing_json_edit: bool,
    language_popup_open: bool,
    json_dirty: bool,
    json_status: String,
    json_status_path: Option<PathBuf>,
    pending_json_action: Option<JsonAction>,
    pending_discard_action: Option<PendingDiscardAction>,
    json_scroll_hovered: bool,
    json_scroll_dragging: bool,
    json_scroll_drag_offset: i32,
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
    StyleColorTarget::TooltipBackground,
    StyleColorTarget::TooltipBorder,
];

fn load_embedded_app_icons() -> (HICON, HICON) {
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


fn discard_dialog_rect(hwnd: HWND) -> RECT {
    let mut client = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client); }
    let width = scale(hwnd, 430);
    let height = scale(hwnd, 184);
    let left = (client.right - width) / 2;
    let top = (client.bottom - height) / 2;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn discard_dialog_button_rect(hwnd: HWND, discard: bool) -> RECT {
    let dialog = discard_dialog_rect(hwnd);
    let width = scale(hwnd, 122);
    let height = scale(hwnd, 36);
    let gap = scale(hwnd, 12);
    let bottom = dialog.bottom - scale(hwnd, 22);
    let right = dialog.right - scale(hwnd, 22);
    if discard {
        RECT {
            left: right - width * 2 - gap,
            top: bottom - height,
            right: right - width - gap,
            bottom,
        }
    } else {
        RECT {
            left: right - width,
            top: bottom - height,
            right,
            bottom,
        }
    }
}

fn request_discard_confirmation(hwnd: HWND, action: PendingDiscardAction) {
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.pending_discard_action = Some(action);
            s.pressed = None;
            s.hovered = None;
        }
    }
    layout_settings_children(hwnd);
    redraw_settings_window(hwnd);
}

fn redraw_settings_window(hwnd: HWND) {
    unsafe {
        let _ = RedrawWindow(
            hwnd,
            None,
            HRGN::default(),
            RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
}


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
        let (large_icon, small_icon) = load_embedded_app_icons();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: GetModuleHandleW(PCWSTR::null()).unwrap().into(),
            hIcon: large_icon,
            hIconSm: small_icon,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);

        let title = native_interop::wide_str("Codex Usage Win");
        let hwnd = match CreateWindowExW(
            WS_EX_APPWINDOW,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN | WS_THICKFRAME,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WINDOW_WIDTH,
            WINDOW_HEIGHT,
            HWND::default(),
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(_) => return,
        };

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

        let font_name = native_interop::wide_str(fonts::face(fonts::FontRole::Ui, Some(snapshot.language)));
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
        let mono_name = native_interop::wide_str(fonts::face(fonts::FontRole::Mono, None));
        let json_font = CreateFontW(
            -s(13),
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
            PCWSTR::from_raw(mono_name.as_ptr()),
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

        let corner_edit = match CreateWindowExW(
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
            HMENU(ID_EDIT_CORNER as usize as *mut _),
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
            corner_edit,
            WM_SETFONT,
            WPARAM(font.0 as usize),
            LPARAM(1),
        );
        let _ = SendMessageW(
            corner_edit,
            EM_SETLIMITTEXT_MSG,
            WPARAM(2),
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

        let msftedit = native_interop::wide_str("Msftedit.dll");
        if LoadLibraryW(PCWSTR::from_raw(msftedit.as_ptr())).is_err() {
            let _ = DestroyWindow(hwnd);
            let _ = DeleteObject(font);
            let _ = DeleteObject(json_font);
            return;
        }
        let rich_edit_class = native_interop::wide_str(RICH_EDIT_CLASS);
        let json_edit = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR::from_raw(rich_edit_class.as_ptr()),
            PCWSTR::from_raw(empty.as_ptr()),
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VSCROLL.0
                    | ES_MULTILINE as u32
                    | ES_AUTOVSCROLL as u32
                    | ES_AUTOHSCROLL as u32
                    | ES_WANTRETURN as u32
                    | ES_NOHIDESEL as u32,
            ),
            0,
            0,
            s(700),
            s(480),
            hwnd,
            HMENU(ID_EDIT_JSON as usize as *mut _),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) {
            Ok(value) => value,
            Err(_) => {
                let _ = DestroyWindow(hwnd);
                let _ = DeleteObject(font);
                let _ = DeleteObject(json_font);
                return;
            }
        };
        let _ = SendMessageW(json_edit, WM_SETFONT, WPARAM(json_font.0 as usize), LPARAM(1));
        hide_json_native_scrollbars(json_edit);
        let _ = SendMessageW(json_edit, EM_SETLIMITTEXT_MSG, WPARAM(JSON_EDIT_LIMIT), LPARAM(0));
        let _ = SendMessageW(
            json_edit,
            EM_SETEVENTMASK_MSG,
            WPARAM(0),
            LPARAM(ENM_CHANGE_MASK),
        );
        let json_background = if snapshot.is_dark {
            Color::from_hex("#1E1E1EFF")
        } else {
            Color::from_hex("#FFFFFFFF")
        };
        let _ = SendMessageW(
            json_edit,
            EM_SETBKGNDCOLOR_MSG,
            WPARAM(0),
            LPARAM(json_background.to_colorref() as isize),
        );

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
                section: Section::General,
                editor: EditorSelection::Color(StyleColorTarget::PanelBackground),
                dragging_slider: None,
                hovered: None,
                pressed: None,
                tracking_mouse_leave: false,
                numeric_edits,
                blur_edit: SendHwnd::from_hwnd(blur_edit),
                corner_edit: SendHwnd::from_hwnd(corner_edit),
                hex_edits,
                json_edit: SendHwnd::from_hwnd(json_edit),
                focused_numeric_edit: None,
                focused_blur_edit: false,
                focused_corner_edit: false,
                focused_hex_edit: None,
                invalid_hex_edits: [false; HEX_EDIT_COUNT],
                syncing_numeric_edits: false,
                syncing_blur_edit: false,
                syncing_corner_edit: false,
                syncing_hex_edits: false,
                syncing_json_edit: false,
                language_popup_open: false,
                json_dirty: false,
                json_status: String::new(),
                json_status_path: None,
                pending_json_action: None,
                pending_discard_action: None,
                json_scroll_hovered: false,
                json_scroll_dragging: false,
                json_scroll_drag_offset: 0,
                edit_brush: edit_brush.0 as isize,
                font: font.0 as isize,
                json_font: json_font.0 as isize,
            });
        }
        layout_numeric_edits(hwnd);
        layout_hex_edits(hwnd);
        layout_settings_children(hwnd);
        reload_json_editor_from_snapshot();
        sync_hex_edits();
        sync_numeric_edits();
        sync_blur_edit();
        sync_corner_edit();
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn sync(snapshot: StyleWindowSnapshot) {
    let (hwnd, section, json_dirty) = {
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
        (s.hwnd.to_hwnd(), s.section, s.json_dirty)
    };
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    layout_settings_children(hwnd);
    if section == Section::Json {
        if !json_dirty {
            reload_json_editor_from_snapshot();
        } else {
            refresh_json_editor_theme();
        }
    }
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
    sync_corner_edit();
    redraw_settings_window(hwnd);
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
        11 => StyleColorTarget::TooltipBackground,
        12 => StyleColorTarget::TooltipBorder,
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
        StyleColorTarget::TooltipBackground => 11,
        StyleColorTarget::TooltipBorder => 12,
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

const SETTINGS_CONTENT_LEFT: i32 = 200;
const SETTINGS_CONTENT_RIGHT: i32 = 940;
const SETTINGS_CARD_RADIUS: i32 = 10;
const SETTINGS_CARD_INSET_X: i32 = 18;
const SETTINGS_CHOICE_LABEL_RIGHT: i32 = 338;
const SETTINGS_CHOICE_LEFT: i32 = 360;
const SETTINGS_CHOICE_WIDTH: i32 = 130;
const SETTINGS_CHOICE_GAP: i32 = 10;
const SETTINGS_EDITOR_HEIGHT: i32 = 164;
const SETTINGS_EDITOR_CHANNEL_GAP: i32 = 30;

fn settings_page_title_rect(hwnd: HWND) -> RECT {
    rect(
        hwnd,
        SETTINGS_CONTENT_LEFT,
        18,
        SETTINGS_CONTENT_RIGHT,
        46,
    )
}

fn settings_card_rect(hwnd: HWND, top: i32, bottom: i32) -> RECT {
    rect(
        hwnd,
        SETTINGS_CONTENT_LEFT,
        top,
        SETTINGS_CONTENT_RIGHT,
        bottom,
    )
}

fn settings_card_label_rect(hwnd: HWND, top: i32, bottom: i32) -> RECT {
    rect(
        hwnd,
        SETTINGS_CONTENT_LEFT + SETTINGS_CARD_INSET_X,
        top,
        SETTINGS_CHOICE_LABEL_RIGHT,
        bottom,
    )
}

fn settings_choice_rect(hwnd: HWND, top: i32, bottom: i32, index: i32) -> RECT {
    let left = SETTINGS_CHOICE_LEFT
        + index * (SETTINGS_CHOICE_WIDTH + SETTINGS_CHOICE_GAP);
    rect(
        hwnd,
        left,
        top,
        left + SETTINGS_CHOICE_WIDTH,
        bottom,
    )
}

fn section_rect(hwnd: HWND, section: Section) -> RECT {
    let (top, bottom) = match section {
        Section::General => (28, 68),
        Section::Preset => (132, 172),
        Section::Panel => (176, 216),
        Section::Tooltip => (220, 260),
        Section::Text => (264, 304),
        Section::Progress => (308, 348),
        Section::Interaction => (352, 392),
        Section::Json => (456, 496),
    };
    rect(hwnd, 20, top, 166, bottom)
}

fn navigation_text_inset(section: Section) -> i32 {
    match section {
        Section::General => 14,
        Section::Preset
        | Section::Panel
        | Section::Tooltip
        | Section::Text
        | Section::Progress
        | Section::Interaction
        | Section::Json => 24,
    }
}

fn theme_rect(hwnd: HWND, mode: ThemeMode) -> RECT {
    let index = match mode {
        ThemeMode::System => 0,
        ThemeMode::Dark => 1,
        ThemeMode::Light => 2,
    };
    rect(hwnd, 314 + index * 108, 70, 414 + index * 108, 104)
}

fn layout_rect(hwnd: HWND, preset: AppearancePreset) -> RECT {
    let index = match preset {
        AppearancePreset::Default => 0,
        AppearancePreset::Minimal => 1,
    };
    rect(hwnd, 314 + index * 108, 116, 414 + index * 108, 150)
}

fn corner_choice_rect(hwnd: HWND, rounded: bool) -> RECT {
    let right = scale(hwnd, 940);
    let width = scale(hwnd, 94);
    let gap = scale(hwnd, 10);
    let top = scale(hwnd, 180);
    let bottom = scale(hwnd, 212);
    if rounded {
        RECT {
            left: right - width,
            top,
            right,
            bottom,
        }
    } else {
        RECT {
            left: right - width * 2 - gap,
            top,
            right: right - width - gap,
            bottom,
        }
    }
}

fn preset_card_rect(hwnd: HWND, preset: ThemePreset) -> RECT {
    let index = match preset {
        ThemePreset::Classic => 0,
        ThemePreset::Ocean => 1,
        ThemePreset::Forest => 2,
    };
    let left = 200 + index * 244;
    rect(hwnd, left, 220, left + 224, 404)
}

fn rows(section: Section) -> &'static [EditorSelection] {
    const PANEL: [EditorSelection; 4] = [
        EditorSelection::CornerRadius,
        EditorSelection::Color(StyleColorTarget::PanelBackground),
        EditorSelection::Color(StyleColorTarget::PanelBorder),
        EditorSelection::Blur,
    ];
    const TOOLTIP: [EditorSelection; 4] = [
        EditorSelection::CornerRadius,
        EditorSelection::Color(StyleColorTarget::TooltipBackground),
        EditorSelection::Color(StyleColorTarget::TooltipBorder),
        EditorSelection::TooltipBlur,
    ];
    const TEXT: [EditorSelection; 4] = [
        EditorSelection::Color(StyleColorTarget::QuotaType),
        EditorSelection::Color(StyleColorTarget::Remaining),
        EditorSelection::Color(StyleColorTarget::ResetTime),
        EditorSelection::Color(StyleColorTarget::Error),
    ];
    const PROGRESS: [EditorSelection; 5] = [
        EditorSelection::CornerRadius,
        EditorSelection::Color(StyleColorTarget::ProgressHigh),
        EditorSelection::Color(StyleColorTarget::ProgressMedium),
        EditorSelection::Color(StyleColorTarget::ProgressLow),
        EditorSelection::Color(StyleColorTarget::ProgressConsumed),
    ];
    const INTERACTION: [EditorSelection; 1] =
        [EditorSelection::Color(StyleColorTarget::DragHandle)];

    match section {
        Section::General | Section::Preset | Section::Json => &[],
        Section::Panel => &PANEL,
        Section::Tooltip => &TOOLTIP,
        Section::Text => &TEXT,
        Section::Progress => &PROGRESS,
        Section::Interaction => &INTERACTION,
    }
}

fn row_rect(hwnd: HWND, index: usize) -> RECT {
    rect(
        hwnd,
        200,
        218 + index as i32 * 50,
        940,
        260 + index as i32 * 50,
    )
}

fn editor_top(section: Section) -> i32 {
    382 + (rows(section).len() as i32 - 3) * 50
}

fn editor_box_rect(hwnd: HWND, section: Section) -> RECT {
    let top = editor_top(section);
    let mut editor = settings_card_rect(hwnd, top, top + SETTINGS_EDITOR_HEIGHT);
    let mut client = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client); }
    let safe_bottom = client.bottom - scale(hwnd, 24);
    editor.bottom = editor.bottom.min(safe_bottom);
    editor
}

fn color_slider_track_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let top = editor_top(section) + 42 + channel_index as i32 * SETTINGS_EDITOR_CHANNEL_GAP;
    rect(hwnd, 330, top, 790, top + 4)
}

fn color_slider_hit_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let track = color_slider_track_rect(hwnd, section, channel_index);
    RECT {
        left: track.left - scale(hwnd, 8),
        top: track.top - scale(hwnd, 10),
        right: track.right + scale(hwnd, 8),
        bottom: track.bottom + scale(hwnd, 10),
    }
}

fn blur_row_index(section: Section) -> Option<usize> {
    rows(section).iter().position(|row| {
        matches!(row, EditorSelection::Blur | EditorSelection::TooltipBlur)
    })
}

fn blur_slider_track_rect(hwnd: HWND, section: Section) -> RECT {
    let row = row_rect(hwnd, blur_row_index(section).unwrap_or(0));
    let center_y = (row.top + row.bottom) / 2;
    RECT {
        left: scale(hwnd, 396),
        top: center_y - scale(hwnd, 2),
        right: scale(hwnd, 770),
        bottom: center_y + scale(hwnd, 2),
    }
}

fn blur_slider_hit_rect(hwnd: HWND, section: Section) -> RECT {
    let track = blur_slider_track_rect(hwnd, section);
    RECT {
        left: track.left - scale(hwnd, 8),
        top: track.top - scale(hwnd, 12),
        right: track.right + scale(hwnd, 8),
        bottom: track.bottom + scale(hwnd, 12),
    }
}

fn inline_numeric_frame_rect(hwnd: HWND, row_index: usize) -> RECT {
    let row = row_rect(hwnd, row_index);
    RECT {
        left: row.right - scale(hwnd, 162),
        top: row.top + scale(hwnd, 6),
        right: row.right - scale(hwnd, 12),
        bottom: row.bottom - scale(hwnd, 6),
    }
}

fn blur_edit_frame_rect(hwnd: HWND, section: Section) -> RECT {
    inline_numeric_frame_rect(hwnd, blur_row_index(section).unwrap_or(0))
}

fn blur_edit_rect(hwnd: HWND, section: Section) -> RECT {
    let frame = blur_edit_frame_rect(hwnd, section);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 28),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

fn blur_suffix_rect(hwnd: HWND, section: Section) -> RECT {
    let frame = blur_edit_frame_rect(hwnd, section);
    RECT {
        left: frame.right - scale(hwnd, 28),
        top: frame.top,
        right: frame.right - scale(hwnd, 8),
        bottom: frame.bottom,
    }
}

fn corner_edit_frame_rect(hwnd: HWND) -> RECT {
    inline_numeric_frame_rect(hwnd, 0)
}

fn corner_edit_rect(hwnd: HWND) -> RECT {
    let frame = corner_edit_frame_rect(hwnd);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 32),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

fn corner_suffix_rect(hwnd: HWND) -> RECT {
    let frame = corner_edit_frame_rect(hwnd);
    RECT {
        left: frame.right - scale(hwnd, 32),
        top: frame.top,
        right: frame.right - scale(hwnd, 8),
        bottom: frame.bottom,
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
        left: row.right - scale(hwnd, 162),
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

fn numeric_edit_frame_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let top = editor_top(section) + 29 + channel_index as i32 * SETTINGS_EDITOR_CHANNEL_GAP;
    rect(hwnd, 812, top, 920, top + 26)
}

fn custom_preset_card_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 200, 438, 424, 622)
}

fn language_button_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 660, 414, 920, 450)
}

fn language_option_count() -> usize {
    LanguageId::ALL.len() + 1
}

fn language_option_rect(hwnd: HWND, index: usize) -> RECT {
    let button = language_button_rect(hwnd);
    let row_h = scale(hwnd, 28);
    let total_h = row_h * language_option_count() as i32;
    let bottom = button.top - scale(hwnd, 6);
    RECT {
        left: button.left,
        top: bottom - total_h + row_h * index as i32,
        right: button.right,
        bottom: bottom - total_h + row_h * (index as i32 + 1),
    }
}

fn language_code_for_index(index: usize) -> String {
    if index == 0 {
        "system".to_string()
    } else {
        LanguageId::ALL
            .get(index - 1)
            .map(|language| language.code().to_string())
            .unwrap_or_else(|| "system".to_string())
    }
}

fn language_label_for_index(index: usize, ui_language: LanguageId) -> &'static str {
    if index == 0 {
        ui_language.strings().system_default
    } else {
        LanguageId::ALL
            .get(index - 1)
            .map(|language| language.native_name())
            .unwrap_or(ui_language.strings().system_default)
    }
}

fn json_edit_rect(hwnd: HWND) -> RECT {
    let mut client = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client); }
    RECT {
        left: scale(hwnd, 200),
        top: scale(hwnd, 108),
        right: client.right - scale(hwnd, 24),
        bottom: client.bottom - scale(hwnd, 140),
    }
}

fn json_edit_text_rect(hwnd: HWND) -> RECT {
    let outer = json_edit_rect(hwnd);
    RECT {
        left: outer.left + scale(hwnd, 2),
        top: outer.top + scale(hwnd, 2),
        right: outer.right - scale(hwnd, 16),
        bottom: outer.bottom - scale(hwnd, 2),
    }
}

fn json_scrollbar_track_rect(hwnd: HWND) -> RECT {
    let outer = json_edit_rect(hwnd);
    RECT {
        left: outer.right - scale(hwnd, 12),
        top: outer.top + scale(hwnd, 8),
        right: outer.right - scale(hwnd, 4),
        bottom: outer.bottom - scale(hwnd, 8),
    }
}

fn json_scroll_thumb_rect(hwnd: HWND) -> Option<RECT> {
    let edit = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref()?.json_edit.to_hwnd()
    };
    let mut info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        ..Default::default()
    };
    unsafe {
        if GetScrollInfo(edit, SB_VERT, &mut info).is_err() {
            return None;
        }
    }
    let total = (info.nMax - info.nMin + 1).max(1) as i64;
    let page = i64::from(info.nPage.max(1));
    if page >= total {
        return None;
    }
    let track = json_scrollbar_track_rect(hwnd);
    let track_h = (track.bottom - track.top).max(1);
    let thumb_h = ((i64::from(track_h) * page / total) as i32)
        .max(scale(hwnd, 28))
        .min(track_h);
    let max_pos = (info.nMax - info.nPage as i32 + 1).max(info.nMin);
    let pos_range = (max_pos - info.nMin).max(1);
    let available = (track_h - thumb_h).max(0);
    let offset = available * (info.nPos - info.nMin).clamp(0, pos_range) / pos_range;
    Some(RECT {
        left: track.left + scale(hwnd, 1),
        top: track.top + offset,
        right: track.right - scale(hwnd, 1),
        bottom: track.top + offset + thumb_h,
    })
}

fn json_scroll_thumb_hit_rect(hwnd: HWND) -> Option<RECT> {
    let thumb = json_scroll_thumb_rect(hwnd)?;
    Some(RECT {
        left: thumb.left - scale(hwnd, 4),
        top: thumb.top - scale(hwnd, 3),
        right: thumb.right + scale(hwnd, 4),
        bottom: thumb.bottom + scale(hwnd, 3),
    })
}

fn update_json_scroll_drag(hwnd: HWND, y: i32) {
    let (edit, drag_offset) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else { return; };
        (s.json_edit.to_hwnd(), s.json_scroll_drag_offset)
    };
    let Some(thumb) = json_scroll_thumb_rect(hwnd) else { return; };
    let track = json_scrollbar_track_rect(hwnd);
    let thumb_h = thumb.bottom - thumb.top;
    let available = (track.bottom - track.top - thumb_h).max(1);
    let top = (y - drag_offset).clamp(track.top, track.bottom - thumb_h);

    let mut info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        ..Default::default()
    };
    unsafe {
        if GetScrollInfo(edit, SB_VERT, &mut info).is_err() {
            return;
        }
        let max_pos = (info.nMax - info.nPage as i32 + 1).max(info.nMin);
        let pos_range = (max_pos - info.nMin).max(1);
        let pos = info.nMin + (top - track.top) * pos_range / available;
        let packed = (SB_THUMBTRACK.0 as usize & 0xFFFF)
            | (((pos as usize) & 0xFFFF) << 16);
        let _ = SendMessageW(edit, WM_VSCROLL, WPARAM(packed), LPARAM(0));
        let _ = InvalidateRect(hwnd, Some(&json_scrollbar_track_rect(hwnd)), false);
    }
}

fn json_action_rect(hwnd: HWND, action: JsonAction) -> RECT {
    let edit = json_edit_rect(hwnd);
    let button_width = scale(hwnd, 108);
    let button_gap = scale(hwnd, 12);
    match action {
        JsonAction::Reload => rect(hwnd, 200, 58, 306, 94),
        JsonAction::Format => rect(hwnd, 318, 58, 424, 94),
        JsonAction::Export => RECT {
            left: edit.right - button_width,
            top: scale(hwnd, 58),
            right: edit.right,
            bottom: scale(hwnd, 94),
        },
        JsonAction::Import => RECT {
            left: edit.right - button_width * 2 - button_gap,
            top: scale(hwnd, 58),
            right: edit.right - button_width - button_gap,
            bottom: scale(hwnd, 94),
        },
        JsonAction::Apply => {
            let mut client = RECT::default();
            unsafe { let _ = GetClientRect(hwnd, &mut client); }
            let right = edit.right;
            let width = button_width;
            let height = scale(hwnd, 36);
            let bottom = client.bottom - scale(hwnd, 24);
            RECT {
                left: right - width,
                top: bottom - height,
                right,
                bottom,
            }
        }
    }
}

fn general_refresh_rect(hwnd: HWND, interval: u32) -> RECT {
    let index = match interval {
        60_000 => 0,
        300_000 => 1,
        900_000 => 2,
        _ => 3,
    };
    settings_choice_rect(hwnd, 68, 102, index)
}

fn general_usage_rect(hwnd: HWND, weekly: bool) -> RECT {
    rect(
        hwnd,
        812,
        if weekly { 202 } else { 162 },
        920,
        if weekly { 234 } else { 194 },
    )
}

fn general_alert_rect(hwnd: HWND, threshold: u8) -> RECT {
    let index = match threshold {
        0 => 0,
        10 => 1,
        20 => 2,
        _ => 3,
    };
    settings_choice_rect(hwnd, 268, 302, index)
}

fn general_startup_rect(hwnd: HWND) -> RECT {
    rect(hwnd, 812, 370, 920, 402)
}

fn numeric_edit_rect(hwnd: HWND, section: Section, channel_index: usize) -> RECT {
    let frame = numeric_edit_frame_rect(hwnd, section, channel_index);
    RECT {
        left: frame.left + scale(hwnd, 3),
        top: frame.top + scale(hwnd, 3),
        right: frame.right - scale(hwnd, 3),
        bottom: frame.bottom - scale(hwnd, 3),
    }
}

fn is_appearance_section(section: Section) -> bool {
    matches!(
        section,
        Section::Preset | Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
    )
}

fn editor_layout_snapshot() -> Option<([SendHwnd; 4], SendHwnd, Section, bool, bool)> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = state.as_ref()?;
    Some((
        s.numeric_edits,
        s.blur_edit,
        s.section,
        matches!(
            s.section,
            Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
        ) && matches!(s.editor, EditorSelection::Color(_)),
        matches!(s.section, Section::Panel | Section::Tooltip),
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
    let Some((numeric_edits, blur_edit, section, show_color, show_blur)) = editor_layout_snapshot() else {
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
            let r = numeric_edit_rect(hwnd, section, index);
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


fn layout_settings_children(hwnd: HWND) {
    let Some((json_edit, section, discard_pending)) = ({
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| (s.json_edit, s.section, s.pending_discard_action.is_some()))
    }) else {
        return;
    };

    unsafe {
        let focused = GetFocus();
        if (section != Section::Json || discard_pending) && focused == json_edit.to_hwnd() {
            let _ = SetFocus(hwnd);
        }

        let json_rect = json_edit_text_rect(hwnd);
        let visible = section == Section::Json && !discard_pending;
        let visibility = if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        let _ = SetWindowPos(
            json_edit.to_hwnd(),
            HWND::default(),
            json_rect.left,
            json_rect.top,
            (json_rect.right - json_rect.left).max(1),
            (json_rect.bottom - json_rect.top).max(1),
            SWP_NOZORDER | SWP_NOACTIVATE | visibility,
        );
        hide_json_native_scrollbars(json_edit.to_hwnd());
        if visible {
            let _ = SetTimer(hwnd, JSON_SCROLLBAR_TIMER_ID, 80, None);
        } else {
            let _ = KillTimer(hwnd, JSON_SCROLLBAR_TIMER_ID);
        }
    }
}


fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn to_windows_newlines(text: &str) -> String {
    normalize_to_lf(text).replace('\n', "\r\n")
}

fn read_large_edit_text_raw(edit: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(edit);
        if len <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(edit, &mut buffer) as usize;
        String::from_utf16_lossy(&buffer[..copied])
    }
}

fn read_large_edit_text(edit: HWND) -> String {
    normalize_to_lf(&read_large_edit_text_raw(edit))
}

fn json_token_color(kind: JsonTokenKind, is_dark: bool) -> Color {
    match (is_dark, kind) {
        (true, JsonTokenKind::Comment) => Color::from_hex("#777777FF"),
        (true, JsonTokenKind::Key) => Color::from_hex("#9CDCFEFF"),
        (true, JsonTokenKind::String) => Color::from_hex("#CE9178FF"),
        (true, JsonTokenKind::Number) => Color::from_hex("#B5CEA8FF"),
        (true, JsonTokenKind::Keyword) => Color::from_hex("#C586C0FF"),
        (false, JsonTokenKind::Comment) => Color::from_hex("#999999FF"),
        (false, JsonTokenKind::Key) => Color::from_hex("#0451A5FF"),
        (false, JsonTokenKind::String) => Color::from_hex("#A31515FF"),
        (false, JsonTokenKind::Number) => Color::from_hex("#098658FF"),
        (false, JsonTokenKind::Keyword) => Color::from_hex("#AF00DBFF"),
    }
}

fn json_default_text_color(is_dark: bool) -> Color {
    if is_dark {
        Color::from_hex("#D4D4D4FF")
    } else {
        Color::from_hex("#202020FF")
    }
}

fn json_editor_background(is_dark: bool) -> Color {
    if is_dark {
        Color::from_hex("#1E1E1EFF")
    } else {
        Color::from_hex("#FFFFFFFF")
    }
}

fn jsonc_highlight_ranges(text: &str) -> Vec<(i32, i32, JsonTokenKind)> {
    let chars: Vec<char> = text.chars().collect();
    let mut utf16_offsets = Vec::with_capacity(chars.len() + 1);
    let mut offset = 0i32;
    utf16_offsets.push(offset);
    for ch in &chars {
        offset += ch.len_utf16() as i32;
        utf16_offsets.push(offset);
    }

    let mut ranges = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            let start = i;
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            ranges.push((
                utf16_offsets[start],
                utf16_offsets[i],
                JsonTokenKind::Comment,
            ));
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2;
            } else {
                i = chars.len();
            }
            ranges.push((
                utf16_offsets[start],
                utf16_offsets[i],
                JsonTokenKind::Comment,
            ));
            continue;
        }
        if chars[i] == '"' {
            let start = i;
            i += 1;
            let mut escaped = false;
            while i < chars.len() {
                let ch = chars[i];
                i += 1;
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    break;
                }
            }
            let mut lookahead = i;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            let kind = if chars.get(lookahead) == Some(&':') {
                JsonTokenKind::Key
            } else {
                JsonTokenKind::String
            };
            ranges.push((utf16_offsets[start], utf16_offsets[i], kind));
            continue;
        }
        if chars[i].is_ascii_digit() || chars[i] == '-' {
            let start = i;
            i += 1;
            while i < chars.len()
                && matches!(
                    chars[i],
                    '0'..='9' | '.' | 'e' | 'E' | '+' | '-'
                )
            {
                i += 1;
            }
            ranges.push((
                utf16_offsets[start],
                utf16_offsets[i],
                JsonTokenKind::Number,
            ));
            continue;
        }

        let remaining: String = chars[i..].iter().take(5).collect();
        let keyword_len = if remaining.starts_with("false") {
            Some(5)
        } else if remaining.starts_with("true") || remaining.starts_with("null") {
            Some(4)
        } else {
            None
        };
        if let Some(len) = keyword_len {
            let end = (i + len).min(chars.len());
            ranges.push((
                utf16_offsets[i],
                utf16_offsets[end],
                JsonTokenKind::Keyword,
            ));
            i = end;
            continue;
        }
        i += 1;
    }
    ranges
}

unsafe fn rich_set_selection(edit: HWND, start: i32, end: i32) {
    let mut range = RichCharRange {
        cp_min: start,
        cp_max: end,
    };
    let _ = SendMessageW(
        edit,
        EM_EXSETSEL_MSG,
        WPARAM(0),
        LPARAM((&mut range as *mut RichCharRange) as isize),
    );
}

unsafe fn rich_set_selected_color(edit: HWND, color: Color) {
    let mut format = RichCharFormatW {
        cb_size: std::mem::size_of::<RichCharFormatW>() as u32,
        dw_mask: CFM_COLOR_MASK,
        cr_text_color: COLORREF(color.to_colorref()),
        ..Default::default()
    };
    let _ = SendMessageW(
        edit,
        EM_SETCHARFORMAT_MSG,
        WPARAM(SCF_SELECTION_FLAG),
        LPARAM((&mut format as *mut RichCharFormatW) as isize),
    );
}

fn syntax_highlight_json_editor(edit: HWND, text: &str, is_dark: bool) {
    unsafe {
        let mut previous_selection = RichCharRange::default();
        let _ = SendMessageW(
            edit,
            EM_EXGETSEL_MSG,
            WPARAM(0),
            LPARAM((&mut previous_selection as *mut RichCharRange) as isize),
        );

        let mut previous_scroll = POINT::default();
        let _ = SendMessageW(
            edit,
            EM_GETSCROLLPOS_MSG,
            WPARAM(0),
            LPARAM((&mut previous_scroll as *mut POINT) as isize),
        );

        // Formatting changes the RichEdit selection repeatedly. Suspend change
        // notifications and painting so this cannot re-enter EN_CHANGE or expose
        // intermediate selection/scroll states while the user is typing.
        let previous_event_mask = SendMessageW(
            edit,
            EM_SETEVENTMASK_MSG,
            WPARAM(0),
            LPARAM(0),
        );
        let _ = SendMessageW(edit, WM_SETREDRAW, WPARAM(0), LPARAM(0));

        rich_set_selection(edit, 0, -1);
        rich_set_selected_color(edit, json_default_text_color(is_dark));

        for (start, end, kind) in jsonc_highlight_ranges(text) {
            rich_set_selection(edit, start, end);
            rich_set_selected_color(edit, json_token_color(kind, is_dark));
        }

        let _ = SendMessageW(
            edit,
            EM_SETBKGNDCOLOR_MSG,
            WPARAM(0),
            LPARAM(json_editor_background(is_dark).to_colorref() as isize),
        );
        let _ = SendMessageW(
            edit,
            EM_EXSETSEL_MSG,
            WPARAM(0),
            LPARAM((&mut previous_selection as *mut RichCharRange) as isize),
        );
        // Restoring the selection may scroll the caret into view, so restore
        // the viewport after the selection and before repainting.
        let _ = SendMessageW(
            edit,
            EM_SETSCROLLPOS_MSG,
            WPARAM(0),
            LPARAM((&previous_scroll as *const POINT) as isize),
        );
        let _ = SendMessageW(
            edit,
            EM_SETEVENTMASK_MSG,
            WPARAM(0),
            LPARAM(previous_event_mask.0),
        );
        let _ = SendMessageW(edit, WM_SETREDRAW, WPARAM(1), LPARAM(0));

        // Erase stale line pixels before repainting. A non-erasing invalidate can
        // leave duplicated/offset glyph fragments after an insertion shifts rows.
        let _ = InvalidateRect(edit, None, true);
        let _ = UpdateWindow(edit);
    }
}

fn write_json_editor(text: &str, status: String, dirty: bool) {
    let (edit, hwnd, is_dark) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.syncing_json_edit = true;
        s.json_status = status;
        s.json_status_path = None;
        s.json_dirty = dirty;
        (s.json_edit.to_hwnd(), s.hwnd.to_hwnd(), s.snapshot.is_dark)
    };

    let normalized = normalize_to_lf(text);
    let windows_text = to_windows_newlines(&normalized);
    set_edit_text_string(edit, &windows_text);
    // RichEdit exposes CRLF through WM_GETTEXT, but its selection character
    // positions count each paragraph break as one character. Highlight against
    // LF-normalized text so token offsets match EM_EXSETSEL coordinates.
    syntax_highlight_json_editor(edit, &normalized, is_dark);

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_json_edit = false;
        }
    }
    layout_settings_children(hwnd);
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn refresh_json_editor_theme() {
    let (edit, hwnd, is_dark) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.hwnd.to_hwnd(), s.snapshot.is_dark)
    };
    let text = normalize_to_lf(&read_large_edit_text_raw(edit));
    syntax_highlight_json_editor(edit, &text, is_dark);
    layout_settings_children(hwnd);
}

fn reload_json_editor_from_snapshot() {
    let (settings, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.snapshot.editable_settings.clone(), s.snapshot.language)
    };
    let text = settings.to_jsonc(language);
    let status = if language == LanguageId::SimplifiedChinese {
        "✓ 已重新载入".to_string()
    } else {
        "✓ Reloaded".to_string()
    };
    write_json_editor(&text, status, false);
}


fn json_error_location(error: &str) -> Option<(usize, usize)> {
    let rest = error.strip_prefix("line ")?;
    let (line, rest) = rest.split_once(", column ")?;
    let (column, _) = rest.split_once(':')?;
    Some((line.trim().parse().ok()?, column.trim().parse().ok()?))
}

fn friendly_json_error(error: &str, language: LanguageId) -> String {
    let zh = language == LanguageId::SimplifiedChinese;
    let location = json_error_location(error);
    let raw_detail = error
        .split_once(": ")
        .map(|(_, detail)| detail)
        .unwrap_or(error);

    let detail = if raw_detail.contains("control character") {
        if zh {
            "字符串中包含非法控制字符。请检查这一行的引号、换行或转义字符。".to_string()
        } else {
            "A string contains an invalid control character. Check quotes, line breaks, and escapes on this line.".to_string()
        }
    } else if raw_detail.contains("unknown field") {
        let field = raw_detail
            .split(char::from(96u8))
            .nth(1)
            .unwrap_or("?");
        if zh {
            format!("未知配置项：{field}。请检查属性名是否拼写正确。")
        } else {
            format!("Unknown setting: {field}. Check the property name.")
        }
    } else if raw_detail.contains("expected #RRGGBB or #RRGGBBAA") {
        if zh {
            "颜色格式无效。允许 #RRGGBB 或 #RRGGBBAA。".to_string()
        } else {
            "Invalid color format. Use #RRGGBB or #RRGGBBAA.".to_string()
        }
    } else if raw_detail.contains("frosted_strength: allowed range is 0-100") {
        if zh {
            "磨砂强度超出范围。允许范围：0–100。".to_string()
        } else {
            "Frosted intensity is out of range. Allowed range: 0–100.".to_string()
        }
    } else if raw_detail.contains("session_5h and weekly cannot both be false") {
        if zh {
            "显示用量设置无效：5 小时额度和每周额度不能同时关闭。".to_string()
        } else {
            "Usage display is invalid: the 5-hour and weekly quotas cannot both be disabled.".to_string()
        }
    } else if raw_detail.contains("quota_alert_percent: allowed values") {
        if zh {
            "额度提醒值无效。允许：0、10、20、30。".to_string()
        } else {
            "Invalid quota alert value. Allowed values: 0, 10, 20, 30.".to_string()
        }
    } else if raw_detail.contains("refresh_interval: allowed values") {
        if zh {
            "刷新频率无效。允许：1m、5m、15m、1h。".to_string()
        } else {
            "Invalid refresh interval. Allowed values: 1m, 5m, 15m, 1h.".to_string()
        }
    } else {
        raw_detail.to_string()
    };

    if let Some((line, column)) = location {
        if zh {
            format!("第 {line} 行，第 {column} 列\n{detail}")
        } else {
            format!("Line {line}, column {column}\n{detail}")
        }
    } else {
        detail
    }
}

fn locate_json_error(edit: HWND, error: &str) {
    let Some((line, column)) = json_error_location(error) else {
        return;
    };
    unsafe {
        let line_index = SendMessageW(
            edit,
            EM_LINEINDEX_MSG,
            WPARAM(line.saturating_sub(1)),
            LPARAM(0),
        )
        .0 as i32;
        if line_index >= 0 {
            let start = line_index.saturating_add(column.saturating_sub(1) as i32);
            rich_set_selection(edit, start, start.saturating_add(1));
            let _ = SetFocus(edit);
        }
    }
}

fn set_json_status(status: String) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_status = status;
        s.json_status_path = None;
        s.hwnd.to_hwnd()
    };
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn set_json_status_with_path(status: String, path: PathBuf) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_status = status;
        s.json_status_path = Some(path);
        s.hwnd.to_hwnd()
    };
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn open_json_status_path() {
    let path = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().and_then(|s| s.json_status_path.clone())
    };
    let Some(path) = path else {
        return;
    };
    let operation = native_interop::wide_str("open");
    let path_wide = native_interop::wide_str(&path.to_string_lossy());
    unsafe {
        let _ = ShellExecuteW(
            HWND::default(),
            PCWSTR::from_raw(operation.as_ptr()),
            PCWSTR::from_raw(path_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn json_action_running_text(action: JsonAction, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match action {
        JsonAction::Reload => if zh { "◌ 载入中" } else { "◌ Reloading" },
        JsonAction::Format => if zh { "◌ 格式化中" } else { "◌ Formatting" },
        JsonAction::Import => if zh { "◌ 导入中" } else { "◌ Importing" },
        JsonAction::Export => if zh { "◌ 导出中" } else { "◌ Exporting" },
        JsonAction::Apply => if zh { "◌ 应用中" } else { "◌ Applying" },
    }
}

fn begin_json_action(hwnd: HWND, action: JsonAction) {
    if action == JsonAction::Apply {
        apply_json_editor();
        return;
    }

    let language = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.pending_json_action.is_some() {
            return;
        }
        s.pending_json_action = Some(action);
        s.json_status = json_action_running_text(action, s.snapshot.language).to_string();
        s.json_status_path = None;
        s.snapshot.language
    };
    let _ = language;
    unsafe {
        let _ = SetTimer(hwnd, JSON_ACTION_TIMER_ID, JSON_ACTION_DELAY_MS, None);
        let _ = InvalidateRect(hwnd, None, false);
        let _ = UpdateWindow(hwnd);
    }
}

fn finish_pending_json_action(hwnd: HWND) {
    let action = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_mut().and_then(|s| s.pending_json_action.take())
    };
    let Some(action) = action else {
        return;
    };
    unsafe {
        let _ = KillTimer(hwnd, JSON_ACTION_TIMER_ID);
    }
    match action {
        JsonAction::Reload => reload_json_editor_from_snapshot(),
        JsonAction::Format => format_json_editor(),
        JsonAction::Import => import_json_file(hwnd),
        JsonAction::Export => export_json_file(hwnd),
        JsonAction::Apply => apply_json_editor(),
    }
}

fn json_action_error(action_zh: &str, action_en: &str, error: &str, language: LanguageId) -> String {
    let detail = friendly_json_error(error, language);
    if language == LanguageId::SimplifiedChinese {
        format!("× {action_zh}\n{detail}")
    } else {
        format!("× {action_en}\n{detail}")
    }
}

fn update_json_validation_status(_mark_dirty: bool) {
    let (edit, syncing, language, hwnd, is_dark, applied_settings) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.json_edit.to_hwnd(),
            s.syncing_json_edit,
            s.snapshot.language,
            s.hwnd.to_hwnd(),
            s.snapshot.is_dark,
            s.snapshot.editable_settings.clone(),
        )
    };
    if syncing {
        return;
    }

    let raw_text = read_large_edit_text_raw(edit);
    let text = normalize_to_lf(&raw_text);
    syntax_highlight_json_editor(edit, &text, is_dark);

    let parsed = parse_jsonc(&text);
    let dirty = match &parsed {
        Ok(settings) => settings != &applied_settings,
        Err(_) => true,
    };

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.json_dirty = dirty;
            if let Err(error) = parsed {
                let detail = friendly_json_error(&error, language);
                s.json_status = if language == LanguageId::SimplifiedChinese {
                    format!("× 配置存在错误\n{detail}")
                } else {
                    format!("× Configuration has errors\n{detail}")
                };
                s.json_status_path = None;
            }
        }
    }
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn format_json_editor() {
    let (edit, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.snapshot.language)
    };
    let raw = read_large_edit_text(edit);
    match parse_jsonc(&raw) {
        Ok(settings) => {
            let dirty = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .is_some_and(|s| settings != s.snapshot.editable_settings)
            };
            let text = settings.to_jsonc(language);
            let status = if language == LanguageId::SimplifiedChinese {
                "✓ 已格式化".to_string()
            } else {
                "✓ Formatted".to_string()
            };
            write_json_editor(&text, status, dirty);
        }
        Err(error) => {
            set_json_status(json_action_error(
                "无法格式化：请先修复配置错误",
                "Cannot format: fix the configuration first",
                &error,
                language,
            ));
            locate_json_error(edit, &error);
        }
    }
}

fn export_default_filename() -> String {
    let time = unsafe { GetLocalTime() };
    format!(
        "codex-usage-win-config-{:04}{:02}{:02}-{:02}{:02}{:02}.json",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
}

fn file_dialog(hwnd: HWND, save: bool, default_name: Option<&str>) -> Option<PathBuf> {
    let mut buffer = vec![0u16; 4096];
    if let Some(name) = default_name {
        let encoded: Vec<u16> = name.encode_utf16().collect();
        let copy_len = encoded.len().min(buffer.len().saturating_sub(1));
        buffer[..copy_len].copy_from_slice(&encoded[..copy_len]);
    }
    let filter: Vec<u16> =
        "JSON/JSONC (*.json;*.jsonc)\0*.json;*.jsonc\0JSON (*.json)\0*.json\0All files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR::from_raw(filter.as_ptr()),
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        Flags: if save {
            OFN_OVERWRITEPROMPT
        } else {
            OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST
        },
        ..Default::default()
    };
    let accepted = unsafe {
        if save {
            GetSaveFileNameW(&mut ofn).as_bool()
        } else {
            GetOpenFileNameW(&mut ofn).as_bool()
        }
    };
    if !accepted {
        return None;
    }
    let len = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
}


fn import_json_file(hwnd: HWND) {
    let Some(path) = file_dialog(hwnd, false, None) else {
        return;
    };
    let text = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(error) => {
            let language = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| s.snapshot.language)
                    .unwrap_or(LanguageId::English)
            };
            set_json_status(if language == LanguageId::SimplifiedChinese {
                format!("× 无法读取导入文件\n{}：{error}", path.display())
            } else {
                format!("× Cannot read imported file\n{}: {error}", path.display())
            });
            return;
        }
    };
    let language = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state
            .as_ref()
            .map(|s| s.snapshot.language)
            .unwrap_or(LanguageId::English)
    };

    match parse_jsonc(&text) {
        Ok(settings) => {
            let dirty = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .is_some_and(|s| settings != s.snapshot.editable_settings)
            };
            let standard = settings.to_jsonc(language);
            let status = if language == LanguageId::SimplifiedChinese {
                "✓ 成功导入".to_string()
            } else {
                "✓ Imported successfully".to_string()
            };
            write_json_editor(&standard, status, dirty);
            set_json_status_with_path(
                if language == LanguageId::SimplifiedChinese {
                    "✓ 成功导入".to_string()
                } else {
                    "✓ Imported successfully".to_string()
                },
                path,
            );
        }
        Err(error) => {
            let status = json_action_error(
                "导入文件存在错误，可直接在编辑器中修复",
                "Imported file has errors; edit it directly below",
                &error,
                language,
            );
            write_json_editor(&text, status, true);
            let edit = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.json_edit.to_hwnd())
            };
            if let Some(edit) = edit {
                locate_json_error(edit, &error);
            }
        }
    }
}

fn export_json_file(hwnd: HWND) {
    let (edit, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.snapshot.language)
    };
    let raw = read_large_edit_text(edit);
    let settings = match parse_jsonc(&raw) {
        Ok(value) => value,
        Err(error) => {
            set_json_status(json_action_error(
                "无法导出：配置存在错误",
                "Cannot export: configuration has errors",
                &error,
                language,
            ));
            locate_json_error(edit, &error);
            return;
        }
    };

    let default_name = export_default_filename();
    let Some(mut path) = file_dialog(hwnd, true, Some(&default_name)) else {
        return;
    };
    if path.extension().is_none() {
        path.set_extension("json");
    }
    let json = match settings.to_pretty_json() {
        Ok(value) => value,
        Err(error) => {
            set_json_status(if language == LanguageId::SimplifiedChinese {
                format!("× 无法导出\n{error}")
            } else {
                format!("× Cannot export\n{error}")
            });
            return;
        }
    };
    match fs::write(&path, json) {
        Ok(()) => {
            set_json_status_with_path(
                if language == LanguageId::SimplifiedChinese {
                    "✓ 成功导出".to_string()
                } else {
                    "✓ Exported successfully".to_string()
                },
                path,
            );
        }
        Err(error) => set_json_status(if language == LanguageId::SimplifiedChinese {
            format!("× 无法写入导出文件\n{}：{error}", path.display())
        } else {
            format!("× Cannot write exported file\n{}: {error}", path.display())
        }),
    }
}

fn apply_json_editor() {
    let (edit, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.snapshot.language)
    };
    let raw = read_large_edit_text(edit);
    let settings = match parse_jsonc(&raw) {
        Ok(value) => value,
        Err(error) => {
            set_json_status(json_action_error(
                "无法应用：配置存在错误",
                "Cannot apply: configuration has errors",
                &error,
                language,
            ));
            locate_json_error(edit, &error);
            return;
        }
    };
    {
        let mut pending = PENDING_EDITABLE_SETTINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *pending = Some(settings.clone());
    }
    send_parent(WM_SETTINGS_JSON_APPLY, 0, 0);
    let (settings, language) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.snapshot.editable_settings.clone(), s.snapshot.language)
    };
    let standard = settings.to_jsonc(language);
    let status = if language == LanguageId::SimplifiedChinese {
        "✓ 已保存，并从当前应用设置重新载入".to_string()
    } else {
        "✓ Saved and reloaded from current application settings".to_string()
    };
    write_json_editor(&standard, status, false);
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
        if !matches!(
            s.section,
            Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
        ) {
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
        let value = match s.section {
            Section::Panel => s.snapshot.active_style.panel_frosted_strength,
            Section::Tooltip => s.snapshot.active_style.tooltip_frosted_strength(),
            _ => return,
        };
        s.syncing_blur_edit = true;
        (s.blur_edit.to_hwnd(), value)
    };
    set_edit_text(edit, value);

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_blur_edit = false;
    }
}

fn sync_corner_edit() {
    let (edit, value) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        let value = match s.section {
            Section::Panel => s.snapshot.active_style.panel_corner_radius,
            Section::Tooltip => s.snapshot.active_style.tooltip_corner_radius,
            Section::Progress => s.snapshot.active_style.progress_corner_radius,
            _ => return,
        };
        s.syncing_corner_edit = true;
        (s.corner_edit.to_hwnd(), value)
    };
    set_edit_text(edit, value);

    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = state.as_mut() {
        s.syncing_corner_edit = false;
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

fn sync_active_style_into_editable(state: &mut PanelState) {
    let editable = EditableThemeStyle::from_theme_style(&state.snapshot.active_style);
    if state.snapshot.is_dark {
        state.snapshot.editable_settings.appearance.dark = editable;
    } else {
        state.snapshot.editable_settings.appearance.light = editable;
    }
}

fn sync_hex_edits() {
    let (edits, values) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if !matches!(
            s.section,
            Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
        ) {
            return;
        }
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
                sync_active_style_into_editable(s);
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
        sync_active_style_into_editable(s);
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
    let (edit, syncing, section) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.blur_edit.to_hwnd(), s.syncing_blur_edit, s.section)
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
        if section == Section::Tooltip {
            s.snapshot.active_style.set_tooltip_frosted_strength(value);
        } else {
            s.snapshot.active_style.panel_frosted_strength = value;
        }
        sync_active_style_into_editable(s);
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

    send_parent(
        if section == Section::Tooltip {
            WM_STYLE_TOOLTIP_BLUR_PREVIEW
        } else {
            WM_STYLE_BLUR_PREVIEW
        },
        value as usize,
        0,
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

fn update_corner_from_numeric_edit() {
    let (edit, syncing, section) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.corner_edit.to_hwnd(), s.syncing_corner_edit, s.section)
    };
    if syncing {
        return;
    }

    let Some(raw_value) = read_edit_value(edit) else {
        return;
    };
    let value = raw_value.min(u16::from(CORNER_RADIUS_MAX)) as u8;

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        match section {
            Section::Panel => s.snapshot.active_style.panel_corner_radius = value,
            Section::Tooltip => s.snapshot.active_style.tooltip_corner_radius = value,
            Section::Progress => s.snapshot.active_style.progress_corner_radius = value,
            _ => return,
        }
        sync_active_style_into_editable(s);
    }

    if raw_value > u16::from(CORNER_RADIUS_MAX) {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_corner_edit = true;
        }
        drop(state);
        set_edit_text(edit, value);
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.syncing_corner_edit = false;
        }
    }

    let section_code = match section {
        Section::Panel => 0usize,
        Section::Tooltip => 1,
        Section::Progress => 2,
        _ => return,
    };
    send_parent(
        WM_STYLE_CORNER_PREVIEW,
        section_code | (usize::from(value) << 8),
        0,
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

fn text_width_px(hwnd: HWND, text: &str) -> i32 {
    let font = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.font).unwrap_or(0)
    };
    unsafe {
        let hdc = GetDC(hwnd);
        if hdc.0.is_null() {
            return 0;
        }
        let old_font = if font != 0 {
            Some(SelectObject(hdc, HGDIOBJ(font as *mut _)))
        } else {
            None
        };
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
        if let Some(old_font) = old_font {
            SelectObject(hdc, old_font);
        }
        let _ = ReleaseDC(hwnd, hdc);
        size.cx
    }
}

fn json_status_path_rect(hwnd: HWND, language: LanguageId) -> RECT {
    let edit_rect = json_edit_rect(hwnd);
    let prefix_width = if language == LanguageId::SimplifiedChinese {
        scale(hwnd, 96)
    } else {
        scale(hwnd, 150)
    };
    let path_text = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state
            .as_ref()
            .and_then(|s| s.json_status_path.as_ref())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let left = scale(hwnd, 200) + prefix_width;
    let mut client = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client); }
    let width = text_width_px(hwnd, &path_text).max(0);
    RECT {
        left,
        top: edit_rect.bottom + scale(hwnd, 8),
        right: (left + width).min(client.right - scale(hwnd, 24)),
        bottom: edit_rect.bottom + scale(hwnd, 34),
    }
}

fn hit_target_at(hwnd: HWND, x: i32, y: i32) -> Option<HitTarget> {
    let discard_pending = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref()?.pending_discard_action.is_some()
    };
    if discard_pending {
        if pt_in_rect(discard_dialog_button_rect(hwnd, true), x, y) {
            return Some(HitTarget::DiscardChanges);
        }
        if pt_in_rect(discard_dialog_button_rect(hwnd, false), x, y) {
            return Some(HitTarget::KeepEditing);
        }
        return None;
    }

    let (section, language_popup_open, json_dirty, json_status_path, language, json_action_pending) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (
            s.section,
            s.language_popup_open,
            s.json_dirty,
            s.json_status_path.is_some(),
            s.snapshot.language,
            s.pending_json_action.is_some(),
        )
    };

    if section == Section::General && language_popup_open {
        for index in 0..language_option_count() {
            if pt_in_rect(language_option_rect(hwnd, index), x, y) {
                return Some(HitTarget::LanguageOption(index));
            }
        }
        if pt_in_rect(language_button_rect(hwnd), x, y) {
            return Some(HitTarget::LanguageToggle);
        }
    }

    for item in [
        Section::General,
        Section::Preset,
        Section::Panel,
        Section::Tooltip,
        Section::Text,
        Section::Progress,
        Section::Interaction,
        Section::Json,
    ] {
        if pt_in_rect(section_rect(hwnd, item), x, y) {
            return Some(HitTarget::Section(item));
        }
    }

    if is_appearance_section(section) {
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
        if matches!(section, Section::Panel | Section::Tooltip | Section::Progress) {
            for rounded in [false, true] {
                if pt_in_rect(corner_choice_rect(hwnd, rounded), x, y) {
                    return Some(HitTarget::CornerShape(rounded));
                }
            }
        }
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
    }

    if section == Section::General {
        if pt_in_rect(language_button_rect(hwnd), x, y) {
            return Some(HitTarget::LanguageToggle);
        }
        for interval in [60_000u32, 300_000, 900_000, 3_600_000] {
            if pt_in_rect(general_refresh_rect(hwnd, interval), x, y) {
                return Some(HitTarget::Refresh(interval));
            }
        }
        if pt_in_rect(general_usage_rect(hwnd, false), x, y) {
            return Some(HitTarget::UsageSession);
        }
        if pt_in_rect(general_usage_rect(hwnd, true), x, y) {
            return Some(HitTarget::UsageWeekly);
        }
        for threshold in [0u8, 10, 20, 30] {
            if pt_in_rect(general_alert_rect(hwnd, threshold), x, y) {
                return Some(HitTarget::Alert(threshold));
            }
        }
        if pt_in_rect(general_startup_rect(hwnd), x, y) {
            return Some(HitTarget::Startup);
        }
    }

    if section == Section::Json {
        if json_status_path && pt_in_rect(json_status_path_rect(hwnd, language), x, y) {
            return Some(HitTarget::JsonStatusPath);
        }
        for action in [
            JsonAction::Reload,
            JsonAction::Format,
            JsonAction::Import,
            JsonAction::Export,
            JsonAction::Apply,
        ] {
            if json_action_pending || (action == JsonAction::Apply && !json_dirty) {
                continue;
            }
            if pt_in_rect(json_action_rect(hwnd, action), x, y) {
                return Some(HitTarget::Json(action));
            }
        }
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
        HitTarget::Refresh(interval) => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.snapshot.editable_settings.general.refresh_interval =
                        match interval {
                            60_000 => "1m",
                            300_000 => "5m",
                            900_000 => "15m",
                            _ => "1h",
                        }
                        .to_string();
                }
            }
            send_parent(WM_SETTINGS_REFRESH_CHANGE, interval as usize, 0);
        }
        HitTarget::UsageSession | HitTarget::UsageWeekly => {
            let mask = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return;
                };
                let usage = &mut s.snapshot.editable_settings.general.show_usage;
                match target {
                    HitTarget::UsageSession if usage.weekly || !usage.session_5h => {
                        usage.session_5h = !usage.session_5h;
                    }
                    HitTarget::UsageWeekly if usage.session_5h || !usage.weekly => {
                        usage.weekly = !usage.weekly;
                    }
                    _ => {}
                }
                usize::from(usage.session_5h) | (usize::from(usage.weekly) << 1)
            };
            send_parent(WM_SETTINGS_USAGE_CHANGE, mask, 0);
        }
        HitTarget::Alert(threshold) => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.snapshot.editable_settings.general.quota_alert_percent = threshold;
                }
            }
            send_parent(WM_SETTINGS_ALERT_CHANGE, threshold as usize, 0);
        }
        HitTarget::Startup => {
            let enabled = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return;
                };
                let value = !s.snapshot.editable_settings.general.start_with_windows;
                s.snapshot.editable_settings.general.start_with_windows = value;
                value
            };
            send_parent(WM_SETTINGS_STARTUP_CHANGE, usize::from(enabled), 0);
        }
        HitTarget::CornerShape(rounded) => {
            let section = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return;
                };
                match s.section {
                    Section::Panel => s.snapshot.active_style.panel_rounded = rounded,
                    Section::Tooltip => s.snapshot.active_style.tooltip_rounded = rounded,
                    Section::Progress => s.snapshot.active_style.progress_rounded = rounded,
                    _ => return,
                }
                let section = s.section;
                sync_active_style_into_editable(s);
                section
            };
            let section_code = match section {
                Section::Panel => 0usize,
                Section::Tooltip => 1,
                Section::Progress => 2,
                _ => return,
            };
            send_parent(
                WM_STYLE_CORNER_PREVIEW,
                section_code | (usize::from(rounded) << 8),
                0,
            );
            send_parent(WM_STYLE_SAVE, 0, 0);
        }
        HitTarget::DiscardChanges => {
            let action = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return;
                };
                s.json_dirty = false;
                s.json_status.clear();
                s.json_status_path = None;
                s.pending_discard_action.take()
            };
            match action {
                Some(PendingDiscardAction::SwitchSection(section)) => set_section(section),
                Some(PendingDiscardAction::Close) => unsafe {
                    send_parent(WM_STYLE_SAVE, 0, 0);
                    let _ = DestroyWindow(hwnd);
                },
                None => {}
            }
        }
        HitTarget::KeepEditing => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.pending_discard_action = None;
                    s.pressed = None;
                    s.hovered = None;
                }
            }
            layout_settings_children(hwnd);
            redraw_settings_window(hwnd);
        }
        HitTarget::JsonStatusPath => open_json_status_path(),
        HitTarget::Json(action) => {
            if action == JsonAction::Apply {
                let dirty = STATE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    .is_some_and(|s| s.json_dirty);
                if dirty {
                    begin_json_action(hwnd, action);
                }
            } else {
                begin_json_action(hwnd, action);
            }
        },
        HitTarget::LanguageToggle => {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = state.as_mut() {
                s.language_popup_open = !s.language_popup_open;
            }
        }
        HitTarget::LanguageOption(index) => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.snapshot.editable_settings.general.language = language_code_for_index(index);
                    s.language_popup_open = false;
                }
            }
            send_parent(WM_SETTINGS_LANGUAGE_CHANGE, index, 0);
        }
    }
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
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
    let (current_section, dirty, hwnd, discard_pending) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.section,
            s.json_dirty,
            s.hwnd.to_hwnd(),
            s.pending_discard_action.is_some(),
        )
    };
    if discard_pending {
        return;
    }
    if current_section == Section::Json && section != Section::Json && dirty {
        request_discard_confirmation(hwnd, PendingDiscardAction::SwitchSection(section));
        return;
    }

    let (hwnd, json_dirty) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.section = section;
        if section != Section::General {
            s.language_popup_open = false;
        }
        if let Some(editor) = rows(section).first().copied() {
            s.editor = editor;
        } else {
            s.focused_numeric_edit = None;
            s.focused_blur_edit = false;
            s.focused_hex_edit = None;
        }
        s.dragging_slider = None;
        s.pressed = None;
        s.hovered = None;
        (s.hwnd.to_hwnd(), s.json_dirty)
    };
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    if section == Section::Json {
        if json_dirty {
            refresh_json_editor_theme();
        } else {
            reload_json_editor_from_snapshot();
        }
    } else {
        layout_settings_children(hwnd);
    }
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
    redraw_settings_window(hwnd);
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

    if matches!(section, Section::Panel | Section::Tooltip) && pt_in_rect(blur_slider_hit_rect(hwnd), x, y) {
        return Some(SliderKind::Blur);
    }

    if !matches!(
        section,
        Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
    ) {
        return None;
    }

    let EditorSelection::Color(_) = editor else {
        return None;
    };
    for index in 0..4 {
        if pt_in_rect(color_slider_hit_rect(hwnd, section, index), x, y) {
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
    let mut blur_update: Option<(Section, u8)> = None;

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
                let value = slider_value_from_x(
                    color_slider_track_rect(hwnd, s.section, index),
                    x,
                    u8::MAX,
                );
                let current = s.snapshot.active_style.color(target);
                let color = match kind {
                    SliderKind::Red => Color::rgba(value, current.g, current.b, current.a),
                    SliderKind::Green => Color::rgba(current.r, value, current.b, current.a),
                    SliderKind::Blue => Color::rgba(current.r, current.g, value, current.a),
                    SliderKind::Alpha => Color::rgba(current.r, current.g, current.b, value),
                    SliderKind::Blur => current,
                };
                s.snapshot.active_style.set_color(target, color);
                sync_active_style_into_editable(s);
                color_update = Some((target, color));
            }
            (_, SliderKind::Blur) if matches!(s.section, Section::Panel | Section::Tooltip) => {
                let value = slider_value_from_x(
                    blur_slider_track_rect(hwnd),
                    x,
                    FROSTED_STRENGTH_MAX,
                );
                if s.section == Section::Tooltip {
                    s.snapshot.active_style.set_tooltip_frosted_strength(value);
                } else {
                    s.snapshot.active_style.panel_frosted_strength = value;
                }
                sync_active_style_into_editable(s);
                blur_update = Some((s.section, value));
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
    if let Some((section, value)) = blur_update {
        sync_blur_edit();
        send_parent(
            if section == Section::Tooltip { WM_STYLE_TOOLTIP_BLUR_PREVIEW } else { WM_STYLE_BLUR_PREVIEW },
            value as usize,
            0,
        );
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
        WM_TIMER => {
            if wparam.0 == JSON_ACTION_TIMER_ID {
                finish_pending_json_action(hwnd);
                return LRESULT(0);
            }
            if wparam.0 == JSON_SCROLLBAR_TIMER_ID {
                let section = {
                    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    state.as_ref().map(|s| s.section)
                };
                if section == Some(Section::Json) {
                    let _ = InvalidateRect(
                        hwnd,
                        Some(&json_scrollbar_track_rect(hwnd)),
                        false,
                    );
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
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

            let cursor_id = if json_scroll_thumb_hit_rect(hwnd)
                .is_some_and(|r| pt_in_rect(r, point.x, point.y))
            {
                IDC_HAND
            } else if slider_kind_at(hwnd, point.x, point.y).is_some() {
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

            let json_scroll_hit = json_scroll_thumb_hit_rect(hwnd)
                .is_some_and(|r| pt_in_rect(r, x, y));
            if json_scroll_hit {
                let thumb = json_scroll_thumb_rect(hwnd).unwrap_or_default();
                {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        s.json_scroll_dragging = true;
                        s.json_scroll_hovered = true;
                        s.json_scroll_drag_offset = (y - thumb.top).clamp(0, thumb.bottom - thumb.top);
                        s.pressed = None;
                    }
                }
                let _ = SetCapture(hwnd);
                let _ = InvalidateRect(hwnd, Some(&json_scrollbar_track_rect(hwnd)), false);
                return LRESULT(0);
            }

            let popup_open = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| s.section == Section::General && s.language_popup_open)
                    .unwrap_or(false)
            };
            if popup_open {
                let target = hit_target_at(hwnd, x, y);
                if !matches!(
                    target,
                    Some(HitTarget::LanguageToggle | HitTarget::LanguageOption(_))
                ) {
                    {
                        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(s) = state.as_mut() {
                            s.language_popup_open = false;
                            s.hovered = None;
                            s.pressed = None;
                        }
                    }
                    let _ = InvalidateRect(hwnd, None, false);
                    return LRESULT(0);
                }
            }

            if let Some(kind) = slider_kind_at(hwnd, x, y) {
                if kind == SliderKind::Blur {
                    let _ = SetFocus(hwnd);
                    let editor = {
                        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                        state.as_ref().map(|s| if s.section == Section::Tooltip { EditorSelection::TooltipBlur } else { EditorSelection::Blur })
                    };
                    if let Some(editor) = editor { select_editor(editor); }
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
            let (x, y) = point_from_lparam(lparam);
            let (scroll_dragging, old_scroll_hover) = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| (s.json_scroll_dragging, s.json_scroll_hovered))
                    .unwrap_or((false, false))
            };
            if scroll_dragging {
                update_json_scroll_drag(hwnd, y);
                return LRESULT(0);
            }
            let scroll_hover = json_scroll_thumb_hit_rect(hwnd)
                .is_some_and(|r| pt_in_rect(r, x, y));
            if scroll_hover != old_scroll_hover {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.json_scroll_hovered = scroll_hover;
                }
                drop(state);
                let _ = InvalidateRect(hwnd, Some(&json_scrollbar_track_rect(hwnd)), false);
            }

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
                let changed = s.hovered.is_some() || s.json_scroll_hovered;
                s.hovered = None;
                s.json_scroll_hovered = false;
                changed
            };
            if changed {
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let (x, y) = point_from_lparam(lparam);
            let was_scroll_dragging = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let Some(s) = state.as_mut() else {
                    return LRESULT(0);
                };
                let value = s.json_scroll_dragging;
                s.json_scroll_dragging = false;
                value
            };
            if was_scroll_dragging {
                let _ = ReleaseCapture();
                update_json_scroll_drag(hwnd, y);
                let _ = InvalidateRect(hwnd, Some(&json_scrollbar_track_rect(hwnd)), false);
                return LRESULT(0);
            }

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
                    {
                        s.json_scroll_dragging = false;
                        s.dragging_slider.take().is_some()
                    },
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

            if control_id == ID_EDIT_JSON && notification == EN_CHANGE_CODE {
                update_json_validation_status(true);
                return LRESULT(0);
            }
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
            layout_settings_children(hwnd);
            redraw_settings_window(hwnd);
            LRESULT(0)
        }
        WM_SIZE => {
            layout_settings_children(hwnd);
            redraw_settings_window(hwnd);
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(lparam.0 as *mut MINMAXINFO);
            info.ptMinTrackSize.x = scale(hwnd, WINDOW_MIN_WIDTH);
            info.ptMinTrackSize.y = scale(hwnd, WINDOW_MIN_HEIGHT);
            LRESULT(0)
        }
        WM_CLOSE => {
            let (dirty, pending) = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state
                    .as_ref()
                    .map(|s| (s.json_dirty, s.pending_discard_action.is_some()))
                    .unwrap_or((false, false))
            };
            if pending {
                return LRESULT(0);
            }
            if dirty {
                request_discard_confirmation(hwnd, PendingDiscardAction::Close);
                return LRESULT(0);
            }
            send_parent(WM_STYLE_SAVE, 0, 0);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        },
        WM_DESTROY => {
            let _ = KillTimer(hwnd, JSON_ACTION_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_SCROLLBAR_TIMER_ID);
            let resources = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.take().map(|s| (s.font, s.json_font, s.edit_brush))
            };
            if let Some((font, json_font, edit_brush)) = resources {
                if font != 0 {
                    let _ = DeleteObject(HGDIOBJ(font as *mut _));
                }
                if json_font != 0 {
                    let _ = DeleteObject(HGDIOBJ(json_font as *mut _));
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

#[allow(clippy::too_many_arguments)]
unsafe fn paint_discard_dialog(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    background: Color,
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    accent: Color,
    primary: Color,
    secondary: Color,
) {
    let zh = snapshot.language == LanguageId::SimplifiedChinese;
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let veil = if snapshot.is_dark {
        Color::from_hex("#17191DFF")
    } else {
        Color::from_hex("#DCE3EBFF")
    };
    fill(hdc, client, veil);

    let dialog = discard_dialog_rect(hwnd);
    fill_rounded_rect(hdc, dialog, card, scale(hwnd, 12));
    draw_rounded_outline_rect(
        hdc,
        dialog,
        if snapshot.is_dark {
            Color::from_hex("#4B5360FF")
        } else {
            Color::from_hex("#B8C3CFFF")
        },
        scale(hwnd, 12),
        1,
    );

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "未保存的 JSON 配置" } else { "Unsaved JSON configuration" },
        RECT {
            left: dialog.left + scale(hwnd, 24),
            top: dialog.top + scale(hwnd, 20),
            right: dialog.right - scale(hwnd, 24),
            bottom: dialog.top + scale(hwnd, 52),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if zh {
            "当前 JSON 配置有未保存的更改。是否放弃更改并继续？"
        } else {
            "The JSON configuration has unsaved changes. Discard them and continue?"
        },
        RECT {
            left: dialog.left + scale(hwnd, 24),
            top: dialog.top + scale(hwnd, 58),
            right: dialog.right - scale(hwnd, 24),
            bottom: dialog.top + scale(hwnd, 112),
        },
        DT_LEFT | DT_VCENTER | DT_WORDBREAK,
    );

    draw_segment(
        hdc,
        discard_dialog_button_rect(hwnd, true),
        false,
        button_background(
            HitTarget::DiscardChanges,
            false,
            hovered,
            pressed,
            ButtonPalette {
                normal: if snapshot.is_dark {
                    Color::from_hex("#5A2A2EFF")
                } else {
                    Color::from_hex("#F4D7DAFF")
                },
                hover: Color::from_hex("#C93C49FF"),
                pressed: Color::from_hex("#A92E39FF"),
                selected: card,
                selected_hover: card_hover,
                selected_pressed: card_pressed,
            },
        ),
        if hovered == Some(HitTarget::DiscardChanges)
            || pressed == Some(HitTarget::DiscardChanges)
        {
            Color::from_hex("#FFFFFFFF")
        } else {
            primary
        },
        if zh { "放弃更改" } else { "Discard" },
    );
    draw_segment(
        hdc,
        discard_dialog_button_rect(hwnd, false),
        true,
        button_background(
            HitTarget::KeepEditing,
            true,
            hovered,
            pressed,
            ButtonPalette {
                normal: accent,
                hover: Color::from_hex("#629CFFFF"),
                pressed: Color::from_hex("#3678E6FF"),
                selected: accent,
                selected_hover: Color::from_hex("#629CFFFF"),
                selected_pressed: Color::from_hex("#3678E6FF"),
            },
        ),
        Color::from_hex("#FFFFFFFF"),
        if zh { "继续编辑" } else { "Keep editing" },
    );
    let _ = background;
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
        json_status,
        json_status_path,
        json_dirty,
        discard_pending,
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
            s.json_status.clone(),
            s.json_status_path.clone(),
            s.json_dirty,
            s.pending_discard_action.is_some(),
            s.font,
        )
    };

    let dark = snapshot.is_dark;
    let background = if dark {
        Color::from_hex("#1F2125FF")
    } else {
        Color::from_hex("#E9EEF4FF")
    };
    let sidebar = if dark {
        Color::from_hex("#191B1FFF")
    } else {
        Color::from_hex("#F5F7FAFF")
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
    fill(
        hdc,
        RECT {
            left: 0,
            top: 0,
            right: scale(hwnd, 180),
            bottom: client.bottom,
        },
        sidebar,
    );
    fill(
        hdc,
        RECT {
            left: scale(hwnd, 179),
            top: 0,
            right: scale(hwnd, 180),
            bottom: client.bottom,
        },
        track_background,
    );

    let old_font = SelectObject(hdc, HGDIOBJ(font as *mut _));
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "外观"
        } else {
            "Appearance"
        },
        rect(hwnd, 20, 92, 166, 120),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "高级"
        } else {
            "Advanced"
        },
        rect(hwnd, 20, 416, 166, 444),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    paint_navigation(
        hdc,
        hwnd,
        &snapshot,
        section,
        hovered,
        pressed,
        background,
        card_hover,
        card_pressed,
        accent,
        primary,
        secondary,
    );

    match section {
        Section::General => paint_general_page(
            hdc,
            hwnd,
            &snapshot,
            hovered,
            pressed,
            card,
            card_hover,
            card_pressed,
            accent,
            accent_hover,
            accent_pressed,
            primary,
            secondary,
        ),
        Section::Json => paint_json_page(
            hdc,
            hwnd,
            &snapshot,
            &json_status,
            json_status_path.as_deref(),
            json_dirty,
            hovered,
            pressed,
            card,
            card_hover,
            card_pressed,
            accent,
            accent_hover,
            accent_pressed,
            primary,
            secondary,
        ),
        _ => paint_appearance_page(
            hdc,
            hwnd,
            &snapshot,
            section,
            editor,
            hovered,
            pressed,
            focused_numeric_edit,
            focused_blur_edit,
            focused_hex_edit,
            &invalid_hex_edits,
            card,
            card_hover,
            card_pressed,
            track_background,
            accent,
            accent_hover,
            accent_pressed,
            primary,
            secondary,
        ),
    }

    if discard_pending {
        paint_discard_dialog(
            hdc,
            hwnd,
            &snapshot,
            hovered,
            pressed,
            background,
            card,
            card_hover,
            card_pressed,
            accent,
            primary,
            secondary,
        );
    }

    SelectObject(hdc, old_font);
    let _ = BitBlt(screen_hdc, 0, 0, width, height, hdc, 0, 0, SRCCOPY);
    SelectObject(hdc, old_bitmap);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(hdc);
    let _ = EndPaint(hwnd, &ps);
}


#[allow(clippy::too_many_arguments)]
unsafe fn paint_navigation(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    section: Section,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    background: Color,
    hover: Color,
    pressed_color: Color,
    accent: Color,
    primary: Color,
    secondary: Color,
) {
    for item in [
        Section::General,
        Section::Preset,
        Section::Panel,
        Section::Tooltip,
        Section::Text,
        Section::Progress,
        Section::Interaction,
        Section::Json,
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
                hover,
                pressed: pressed_color,
                selected: hover,
                selected_hover: hover,
                selected_pressed: pressed_color,
            },
        );
        fill_rounded_rect(hdc, r, section_bg, scale(hwnd, 7));
        if selected {
            fill_rounded_rect(
                hdc,
                RECT {
                    right: r.left + scale(hwnd, 3),
                    ..r
                },
                accent,
                scale(hwnd, 2),
            );
        }
        let _ = SetTextColor(
            hdc,
            COLORREF(if selected { primary } else { secondary }.to_colorref()),
        );
        draw_text(
            hdc,
            section_label(item, snapshot.language),
            RECT {
                left: r.left + scale(hwnd, navigation_text_inset(item)),
                ..r
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_general_page(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    accent: Color,
    accent_hover: Color,
    accent_pressed: Color,
    primary: Color,
    secondary: Color,
) {
    let zh = snapshot.language == LanguageId::SimplifiedChinese;
    let strings = snapshot.language.strings();
    let general = &snapshot.editable_settings.general;
    let current_interval = match general.refresh_interval.as_str() {
        "1m" => 60_000,
        "5m" => 300_000,
        "1h" => 3_600_000,
        _ => 900_000,
    };
    let popup_open = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.language_popup_open).unwrap_or(false)
    };

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "常规" } else { "General" },
        settings_page_title_rect(hwnd),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    for r in [
        settings_card_rect(hwnd, 58, 112),
        settings_card_rect(hwnd, 128, 242),
        settings_card_rect(hwnd, 258, 312),
        settings_card_rect(hwnd, 328, 464),
    ] {
        fill_rounded_rect(hdc, r, card, scale(hwnd, SETTINGS_CARD_RADIUS));
    }

    // Refresh frequency and quota alerts intentionally share identical card
    // padding, label width, option width and option gaps.
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        strings.update_frequency,
        settings_card_label_rect(hwnd, 68, 102),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (interval, label) in [
        (60_000u32, strings.one_minute),
        (300_000, strings.five_minutes),
        (900_000, strings.fifteen_minutes),
        (3_600_000, strings.one_hour),
    ] {
        let selected = current_interval == interval;
        let target = HitTarget::Refresh(interval);
        draw_segment(
            hdc,
            general_refresh_rect(hwnd, interval),
            selected,
            button_background(
                target,
                selected,
                hovered,
                pressed,
                ButtonPalette {
                    normal: card_hover,
                    hover: card_pressed,
                    pressed: card_pressed,
                    selected: accent,
                    selected_hover: accent_hover,
                    selected_pressed: accent_pressed,
                },
            ),
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            label,
        );
    }

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "显示用量" } else { "Usage display" },
        rect(hwnd, 218, 138, 500, 162),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if zh { "5 小时额度" } else { "5-hour quota" },
        rect(hwnd, 238, 162, 650, 194),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if zh { "每周额度" } else { "Weekly quota" },
        rect(hwnd, 238, 202, 650, 234),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (target, enabled, weekly) in [
        (HitTarget::UsageSession, general.show_usage.session_5h, false),
        (HitTarget::UsageWeekly, general.show_usage.weekly, true),
    ] {
        draw_switch(
            hdc,
            general_usage_rect(hwnd, weekly),
            enabled,
            button_background(
                target,
                enabled,
                hovered,
                pressed,
                ButtonPalette {
                    normal: card_hover,
                    hover: card_pressed,
                    pressed: card_pressed,
                    selected: accent,
                    selected_hover: accent_hover,
                    selected_pressed: accent_pressed,
                },
            ),
        );
    }

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "额度提醒" } else { "Quota alerts" },
        settings_card_label_rect(hwnd, 268, 302),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (threshold, zh_label, en_label) in [
        (0u8, "关闭", "Off"),
        (10, "10%", "10%"),
        (20, "20%", "20%"),
        (30, "30%", "30%"),
    ] {
        let selected = general.quota_alert_percent == threshold;
        let target = HitTarget::Alert(threshold);
        draw_segment(
            hdc,
            general_alert_rect(hwnd, threshold),
            selected,
            button_background(
                target,
                selected,
                hovered,
                pressed,
                ButtonPalette {
                    normal: card_hover,
                    hover: card_pressed,
                    pressed: card_pressed,
                    selected: accent,
                    selected_hover: accent_hover,
                    selected_pressed: accent_pressed,
                },
            ),
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            if zh { zh_label } else { en_label },
        );
    }

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "应用" } else { "Application" },
        rect(hwnd, 218, 338, 500, 362),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        strings.start_with_windows,
        rect(hwnd, 238, 370, 650, 402),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let startup = general.start_with_windows;
    draw_switch(
        hdc,
        general_startup_rect(hwnd),
        startup,
        button_background(
            HitTarget::Startup,
            startup,
            hovered,
            pressed,
            ButtonPalette {
                normal: card_hover,
                hover: card_pressed,
                pressed: card_pressed,
                selected: accent,
                selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        ),
    );

    draw_text(
        hdc,
        strings.language,
        rect(hwnd, 238, 414, 600, 450),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let current_language_index = if general.language == "system" {
        0
    } else {
        LanguageId::ALL
            .iter()
            .position(|language| language.code() == general.language)
            .map(|index| index + 1)
            .unwrap_or(0)
    };
    let language_target = HitTarget::LanguageToggle;
    let language_button = language_button_rect(hwnd);
    let language_background = button_background(
        language_target,
        false,
        hovered,
        pressed,
        ButtonPalette {
            normal: card_hover,
            hover: card_pressed,
            pressed: card_pressed,
            selected: card_hover,
            selected_hover: card_pressed,
            selected_pressed: card_pressed,
        },
    );
    fill_rounded_rect(hdc, language_button, language_background, scale(hwnd, 8));
    draw_rounded_outline_rect(
        hdc,
        language_button,
        if popup_open {
            accent
        } else {
            subtle_control_border(language_background)
        },
        scale(hwnd, 8),
        1,
    );
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        language_label_for_index(current_language_index, snapshot.language),
        RECT {
            left: language_button.left + scale(hwnd, 12),
            top: language_button.top,
            right: language_button.right - scale(hwnd, 38),
            bottom: language_button.bottom,
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if popup_open { "▲" } else { "▼" },
        RECT {
            left: language_button.right - scale(hwnd, 34),
            top: language_button.top,
            right: language_button.right - scale(hwnd, 8),
            bottom: language_button.bottom,
        },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    if popup_open {
        for index in 0..language_option_count() {
            let option = language_option_rect(hwnd, index);
            let selected = current_language_index == index;
            let target = HitTarget::LanguageOption(index);
            fill_rounded_rect(
                hdc,
                option,
                button_background(
                    target,
                    selected,
                    hovered,
                    pressed,
                    ButtonPalette {
                        normal: card,
                        hover: card_hover,
                        pressed: card_pressed,
                        selected: card_hover,
                        selected_hover: card_pressed,
                        selected_pressed: card_pressed,
                    },
                ),
                scale(hwnd, 6),
            );
            let _ = SetTextColor(
                hdc,
                COLORREF(if selected { accent } else { primary }.to_colorref()),
            );
            draw_text(
                hdc,
                language_label_for_index(index, snapshot.language),
                RECT {
                    left: option.left + scale(hwnd, 12),
                    top: option.top,
                    right: option.right - scale(hwnd, 34),
                    bottom: option.bottom,
                },
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
            if selected {
                draw_text(
                    hdc,
                    "✓",
                    RECT {
                        left: option.right - scale(hwnd, 30),
                        top: option.top,
                        right: option.right - scale(hwnd, 8),
                        bottom: option.bottom,
                    },
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
        let first = language_option_rect(hwnd, 0);
        let last = language_option_rect(hwnd, language_option_count() - 1);
        draw_rounded_outline_rect(
            hdc,
            RECT {
                left: first.left,
                top: first.top,
                right: last.right,
                bottom: last.bottom,
            },
            accent,
            scale(hwnd, 7),
            1,
        );
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_json_page(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    status: &str,
    status_path: Option<&std::path::Path>,
    dirty: bool,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    accent: Color,
    accent_hover: Color,
    accent_pressed: Color,
    primary: Color,
    secondary: Color,
) {
    let zh = snapshot.language == LanguageId::SimplifiedChinese;
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "JSON 配置" } else { "JSON configuration" },
        settings_page_title_rect(hwnd),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (action, zh_label, en_label) in [
        (JsonAction::Reload, "重新载入", "Reload"),
        (JsonAction::Format, "格式化", "Format"),
        (JsonAction::Import, "导入", "Import"),
        (JsonAction::Export, "导出", "Export"),
    ] {
        let target = HitTarget::Json(action);
        draw_segment(
            hdc,
            json_action_rect(hwnd, action),
            false,
            button_background(
                target,
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
            if zh { zh_label } else { en_label },
        );
    }

    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let edit_rect = json_edit_rect(hwnd);
    let editor_background = json_editor_background(snapshot.is_dark);
    fill_rounded_rect(hdc, edit_rect, editor_background, scale(hwnd, 7));
    draw_rounded_outline_rect(
        hdc,
        edit_rect,
        if snapshot.is_dark {
            Color::from_hex("#59606AFF")
        } else {
            Color::from_hex("#AEB9C5FF")
        },
        scale(hwnd, 7),
        1,
    );
    let scroll_hovered = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.json_scroll_hovered).unwrap_or(false)
    };
    if let Some(thumb) = json_scroll_thumb_rect(hwnd) {
        fill_rounded_rect(
            hdc,
            thumb,
            if scroll_hovered {
                if snapshot.is_dark {
                    Color::from_hex("#777E88FF")
                } else {
                    Color::from_hex("#666D76FF")
                }
            } else if snapshot.is_dark {
                Color::from_hex("#555B64FF")
            } else {
                Color::from_hex("#747B84FF")
            },
            ((thumb.right - thumb.left) / 2).max(1),
        );
    }
    let running = status.starts_with('◌');
    let success = !status.starts_with('×') && !running;
    let status_color = if running {
        Color::from_hex("#8FA8C7FF")
    } else if success {
        Color::from_hex("#55B879FF")
    } else {
        Color::from_hex("#D95C5CFF")
    };
    let _ = SetTextColor(hdc, COLORREF(status_color.to_colorref()));

    let status_top = edit_rect.bottom + scale(hwnd, 8);
    let prefix_width = if zh { scale(hwnd, 96) } else { scale(hwnd, 150) };
    draw_text(
        hdc,
        status,
        RECT {
            left: scale(hwnd, 200),
            top: status_top,
            right: if status_path.is_some() {
                scale(hwnd, 200) + prefix_width
            } else {
                scale(hwnd, 760)
            },
            bottom: status_top + scale(hwnd, 26),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    if let Some(path) = status_path {
        let path_text = path.to_string_lossy();
        let path_left = scale(hwnd, 200) + prefix_width;
        let path_width = text_width_px(hwnd, &path_text).max(0);
        let path_rect = RECT {
            left: path_left,
            top: status_top,
            right: (path_left + path_width).min(client.right - scale(hwnd, 24)),
            bottom: status_top + scale(hwnd, 26),
        };
        let _ = SetTextColor(hdc, COLORREF(status_color.to_colorref()));
        draw_text(hdc, &path_text, path_rect, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        // Underline the clickable path area.
        let y = path_rect.bottom - scale(hwnd, 4);
        let pen = CreatePen(PS_SOLID, 1, COLORREF(status_color.to_colorref()));
        let old_pen = SelectObject(hdc, pen);
        let _ = MoveToEx(hdc, path_rect.left, y, None);
        let _ = LineTo(hdc, path_rect.right, y);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen);
    }

    if dirty {
        let _ = SetTextColor(hdc, COLORREF(Color::from_hex("#D69E2EFF").to_colorref()));
        draw_text(
            hdc,
            if zh { "● 有未保存更改" } else { "● Unsaved changes" },
            RECT {
                left: scale(hwnd, 200),
                top: status_top + scale(hwnd, 28),
                right: scale(hwnd, 630),
                bottom: status_top + scale(hwnd, 54),
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }

    let apply = JsonAction::Apply;
    let apply_background = if dirty {
        button_background(
            HitTarget::Json(apply),
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
        )
    } else {
        card_pressed
    };
    draw_segment(
        hdc,
        json_action_rect(hwnd, apply),
        dirty,
        apply_background,
        if dirty {
            Color::from_hex("#FFFFFFFF")
        } else {
            secondary
        },
        if zh { "应用" } else { "Apply" },
    );
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_appearance_page(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    section: Section,
    editor: EditorSelection,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    focused_numeric_edit: Option<usize>,
    focused_blur_edit: bool,
    focused_hex_edit: Option<StyleColorTarget>,
    invalid_hex_edits: &[bool; HEX_EDIT_COUNT],
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    track_background: Color,
    accent: Color,
    accent_hover: Color,
    accent_pressed: Color,
    primary: Color,
    secondary: Color,
) {
    let zh = snapshot.language == LanguageId::SimplifiedChinese;

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "外观" } else { "Appearance" },
        settings_page_title_rect(hwnd),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    fill_rounded_rect(
        hdc,
        settings_card_rect(hwnd, 58, 164),
        card,
        scale(hwnd, SETTINGS_CARD_RADIUS),
    );
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if zh { "主题" } else { "Theme" },
        rect(hwnd, 218, 70, 300, 104),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if zh { "排版" } else { "Layout" },
        rect(hwnd, 218, 116, 300, 150),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
        let selected = snapshot.theme_mode == mode;
        let target = HitTarget::Theme(mode);
        draw_segment(
            hdc,
            theme_rect(hwnd, mode),
            selected,
            button_background(
                target,
                selected,
                hovered,
                pressed,
                ButtonPalette {
                    normal: card_hover,
                    hover: card_pressed,
                    pressed: card_pressed,
                    selected: accent,
                    selected_hover: accent_hover,
                    selected_pressed: accent_pressed,
                },
            ),
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            match (zh, mode) {
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
        draw_segment(
            hdc,
            layout_rect(hwnd, preset),
            selected,
            button_background(
                target,
                selected,
                hovered,
                pressed,
                ButtonPalette {
                    normal: card_hover,
                    hover: card_pressed,
                    pressed: card_pressed,
                    selected: accent,
                    selected_hover: accent_hover,
                    selected_pressed: accent_pressed,
                },
            ),
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            match (zh, preset) {
                (true, AppearancePreset::Default) => "默认",
                (true, AppearancePreset::Minimal) => "极简",
                (false, AppearancePreset::Default) => "Default",
                (false, AppearancePreset::Minimal) => "Minimal",
            },
        );
    }

    if section == Section::Preset {
        paint_preset_gallery(
            hdc,
            hwnd,
            snapshot,
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
        return;
    }

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        section_label(section, snapshot.language),
        rect(hwnd, 200, 180, 600, 212),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    if matches!(section, Section::Panel | Section::Tooltip | Section::Progress) {
        let rounded = match section {
            Section::Panel => snapshot.active_style.panel_rounded,
            Section::Tooltip => snapshot.active_style.tooltip_rounded,
            Section::Progress => snapshot.active_style.progress_rounded,
            _ => false,
        };
        for (value, zh_label, en_label) in [
            (false, "直角", "Square"),
            (true, "圆角", "Rounded"),
        ] {
            let target = HitTarget::CornerShape(value);
            let selected = rounded == value;
            draw_segment(
                hdc,
                corner_choice_rect(hwnd, value),
                selected,
                button_background(
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
                ),
                if selected { Color::from_hex("#FFFFFFFF") } else { primary },
                if zh { zh_label } else { en_label },
            );
        }
    }

    for (index, row) in rows(section).iter().copied().enumerate() {
        let r = row_rect(hwnd, index);
        let selected = row == editor;
        let target = HitTarget::Row(row);
        fill_rounded_rect(
            hdc,
            r,
            button_background(
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
            ),
            scale(hwnd, 8),
        );
        let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
        draw_text(
            hdc,
            row_label(row, snapshot.language),
            RECT {
                left: r.left + scale(hwnd, 18),
                right: r.left + scale(hwnd, 280),
                ..r
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        match row {
            EditorSelection::Color(target) => {
                let color = snapshot.active_style.color(target);
                fill_rounded_rect(
                    hdc,
                    RECT {
                        left: r.right - scale(hwnd, 202),
                        top: r.top + scale(hwnd, 10),
                        right: r.right - scale(hwnd, 172),
                        bottom: r.bottom - scale(hwnd, 10),
                    },
                    color,
                    scale(hwnd, 4),
                );
            }
            EditorSelection::Blur | EditorSelection::TooltipBlur => {
                let value = if section == Section::Tooltip {
                    snapshot.active_style.tooltip_frosted_strength()
                } else {
                    snapshot.active_style.panel_frosted_strength
                };
                draw_slider(
                    hdc,
                    hwnd,
                    blur_slider_track_rect(hwnd),
                    value,
                    FROSTED_STRENGTH_MAX,
                    track_background,
                    accent,
                );
            }
        }
    }

    paint_hex_edit_frames(
        hdc,
        hwnd,
        section,
        snapshot.is_dark,
        focused_hex_edit,
        invalid_hex_edits,
        EditFramePalette {
            border: track_background,
            accent,
        },
    );

    if matches!(section, Section::Panel | Section::Tooltip) {
        paint_blur_edit_frame(
            hdc,
            hwnd,
            snapshot.is_dark,
            focused_blur_edit,
            track_background,
            accent,
            secondary,
        );
    }

    if matches!(editor, EditorSelection::Color(_)) {
        fill_rounded_rect(
            hdc,
            editor_box_rect(hwnd, section),
            card,
            scale(hwnd, SETTINGS_CARD_RADIUS),
        );
        paint_numeric_edit_frames(
            hdc,
            hwnd,
            section,
            snapshot.is_dark,
            focused_numeric_edit,
            track_background,
            accent,
        );
        paint_editor(
            hdc,
            hwnd,
            section,
            snapshot,
            editor,
            EditorPalette {
                secondary,
                track_background,
                accent,
            },
        );
    }
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
        rect(hwnd, 200, 180, 940, 212),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let mut matched = false;
    for preset in ThemePreset::ALL {
        let r = preset_card_rect(hwnd, preset);
        let target = HitTarget::Preset(preset);
        let selected = snapshot
            .active_style
            .matches_preset(snapshot.is_dark, preset);
        matched |= selected;
        let surface = if pressed == Some(target) {
            card_pressed
        } else if hovered == Some(target) {
            card_hover
        } else {
            card
        };
        let style = ThemeStyle::preset(snapshot.is_dark, preset);
        paint_style_preview_card(
            hdc,
            hwnd,
            r,
            &style,
            preset_label(preset, snapshot.is_dark, snapshot.language),
            selected,
            surface,
            border,
            accent,
            primary,
        );
    }

    if !matched {
        let zh = snapshot.language == LanguageId::SimplifiedChinese;
        let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
        draw_text(
            hdc,
            if zh { "当前自定义" } else { "Current custom" },
            rect(hwnd, 200, 400, 940, 430),
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        paint_style_preview_card(
            hdc,
            hwnd,
            custom_preset_card_rect(hwnd),
            &snapshot.active_style,
            if zh { "自定义" } else { "Custom" },
            true,
            card,
            border,
            accent,
            primary,
        );
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_style_preview_card(
    hdc: HDC,
    hwnd: HWND,
    r: RECT,
    style: &ThemeStyle,
    label: &str,
    selected: bool,
    surface: Color,
    border: Color,
    accent: Color,
    primary: Color,
) {
    fill_rounded_rect(hdc, r, surface, scale(hwnd, SETTINGS_CARD_RADIUS));
    draw_rounded_outline_rect(
        hdc,
        r,
        if selected { accent } else { border },
        scale(hwnd, SETTINGS_CARD_RADIUS),
        if selected { 2 } else { 1 },
    );
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        label,
        RECT {
            left: r.left + scale(hwnd, 12),
            top: r.top + scale(hwnd, 8),
            right: r.right - scale(hwnd, 12),
            bottom: r.top + scale(hwnd, 38),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let preview = RECT {
        left: r.left + scale(hwnd, 12),
        top: r.top + scale(hwnd, 46),
        right: r.right - scale(hwnd, 12),
        bottom: r.top + scale(hwnd, 138),
    };
    if style.panel_rounded {
        fill_rounded_rect(
            hdc,
            preview,
            style.color(StyleColorTarget::PanelBackground),
            scale(hwnd, 8),
        );
        draw_rounded_outline_rect(
            hdc,
            preview,
            style.color(StyleColorTarget::PanelBorder),
            scale(hwnd, 8),
            1,
        );
    } else {
        fill(hdc, preview, style.color(StyleColorTarget::PanelBackground));
        draw_outline_rect(hdc, preview, style.color(StyleColorTarget::PanelBorder));
    }

    let text_left = preview.left + scale(hwnd, 10);
    let text_right = preview.right - scale(hwnd, 10);
    let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::QuotaType).to_colorref()));
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
    let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
    draw_text(
        hdc,
        "82%",
        RECT {
            left: text_left,
            top: preview.top + scale(hwnd, 29),
            right: preview.left + scale(hwnd, 88),
            bottom: preview.top + scale(hwnd, 53),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()));
    draw_text(
        hdc,
        "21:30",
        RECT {
            left: preview.left + scale(hwnd, 86),
            top: preview.top + scale(hwnd, 29),
            right: text_right,
            bottom: preview.top + scale(hwnd, 53),
        },
        DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
    );

    let tooltip_preview = RECT {
        left: preview.right - scale(hwnd, 76),
        top: preview.top + scale(hwnd, 6),
        right: preview.right - scale(hwnd, 8),
        bottom: preview.top + scale(hwnd, 30),
    };
    if style.tooltip_rounded {
        fill_rounded_rect(
            hdc,
            tooltip_preview,
            style.color(StyleColorTarget::TooltipBackground),
            scale(hwnd, 5),
        );
        draw_rounded_outline_rect(
            hdc,
            tooltip_preview,
            style.color(StyleColorTarget::TooltipBorder),
            scale(hwnd, 5),
            1,
        );
    } else {
        fill(
            hdc,
            tooltip_preview,
            style.color(StyleColorTarget::TooltipBackground),
        );
        draw_outline_rect(
            hdc,
            tooltip_preview,
            style.color(StyleColorTarget::TooltipBorder),
        );
    }
    let _ = SetTextColor(
        hdc,
        COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()),
    );
    draw_text(
        hdc,
        "21:30",
        tooltip_preview,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );

    let progress = RECT {
        left: text_left,
        top: preview.top + scale(hwnd, 65),
        right: text_right,
        bottom: preview.top + scale(hwnd, 72),
    };
    let progress_fill = RECT {
        right: progress.left + (progress.right - progress.left) * 72 / 100,
        ..progress
    };
    if style.progress_rounded {
        let radius = ((progress.bottom - progress.top) / 2).max(1);
        fill_rounded_rect(
            hdc,
            progress,
            style.color(StyleColorTarget::ProgressConsumed),
            radius,
        );
        fill_rounded_rect(
            hdc,
            progress_fill,
            style.color(StyleColorTarget::ProgressHigh),
            radius.min(((progress_fill.right - progress_fill.left) / 2).max(1)),
        );
    } else {
        fill(hdc, progress, style.color(StyleColorTarget::ProgressConsumed));
        fill(
            hdc,
            progress_fill,
            style.color(StyleColorTarget::ProgressHigh),
        );
    }

    for (index, target) in [
        StyleColorTarget::ProgressHigh,
        StyleColorTarget::ProgressMedium,
        StyleColorTarget::ProgressLow,
        StyleColorTarget::ProgressConsumed,
    ]
    .iter()
    .copied()
    .enumerate()
    {
        let left = r.left + scale(hwnd, 14 + index as i32 * 48);
        fill_rounded_rect(
            hdc,
            RECT {
                left,
                top: r.top + scale(hwnd, 158),
                right: left + scale(hwnd, 34),
                bottom: r.top + scale(hwnd, 172),
            },
            style.color(target),
            scale(hwnd, 4),
        );
    }

    if selected {
        fill_rounded_rect(
            hdc,
            RECT {
                left: r.right - scale(hwnd, 24),
                top: r.top + scale(hwnd, 12),
                right: r.right - scale(hwnd, 12),
                bottom: r.top + scale(hwnd, 24),
            },
            accent,
            scale(hwnd, 3),
        );
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
        fill_rounded_rect(hdc, frame, background, scale(hwnd, 6));
        draw_rounded_outline_rect(
            hdc,
            frame,
            if invalid[index] {
                error
            } else if focused == Some(target) {
                accent
            } else {
                border
            },
            scale(hwnd, 6),
            1,
        );
    }
}

unsafe fn paint_numeric_edit_frames(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
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
        let frame = numeric_edit_frame_rect(hwnd, section, index);
        fill_rounded_rect(hdc, frame, background, scale(hwnd, 6));
        draw_rounded_outline_rect(
            hdc,
            frame,
            if focused == Some(index) { accent } else { border },
            scale(hwnd, 6),
            1,
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
    suffix_color: Color,
) {
    let background = if is_dark {
        Color::from_hex("#20242AFF")
    } else {
        Color::from_hex("#EEF3F8FF")
    };
    let frame = blur_edit_frame_rect(hwnd);
    fill_rounded_rect(hdc, frame, background, scale(hwnd, 6));
    draw_rounded_outline_rect(
        hdc,
        frame,
        if focused { accent } else { border },
        scale(hwnd, 6),
        1,
    );

    let _ = SetTextColor(hdc, COLORREF(suffix_color.to_colorref()));
    draw_text(
        hdc,
        "%",
        blur_suffix_rect(hwnd),
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

unsafe fn paint_editor(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
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
    let editor_top = editor_top(section);
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "颜色通道"
        } else {
            "Color channels"
        },
        rect(hwnd, 218, editor_top + 4, 420, editor_top + 28),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (index, (label, value)) in ["R", "G", "B", "A"].iter().zip(values).enumerate() {
        let center =
            editor_top + 44 + index as i32 * SETTINGS_EDITOR_CHANNEL_GAP;
        draw_text(
            hdc,
            label,
            rect(hwnd, 224, center - 14, 272, center + 14),
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        draw_slider(
            hdc,
            hwnd,
            color_slider_track_rect(hwnd, section, index),
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
    let track_height = (track.bottom - track.top).max(1);
    fill_rounded_rect(hdc, track, track_background, (track_height / 2).max(1));

    let width = (track.right - track.left).max(1);
    let thumb_x = track.left + width * i32::from(value) / i32::from(max.max(1));
    let filled = RECT {
        right: thumb_x.max(track.left),
        ..track
    };
    if filled.right > filled.left {
        fill_rounded_rect(hdc, filled, accent, (track_height / 2).max(1));
    }

    let radius = scale(hwnd, 6);
    let center_y = (track.top + track.bottom) / 2;
    let thumb = RECT {
        left: thumb_x - radius,
        top: center_y - radius,
        right: thumb_x + radius,
        bottom: center_y + radius,
    };
    fill_rounded_rect(hdc, thumb, accent, radius);
}

fn section_label(section: Section, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match section {
        Section::General => {
            if zh { "常规" } else { "General" }
        }
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
        Section::Tooltip => {
            if zh { "浮框" } else { "Tooltip" }
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
        Section::Json => {
            if zh { "JSON 配置" } else { "JSON config" }
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
        EditorSelection::Blur | EditorSelection::TooltipBlur => {
            if zh { "磨砂强度" } else { "Frosted intensity" }
        }
        EditorSelection::Color(StyleColorTarget::TooltipBackground) => {
            if zh { "背景颜色" } else { "Background" }
        }
        EditorSelection::Color(StyleColorTarget::TooltipBorder) => {
            if zh { "边框颜色" } else { "Border" }
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
    let height = (rect.bottom - rect.top).max(1);
    let radius = (height / 4).clamp(5, 10);
    fill_rounded_rect(hdc, rect, background, radius);

    let border = if selected {
        Color::from_hex("#76A7FFFF")
    } else {
        subtle_control_border(background)
    };
    draw_rounded_outline_rect(hdc, rect, border, radius, 1);

    let _ = SetTextColor(hdc, COLORREF(foreground.to_colorref()));
    draw_text(hdc, text, rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
}

fn subtle_control_border(background: Color) -> Color {
    let brightness = background.r as u16 + background.g as u16 + background.b as u16;
    if brightness < 384 {
        Color::from_hex("#4B5360FF")
    } else {
        Color::from_hex("#B8C3CFFF")
    }
}

unsafe fn fill_rounded_rect(hdc: HDC, rect: RECT, color: Color, radius: i32) {
    if native_interop::draw_antialiased_rounded_rect(
        hdc,
        rect,
        radius.max(1) as f32,
        Some(color),
        None,
    ) {
        return;
    }

    // Conservative fallback for systems where Direct2D DC rendering is unavailable.
    let brush = CreateSolidBrush(COLORREF(color.to_colorref()));
    let pen = CreatePen(PS_SOLID, 1, COLORREF(color.to_colorref()));
    let old_brush = SelectObject(hdc, brush);
    let old_pen = SelectObject(hdc, pen);
    let diameter = (radius.max(1) * 2).max(2);
    let _ = RoundRect(
        hdc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        diameter,
        diameter,
    );
    SelectObject(hdc, old_pen);
    SelectObject(hdc, old_brush);
    let _ = DeleteObject(pen);
    let _ = DeleteObject(brush);
}

unsafe fn draw_rounded_outline_rect(
    hdc: HDC,
    rect: RECT,
    color: Color,
    radius: i32,
    width: i32,
) {
    if native_interop::draw_antialiased_rounded_rect(
        hdc,
        rect,
        radius.max(1) as f32,
        None,
        Some((color, width.max(1) as f32)),
    ) {
        return;
    }

    // Conservative fallback for systems where Direct2D DC rendering is unavailable.
    let pen = CreatePen(PS_SOLID, width.max(1), COLORREF(color.to_colorref()));
    let old_pen = SelectObject(hdc, pen);
    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    let diameter = (radius.max(1) * 2).max(2);
    let _ = RoundRect(
        hdc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        diameter,
        diameter,
    );
    SelectObject(hdc, old_brush);
    SelectObject(hdc, old_pen);
    let _ = DeleteObject(pen);
}

unsafe fn draw_switch(hdc: HDC, hit_rect: RECT, enabled: bool, track_color: Color) {
    let hit_width = (hit_rect.right - hit_rect.left).max(1);
    let hit_height = (hit_rect.bottom - hit_rect.top).max(1);
    let track_height = (hit_height * 11 / 16).max(12);
    let track_width = (track_height * 20 / 11).min(hit_width);
    let track = RECT {
        left: hit_rect.right - track_width,
        top: hit_rect.top + (hit_height - track_height) / 2,
        right: hit_rect.right,
        bottom: hit_rect.top + (hit_height - track_height) / 2 + track_height,
    };

    fill_rounded_rect(hdc, track, track_color, track_height / 2);
    draw_rounded_outline_rect(
        hdc,
        track,
        subtle_control_border(track_color),
        track_height / 2,
        1,
    );

    let knob_size = (track_height - 4).max(8);
    let knob_top = track.top + (track_height - knob_size) / 2;
    let knob_left = if enabled {
        track.right - knob_size - 2
    } else {
        track.left + 2
    };
    let knob_rect = RECT {
        left: knob_left,
        top: knob_top,
        right: knob_left + knob_size,
        bottom: knob_top + knob_size,
    };
    fill_rounded_rect(
        hdc,
        knob_rect,
        Color::from_hex("#FFFFFFFF"),
        knob_size / 2,
    );
    draw_rounded_outline_rect(
        hdc,
        knob_rect,
        Color::from_hex("#E7ECF2FF"),
        knob_size / 2,
        1,
    );
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
                editable_settings: EditableSettings {
                    schema_version: 1,
                    general: crate::settings_model::EditableGeneral {
                        refresh_interval: "15m".into(),
                        show_usage: crate::settings_model::EditableUsage {
                            session_5h: true,
                            weekly: true,
                        },
                        quota_alert_percent: 0,
                        start_with_windows: false,
                        language: "system".into(),
                    },
                    appearance: crate::settings_model::EditableAppearance {
                        theme: "dark".into(),
                        layout: "default".into(),
                        dark: EditableThemeStyle::from_theme_style(&ThemeStyle::dark_default()),
                        light: EditableThemeStyle::from_theme_style(&ThemeStyle::light_default()),
                    },
                },
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

            let json_edit = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().unwrap().json_edit.to_hwnd()
            };
            set_section(Section::Json);
            assert!(
                IsWindowVisible(json_edit).as_bool(),
                "JSON editor must be visible on the JSON settings page"
            );
            set_section(Section::Preset);
            assert!(
                !IsWindowVisible(json_edit).as_bool(),
                "JSON editor must be hidden immediately after leaving the JSON settings page"
            );

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

            let (_, _, _, show_rgba, show_blur) = editor_layout_snapshot().unwrap();
            assert!(show_rgba);
            assert!(show_blur);

            select_editor(EditorSelection::Blur);
            let (_, _, _, show_rgba, show_blur) = editor_layout_snapshot().unwrap();
            assert!(!show_rgba, "blur selection must clear the lower RGBA editor");
            assert!(show_blur, "blur control must remain visible inline");

            let _ = DestroyWindow(hwnd);

            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            assert!(state.is_none(), "style settings state must be released on destroy");
        }
    }
}
