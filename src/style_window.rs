use std::cell::RefCell;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::io::Write;
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
use windows::Win32::UI::Shell::{
    DefSubclassProc, ExtractIconExW, RemoveWindowSubclass, SetWindowSubclass, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::appearance::{AppearancePreset, DesktopDirection, DisplayPosition};
use crate::fonts;
use crate::localization::LanguageId;
use crate::native_interop::{self, Color, WM_APP};
use crate::settings_model::{parse_jsonc, EditableSettings, EditableThemeStyle};
use crate::style::{
    panel_corner_radius_max, progress_corner_radius_max, tooltip_corner_radius_max,
    StyleColorTarget, ThemeMode, ThemePreset, ThemeStyle, FROSTED_STRENGTH_MAX,
};

// Keep this block well away from updater.rs (WM_APP + 21..23).
pub const WM_STYLE_COLOR_PREVIEW: u32 = WM_APP + 120;
pub const WM_STYLE_BLUR_PREVIEW: u32 = WM_APP + 121;
pub const WM_STYLE_SAVE: u32 = WM_APP + 122;
pub const WM_STYLE_THEME_CHANGE: u32 = WM_APP + 123;
pub const WM_STYLE_LAYOUT_CHANGE: u32 = WM_APP + 124;
pub const WM_STYLE_POSITION_CHANGE: u32 = WM_APP + 125;
pub const WM_STYLE_DESKTOP_DIRECTION_CHANGE: u32 = WM_APP + 139;
pub const WM_STYLE_DESKTOP_AUTO_COLLAPSE_CHANGE: u32 = WM_APP + 145;
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
const JSON_SAVE_MASK_CLASS: &str = "CodexUsageJsonSaveMaskV1";
const JSON_PAGE_SURFACE_CLASS: &str = "CodexUsageJsonPageSurfaceV1";
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
const JSON_SAVE_MASK_ALPHA: u8 = 128;
const RICH_EDIT_CLASS: &str = "RICHEDIT50W";
const EM_SETBKGNDCOLOR_MSG: u32 = WM_USER + 67;
const EM_SETCHARFORMAT_MSG: u32 = WM_USER + 68;
const EM_SETEVENTMASK_MSG: u32 = WM_USER + 69;
const EM_EXGETSEL_MSG: u32 = WM_USER + 52;
const EM_EXSETSEL_MSG: u32 = WM_USER + 55;
const EM_GETLINECOUNT_MSG: u32 = 0x00BA;
const EM_LINESCROLL_MSG: u32 = 0x00B6;
const EM_LINEINDEX_MSG: u32 = 0x00BB;
const EM_GETFIRSTVISIBLELINE_MSG: u32 = 0x00CE;
const EM_GETSCROLLPOS_MSG: u32 = WM_USER + 221;
const EM_SETSCROLLPOS_MSG: u32 = WM_USER + 222;
const SCF_SELECTION_FLAG: usize = 0x0001;
const CFM_COLOR_MASK: u32 = 0x40000000;
const ENM_CHANGE_MASK: isize = 0x00000001;
const DESKTOP_SUBMENU_TIMER_ID: usize = 0x4A59;
// Poll with a finer timer while the short animation is active; throttle paints to ~60Hz.
const DESKTOP_SUBMENU_FRAME_MS: u32 = 8;
const DESKTOP_SUBMENU_MIN_FRAME_MS: u64 = 15;
const DESKTOP_SUBMENU_DURATION_MS: u64 = 220;
const DESKTOP_SUBMENU_CLOSE_DURATION_MS: u64 = 190;
const DESKTOP_SUBMENU_ICON_SIZE: i32 = 18;
const JSON_ACTION_TIMER_ID: usize = 0x4A53;
const JSON_SCROLLBAR_TIMER_ID: usize = 0x4A54;
const JSON_VALIDATION_TIMER_ID: usize = 0x4A55;
const JSON_EDIT_SUBCLASS_ID: usize = 0x4A56;
const JSON_ERROR_SHAKE_TIMER_ID: usize = 0x4A57;
const SETTINGS_EDIT_SUBCLASS_ID: usize = 0x4A58;
const JSON_ACTION_DELAY_MS: u32 = 200;
const JSON_VALIDATION_DELAY_MS: u32 = 90;
const JSON_ERROR_SHAKE_INTERVAL_MS: u32 = 16;
const JSON_ERROR_SHAKE_STEPS: u8 = 10;
const JSON_WHEEL_DELTA: i32 = 120;
const JSON_WHEEL_LINES_PER_NOTCH: i32 = 3;

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
    pub display_position: DisplayPosition,
    pub desktop_direction: DesktopDirection,
    pub desktop_auto_collapse: bool,
    // Runtime orientation, not a persisted appearance setting.
    pub vertical_taskbar_mode: bool,
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
enum WheelNumericTarget {
    Color(usize),
    Blur,
    Corner,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsIcon {
    General,
    Preset,
    Panel,
    Tooltip,
    Text,
    Progress,
    Interaction,
    Json,
    ThemeSystem,
    ThemeDark,
    ThemeLight,
    LayoutDefault,
    LayoutMinimal,
    PresetClassic,
    PresetOcean,
    PresetForest,
    PresetCustom,
    PresetCloud,
    PresetBay,
    PresetWheat,
    JsonReload,
    JsonFormat,
    JsonImport,
    JsonExport,
    JsonApply,
    DirectionHorizontal,
    DirectionVertical,
    ToggleOn,
    ToggleOff,
    PositionTaskbar,
    PositionDesktop,
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
enum JsonSaveFeedback {
    Saving,
    Saved,
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
    Position(DisplayPosition),
    DesktopDirection(DesktopDirection),
    DesktopAutoCollapse(bool),
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

/// Visual-only animation. Persisted desktop settings never depend on this
/// state. Retargeting first samples the in-flight interpolation so rapid
/// Desktop/Taskbar reversals cannot jump or spawn overlapping animations.
#[derive(Clone, Copy, Debug)]
struct DesktopSubmenuAnimation {
    progress: f32,
    from: f32,
    target: f32,
    started: Instant,
    duration_ms: u64,
    last_frame: Option<Instant>,
}
impl DesktopSubmenuAnimation {
    fn initial(desktop: bool) -> Self {
        let progress = if desktop { 1.0 } else { 0.0 };
        Self {
            progress, from: progress, target: progress,
            started: Instant::now(), duration_ms: DESKTOP_SUBMENU_DURATION_MS,
            last_frame: None,
        }
    }
    fn active(&self) -> bool {
        (self.progress - self.target).abs() > 0.0001
    }
    fn frame_due(&mut self, now: Instant) -> bool {
        if !self.active() {
            return false;
        }
        if self.last_frame.is_some_and(|last| {
            now.saturating_duration_since(last) < Duration::from_millis(DESKTOP_SUBMENU_MIN_FRAME_MS)
        }) {
            return false;
        }
        self.last_frame = Some(now);
        true
    }
    fn advance(&mut self, now: Instant) -> bool {
        if !self.active() { return false; }
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32() * 1000.0;
        let t = (elapsed / self.duration_ms.max(1) as f32).clamp(0.0, 1.0);
        // Smoothstep keeps both endpoints at zero velocity, preventing the
        // reveal edge from snapping at the end of an open/close animation.
        let eased = t * t * (3.0 - 2.0 * t);
        self.progress = if t >= 1.0 {
            self.target
        } else {
            self.from + (self.target - self.from) * eased
        };
        true
    }
    fn retarget(&mut self, desktop: bool, now: Instant) -> bool {
        self.advance(now);
        let target = if desktop { 1.0 } else { 0.0 };
        if (self.target - target).abs() < 0.0001 { return false; }
        self.from = self.progress;
        self.target = target;
        self.started = now;
        self.last_frame = None;
        let full_duration_ms = if desktop {
            DESKTOP_SUBMENU_DURATION_MS
        } else {
            DESKTOP_SUBMENU_CLOSE_DURATION_MS
        };
        self.duration_ms = (full_duration_ms as f32
            * (self.target - self.from).abs()).round().max(72.0) as u64;
        true
    }
    fn hit_enabled(&self) -> bool {
        self.target > 0.5 && self.progress >= 0.98
    }
}

// Diagnostics are opt-in: set CODEX_USAGE_ANIMATION_METRICS=1 before launch.
// A short CSV-like summary is appended only after the menu finishes animating.
struct SubmenuFrameMetrics {
    first: Instant,
    last: Instant,
    frames: usize,
    intervals_ms: Vec<f64>,
    total_paint_ms: f64,
    max_paint_ms: f64,
}

thread_local! {
    static SUBMENU_FRAME_METRICS: RefCell<Option<SubmenuFrameMetrics>> =
        const { RefCell::new(None) };
    static PRECISE_ANIMATION_TIMER: RefCell<bool> = const { RefCell::new(false) };
}

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}

fn begin_precise_animation_timer() {
    PRECISE_ANIMATION_TIMER.with(|slot| {
        let mut enabled = slot.borrow_mut();
        if !*enabled {
            // Temporarily request a finer timer resolution; always release
            // it after the animation to avoid affecting idle power usage.
            *enabled = unsafe { timeBeginPeriod(1) == 0 };
        }
    });
}

fn end_precise_animation_timer() {
    PRECISE_ANIMATION_TIMER.with(|slot| {
        let mut enabled = slot.borrow_mut();
        if *enabled {
            unsafe { let _ = timeEndPeriod(1); }
            *enabled = false;
        }
    });
}

fn submenu_p95(values: &[f64]) -> f64 {
    if values.is_empty() { return 0.0; }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() * 95).div_ceil(100)).saturating_sub(1)]
}

fn record_submenu_frame(paint_begin: Instant, paint_end: Instant, animating: bool) {
    if std::env::var_os("CODEX_USAGE_ANIMATION_METRICS").is_none() {
        return;
    }
    SUBMENU_FRAME_METRICS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() && animating {
            *slot = Some(SubmenuFrameMetrics {
                first: paint_end, last: paint_end, frames: 0,
                intervals_ms: Vec::with_capacity(32),
                total_paint_ms: 0.0, max_paint_ms: 0.0,
            });
        }
        let Some(metric) = slot.as_mut() else { return; };
        if metric.frames > 0 {
            metric.intervals_ms.push(paint_end.duration_since(metric.last).as_secs_f64() * 1000.0);
        }
        metric.frames += 1;
        metric.last = paint_end;
        let draw_ms = paint_end.duration_since(paint_begin).as_secs_f64() * 1000.0;
        metric.total_paint_ms += draw_ms;
        metric.max_paint_ms = metric.max_paint_ms.max(draw_ms);
        if !animating {
            let elapsed_ms = metric.last.duration_since(metric.first).as_secs_f64() * 1000.0;
            let fps = if elapsed_ms > 0.0 {
                (metric.frames.saturating_sub(1) as f64) * 1000.0 / elapsed_ms
            } else { 0.0 };
            let mean_paint = metric.total_paint_ms / metric.frames as f64;
            let p95 = submenu_p95(&metric.intervals_ms);
            let gaps = metric.intervals_ms.iter().filter(|v| **v > 25.0).count();
            let line = format!(
                "frames={},fps={:.1},p95_frame_ms={:.1},mean_paint_ms={:.1},max_paint_ms={:.1},gaps_over_25ms={}\n",
                metric.frames, fps, p95, mean_paint, metric.max_paint_ms, gaps,
            );
            let path = std::env::temp_dir().join("codex-usage-animation-metrics.log");
            if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(line.as_bytes());
            }
            *slot = None;
        }
    });
}

/// Logical 96-DPI bounds. The lower foot contains the existing Desktop
/// button; the right well fits strictly within the original 740px card.
#[derive(Clone, Copy, Debug)]
struct DesktopSubmenuLayout {
    well: RECT,
    foot: RECT,
    direction_label: RECT,
    collapse_label: RECT,
    horizontal: RECT,
    vertical: RECT,
    enabled: RECT,
    disabled: RECT,
}
fn desktop_submenu_layout() -> DesktopSubmenuLayout {
    DesktopSubmenuLayout {
        // Keep the L-shaped join anchored to the Desktop button, but narrow
        // the right well and its label column to remove unused horizontal space.
        well: RECT { left: 636, top: 65, right: 894, bottom: 201 },
        // Keep the 4px inset around Desktop without touching Taskbar.
        foot: RECT { left: 418, top: 155, right: 654, bottom: 201 },
        direction_label: RECT { left: 650, top: 88, right: 728, bottom: 122 },
        collapse_label: RECT { left: 650, top: 146, right: 728, bottom: 180 },
        horizontal: RECT { left: 736, top: 88, right: 803, bottom: 122 },
        vertical: RECT { left: 811, top: 88, right: 878, bottom: 122 },
        enabled: RECT { left: 736, top: 146, right: 803, bottom: 180 },
        disabled: RECT { left: 811, top: 146, right: 878, bottom: 180 },
    }
}
fn scaled_rect(hwnd: HWND, bounds: RECT) -> RECT {
    rect(hwnd, bounds.left, bounds.top, bounds.right, bounds.bottom)
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
    desktop_submenu: DesktopSubmenuAnimation,
    numeric_edits: [SendHwnd; 4],
    blur_edit: SendHwnd,
    corner_edit: SendHwnd,
    hex_edits: [SendHwnd; HEX_EDIT_COUNT],
    json_edit: SendHwnd,
    // Opaque sibling covering the complete content pane whenever JSON is active.
    // Preset-page pixels must never be visible around the RichEdit child.
    json_page_surface: SendHwnd,
    json_save_mask: SendHwnd,
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
    json_save_feedback: Option<JsonSaveFeedback>,
    json_error_shake_step: u8,
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
    let width = scale(hwnd, 136);
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
        unsafe {
            if IsWindow(hwnd).as_bool() {
                // Never overwrite draft/JSON editor content when the user
                // merely reopens an already-existing settings window.
                if IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                } else {
                    let _ = ShowWindow(hwnd, SW_SHOW);
                }
                // Raise within the normal-window band, never leave settings
                // permanently TOPMOST. The popup that launched this request
                // has already been dismissed by the deferred command.
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    0, 0, 0, 0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                let _ = SetForegroundWindow(hwnd);
                return;
            }
        }
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
        // The settings page is GDI double-buffered, but RichEdit is a separate
        // child HWND. Compose the entire window tree as one buffered frame so
        // a new editor can never be layered over the previous page's pixels.
        let hwnd = match CreateWindowExW(
            WS_EX_APPWINDOW | WS_EX_COMPOSITED,
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
            let _ = SetWindowSubclass(
                edit,
                Some(settings_edit_subclass_proc),
                SETTINGS_EDIT_SUBCLASS_ID,
                hwnd.0 as usize,
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
        let _ = SetWindowSubclass(
            blur_edit,
            Some(settings_edit_subclass_proc),
            SETTINGS_EDIT_SUBCLASS_ID,
            hwnd.0 as usize,
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
        let _ = SetWindowSubclass(
            corner_edit,
            Some(settings_edit_subclass_proc),
            SETTINGS_EDIT_SUBCLASS_ID,
            hwnd.0 as usize,
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

        // Create the opaque JSON content pane before RichEdit. As a sibling
        // child, it stays BELOW RichEdit in Z order but ABOVE all stale parent
        // page pixels. Painting the entire pane prevents mixed-page frames.
        let json_page_class = native_interop::wide_str(JSON_PAGE_SURFACE_CLASS);
        let page_wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(json_page_surface_wnd_proc),
            hInstance: GetModuleHandleW(PCWSTR::null()).unwrap().into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(json_page_class.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&page_wc);
        let json_page_surface = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR::from_raw(json_page_class.as_ptr()),
            PCWSTR::from_raw(empty.as_ptr()),
            WS_CHILD | WS_CLIPSIBLINGS,
            0, 0, 1, 1,
            hwnd,
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) {
            Ok(page) => page,
            Err(_) => {
                let _ = DestroyWindow(hwnd);
                let _ = DeleteObject(font);
                let _ = DeleteObject(json_font);
                return;
            }
        };

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
                    | WS_CLIPSIBLINGS.0
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
        let _ = SetWindowSubclass(
            json_edit,
            Some(json_edit_subclass_proc),
            JSON_EDIT_SUBCLASS_ID,
            0,
        );
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
                snapshot: snapshot.clone(),
                section: Section::General,
                editor: EditorSelection::Color(StyleColorTarget::PanelBackground),
                dragging_slider: None,
                hovered: None,
                pressed: None,
                tracking_mouse_leave: false,
                desktop_submenu: DesktopSubmenuAnimation::initial(
                    snapshot.display_position == DisplayPosition::Desktop,
                ),
                numeric_edits,
                blur_edit: SendHwnd::from_hwnd(blur_edit),
                corner_edit: SendHwnd::from_hwnd(corner_edit),
                hex_edits,
                json_edit: SendHwnd::from_hwnd(json_edit),
                json_page_surface: SendHwnd::from_hwnd(json_page_surface),
                json_save_mask: SendHwnd::from_hwnd(HWND::default()),
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
                json_save_feedback: None,
                json_error_shake_step: 0,
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

/// Keep the preset gallery in sync when the widget moves between horizontal
/// and vertical taskbars without discarding active style edits.
pub fn set_preview_orientation(vertical: bool) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else { return };
        if s.snapshot.vertical_taskbar_mode == vertical { return; }
        s.snapshot.vertical_taskbar_mode = vertical;
        s.hwnd.to_hwnd()
    };
    unsafe { let _ = InvalidateRect(hwnd, None, false); }
}

pub fn sync(snapshot: StyleWindowSnapshot) {
    let (hwnd, section, json_dirty, json_apply_pending, submenu_animated) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        let changed_position = s.snapshot.display_position != snapshot.display_position;
        let submenu_animated = if changed_position {
            s.desktop_submenu.retarget(
                snapshot.display_position == DisplayPosition::Desktop,
                Instant::now(),
            )
        } else { false };
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
        (
            s.hwnd.to_hwnd(),
            s.section,
            s.json_dirty,
            s.pending_json_action == Some(JsonAction::Apply),
            submenu_animated,
        )
    };
    if submenu_animated {
        begin_precise_animation_timer();
        unsafe { let _ = SetTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID, DESKTOP_SUBMENU_FRAME_MS, None); }
    }
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    layout_settings_children(hwnd);
    if section == Section::Json && !json_apply_pending {
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

fn navigation_text_inset(_section: Section) -> i32 {
    44
}

fn navigation_icon(section: Section) -> SettingsIcon {
    match section {
        Section::General => SettingsIcon::General,
        Section::Preset => SettingsIcon::Preset,
        Section::Panel => SettingsIcon::Panel,
        Section::Tooltip => SettingsIcon::Tooltip,
        Section::Text => SettingsIcon::Text,
        Section::Progress => SettingsIcon::Progress,
        Section::Interaction => SettingsIcon::Interaction,
        Section::Json => SettingsIcon::Json,
    }
}

fn theme_icon(mode: ThemeMode) -> SettingsIcon {
    match mode {
        ThemeMode::System => SettingsIcon::ThemeSystem,
        ThemeMode::Dark => SettingsIcon::ThemeDark,
        ThemeMode::Light => SettingsIcon::ThemeLight,
    }
}

fn layout_icon(preset: AppearancePreset) -> SettingsIcon {
    match preset {
        AppearancePreset::Default => SettingsIcon::LayoutDefault,
        AppearancePreset::Minimal => SettingsIcon::LayoutMinimal,
    }
}

fn preset_icon(preset: ThemePreset, is_dark: bool) -> SettingsIcon {
    match (is_dark, preset) {
        (true, ThemePreset::Classic) => SettingsIcon::PresetClassic,
        (true, ThemePreset::Ocean) => SettingsIcon::PresetOcean,
        (true, ThemePreset::Forest) => SettingsIcon::PresetForest,
        (false, ThemePreset::Classic) => SettingsIcon::PresetCloud,
        (false, ThemePreset::Ocean) => SettingsIcon::PresetBay,
        (false, ThemePreset::Forest) => SettingsIcon::PresetWheat,
    }
}

fn json_action_icon(action: JsonAction) -> SettingsIcon {
    match action {
        JsonAction::Reload => SettingsIcon::JsonReload,
        JsonAction::Format => SettingsIcon::JsonFormat,
        JsonAction::Import => SettingsIcon::JsonImport,
        JsonAction::Export => SettingsIcon::JsonExport,
        JsonAction::Apply => SettingsIcon::JsonApply,
    }
}

// Shared appearance-button grid: all rows use equal widths and 8px gaps.
const APPEARANCE_BUTTON_LEFT: i32 = 314;
const APPEARANCE_BUTTON_WIDTH: i32 = 100;
const APPEARANCE_BUTTON_GAP: i32 = 8;
const APPEARANCE_BUTTON_HEIGHT: i32 = 34;

fn appearance_button_rect(hwnd: HWND, column: i32, top: i32) -> RECT {
    let left = APPEARANCE_BUTTON_LEFT
        + column * (APPEARANCE_BUTTON_WIDTH + APPEARANCE_BUTTON_GAP);
    rect(
        hwnd, left, top,
        left + APPEARANCE_BUTTON_WIDTH, top + APPEARANCE_BUTTON_HEIGHT,
    )
}

fn theme_rect(hwnd: HWND, mode: ThemeMode) -> RECT {
    let column = match mode {
        ThemeMode::System => 0,
        ThemeMode::Dark => 1,
        ThemeMode::Light => 2,
    };
    appearance_button_rect(hwnd, column, 70)
}

fn position_rect(hwnd: HWND, position: DisplayPosition) -> RECT {
    let column = if position == DisplayPosition::Desktop { 1 } else { 0 };
    appearance_button_rect(hwnd, column, 162)
}

fn desktop_direction_rect(hwnd: HWND, direction: DesktopDirection) -> RECT {
    let layout = desktop_submenu_layout();
    scaled_rect(hwnd, if direction == DesktopDirection::Vertical {
        layout.vertical
    } else {
        layout.horizontal
    })
}

// The controls are stable throughout animation; painted displacement is
// disabled for hit testing until the subpanel has almost fully expanded.
fn desktop_auto_collapse_rect(hwnd: HWND, enabled: bool) -> RECT {
    let layout = desktop_submenu_layout();
    scaled_rect(hwnd, if enabled { layout.enabled } else { layout.disabled })
}

fn layout_rect(hwnd: HWND, preset: AppearancePreset) -> RECT {
    let column = match preset {
        AppearancePreset::Default => 0,
        AppearancePreset::Minimal => 1,
    };
    appearance_button_rect(hwnd, column, 116)
}


// Four tall, narrow cards fit the complete 740px settings content band.
const PRESET_GALLERY_LEFT: i32 = 200;
// No duplicate dark/light preset group heading: cards follow the shared appearance options.
const PRESET_CARD_TOP: i32 = 226;
const PRESET_CARD_WIDTH: i32 = 172;
const PRESET_CARD_HEIGHT: i32 = 350;
const PRESET_CARD_MINIMAL_HEIGHT: i32 = 250;
const PRESET_CARD_GAP: i32 = 16;

fn preset_gallery_card_rect(index: i32, minimal: bool) -> RECT {
    let left = PRESET_GALLERY_LEFT + index * (PRESET_CARD_WIDTH + PRESET_CARD_GAP);
    let height = if minimal { PRESET_CARD_MINIMAL_HEIGHT } else { PRESET_CARD_HEIGHT };
    RECT { left, top: PRESET_CARD_TOP, right: left + PRESET_CARD_WIDTH,
        bottom: PRESET_CARD_TOP + height }
}

fn preview_vertical_for_snapshot(snapshot: &StyleWindowSnapshot) -> bool {
    if snapshot.display_position == DisplayPosition::Desktop {
        snapshot.desktop_direction == DesktopDirection::Vertical
    } else {
        snapshot.vertical_taskbar_mode
    }
}

fn preview_layout_flags() -> (bool, bool) {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.as_ref().map(|s| (
        preview_vertical_for_snapshot(&s.snapshot),
        s.snapshot.appearance_preset == AppearancePreset::Minimal,
    )).unwrap_or((false, false))
}

fn preset_card_rect_for_orientation(
    hwnd: HWND, preset: ThemePreset, vertical: bool, minimal: bool,
) -> RECT {
    let index = match preset {
        ThemePreset::Classic => 0,
        ThemePreset::Ocean => 1,
        ThemePreset::Forest => 2,
    };
    if vertical {
        let r = preset_gallery_card_rect(index, minimal);
        rect(hwnd, r.left, r.top, r.right, r.bottom)
    } else {
        // Restore the original three-across horizontal gallery.
        let left = 200 + index * 244;
        rect(hwnd, left, PRESET_CARD_TOP, left + 224, PRESET_CARD_TOP + 184)
    }
}

fn preset_card_rect(hwnd: HWND, preset: ThemePreset) -> RECT {
    let (vertical, minimal) = preview_layout_flags();
    preset_card_rect_for_orientation(hwnd, preset, vertical, minimal)
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

fn custom_preset_card_rect_for_orientation(hwnd: HWND, vertical: bool, minimal: bool) -> RECT {
    if vertical {
        let r = preset_gallery_card_rect(3, minimal);
        rect(hwnd, r.left, r.top, r.right, r.bottom)
    } else {
        // Restore original second-row custom preview for horizontal taskbars.
        rect(hwnd, 200, PRESET_CARD_TOP + 200, 424, PRESET_CARD_TOP + 384)
    }
}
fn custom_preset_card_rect(hwnd: HWND) -> RECT {
    let (vertical, minimal) = preview_layout_flags();
    custom_preset_card_rect_for_orientation(hwnd, vertical, minimal)
}

fn language_button_rect(hwnd: HWND) -> RECT {
    // Chinese / English only: keep the selector compact and right-aligned.
    rect(hwnd, 760, 414, 920, 450)
}

fn language_option_count() -> usize {
    LanguageId::SELECTABLE.len()
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

fn language_popup_rect(hwnd: HWND) -> RECT {
    let first = language_option_rect(hwnd, 0);
    let last = language_option_rect(hwnd, language_option_count() - 1);
    RECT {
        left: first.left,
        top: first.top,
        right: last.right,
        bottom: last.bottom,
    }
}

fn language_code_for_index(index: usize) -> String {
    LanguageId::SELECTABLE
        .get(index)
        .map(|language| language.code().to_string())
        .unwrap_or_else(|| LanguageId::English.code().to_string())
}

fn language_label_for_index(index: usize, ui_language: LanguageId) -> &'static str {
    match LanguageId::SELECTABLE.get(index).copied() {
        Some(LanguageId::SimplifiedChinese) => "中文",
        Some(LanguageId::English) => "English",
        None => ui_language.strings().system_default,
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

fn json_save_mask_rect(hwnd: HWND) -> RECT {
    let outer = json_edit_rect(hwnd);
    RECT {
        left: outer.left + scale(hwnd, 2),
        top: outer.top + scale(hwnd, 2),
        right: outer.right - scale(hwnd, 2),
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

fn json_scroll_line_metrics() -> Option<(HWND, i32, i32, i32)> {
    let (edit, json_font) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.json_edit.to_hwnd(), s.json_font)
    };
    unsafe {
        let total_lines = SendMessageW(edit, EM_GETLINECOUNT_MSG, WPARAM(0), LPARAM(0)).0 as i32;
        let first_visible =
            SendMessageW(edit, EM_GETFIRSTVISIBLELINE_MSG, WPARAM(0), LPARAM(0)).0 as i32;
        let mut client = RECT::default();
        let _ = GetClientRect(edit, &mut client);
        let hdc = GetDC(edit);
        if hdc.0.is_null() {
            return None;
        }
        let old_font = if json_font != 0 {
            Some(SelectObject(hdc, HGDIOBJ(json_font as *mut _)))
        } else {
            None
        };
        let mut metrics = TEXTMETRICW::default();
        let _ = GetTextMetricsW(hdc, &mut metrics);
        if let Some(old_font) = old_font {
            SelectObject(hdc, old_font);
        }
        let _ = ReleaseDC(edit, hdc);
        let line_height = metrics.tmHeight.max(1);
        let visible_lines = ((client.bottom - client.top).max(1) / line_height).max(1);
        Some((edit, total_lines.max(1), visible_lines, first_visible.max(0)))
    }
}

fn json_scroll_thumb_rect(hwnd: HWND) -> Option<RECT> {
    let (_, total_lines, visible_lines, first_visible) = json_scroll_line_metrics()?;
    if visible_lines >= total_lines {
        return None;
    }
    let track = json_scrollbar_track_rect(hwnd);
    let track_h = (track.bottom - track.top).max(1);
    let thumb_h = (track_h * visible_lines / total_lines)
        .max(scale(hwnd, 28))
        .min(track_h);
    let max_first = (total_lines - visible_lines).max(1);
    let available = (track_h - thumb_h).max(0);
    let offset = available * first_visible.clamp(0, max_first) / max_first;
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

fn json_editor_active() -> bool {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state
        .as_ref()
        .is_some_and(|s| s.section == Section::Json && s.pending_discard_action.is_none())
}

fn scroll_json_editor_lines(edit: HWND, panel: HWND, lines: i32) {
    if lines == 0 {
        return;
    }
    unsafe {
        let _ = SendMessageW(edit, EM_LINESCROLL_MSG, WPARAM(0), LPARAM(lines as isize));
        let _ = RedrawWindow(
            edit,
            None,
            HRGN::default(),
            RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
        );
        let _ = InvalidateRect(panel, Some(&json_scrollbar_track_rect(panel)), false);
        let _ = UpdateWindow(panel);
    }
}

unsafe extern "system" fn settings_edit_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    ref_data: usize,
) -> LRESULT {
    if msg == WM_MOUSEWHEEL {
        let owner = HWND(ref_data as *mut _);
        if !owner.0.is_null() {
            return SendMessageW(owner, msg, wparam, lparam);
        }
        return LRESULT(0);
    }
    if msg == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(
            hwnd,
            Some(settings_edit_subclass_proc),
            SETTINGS_EDIT_SUBCLASS_ID,
        );
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

unsafe extern "system" fn json_edit_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    let saving = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state
            .as_ref()
            .is_some_and(|s| s.json_save_feedback == Some(JsonSaveFeedback::Saving))
    };
    if saving && matches!(msg, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
        return LRESULT(0);
    }

    if msg == WM_MOUSEWHEEL {
        let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        if delta != 0 {
            let notches = ((delta.abs() + JSON_WHEEL_DELTA - 1) / JSON_WHEEL_DELTA).max(1);
            let lines = if delta > 0 {
                -JSON_WHEEL_LINES_PER_NOTCH * notches
            } else {
                JSON_WHEEL_LINES_PER_NOTCH * notches
            };
            let panel = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.hwnd.to_hwnd())
            };
            if let Some(panel) = panel {
                scroll_json_editor_lines(hwnd, panel, lines);
            } else {
                let _ = SendMessageW(hwnd, EM_LINESCROLL_MSG, WPARAM(0), LPARAM(lines as isize));
            }
            return LRESULT(0);
        }
    }

    if msg == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(json_edit_subclass_proc), JSON_EDIT_SUBCLASS_ID);
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

fn update_json_scroll_drag(hwnd: HWND, y: i32) {
    let drag_offset = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else { return; };
        s.json_scroll_drag_offset
    };
    let Some((edit, total_lines, visible_lines, first_visible)) = json_scroll_line_metrics() else {
        return;
    };
    let Some(thumb) = json_scroll_thumb_rect(hwnd) else { return; };
    let track = json_scrollbar_track_rect(hwnd);
    let thumb_h = thumb.bottom - thumb.top;
    let available = (track.bottom - track.top - thumb_h).max(1);
    let top = (y - drag_offset).clamp(track.top, track.bottom - thumb_h);
    let max_first = (total_lines - visible_lines).max(1);
    let target_first =
        ((top - track.top) * max_first + available / 2) / available;
    let delta = target_first - first_visible;
    if delta != 0 {
        unsafe {
            let _ = SendMessageW(edit, EM_LINESCROLL_MSG, WPARAM(0), LPARAM(delta as isize));
            let _ = RedrawWindow(
                edit,
                None,
                HRGN::default(),
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );
        }
    }
    unsafe {
        let _ = InvalidateRect(hwnd, Some(&json_scrollbar_track_rect(hwnd)), false);
        let _ = UpdateWindow(hwnd);
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

#[allow(clippy::type_complexity)]
fn editor_layout_snapshot(
) -> Option<([SendHwnd; 4], SendHwnd, SendHwnd, Section, bool, bool, bool)> {
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = state.as_ref()?;
    Some((
        s.numeric_edits,
        s.blur_edit,
        s.corner_edit,
        s.section,
        matches!(
            s.section,
            Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
        ) && matches!(s.editor, EditorSelection::Color(_)),
        matches!(s.section, Section::Panel | Section::Tooltip),
        matches!(s.section, Section::Panel | Section::Tooltip | Section::Progress),
    ))
}

fn release_editor_focus_before_layout(
    hwnd: HWND,
    numeric_edits: &[SendHwnd; 4],
    blur_edit: SendHwnd,
    corner_edit: SendHwnd,
    show_color: bool,
    show_blur: bool,
    show_corner: bool,
) {
    unsafe {
        let focused = GetFocus();
        let hiding_focused_color =
            !show_color && numeric_edits.iter().any(|edit| edit.to_hwnd() == focused);
        let hiding_focused_blur = !show_blur && blur_edit.to_hwnd() == focused;
        let hiding_focused_corner = !show_corner && corner_edit.to_hwnd() == focused;

        if hiding_focused_color || hiding_focused_blur || hiding_focused_corner {
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
    let Some((numeric_edits, blur_edit, corner_edit, section, show_color, show_blur, show_corner)) =
        editor_layout_snapshot()
    else {
        return;
    };

    release_editor_focus_before_layout(
        hwnd,
        &numeric_edits,
        blur_edit,
        corner_edit,
        show_color,
        show_blur,
        show_corner,
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
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
            );
            let _ = ShowWindow(
                edit.to_hwnd(),
                if show_color { SW_SHOW } else { SW_HIDE },
            );
        }

        let blur_rect = blur_edit_rect(hwnd, section);
        let _ = SetWindowPos(
            blur_edit.to_hwnd(),
            HWND::default(),
            blur_rect.left,
            blur_rect.top,
            blur_rect.right - blur_rect.left,
            blur_rect.bottom - blur_rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        );
        let _ = ShowWindow(
            blur_edit.to_hwnd(),
            if show_blur { SW_SHOW } else { SW_HIDE },
        );

        let corner_rect = corner_edit_rect(hwnd);
        let _ = SetWindowPos(
            corner_edit.to_hwnd(),
            HWND::default(),
            corner_rect.left,
            corner_rect.top,
            corner_rect.right - corner_rect.left,
            corner_rect.bottom - corner_rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        );
        let _ = ShowWindow(
            corner_edit.to_hwnd(),
            if show_corner { SW_SHOW } else { SW_HIDE },
        );
    }
}


// JSON is rendered in a full opaque child content pane rather than directly
// over the previous appearance page. The editor is a sibling *above* this pane,
// so its background cannot accidentally reveal old preset/gallery pixels.
unsafe extern "system" fn json_page_surface_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        // Clicking JSON toolbar controls must still reach the parent hit-test.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_PAINT => {
            paint_json_page_surface(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn paint_json_page_surface(page: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let screen = BeginPaint(page, &mut ps);
    let mut client = RECT::default();
    let _ = GetClientRect(page, &mut client);
    let width = (client.right - client.left).max(1);
    let height = (client.bottom - client.top).max(1);
    let dc = CreateCompatibleDC(screen);
    if dc.0.is_null() {
        let _ = EndPaint(page, &ps);
        return;
    }
    let bitmap = CreateCompatibleBitmap(screen, width, height);
    if bitmap.0.is_null() {
        let _ = DeleteDC(dc);
        let _ = EndPaint(page, &ps);
        return;
    }
    let previous_bitmap = SelectObject(dc, HGDIOBJ(bitmap.0));
    let state_snapshot = {
        let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|s| (
            s.hwnd.to_hwnd(), s.snapshot.clone(),
            s.json_status.clone(), s.json_status_path.clone(),
            s.json_save_feedback, s.json_error_shake_step, s.json_dirty,
            s.hovered, s.pressed, s.font,
        ))
    };
    if let Some((
        owner, snapshot, status, status_path, feedback, shake_step,
        dirty, hovered, pressed, font,
    )) = state_snapshot {
        let left = scale(owner, 180);
        // Reuse JSON painting geometry, which is expressed in the parent's
        // client coordinates. Only the viewport origin changes here.
        let _ = SetViewportOrgEx(dc, -left, 0, None);
        let background = if snapshot.is_dark {
            Color::from_hex("#1F2125FF")
        } else {
            Color::from_hex("#E9EEF4FF")
        };
        fill(dc, RECT { left, top: 0, right: left + width, bottom: height }, background);
        let old_font = SelectObject(dc, HGDIOBJ(font as *mut _));
        let _ = SetBkMode(dc, TRANSPARENT);
        let primary = if snapshot.is_dark {
            Color::from_hex("#F2F3F5FF")
        } else {
            Color::from_hex("#202124FF")
        };
        let secondary = if snapshot.is_dark {
            Color::from_hex("#A8ADB5FF")
        } else {
            Color::from_hex("#666B73FF")
        };
        let card = if snapshot.is_dark {
            Color::from_hex("#292C31FF")
        } else {
            Color::from_hex("#FFFFFFFF")
        };
        let card_hover = if snapshot.is_dark {
            Color::from_hex("#343840FF")
        } else {
            Color::from_hex("#DCE5EFFF")
        };
        let card_pressed = if snapshot.is_dark {
            Color::from_hex("#414751FF")
        } else {
            Color::from_hex("#CBD7E4FF")
        };
        paint_json_page(
            dc, owner, &snapshot, &status, status_path.as_deref(),
            feedback, shake_step, dirty, hovered, pressed,
            card, card_hover, card_pressed,
            Color::from_hex("#4C8DFFFF"),
            Color::from_hex("#629CFFFF"),
            Color::from_hex("#3678E6FF"),
            primary, secondary,
        );
        SelectObject(dc, old_font);
        let _ = SetViewportOrgEx(dc, 0, 0, None);
    }
    let _ = BitBlt(screen, 0, 0, width, height, dc, 0, 0, SRCCOPY);
    SelectObject(dc, previous_bitmap);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(dc);
    let _ = EndPaint(page, &ps);
}

unsafe extern "system" fn json_save_mask_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        // The save mask is a real interaction barrier for the JSON editor.
        // Keep it hit-testable so mouse clicks cannot pass through to the
        // RichEdit or the parent's custom scrollbar.
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_MOUSEWHEEL
        | WM_MOUSEHWHEEL
        | WM_LBUTTONDOWN
        | WM_LBUTTONUP
        | WM_LBUTTONDBLCLK
        | WM_RBUTTONDOWN
        | WM_RBUTTONUP
        | WM_RBUTTONDBLCLK
        | WM_MBUTTONDOWN
        | WM_MBUTTONUP
        | WM_MBUTTONDBLCLK
        | WM_XBUTTONDOWN
        | WM_XBUTTONUP
        | WM_XBUTTONDBLCLK => LRESULT(0),
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(None, IDC_ARROW).unwrap_or_default());
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let is_dark = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.snapshot.is_dark).unwrap_or(false)
            };
            fill(
                hdc,
                client,
                if is_dark {
                    Color::from_hex("#0F141AFF")
                } else {
                    Color::from_hex("#AEB8C4FF")
                },
            );
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn ensure_json_save_mask(owner: HWND) -> Option<HWND> {
    let existing = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.json_save_mask.to_hwnd())
    }?;
    if !existing.0.is_null() {
        return Some(existing);
    }

    unsafe {
        let class_name = native_interop::wide_str(JSON_SAVE_MASK_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(json_save_mask_wnd_proc),
            hInstance: GetModuleHandleW(PCWSTR::null()).unwrap().into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);

        let empty = native_interop::wide_str("");
        let mask = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(empty.as_ptr()),
            WS_POPUP,
            0,
            0,
            1,
            1,
            owner,
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ).ok()?;

        let _ = SetLayeredWindowAttributes(
            mask,
            COLORREF(0),
            JSON_SAVE_MASK_ALPHA,
            LWA_ALPHA,
        );

        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.json_save_mask = SendHwnd::from_hwnd(mask);
        }
        Some(mask)
    }
}

fn json_editor_should_be_visible(section: Section, discard_pending: bool) -> bool {
    section == Section::Json && !discard_pending
}

fn layout_settings_children(hwnd: HWND) {
    let Some((json_edit, json_page_surface, json_save_mask, section, discard_pending, save_mask_visible)) = ({
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| {
            (
                s.json_edit,
                s.json_page_surface,
                s.json_save_mask,
                s.section,
                s.pending_discard_action.is_some(),
                s.section == Section::Json
                    && s.pending_discard_action.is_none()
                    && s.json_save_feedback == Some(JsonSaveFeedback::Saving),
            )
        })
    }) else {
        return;
    };

    unsafe {
        let focused = GetFocus();
        let visible = json_editor_should_be_visible(section, discard_pending);
        if !visible && focused == json_edit.to_hwnd() {
            let _ = SetFocus(hwnd);
        }

        // Reveal the full opaque JSON pane before revealing RichEdit. Even if
        // the parent still displays a cached preset frame, the entire right
        // content region is now covered by the correctly painted JSON page.
        let page = json_page_surface.to_hwnd();
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let page_left = scale(hwnd, 180);
        if !visible {
            let _ = ShowWindow(json_edit.to_hwnd(), SW_HIDE);
        }
        let _ = SetWindowPos(
            page,
            HWND::default(),
            page_left, 0,
            (client.right - page_left).max(1),
            client.bottom.max(1),
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS
                | if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW },
        );
        if visible {
            let _ = RedrawWindow(
                page, None, HRGN::default(),
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );
        }

        let json_rect = json_edit_text_rect(hwnd);
        let visibility = if visible { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        // Both windows are siblings. WS_EX_COMPOSITED uses their *actual*
        // Z-order, not the order in which SetWindowPos was called. The opaque
        // JSON page must stay BEHIND RichEdit or its text is fully covered.
        // SWP_NOZORDER here left the pane in front on some Windows systems.
        let _ = SetWindowPos(
            json_edit.to_hwnd(),
            HWND_TOP,
            json_rect.left,
            json_rect.top,
            (json_rect.right - json_rect.left).max(1),
            (json_rect.bottom - json_rect.top).max(1),
            SWP_NOACTIVATE | SWP_NOCOPYBITS | visibility,
        );

        let mask_hwnd = json_save_mask.to_hwnd();
        if !mask_hwnd.0.is_null() {
            if save_mask_visible {
                let mask_rect = json_save_mask_rect(hwnd);
                let mut mask_origin = POINT {
                    x: mask_rect.left,
                    y: mask_rect.top,
                };
                let _ = ClientToScreen(hwnd, &mut mask_origin);
                let _ = SetWindowPos(
                    mask_hwnd,
                    HWND_TOP,
                    mask_origin.x,
                    mask_origin.y,
                    (mask_rect.right - mask_rect.left).max(1),
                    (mask_rect.bottom - mask_rect.top).max(1),
                    SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_SHOWWINDOW,
                );
                // The saving state only lasts 200 ms. Paint the layered popup
                // synchronously so the mask is visible for that full interval.
                let _ = InvalidateRect(mask_hwnd, None, true);
                let _ = UpdateWindow(mask_hwnd);
            } else {
                let _ = ShowWindow(mask_hwnd, SW_HIDE);
            }
        }

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
        s.json_save_feedback = None;
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

#[derive(Clone, Copy)]
enum JsonErrorPhrase {
    Heading,
    ControlCharacter,
    UnknownSetting,
    MissingSetting,
    DuplicateSetting,
    InvalidColor,
    FrostedRange,
    CornerRange,
    UsageDisplay,
    QuotaAlert,
    RefreshInterval,
    SchemaVersion,
    UnsupportedLanguage,
    Theme,
    Layout,
    InvalidType,
    UnterminatedComment,
    GenericSyntax,
}

fn json_error_phrase(language: LanguageId, phrase: JsonErrorPhrase) -> &'static str {
    use JsonErrorPhrase::*;
    match language {
        LanguageId::English => match phrase {
            Heading => "Configuration has errors",
            ControlCharacter => "A string contains an invalid control character. Check quotes, line breaks, and escapes.",
            UnknownSetting => "Unknown setting. Check the property name.",
            MissingSetting => "A required setting is missing.",
            DuplicateSetting => "A setting is defined more than once.",
            InvalidColor => "Invalid color format. Use #RRGGBB or #RRGGBBAA.",
            FrostedRange => "Value is out of range. Allowed range: 0–100.",
            CornerRange => "Value is out of range. Allowed range: 0–24.",
            UsageDisplay => "`session_5h` and `weekly` cannot both be false.",
            QuotaAlert => "Invalid value. Allowed values: 0, 10, 20, 30.",
            RefreshInterval => "Invalid value. Allowed values: 1m, 5m, 15m, 1h.",
            SchemaVersion => "Unsupported value.",
            UnsupportedLanguage => "Unsupported value.",
            Theme => "Invalid value. Allowed values: system, dark, light.",
            Layout => "Invalid value. Allowed values: default, minimal.",
            InvalidType => "A configuration value has the wrong type.",
            UnterminatedComment => "A block comment is not closed.",
            GenericSyntax => "Invalid JSON syntax or configuration value near this position.",
        },
        LanguageId::SimplifiedChinese => match phrase {
            Heading => "配置存在错误",
            ControlCharacter => "字符串中包含非法控制字符。请检查引号、换行或转义字符。",
            UnknownSetting => "存在未知配置项。请检查属性名是否拼写正确。",
            MissingSetting => "缺少必填配置项。",
            DuplicateSetting => "同一配置项被重复定义。",
            InvalidColor => "颜色格式无效。请使用 #RRGGBB 或 #RRGGBBAA。",
            FrostedRange => "值超出范围。允许范围：0–100。",
            CornerRange => "值超出范围。允许范围：0–24。",
            UsageDisplay => "`session_5h` 和 `weekly` 不能同时为 false。",
            QuotaAlert => "值无效。允许：0、10、20、30。",
            RefreshInterval => "值无效。允许：1m、5m、15m、1h。",
            SchemaVersion => "值不受支持。",
            UnsupportedLanguage => "值不受支持。",
            Theme => "值无效。允许：system、dark、light。",
            Layout => "值无效。允许：default、minimal。",
            InvalidType => "配置值类型不正确。",
            UnterminatedComment => "块注释未正确结束。",
            GenericSyntax => "此位置附近的 JSON 语法或配置值无效。",
        },
    }
}

fn json_error_location_text(
    language: LanguageId, line: usize, column: usize, detail: &str,
) -> String {
    match language {
        LanguageId::English => format!("Line {line}, column {column}: {detail}"),
        LanguageId::SimplifiedChinese => format!("第 {line} 行，第 {column} 列：{detail}"),
    }
}

fn json_error_status(language: LanguageId, detail: &str) -> String {
    let heading = json_error_phrase(language, JsonErrorPhrase::Heading);
    match language {
        LanguageId::SimplifiedChinese => format!("× {heading}：{detail}"),
        _ => format!("× {heading}: {detail}"),
    }
}

fn json_error_key(error: &str) -> Option<String> {
    // Business validation errors use the exact dotted JSON path before the
    // first colon, e.g. "general.refresh_interval: allowed values ...".
    // Keep that path verbatim in localized UI so users can find the setting.
    let without_location = if let Some((_, detail)) = error.split_once(": ") {
        if error.starts_with("line ") {
            detail
        } else {
            error
        }
    } else {
        error
    };

    if let Some((candidate, _)) = without_location.split_once(':') {
        let candidate = candidate.trim();
        if !candidate.is_empty()
            && candidate
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
        {
            return candidate
                .rsplit('.')
                .next()
                .map(str::to_string);
        }
    }

    // serde field errors quote the actual JSON key with backticks.
    for marker in ["unknown field `", "missing field `", "duplicate field `"] {
        if let Some(rest) = error.split_once(marker).map(|(_, rest)| rest) {
            if let Some((key, _)) = rest.split_once('`') {
                if !key.is_empty() {
                    return Some(key.to_string());
                }
            }
        }
    }
    None
}

fn localized_corner_range_detail(error: &str, language: LanguageId) -> Option<String> {
    let marker = "corner_radius: allowed range is 0-";
    let tail = error.split(marker).nth(1)?;
    let max: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if max.is_empty() {
        return None;
    }
    Some(if language == LanguageId::SimplifiedChinese {
        format!("值超出范围。允许范围：0–{max}。")
    } else {
        format!("Value is out of range. Allowed range: 0–{max}.")
    })
}

fn friendly_json_error(error: &str, language: LanguageId) -> String {
    let location = json_error_location(error);
    let full = error;
    let phrase = if full.contains("control character") {
        JsonErrorPhrase::ControlCharacter
    } else if full.contains("unknown field") {
        JsonErrorPhrase::UnknownSetting
    } else if full.contains("missing field") {
        JsonErrorPhrase::MissingSetting
    } else if full.contains("duplicate field") {
        JsonErrorPhrase::DuplicateSetting
    } else if full.contains("expected #RRGGBB or #RRGGBBAA")
        || full.contains("invalid color")
    {
        JsonErrorPhrase::InvalidColor
    } else if full.contains("frosted_strength: allowed range is 0-100") {
        JsonErrorPhrase::FrostedRange
    } else if full.contains("corner_radius: allowed range is") {
        JsonErrorPhrase::CornerRange
    } else if full.contains("session_5h and weekly cannot both be false") {
        JsonErrorPhrase::UsageDisplay
    } else if full.contains("quota_alert_percent: allowed values") {
        JsonErrorPhrase::QuotaAlert
    } else if full.contains("refresh_interval: allowed values") {
        JsonErrorPhrase::RefreshInterval
    } else if full.contains("schema_version: expected") {
        JsonErrorPhrase::SchemaVersion
    } else if full.contains("general.language: unsupported language code") {
        JsonErrorPhrase::UnsupportedLanguage
    } else if full.contains("appearance.theme: allowed values") {
        JsonErrorPhrase::Theme
    } else if full.contains("appearance.layout: allowed values") {
        JsonErrorPhrase::Layout
    } else if full.contains("invalid type") {
        JsonErrorPhrase::InvalidType
    } else if full.contains("unterminated block comment") {
        JsonErrorPhrase::UnterminatedComment
    } else {
        JsonErrorPhrase::GenericSyntax
    };

    let localized = if matches!(phrase, JsonErrorPhrase::CornerRange) {
        localized_corner_range_detail(error, language)
            .unwrap_or_else(|| json_error_phrase(language, phrase).to_string())
    } else {
        json_error_phrase(language, phrase).to_string()
    };
    let detail = if let Some(key) = json_error_key(error) {
        match language {
            LanguageId::SimplifiedChinese => format!("`{key}`：{localized}"),
            _ => format!("`{key}`: {localized}"),
        }
    } else {
        localized
    };

    if let Some((line, column)) = location {
        json_error_location_text(language, line, column, &detail)
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

fn start_json_error_shake(hwnd: HWND) {
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_error_shake_step = 1;
    }
    unsafe {
        let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        let _ = SetTimer(
            hwnd,
            JSON_ERROR_SHAKE_TIMER_ID,
            JSON_ERROR_SHAKE_INTERVAL_MS,
            None,
        );
        let _ = InvalidateRect(hwnd, None, false);
        let _ = UpdateWindow(hwnd);
    }
}

fn advance_json_error_shake(hwnd: HWND) {
    let finished = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.json_error_shake_step == 0 {
            true
        } else if s.json_error_shake_step >= JSON_ERROR_SHAKE_STEPS {
            s.json_error_shake_step = 0;
            true
        } else {
            s.json_error_shake_step += 1;
            false
        }
    };
    unsafe {
        if finished {
            let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        }
        let _ = InvalidateRect(hwnd, None, false);
        let _ = UpdateWindow(hwnd);
    }
}

fn json_error_shake_offset(hwnd: HWND, step: u8) -> i32 {
    if step == 0 || JSON_ERROR_SHAKE_STEPS <= 1 {
        return 0;
    }
    let progress = f32::from(step.saturating_sub(1))
        / f32::from(JSON_ERROR_SHAKE_STEPS.saturating_sub(1));
    let amplitude = scale(hwnd, 4) as f32 * (1.0 - progress);
    let phase = progress * std::f32::consts::PI * 4.0;
    (amplitude * phase.sin()).round() as i32
}

fn show_json_apply_validation_error(
    hwnd: HWND,
    edit: HWND,
    language: LanguageId,
    error: &str,
) {
    let detail = friendly_json_error(error, language);
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_save_feedback = None;
        s.json_status_path = None;
        s.json_status = json_error_status(language, &detail);
    }
    locate_json_error(edit, error);
    start_json_error_shake(hwnd);
}

fn begin_json_apply(hwnd: HWND) {
    let (edit, language, already_running) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.json_edit.to_hwnd(),
            s.snapshot.language,
            s.pending_json_action.is_some(),
        )
    };
    if already_running {
        return;
    }

    let raw = read_large_edit_text(edit);
    match parse_jsonc(&raw) {
        Ok(_) => begin_json_action(hwnd, JsonAction::Apply),
        Err(error) => show_json_apply_validation_error(hwnd, edit, language, &error),
    }
}

fn begin_json_action(hwnd: HWND, action: JsonAction) {
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.pending_json_action.is_some() {
            return;
        }
        s.pending_json_action = Some(action);
        if action == JsonAction::Apply {
            s.json_save_feedback = Some(JsonSaveFeedback::Saving);
            s.json_error_shake_step = 0;
        } else {
            s.json_save_feedback = None;
            s.json_status = json_action_running_text(action, s.snapshot.language).to_string();
            s.json_status_path = None;
        }
    }
    if action == JsonAction::Apply {
        unsafe {
            let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        }
        let _ = ensure_json_save_mask(hwnd);
        layout_settings_children(hwnd);
    }
    unsafe {
        let _ = SetTimer(hwnd, JSON_ACTION_TIMER_ID, JSON_ACTION_DELAY_MS, None);
        let _ = InvalidateRect(hwnd, None, false);
        let _ = UpdateWindow(hwnd);
    }
}

fn finish_pending_json_action(hwnd: HWND) {
    let action = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().and_then(|s| s.pending_json_action)
    };
    let Some(action) = action else {
        return;
    };
    unsafe {
        let _ = KillTimer(hwnd, JSON_ACTION_TIMER_ID);
    }

    if action == JsonAction::Apply {
        // Keep Apply marked pending while the synchronous parent handler updates
        // application state and calls style_window::sync(). That sync must update
        // the snapshot without touching/reloading the JSON RichEdit contents.
        apply_json_editor();
        {
            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(s) = state.as_mut() {
                if s.pending_json_action == Some(JsonAction::Apply) {
                    s.pending_json_action = None;
                }
            }
        }
        layout_settings_children(hwnd);
        redraw_settings_window(hwnd);
        return;
    }

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.pending_json_action = None;
        }
    }
    match action {
        JsonAction::Reload => reload_json_editor_from_snapshot(),
        JsonAction::Format => format_json_editor(),
        JsonAction::Import => import_json_file(hwnd),
        JsonAction::Export => export_json_file(hwnd),
        JsonAction::Apply => unreachable!(),
    }
}

fn json_action_error(action_zh: &str, action_en: &str, error: &str, language: LanguageId) -> String {
    let detail = friendly_json_error(error, language);
    match language {
        LanguageId::SimplifiedChinese => format!("× {action_zh}：{detail}"),
        LanguageId::English => format!("× {action_en}: {detail}"),
    }
}

fn schedule_json_validation(hwnd: HWND) {
    let should_schedule = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.syncing_json_edit {
            false
        } else {
            // Any user edit after a successful save replaces the saved indicator
            // with the normal unsaved-changes state immediately.
            s.json_save_feedback = None;
            s.json_error_shake_step = 0;
            // Mark dirty immediately so closing the window before the debounce
            // fires still triggers the unsaved-changes confirmation.
            s.json_dirty = true;
            true
        }
    };
    if !should_schedule {
        return;
    }

    unsafe {
        let _ = KillTimer(hwnd, JSON_VALIDATION_TIMER_ID);
        let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        let _ = SetTimer(
            hwnd,
            JSON_VALIDATION_TIMER_ID,
            JSON_VALIDATION_DELAY_MS,
            None,
        );
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn update_json_validation_status(_mark_dirty: bool) {
    let (edit, syncing, language, hwnd, applied_settings) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.json_edit.to_hwnd(),
            s.syncing_json_edit,
            s.snapshot.language,
            s.hwnd.to_hwnd(),
            s.snapshot.editable_settings.clone(),
        )
    };
    if syncing {
        return;
    }

    // Keep live validation read-only with respect to the RichEdit control.
    // Reformatting character ranges while handling a user's edit can re-enter
    // RichEdit internals and stall the process when an invalid document becomes
    // valid. Full syntax highlighting is still applied on reload/format/import
    // and when the editor theme is refreshed.
    let text = normalize_to_lf(&read_large_edit_text_raw(edit));
    let parsed = parse_jsonc(&text);
    let dirty = match &parsed {
        Ok(settings) => settings != &applied_settings,
        Err(_) => true,
    };

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.json_dirty = dirty;
            match parsed {
                Err(error) => {
                    let detail = friendly_json_error(&error, language);
                    s.json_status = json_error_status(language, &detail);
                    s.json_status_path = None;
                }
                Ok(_) => {
                    if s.json_status.starts_with('×') {
                        s.json_status.clear();
                        s.json_status_path = None;
                    }
                }
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
    let (edit, language, hwnd) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.json_edit.to_hwnd(), s.snapshot.language, s.hwnd.to_hwnd())
    };
    let raw = read_large_edit_text(edit);
    let settings = match parse_jsonc(&raw) {
        Ok(value) => value,
        Err(error) => {
            show_json_apply_validation_error(hwnd, edit, language, &error);
            return;
        }
    };
    {
        let mut pending = PENDING_EDITABLE_SETTINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *pending = Some(settings);
    }

    // WM_SETTINGS_JSON_APPLY is handled synchronously by the parent. Keep the
    // editor dirty until it returns so the parent's style_window::sync() cannot
    // reload the pre-apply snapshot into the RichEdit control.
    send_parent(WM_SETTINGS_JSON_APPLY, 0, 0);

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.json_dirty = false;
            s.json_save_feedback = Some(JsonSaveFeedback::Saved);
            if s.json_status.starts_with('×') {
                s.json_status.clear();
                s.json_status_path = None;
            }
        }
    }

    // The parent sync has already updated the snapshot/theme. Refresh syntax
    // colors in place, without replacing the user's JSON text or caret.
    refresh_json_editor_theme();
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
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
                    SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
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

fn corner_radius_max_for_section(
    preset: AppearancePreset,
    section: Section,
) -> Option<u8> {
    match section {
        Section::Panel => Some(panel_corner_radius_max(preset)),
        Section::Tooltip => Some(tooltip_corner_radius_max()),
        Section::Progress => Some(progress_corner_radius_max(preset)),
        _ => None,
    }
}

fn update_corner_from_numeric_edit() {
    let (edit, syncing, section, radius_max) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        let Some(radius_max) =
            corner_radius_max_for_section(s.snapshot.appearance_preset, s.section)
        else {
            return;
        };
        (
            s.corner_edit.to_hwnd(),
            s.syncing_corner_edit,
            s.section,
            radius_max,
        )
    };
    if syncing {
        return;
    }

    let Some(raw_value) = read_edit_value(edit) else {
        return;
    };
    let value = raw_value.min(u16::from(radius_max)) as u8;

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

    if raw_value > u16::from(radius_max) {
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
        for position in [DisplayPosition::Taskbar, DisplayPosition::Desktop] {
            if pt_in_rect(position_rect(hwnd, position), x, y) {
                return Some(HitTarget::Position(position));
            }
        }
        if STATE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| {
            s.snapshot.display_position == DisplayPosition::Desktop
                && s.desktop_submenu.hit_enabled()
        }) {
            for enabled in [true, false] {
                if pt_in_rect(desktop_auto_collapse_rect(hwnd, enabled), x, y) {
                    return Some(HitTarget::DesktopAutoCollapse(enabled));
                }
            }
            for direction in [DesktopDirection::Horizontal, DesktopDirection::Vertical] {
                if pt_in_rect(desktop_direction_rect(hwnd, direction), x, y) {
                    return Some(HitTarget::DesktopDirection(direction));
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
                if preset == AppearancePreset::Minimal { 1 } else { 0 },
                0,
            );
        }
        HitTarget::Position(position) => {
            send_parent(
                WM_STYLE_POSITION_CHANGE,
                if position == DisplayPosition::Desktop { 1 } else { 0 },
                0,
            );
        }
        HitTarget::DesktopDirection(direction) => {
            send_parent(
                WM_STYLE_DESKTOP_DIRECTION_CHANGE,
                if direction == DesktopDirection::Vertical { 1 } else { 0 }, 0,
            );
        }
        HitTarget::DesktopAutoCollapse(enabled) => {
            send_parent(WM_STYLE_DESKTOP_AUTO_COLLAPSE_CHANGE, usize::from(enabled), 0);
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
                    begin_json_apply(hwnd);
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
    if discard_pending || current_section == section {
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
            s.focused_corner_edit = false;
            s.focused_hex_edit = None;
        }
        s.dragging_slider = None;
        s.pressed = None;
        s.hovered = None;
        (s.hwnd.to_hwnd(), s.json_dirty)
    };
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
    // Prepare RichEdit's position/content while the parent has not yet
    // committed a new composited frame. The window-level WS_EX_COMPOSITED
    // buffers these child and parent updates together.
    layout_settings_children(hwnd);
    if section == Section::Json {
        if json_dirty {
            refresh_json_editor_theme();
        } else {
            reload_json_editor_from_snapshot();
        }
    }
    sync_hex_edits();
    sync_numeric_edits();
    sync_blur_edit();
    sync_corner_edit();
    // Commit the new parent page and all visible child controls as one
    // composited frame. There is no timer-based partial display state.
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
    sync_corner_edit();
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

    if matches!(section, Section::Panel | Section::Tooltip) && pt_in_rect(blur_slider_hit_rect(hwnd, section), x, y) {
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

fn wheel_numeric_target_at(
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Option<WheelNumericTarget> {
    let (section, editor) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.section, s.editor)
    };

    if matches!(
        section,
        Section::Panel | Section::Tooltip | Section::Text | Section::Progress | Section::Interaction
    ) && matches!(editor, EditorSelection::Color(_))
    {
        for index in 0..4 {
            if pt_in_rect(numeric_edit_frame_rect(hwnd, section, index), x, y) {
                return Some(WheelNumericTarget::Color(index));
            }
        }
    }

    if matches!(section, Section::Panel | Section::Tooltip)
        && pt_in_rect(blur_edit_frame_rect(hwnd, section), x, y)
    {
        return Some(WheelNumericTarget::Blur);
    }

    if matches!(section, Section::Panel | Section::Tooltip | Section::Progress)
        && pt_in_rect(corner_edit_frame_rect(hwnd), x, y)
    {
        return Some(WheelNumericTarget::Corner);
    }

    None
}

fn adjust_numeric_by_wheel(target: WheelNumericTarget, direction: i32) {
    if direction == 0 {
        return;
    }
    let (edit, current, max) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        match target {
            WheelNumericTarget::Color(index) => {
                let EditorSelection::Color(color_target) = s.editor else {
                    return;
                };
                let color = s.snapshot.active_style.color(color_target);
                let values = [color.r, color.g, color.b, color.a];
                let Some(current) = values.get(index).copied() else {
                    return;
                };
                (s.numeric_edits[index].to_hwnd(), current, u8::MAX)
            }
            WheelNumericTarget::Blur => {
                let current = match s.section {
                    Section::Panel => s.snapshot.active_style.panel_frosted_strength,
                    Section::Tooltip => s.snapshot.active_style.tooltip_frosted_strength(),
                    _ => return,
                };
                (s.blur_edit.to_hwnd(), current, FROSTED_STRENGTH_MAX)
            }
            WheelNumericTarget::Corner => {
                let Some(max) =
                    corner_radius_max_for_section(s.snapshot.appearance_preset, s.section)
                else {
                    return;
                };
                let current = match s.section {
                    Section::Panel => s.snapshot.active_style.panel_corner_radius,
                    Section::Tooltip => s.snapshot.active_style.tooltip_corner_radius,
                    Section::Progress => s.snapshot.active_style.progress_corner_radius,
                    _ => return,
                };
                (s.corner_edit.to_hwnd(), current, max)
            }
        }
    };

    let next = (i32::from(current) + direction.signum())
        .clamp(0, i32::from(max)) as u8;
    if next != current {
        set_edit_text(edit, next);
    }
}

fn adjust_slider_by_wheel(hwnd: HWND, kind: SliderKind, direction: i32) {
    if direction == 0 {
        return;
    }
    let (current, max, track) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        match kind {
            SliderKind::Blur => {
                let current = match s.section {
                    Section::Panel => s.snapshot.active_style.panel_frosted_strength,
                    Section::Tooltip => s.snapshot.active_style.tooltip_frosted_strength(),
                    _ => return,
                };
                (
                    current,
                    FROSTED_STRENGTH_MAX,
                    blur_slider_track_rect(hwnd, s.section),
                )
            }
            SliderKind::Red | SliderKind::Green | SliderKind::Blue | SliderKind::Alpha => {
                let EditorSelection::Color(target) = s.editor else {
                    return;
                };
                let color = s.snapshot.active_style.color(target);
                let index = match kind {
                    SliderKind::Red => 0,
                    SliderKind::Green => 1,
                    SliderKind::Blue => 2,
                    SliderKind::Alpha => 3,
                    SliderKind::Blur => unreachable!(),
                };
                (
                    [color.r, color.g, color.b, color.a][index],
                    u8::MAX,
                    color_slider_track_rect(hwnd, s.section, index),
                )
            }
        }
    };

    let next = (i32::from(current) + direction.signum())
        .clamp(0, i32::from(max)) as u8;
    if next == current {
        return;
    }
    let width = (track.right - track.left).max(1);
    let denominator = i32::from(max.max(1));
    let x = track.left + (i32::from(next) * width + denominator / 2) / denominator;
    update_slider(hwnd, kind, x);
    send_parent(WM_STYLE_SAVE, 0, 0);
}

fn handle_settings_mouse_wheel(hwnd: HWND, wparam: WPARAM) -> bool {
    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    if delta == 0 {
        return false;
    }

    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
        let _ = ScreenToClient(hwnd, &mut point);
    }

    if let Some(target) = wheel_numeric_target_at(hwnd, point.x, point.y) {
        adjust_numeric_by_wheel(target, delta);
        return true;
    }
    if let Some(kind) = slider_kind_at(hwnd, point.x, point.y) {
        adjust_slider_by_wheel(hwnd, kind, delta);
        return true;
    }
    false
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
                    blur_slider_track_rect(hwnd, s.section),
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
            // JSON toolbar hover, status and custom scrollbar are painted on
            // the opaque child pane, so invalidate it when the parent changes.
            let page = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().and_then(|s|
                    (s.section == Section::Json).then_some(s.json_page_surface.to_hwnd())
                )
            };
            if let Some(page) = page {
                let _ = InvalidateRect(page, None, false);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == DESKTOP_SUBMENU_TIMER_ID {
                let (still_active, rendered) = {
                    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(s) = state.as_mut() {
                        let now = Instant::now();
                        let due = s.desktop_submenu.frame_due(now);
                        if due { s.desktop_submenu.advance(now); }
                        (s.desktop_submenu.active(), due)
                    } else { (false, false) }
                };
                if !still_active {
                    let _ = KillTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID);
                    end_precise_animation_timer();
                }
                if rendered {
                    let _ = InvalidateRect(hwnd, Some(&settings_card_rect(hwnd, 58, 206)), false);
                }
                return LRESULT(0);
            }
            if wparam.0 == JSON_ACTION_TIMER_ID {
                finish_pending_json_action(hwnd);
                return LRESULT(0);
            }
            if wparam.0 == JSON_VALIDATION_TIMER_ID {
                let _ = KillTimer(hwnd, JSON_VALIDATION_TIMER_ID);
                update_json_validation_status(false);
                return LRESULT(0);
            }
            if wparam.0 == JSON_ERROR_SHAKE_TIMER_ID {
                advance_json_error_shake(hwnd);
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
        WM_MOUSEWHEEL => {
            if handle_settings_mouse_wheel(hwnd, wparam) {
                LRESULT(0)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
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
                            || s.corner_edit.to_hwnd() == cursor_hwnd
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

            let json_scroll_hit = json_editor_active()
                && json_scroll_thumb_hit_rect(hwnd)
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

            if json_editor_active() {
                let track = json_scrollbar_track_rect(hwnd);
                if pt_in_rect(track, x, y) {
                    if let (Some(thumb), Some((edit, _, visible_lines, _))) =
                        (json_scroll_thumb_rect(hwnd), json_scroll_line_metrics())
                    {
                        let direction = if y < thumb.top {
                            -1
                        } else if y >= thumb.bottom {
                            1
                        } else {
                            0
                        };
                        if direction != 0 {
                            let page_lines = visible_lines.saturating_sub(1).max(1);
                            let _ = SetFocus(edit);
                            scroll_json_editor_lines(edit, hwnd, direction * page_lines);
                            return LRESULT(0);
                        }
                    }
                }
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
            let scroll_hover = json_editor_active()
                && json_scroll_thumb_rect(hwnd).is_some()
                && pt_in_rect(json_scrollbar_track_rect(hwnd), x, y);
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
                schedule_json_validation(hwnd);
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

            if control_id == ID_EDIT_CORNER {
                match notification {
                    EN_CHANGE_CODE => {
                        update_corner_from_numeric_edit();
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_corner_edit = true;
                            }
                        }
                        select_editor(EditorSelection::CornerRadius);
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_corner_edit = false;
                            }
                        }
                        sync_corner_edit();
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
                        let editor = {
                            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            state
                                .as_ref()
                                .map(|s| {
                                    if s.section == Section::Tooltip {
                                        EditorSelection::TooltipBlur
                                    } else {
                                        EditorSelection::Blur
                                    }
                                })
                                .unwrap_or(EditorSelection::Blur)
                        };
                        select_editor(editor);
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
            end_precise_animation_timer();
            SUBMENU_CARD_FRAMES.with(|cell| *cell.borrow_mut() = None);
            SETTINGS_PAINT_BUFFER.with(|cell| *cell.borrow_mut() = None);
            let _ = KillTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_ACTION_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_SCROLLBAR_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_VALIDATION_TIMER_ID);
            let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
            let resources = {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.take().map(|s| (s.font, s.json_font, s.edit_brush, s.json_save_mask))
            };
            if let Some((font, json_font, edit_brush, json_save_mask)) = resources {
                if font != 0 {
                    let _ = DeleteObject(HGDIOBJ(font as *mut _));
                }
                if json_font != 0 {
                    let _ = DeleteObject(HGDIOBJ(json_font as *mut _));
                }
                if edit_brush != 0 {
                    let _ = DeleteObject(HGDIOBJ(edit_brush as *mut _));
                }
                let mask = json_save_mask.to_hwnd();
                if !mask.0.is_null() {
                    let _ = DestroyWindow(mask);
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
        if zh { "未保存的更改" } else { "Unsaved changes" },
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
            "当前 JSON 配置尚未保存。继续操作将丢失这些更改。"
        } else {
            "The current JSON configuration has not been saved. Continuing will discard these changes."
        },
        RECT {
            left: dialog.left + scale(hwnd, 24),
            top: dialog.top + scale(hwnd, 58),
            right: dialog.right - scale(hwnd, 24),
            bottom: dialog.top + scale(hwnd, 112),
        },
        DT_LEFT | DT_VCENTER | DT_WORDBREAK,
    );

    let discard_normal = if snapshot.is_dark {
        Color::from_hex("#8F5054FF")
    } else {
        Color::from_hex("#B65F63FF")
    };
    let discard_hover = if snapshot.is_dark {
        Color::from_hex("#A25A5EFF")
    } else {
        Color::from_hex("#C56A6EFF")
    };
    let discard_pressed = if snapshot.is_dark {
        Color::from_hex("#7E464AFF")
    } else {
        Color::from_hex("#A95357FF")
    };
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
                normal: discard_normal,
                hover: discard_hover,
                pressed: discard_pressed,
                selected: discard_normal,
                selected_hover: discard_hover,
                selected_pressed: discard_pressed,
            },
        ),
        Color::from_hex("#FFF8F8FF"),
        if zh { "丢弃更改" } else { "Discard changes" },
    );
    draw_segment(
        hdc,
        discard_dialog_button_rect(hwnd, false),
        false,
        button_background(
            HitTarget::KeepEditing,
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
        ),
        primary,
        if zh { "返回编辑" } else { "Return to editing" },
    );
    let _ = background;
}

// Reuse the offscreen DC and bitmap between WM_PAINT frames. Previously
// every 16 ms tick allocated and released a full-window compatible bitmap.
struct SettingsPaintBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl SettingsPaintBuffer {
    unsafe fn create(screen: HDC, width: i32, height: i32) -> Option<Self> {
        let dc = CreateCompatibleDC(screen);
        if dc.0.is_null() {
            return None;
        }
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        if bitmap.0.is_null() {
            let _ = DeleteDC(dc);
            return None;
        }
        let previous_bitmap = SelectObject(dc, HGDIOBJ(bitmap.0));
        if previous_bitmap.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(dc);
            return None;
        }
        Some(Self { dc, bitmap, previous_bitmap, width, height })
    }
}

impl Drop for SettingsPaintBuffer {
    fn drop(&mut self) {
        unsafe {
            let _ = SelectObject(self.dc, self.previous_bitmap);
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

// Two immutable endpoints for the shared appearance card: Taskbar and Desktop.
// Every animation frame cross-fades the rasterized card instead of issuing
// dozens of Win32/Direct2D draw calls and repeated icon/text measurements.
#[derive(Clone, Copy, PartialEq, Eq)]
struct SubmenuCardKey {
    width: i32,
    height: i32,
    dpi: u32,
    dark: bool,
    zh: bool,
    theme: ThemeMode,
    layout: AppearancePreset,
    direction: DesktopDirection,
    collapse: bool,
}

impl SubmenuCardKey {
    fn new(hwnd: HWND, width: i32, height: i32, snapshot: &StyleWindowSnapshot) -> Self {
        Self {
            width, height, dpi: unsafe { GetDpiForWindow(hwnd) },
            dark: snapshot.is_dark,
            zh: snapshot.language == LanguageId::SimplifiedChinese,
            theme: snapshot.theme_mode, layout: snapshot.appearance_preset,
            direction: snapshot.desktop_direction, collapse: snapshot.desktop_auto_collapse,
        }
    }
}

struct SubmenuCardFrames {
    key: SubmenuCardKey,
    closed: SettingsPaintBuffer,
    open: SettingsPaintBuffer,
}

thread_local! {
    static SETTINGS_PAINT_BUFFER: RefCell<Option<SettingsPaintBuffer>> =
        const { RefCell::new(None) };
    static SUBMENU_CARD_FRAMES: RefCell<Option<SubmenuCardFrames>> =
        const { RefCell::new(None) };
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_submenu_endpoint(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    section: Section,
    editor: EditorSelection,
    font: isize,
    card: Color, hover: Color, pressed_color: Color, track: Color,
    accent: Color, accent_hover: Color, accent_pressed: Color,
    primary: Color, secondary: Color,
) {
    let underlay = if snapshot.is_dark {
        Color::from_hex("#1F2125FF")
    } else {
        Color::from_hex("#E9EEF4FF")
    };
    fill(hdc, settings_card_rect(hwnd, 58, 206), underlay);
    let prior_font = SelectObject(hdc, HGDIOBJ(font as *mut _));
    let _ = SetBkMode(hdc, TRANSPARENT);
    // Hover and pressed effects are intentionally excluded from the cached
    // animation; normal painting immediately restores live hover feedback.
    paint_appearance_page(
        hdc, hwnd, snapshot, section, editor,
        if snapshot.display_position == DisplayPosition::Desktop { 1.0 } else { 0.0 },
        true, None, None, None, false, false, None, &[false; HEX_EDIT_COUNT],
        card, hover, pressed_color, track, accent, accent_hover,
        accent_pressed, primary, secondary,
    );
    SelectObject(hdc, prior_font);
}

#[allow(clippy::too_many_arguments)]
unsafe fn composite_cached_submenu_card(
    screen: HDC, dest: HDC, hwnd: HWND, width: i32, height: i32,
    snapshot: &StyleWindowSnapshot, section: Section, editor: EditorSelection,
    font: isize, card: Color, hover: Color, pressed_color: Color,
    track: Color, accent: Color, accent_hover: Color, accent_pressed: Color,
    primary: Color, secondary: Color, progress: f32,
) -> bool {
    let key = SubmenuCardKey::new(hwnd, width, height, snapshot);
    SUBMENU_CARD_FRAMES.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.as_ref().is_none_or(|frames| frames.key != key) {
            let Some(closed) = SettingsPaintBuffer::create(screen, width, height) else {
                return false;
            };
            let Some(open) = SettingsPaintBuffer::create(screen, width, height) else {
                return false;
            };
            let mut closed_snapshot = snapshot.clone();
            closed_snapshot.display_position = DisplayPosition::Taskbar;
            let mut open_snapshot = snapshot.clone();
            open_snapshot.display_position = DisplayPosition::Desktop;
            paint_submenu_endpoint(closed.dc, hwnd, &closed_snapshot, section, editor,
                font, card, hover, pressed_color, track, accent, accent_hover,
                accent_pressed, primary, secondary);
            paint_submenu_endpoint(open.dc, hwnd, &open_snapshot, section, editor,
                font, card, hover, pressed_color, track, accent, accent_hover,
                accent_pressed, primary, secondary);
            *cache = Some(SubmenuCardFrames { key, closed, open });
        }
        let Some(frames) = cache.as_ref() else { return false };
        let card_bounds = settings_card_rect(hwnd, 58, 206);
        let w = card_bounds.right - card_bounds.left;
        let h = card_bounds.bottom - card_bounds.top;
        let copied = BitBlt(dest, card_bounds.left, card_bounds.top, w, h,
            frames.closed.dc, card_bounds.left, card_bounds.top, SRCCOPY);
        if !copied.as_bool() { return false; }
        let opacity = (progress.clamp(0.0, 1.0) * 255.0).round() as u8;
        opacity == 0 || native_interop::alpha_blend_cached_card(
            dest, frames.open.dc, card_bounds, opacity)
    })
}

unsafe fn paint(hwnd: HWND) {
    let paint_begin = Instant::now();
    let mut ps = PAINTSTRUCT::default();
    let screen_hdc = BeginPaint(hwnd, &mut ps);

    let mut paint_client = RECT::default();
    let _ = GetClientRect(hwnd, &mut paint_client);
    let width = (paint_client.right - paint_client.left).max(1);
    let height = (paint_client.bottom - paint_client.top).max(1);

    SETTINGS_PAINT_BUFFER.with(|cell| {
        let mut cached = cell.borrow_mut();
        if cached.as_ref().is_none_or(|backbuffer| {
            backbuffer.width != width || backbuffer.height != height
        }) {
            *cached = SettingsPaintBuffer::create(screen_hdc, width, height);
        }
        let Some(backbuffer) = cached.as_ref() else {
            let _ = EndPaint(hwnd, &ps);
            return;
        };
        let hdc = backbuffer.dc;

    let (
        snapshot,
        section,
        editor,
        hovered,
        pressed,
        focused_numeric_edit,
        focused_blur_edit,
        focused_corner_edit,
        focused_hex_edit,
        invalid_hex_edits,
        json_status,
        json_status_path,
        json_save_feedback,
        json_error_shake_step,
        json_dirty,
        discard_pending,
        font,
        desktop_submenu_progress,
        desktop_submenu_active,
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
            s.focused_corner_edit,
            s.focused_hex_edit,
            s.invalid_hex_edits,
            s.json_status.clone(),
            s.json_status_path.clone(),
            s.json_save_feedback,
            s.json_error_shake_step,
            s.json_dirty,
            s.pending_discard_action.is_some(),
            s.font,
            s.desktop_submenu.progress,
            s.desktop_submenu.active(),
        )
    };
    // WM_TIMER dirties exactly the shared top card. Keep all drawing outside
    // that card intact in the cached buffer, skipping expensive preset previews.
    let dirty = ps.rcPaint;
    let card_bounds = settings_card_rect(hwnd, 58, 206);
    let submenu_only = !matches!(section, Section::General | Section::Json)
        && dirty.left == card_bounds.left && dirty.top == card_bounds.top
        && dirty.right == card_bounds.right && dirty.bottom == card_bounds.bottom;

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

    // Fast path: the animation changes only the shared settings card. Blend
    // two cached complete card states; the first frame builds the snapshots,
    // and all following frames are just BitBlt + AlphaBlend.
    if submenu_only && desktop_submenu_active && !discard_pending &&
        composite_cached_submenu_card(
            screen_hdc, hdc, hwnd, width, height, &snapshot, section, editor,
            font, card, card_hover, card_pressed, track_background,
            accent, accent_hover, accent_pressed, primary, secondary,
            desktop_submenu_progress,
        )
    {
        let rect = ps.rcPaint;
        let _ = BitBlt(screen_hdc, rect.left, rect.top,
            rect.right - rect.left, rect.bottom - rect.top,
            hdc, rect.left, rect.top, SRCCOPY);
        let _ = EndPaint(hwnd, &ps);
        record_submenu_frame(paint_begin, Instant::now(), desktop_submenu_active);
        return;
    }

    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let old_font = SelectObject(hdc, HGDIOBJ(font as *mut _));
    let _ = SetBkMode(hdc, TRANSPARENT);
    if !submenu_only {
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
    }

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
            json_save_feedback,
            json_error_shake_step,
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
            desktop_submenu_progress,
            submenu_only,
            hovered,
            pressed,
            focused_numeric_edit,
            focused_blur_edit,
            focused_corner_edit,
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
            primary,
            secondary,
        );
    }

    SelectObject(hdc, old_font);
    // Only transfer the invalid rectangle, rather than the full window.
    // Full-frame backing pixels remain available for subsequent repaints.
    let dirty = ps.rcPaint;
    if dirty.right > dirty.left && dirty.bottom > dirty.top {
        let _ = BitBlt(screen_hdc, dirty.left, dirty.top,
            dirty.right - dirty.left, dirty.bottom - dirty.top,
            hdc, dirty.left, dirty.top, SRCCOPY);
    }
    let _ = EndPaint(hwnd, &ps);
    record_submenu_frame(paint_begin, Instant::now(), desktop_submenu_active);
    });
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
        let icon_color = if selected { accent } else { secondary };
        let icon_size = scale(hwnd, 20);
        let icon_rect = RECT {
            left: r.left + scale(hwnd, 14),
            top: r.top + ((r.bottom - r.top) - icon_size) / 2,
            right: r.left + scale(hwnd, 14) + icon_size,
            bottom: r.top + ((r.bottom - r.top) - icon_size) / 2 + icon_size,
        };
        draw_settings_icon(
            hdc,
            hwnd,
            navigation_icon(item),
            icon_rect,
            icon_color,
            section_bg,
        );

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
        "5H",
        rect(hwnd, 238, 162, 650, 194),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        "7D",
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

    let selected_language = if general.language == "system" {
        snapshot.language
    } else {
        LanguageId::from_code(&general.language)
            .map(LanguageId::ui_supported)
            .unwrap_or(snapshot.language)
    };
    let current_language_index = LanguageId::SELECTABLE
        .iter()
        .position(|language| *language == selected_language)
        .unwrap_or_else(|| {
            LanguageId::SELECTABLE
                .iter()
                .position(|language| *language == LanguageId::English)
                .unwrap_or(0)
        });
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
        // Paint one opaque popup surface first. The option rows use rounded
        // corners individually; without this backing surface their clipped
        // corners expose controls from the General page underneath.
        let popup_rect = language_popup_rect(hwnd);
        fill_rounded_rect(hdc, popup_rect, card, scale(hwnd, 7));

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
        draw_rounded_outline_rect(
            hdc,
            popup_rect,
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
    save_feedback: Option<JsonSaveFeedback>,
    error_shake_step: u8,
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
        let action_background = button_background(
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
        );
        draw_segment_with_icon(
            hdc,
            hwnd,
            json_action_rect(hwnd, action),
            false,
            action_background,
            primary,
            json_action_icon(action),
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
        if scroll_hovered {
            let track = json_scrollbar_track_rect(hwnd);
            fill_rounded_rect(
                hdc,
                track,
                if snapshot.is_dark {
                    // Composite equivalent of a very low-opacity light gray over
                    // the JSON editor background; GDI brushes do not alpha blend.
                    Color::from_hex("#303033FF")
                } else {
                    Color::from_hex("#F1F2F4FF")
                },
                ((track.right - track.left) / 2).max(1),
            );
        }
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

    // Invalid Apply uses a short horizontal shake whose amplitude decays
    // smoothly to zero. Font size stays stable to avoid layout jitter.
    let error_shake_x = if !success {
        json_error_shake_offset(hwnd, error_shake_step)
    } else {
        0
    };

    draw_text(
        hdc,
        status,
        RECT {
            left: scale(hwnd, 200) + error_shake_x,
            top: status_top,
            right: if status_path.is_some() {
                scale(hwnd, 200) + prefix_width + error_shake_x
            } else {
                client.right - scale(hwnd, 24) + error_shake_x
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

    let second_line = match save_feedback {
        Some(JsonSaveFeedback::Saving) => Some((
            if zh { "◌ 保存中" } else { "◌ Saving" },
            Color::from_hex("#8FA8C7FF"),
        )),
        Some(JsonSaveFeedback::Saved) => Some((
            if zh { "✓ 保存成功" } else { "✓ Saved" },
            Color::from_hex("#55B879FF"),
        )),
        None if dirty => Some((
            if zh { "● 有未保存更改" } else { "● Unsaved changes" },
            Color::from_hex("#D69E2EFF"),
        )),
        None => None,
    };
    if let Some((text, color)) = second_line {
        let _ = SetTextColor(hdc, COLORREF(color.to_colorref()));
        draw_text(
            hdc,
            text,
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
    draw_segment_with_icon(
        hdc,
        hwnd,
        json_action_rect(hwnd, apply),
        dirty,
        apply_background,
        if dirty {
            Color::from_hex("#FFFFFFFF")
        } else {
            secondary
        },
        json_action_icon(apply),
        if zh { "应用" } else { "Apply" },
    );
}


fn submenu_mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let c = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color::rgba(c(a.r,b.r),c(a.g,b.g),c(a.b,b.b),c(a.a,b.a))
}
fn submenu_content_alpha(progress: f32) -> f32 {
    // Fade complete controls in as a group, rather than revealing clipped
    // fragments one after another. Reverse playback naturally fades them out.
    let p = ((progress - 0.08) / 0.82).clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

fn submenu_content_offset(progress: f32) -> i32 {
    // A subtle slide from the right, measured in logical (96 DPI) pixels.
    ((1.0 - submenu_content_alpha(progress)) * 8.0).round() as i32
}

fn submenu_background_alpha(progress: f32) -> f32 {
    progress.clamp(0.0, 1.0)
}

/// One anti-aliased, continuous L-shaped inset. The old implementation
/// overpainted two rounded rectangles and four opaque shadow bars; its joined
/// horizontal/vertical edges produced the heavy black seams in V24.28.
/// A native Direct2D path now owns the union AND its low-contrast inner shadow.
unsafe fn paint_submenu_well(
    hdc: HDC, hwnd: HWND, progress: f32, card: Color, dark: bool,
) -> Color {
    let top_target = if dark {
        Color::from_hex("#252A30FF")
    } else {
        Color::from_hex("#E4EBF2FF")
    };
    let bottom_target = if dark {
        Color::from_hex("#22272DFF")
    } else {
        Color::from_hex("#E0E7EFFF")
    };
    if progress <= 0.0 { return top_target; }
    let l = desktop_submenu_layout();
    // Paint the WHOLE L-shaped surface on every animated frame. A sweeping
    // clipping edge used to hide nearly half the animation before the well
    // appeared, then visibly cut glyphs and controls during the reveal.
    let opacity = submenu_background_alpha(progress);
    let top = submenu_mix(card, top_target, opacity);
    let bottom = submenu_mix(card, bottom_target, opacity);
    let well = scaled_rect(hwnd, l.well);
    let foot = scaled_rect(hwnd, l.foot);
    let reveal = well.right;
    let shadow_opacity = if dark { 0.12 } else { 0.075 } * opacity;
    if !native_interop::draw_antialiased_desktop_inset_well(
        hdc, well, foot, reveal, scale(hwnd, 11) as f32,
        top, bottom, shadow_opacity,
    ) {
        // Degraded systems without the Direct2D DC renderer: use a subtle
        // solid inset, but never restore the hard black shadow rectangles.
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(
            hdc, foot.left, well.top, reveal, well.bottom,
        );
        fill_rounded_rect(hdc, well, top, scale(hwnd, 11));
        fill_rounded_rect(hdc, foot, top, scale(hwnd, 11));
        fill(hdc, rect(hwnd, 641, 163, 652, 191), top);
        let _ = RestoreDC(hdc, saved);
    }
    // This is also the backing color used when fading the submenu labels and
    // controls. Its exact value is independent of the persistent user theme.
    top
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_submenu_choice(hdc:HDC,hwnd:HWND,bounds:RECT,icon:SettingsIcon,
    label:&str,selected:bool,target:HitTarget,hovered:Option<HitTarget>,
    pressed:Option<HitTarget>,alpha:f32,inset:Color,palette:ButtonPalette,primary:Color) {
    if alpha<=0.005 {return;}
    let mut r=scaled_rect(hwnd,bounds);
    let shift=scale(hwnd,((1.0-alpha)*8.0).round() as i32);
    r.left+=shift; r.right+=shift;
    let desired=button_background(target,selected,hovered,pressed,palette);
    let bg=submenu_mix(inset,desired,alpha);
    let fg=submenu_mix(inset,if selected {Color::from_hex("#FFFFFFFF")}else{primary},alpha);
    fill_rounded_rect(hdc,r,bg,scale(hwnd,7));
    let outline=submenu_mix(inset,if selected{Color::from_hex("#76A7FFFF")}
        else{subtle_control_border(desired)},alpha);
    draw_rounded_outline_rect(hdc,r,outline,scale(hwnd,7),1);
    // V24.32: the 14px glyphs in V24.31 lost the silhouette at 100% DPI.
    // Use 18px and reserve text width even on English / scaled displays.
    let gap=scale(hwnd,2);
    let text_width=text_width_px(hwnd,label);
    let size=scale(hwnd,DESKTOP_SUBMENU_ICON_SIZE)
        .min((r.right-r.left-text_width-gap-scale(hwnd,6)).max(scale(hwnd,12)));
    let group=size+gap+text_width;
    let left=r.left+((r.right-r.left-group)/2).max(scale(hwnd,3));
    let top=r.top+(r.bottom-r.top-size)/2;
    let ir=RECT{left,top,right:left+size,bottom:top+size};
    draw_settings_icon(hdc,hwnd,icon,ir,fg,bg);
    let _=SetTextColor(hdc,COLORREF(fg.to_colorref()));
    draw_text(hdc,label,RECT{left:ir.right+gap,top:r.top,right:r.right-scale(hwnd,2),
        bottom:r.bottom},DT_LEFT|DT_VCENTER|DT_SINGLELINE);
}
#[allow(clippy::too_many_arguments)]
unsafe fn paint_submenu_content(hdc:HDC,hwnd:HWND,snapshot:&StyleWindowSnapshot,
    progress:f32,inset:Color,hovered:Option<HitTarget>,pressed:Option<HitTarget>,
    palette:ButtonPalette,primary:Color,secondary:Color) {
    let opacity=submenu_content_alpha(progress);
    if opacity<=0.005 {return;}
    let l=desktop_submenu_layout();
    // Clip only to the FIXED well bounds, never to an animated vertical edge.
    // The background, labels and controls now move/fade as a complete unit.
    let saved=SaveDC(hdc);
    let _=IntersectClipRect(hdc,scale(hwnd,l.well.left),scale(hwnd,l.well.top),
        scale(hwnd,l.well.right),scale(hwnd,l.well.bottom));
    let zh=snapshot.language==LanguageId::SimplifiedChinese;
    let dx=scale(hwnd,submenu_content_offset(progress));
    let txt=|mut r:RECT|{r.left+=dx;r.right+=dx;r};
    // The inset and selected Desktop button already convey the hierarchy;
    // avoid a redundant secondary-panel title.
    let _=SetTextColor(hdc,COLORREF(submenu_mix(inset,secondary,opacity).to_colorref()));
    draw_text(hdc,if zh{"方向"}else{"Direction"},txt(scaled_rect(hwnd,l.direction_label)),
        DT_LEFT|DT_VCENTER|DT_SINGLELINE);
    draw_text(hdc,if zh{"收缩"}else{"Collapse"},
        txt(scaled_rect(hwnd,l.collapse_label)),DT_LEFT|DT_VCENTER|DT_SINGLELINE);
    for direction in [DesktopDirection::Horizontal,DesktopDirection::Vertical] {
        let horizontal=direction==DesktopDirection::Horizontal;
        paint_submenu_choice(hdc,hwnd,if horizontal{l.horizontal}else{l.vertical},
            if horizontal{SettingsIcon::DirectionHorizontal}else{SettingsIcon::DirectionVertical},
            match(zh,horizontal){(true,true)=>"横向",(true,false)=>"纵向",
                (false,true)=>"Horz",(false,false)=>"Vert"},
            snapshot.desktop_direction==direction,HitTarget::DesktopDirection(direction),
            hovered,pressed,opacity,inset,palette,primary);
    }
    for enabled in [true,false] {
        paint_submenu_choice(hdc,hwnd,if enabled{l.enabled}else{l.disabled},
            if enabled{SettingsIcon::ToggleOn}else{SettingsIcon::ToggleOff},
            match(zh,enabled){(true,true)=>"开启",(true,false)=>"关闭",
                (false,true)=>"On",(false,false)=>"Off"},
            snapshot.desktop_auto_collapse==enabled,HitTarget::DesktopAutoCollapse(enabled),
            hovered,pressed,opacity,inset,palette,primary);
    }
    let _=RestoreDC(hdc,saved);
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_appearance_page(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    section: Section,
    editor: EditorSelection,
    desktop_submenu_progress: f32,
    submenu_only: bool,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    focused_numeric_edit: Option<usize>,
    focused_blur_edit: bool,
    focused_corner_edit: bool,
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
        &appearance_page_title(section, snapshot.language),
        settings_page_title_rect(hwnd),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    fill_rounded_rect(
        hdc,
        settings_card_rect(hwnd, 58, 206),
        card,
        scale(hwnd, SETTINGS_CARD_RADIUS),
    );
    let submenu_inset = paint_submenu_well(
        hdc, hwnd, desktop_submenu_progress, card, snapshot.is_dark,
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
    draw_text(
        hdc,
        if zh { "位置" } else { "Position" },
        rect(hwnd, 218, 162, 300, 196),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    for mode in [ThemeMode::System, ThemeMode::Dark, ThemeMode::Light] {
        let selected = snapshot.theme_mode == mode;
        let target = HitTarget::Theme(mode);
        let control_background = button_background(
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
        );
        draw_segment_with_icon(
            hdc,
            hwnd,
            theme_rect(hwnd, mode),
            selected,
            control_background,
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            theme_icon(mode),
            match (zh, mode) {
                (true, ThemeMode::System) => "系统",
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
        let control_background = button_background(
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
        );
        draw_segment_with_icon(
            hdc,
            hwnd,
            layout_rect(hwnd, preset),
            selected,
            control_background,
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            layout_icon(preset),
            match (zh, preset) {
                (true, AppearancePreset::Default) => "默认",
                (true, AppearancePreset::Minimal) => "极简",
                (false, AppearancePreset::Default) => "Default",
                (false, AppearancePreset::Minimal) => "Minimal",
            },
        );
    }


    for position in [DisplayPosition::Taskbar, DisplayPosition::Desktop] {
        let selected = snapshot.display_position == position;
        let target = HitTarget::Position(position);
        let background = button_background(
            target, selected, hovered, pressed,
            ButtonPalette {
                normal: card_hover, hover: card_pressed, pressed: card_pressed,
                selected: accent, selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        );
        draw_segment_with_icon(
            hdc, hwnd, position_rect(hwnd, position), selected,
            background,
            if selected { Color::from_hex("#FFFFFFFF") } else { primary },
            if position == DisplayPosition::Taskbar {
                SettingsIcon::PositionTaskbar
            } else {
                SettingsIcon::PositionDesktop
            },
            match (zh, position) {
                (true, DisplayPosition::Taskbar) => "任务栏",
                (true, DisplayPosition::Desktop) => "桌面",
                (false, DisplayPosition::Taskbar) => "Taskbar",
                (false, DisplayPosition::Desktop) => "Desktop",
            },
        );
    }

    if desktop_submenu_progress > 0.0 {
        paint_submenu_content(hdc,hwnd,snapshot,desktop_submenu_progress,
            submenu_inset,hovered,pressed,
            ButtonPalette{
                normal:card_hover,hover:card_pressed,pressed:card_pressed,
                selected:accent,selected_hover:accent_hover,selected_pressed:accent_pressed,
            },primary,secondary);
    }

    if submenu_only { return; }

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

    // The current section name belongs in the page heading. Drawing it at
    // y=180..212 would overlap the shared appearance card (y=58..206).
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
            EditorSelection::CornerRadius => {}
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
                    blur_slider_track_rect(hwnd, section),
                    value,
                    FROSTED_STRENGTH_MAX,
                    track_background,
                    accent,
                );
            }
        }
    }

    if matches!(section, Section::Panel | Section::Tooltip) {
        paint_frosted_alpha_hint(hdc, hwnd, snapshot.language, secondary);
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

    if matches!(section, Section::Panel | Section::Tooltip | Section::Progress) {
        paint_corner_edit_frame(
            hdc,
            hwnd,
            snapshot.is_dark,
            focused_corner_edit,
            track_background,
            accent,
            secondary,
        );
    }

    if matches!(section, Section::Panel | Section::Tooltip) {
        paint_blur_edit_frame(
            hdc,
            hwnd,
            section,
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

fn preset_label(preset: ThemePreset, is_dark: bool, language: LanguageId) -> &'static str {
    match (language == LanguageId::SimplifiedChinese, is_dark, preset) {
        (true, true, ThemePreset::Classic) => "石墨",
        (true, true, ThemePreset::Ocean) => "深海",
        (true, true, ThemePreset::Forest) => "松影",
        (false, true, ThemePreset::Classic) => "Graphite",
        (false, true, ThemePreset::Ocean) => "Deep Sea",
        (false, true, ThemePreset::Forest) => "Pine Shade",
        (true, false, ThemePreset::Classic) => "云瓷",
        (true, false, ThemePreset::Ocean) => "晴湾",
        (true, false, ThemePreset::Forest) => "麦光",
        (false, false, ThemePreset::Classic) => "Cloud Porcelain",
        (false, false, ThemePreset::Ocean) => "Clear Bay",
        (false, false, ThemePreset::Forest) => "Wheat Glow",
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
    // Preset is part of the page title ("Appearance · Presets").
    // The gallery begins just below the shared appearance controls.
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
            preset_icon(preset, snapshot.is_dark),
            preset_label(preset, snapshot.is_dark, snapshot.language),
            selected,
            surface,
            border,
            accent,
            primary,
            preview_vertical_for_snapshot(snapshot),
            snapshot.appearance_preset == AppearancePreset::Minimal,
            snapshot.display_position == DisplayPosition::Desktop,
        );
    }

    if !matched {
        let zh = snapshot.language == LanguageId::SimplifiedChinese;
        paint_style_preview_card(
            hdc,
            hwnd,
            custom_preset_card_rect(hwnd),
            &snapshot.active_style,
            SettingsIcon::PresetCustom,
            if zh { "自定义" } else { "Custom" },
            true,
            card,
            border,
            accent,
            primary,
            preview_vertical_for_snapshot(snapshot),
            snapshot.appearance_preset == AppearancePreset::Minimal,
            snapshot.display_position == DisplayPosition::Desktop,
        );
    }
}

// Match the vertical widget's 36-degree bottom opening and clockwise
// remaining-quota fill. Angles are measured from 12 o'clock.
const PREVIEW_RING_GAP_HALF_RADIANS: f32 = std::f32::consts::PI / 10.0;

fn preview_ring_angles(remaining: f32) -> (f32, f32, f32) {
    let start = std::f32::consts::PI + PREVIEW_RING_GAP_HALF_RADIANS;
    let sweep = std::f32::consts::TAU - 2.0 * PREVIEW_RING_GAP_HALF_RADIANS;
    (start, start + sweep, start + sweep * (remaining / 100.0).clamp(0.0, 1.0))
}

fn preview_remaining_color(remaining: f32) -> StyleColorTarget {
    if remaining > 50.0 {
        StyleColorTarget::ProgressHigh
    } else if remaining > 20.0 {
        StyleColorTarget::ProgressMedium
    } else {
        StyleColorTarget::ProgressLow
    }
}

// A four-by-four coverage raster instead of the jagged GDI Arc stroke.
// The ring remains clockwise, displays remaining quota and keeps its
// 36-degree bottom opening at 0% through 100%.
const PREVIEW_RING_SAMPLES: i32 = 4;

#[allow(clippy::too_many_arguments)]
fn preview_ring_pixel(
    px: i32, py: i32, size: i32, radius: f32, stroke: f32,
    remaining: f32, background: Color, track: Color, accent: Color,
) -> u32 {
    let (start, end, _) = preview_ring_angles(remaining);
    let sweep = end - start;
    let fraction = (remaining / 100.0).clamp(0.0, 1.0);
    let track = track.blend_over(background);
    let accent = accent.blend_over(background);
    let count = (PREVIEW_RING_SAMPLES * PREVIEW_RING_SAMPLES) as u32;
    let (mut red, mut green, mut blue) = (0u32, 0u32, 0u32);
    for sy in 0..PREVIEW_RING_SAMPLES {
        for sx in 0..PREVIEW_RING_SAMPLES {
            let dx = px as f32 + (sx as f32 + 0.5) / PREVIEW_RING_SAMPLES as f32
                - size as f32 * 0.5;
            let dy = py as f32 + (sy as f32 + 0.5) / PREVIEW_RING_SAMPLES as f32
                - size as f32 * 0.5;
            let distance = (dx * dx + dy * dy).sqrt();
            let color = if (distance - radius).abs() <= stroke * 0.5 {
                let angle = dx.atan2(-dy).rem_euclid(std::f32::consts::TAU);
                let offset = (angle - start).rem_euclid(std::f32::consts::TAU);
                if offset <= sweep {
                    if offset / sweep <= fraction { accent } else { track }
                } else {
                    background
                }
            } else {
                background
            };
            red += u32::from(color.r);
            green += u32::from(color.g);
            blue += u32::from(color.b);
        }
    }
    let r = (red + count / 2) / count;
    let g = (green + count / 2) / count;
    let b = (blue + count / 2) / count;
    (r << 16) | (g << 8) | b
}

unsafe fn paint_preview_remaining_ring(
    hdc: HDC, hwnd: HWND, cx: i32, cy: i32, remaining: f32, style: &ThemeStyle,
) {
    let size = scale(hwnd, 36).max(8);
    let radius = scale(hwnd, 15).max(2) as f32;
    let stroke = scale(hwnd, 3).max(1) as f32;
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let dc = CreateCompatibleDC(hdc);
    if dc.0.is_null() { return; }
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let dib = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        .unwrap_or_default();
    if dib.is_invalid() || bits.is_null() {
        let _ = DeleteDC(dc);
        return;
    }
    let old_bitmap = SelectObject(dc, dib);
    let pixels = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
    let background = style.color(StyleColorTarget::PanelBackground);
    let track = style.color(StyleColorTarget::ProgressConsumed);
    let accent = style.color(preview_remaining_color(remaining));
    for y in 0..size {
        for x in 0..size {
            pixels[(y * size + x) as usize] = preview_ring_pixel(
                x, y, size, radius, stroke, remaining, background, track, accent,
            );
        }
    }
    let _ = BitBlt(hdc, cx - size / 2, cy - size / 2, size, size, dc, 0, 0, SRCCOPY);
    let _ = SelectObject(dc, old_bitmap);
    let _ = DeleteObject(dib);
    let _ = DeleteDC(dc);
}

// Use the same textual template as the live usage tooltip. The preview
// deliberately uses sample data and wraps *only* at the final separator
// between the date and time when a theme card cannot fit one line.
fn preview_reset_tooltip_lines(minimal: bool) -> (String, String) {
    let preset = if minimal { AppearancePreset::Minimal } else { AppearancePreset::Default };
    let sample = crate::appearance::quota_reset_tooltip_text(
        preset, "5H", "YYYY-MM-DD HH:mm",
    );
    let (date, time) = sample.rsplit_once(' ')
        .expect("sample full local timestamp contains a time separator");
    (date.to_owned(), time.to_owned())
}

fn portrait_preview_tooltip_rect(
    canvas: RECT, widget: RECT, logical: impl Fn(i32) -> i32,
) -> RECT {
    let width = logical(128).min(canvas.right - canvas.left - logical(12));
    let height = logical(40);
    let left = canvas.left + (canvas.right - canvas.left - width) / 2;
    let top = widget.bottom + logical(7);
    RECT { left, top, right: left + width, bottom: top + height }
}

unsafe fn paint_portrait_preview_tooltip(
    hdc: HDC, hwnd: HWND, canvas: RECT, widget: RECT,
    style: &ThemeStyle, minimal: bool,
) {
    // The preview always includes a sample floating tooltip to demonstrate
    // theme customization. It sits below the widget rather than obscuring it.
    // Runtime hover rules are unchanged (reset only / percent only).
    let tooltip = portrait_preview_tooltip_rect(canvas, widget, |v| scale(hwnd, v));
    let radius = scale(hwnd, i32::from(style.tooltip_corner_radius))
        .min(((tooltip.right - tooltip.left).min(tooltip.bottom - tooltip.top) / 2).max(0));
    if radius > 0 {
        fill_rounded_rect(hdc, tooltip, style.color(StyleColorTarget::TooltipBackground), radius);
        draw_rounded_outline_rect(
            hdc, tooltip, style.color(StyleColorTarget::TooltipBorder), radius, 1,
        );
    } else {
        fill(hdc, tooltip, style.color(StyleColorTarget::TooltipBackground));
        draw_outline_rect(hdc, tooltip, style.color(StyleColorTarget::TooltipBorder));
    }
    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
    let font = CreateFontW(
        -scale(hwnd, 10).max(1), 0, 0, 0, FW_MEDIUM.0 as i32, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32, ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, PCWSTR::from_raw(face.as_ptr()),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, TRANSPARENT);
    let text_color = style.color(StyleColorTarget::ResetTime);
    let _ = SetTextColor(hdc, COLORREF(text_color.to_colorref()));
    let (date, time) = preview_reset_tooltip_lines(minimal);
    draw_text(
        hdc,
        &date,
        RECT {
            left: tooltip.left + scale(hwnd, 3),
            top: tooltip.top + scale(hwnd, 3),
            right: tooltip.right - scale(hwnd, 3),
            bottom: tooltip.top + scale(hwnd, 21),
        },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc, &time,
        RECT {
            left: tooltip.left + scale(hwnd, 3),
            top: tooltip.top + scale(hwnd, 20),
            right: tooltip.right - scale(hwnd, 3),
            bottom: tooltip.bottom - scale(hwnd, 2),
        },
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
}

// Preview the actual portrait taskbar widget rather than a stretched card
// illustration. The same 50x168 proportion and 20/94px row anchors are used
// for all four themes; only each theme's colors and corners change.
const PORTRAIT_PREVIEW_WIDGET_W: i32 = 50;
const PORTRAIT_PREVIEW_WIDGET_H: i32 = 168;
const PORTRAIT_PREVIEW_MINIMAL_H: i32 = 82;

fn portrait_preview_widget_bounds(
    canvas: RECT, minimal: bool, logical: impl Fn(i32) -> i32,
) -> RECT {
    let width = logical(PORTRAIT_PREVIEW_WIDGET_W);
    let height = logical(if minimal {
        PORTRAIT_PREVIEW_MINIMAL_H
    } else {
        PORTRAIT_PREVIEW_WIDGET_H
    });
    let left = canvas.left + (canvas.right - canvas.left - width) / 2;
    // Align the widget together with its tooltip, rather than centering
    // the widget alone (which left a large dead area above both).
    let group_height = height + logical(7 + 40);
    let available = (canvas.bottom - canvas.top - group_height).max(0);
    let top = canvas.top + available / 3;
    RECT { left, top, right: left + width, bottom: top + height }
}


const DESKTOP_PREVIEW_WIDGET_H: i32 = 164; // V22: 2 * 10px padding + 2 * 70px rows + 4px gap
const DESKTOP_PREVIEW_MINIMAL_H: i32 = 74; // V22: 2 * 10px padding + 2 * 24px rows + 6px gap
const PREVIEW_DRAG_TAB_H: i32 = 16;
const PREVIEW_DRAG_TAB_W: i32 = 36;
const PREVIEW_DRAG_TAB_GAP: i32 = 4;
const PREVIEW_TASKBAR_GRIP_W: i32 = 16;
const PREVIEW_HORIZONTAL_TOOLTIP_H: i32 = 36;

fn desktop_portrait_preview_bounds(
    canvas: RECT, minimal: bool, logical: impl Fn(i32) -> i32,
) -> (RECT, RECT) {
    let width = logical(PORTRAIT_PREVIEW_WIDGET_W);
    let height = logical(if minimal { DESKTOP_PREVIEW_MINIMAL_H }
        else { DESKTOP_PREVIEW_WIDGET_H });
    let tab_height = logical(PREVIEW_DRAG_TAB_H);
    let tab_gap = logical(PREVIEW_DRAG_TAB_GAP);
    let group_height = tab_height + tab_gap + height + logical(7 + 40);
    let top = canvas.top + (canvas.bottom - canvas.top - group_height).max(0) / 2;
    let left = canvas.left + (canvas.right - canvas.left - width) / 2;
    let body_top = top + tab_height + tab_gap;
    let body = RECT { left, top: body_top, right: left + width,
        bottom: body_top + height };
    let tab_width = logical(PREVIEW_DRAG_TAB_W);
    let tab = RECT { left: (body.left + body.right - tab_width) / 2, top,
        right: (body.left + body.right + tab_width) / 2, bottom: top + tab_height };
    (body, tab)
}

// The desktop tab has its own panel surface and is never painted inside
// the body. A single six-dot renderer is shared by both orientations.
unsafe fn paint_preview_drag_tab(
    hdc: HDC, hwnd: HWND, tab: RECT, style: &ThemeStyle,
) {
    let radius = scale(hwnd, i32::from(style.panel_corner_radius))
        .min(((tab.right - tab.left).min(tab.bottom - tab.top) / 2).max(0));
    if radius > 0 {
        fill_rounded_rect(hdc, tab, style.color(StyleColorTarget::PanelBackground), radius);
        draw_rounded_outline_rect(hdc, tab, style.color(StyleColorTarget::PanelBorder), radius, 1);
    } else {
        fill(hdc, tab, style.color(StyleColorTarget::PanelBackground));
        draw_outline_rect(hdc, tab, style.color(StyleColorTarget::PanelBorder));
    }
    let dot = scale(hwnd, 2).max(1);
    let gap_x = scale(hwnd, 2).max(1);
    let gap_y = scale(hwnd, 1).max(1);
    let dots_w = 3 * dot + 2 * gap_x;
    let dots_h = 2 * dot + gap_y;
    let start_x = (tab.left + tab.right - dots_w) / 2;
    let start_y = (tab.top + tab.bottom - dots_h) / 2;
    for row in 0..2 {
        for col in 0..3 {
            let x = start_x + col * (dot + gap_x);
            let y = start_y + row * (dot + gap_y);
            fill(hdc, RECT { left: x, top: y, right: x + dot, bottom: y + dot },
                style.color(StyleColorTarget::DragHandle));
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct PortraitPreviewRow {
    label: RECT,
    center_x: i32,
    center_y: i32,
    reset: RECT,
}

fn portrait_preview_row_for_mode(
    widget: RECT, index: i32, desktop: bool, logical: impl Fn(i32) -> i32,
) -> PortraitPreviewRow {
    let top = widget.top + logical((if desktop { 10 } else { 20 }) + index * 74);
    PortraitPreviewRow {
        label: RECT {
            left: widget.left + logical(2),
            top,
            right: widget.right - logical(2),
            bottom: top + logical(18),
        },
        center_x: (widget.left + widget.right) / 2,
        center_y: top + logical(36),
        reset: RECT {
            left: widget.left + logical(2),
            top: top + logical(54),
            right: widget.right - logical(2),
            bottom: top + logical(72),
        },
    }
}

#[cfg(test)]
fn portrait_preview_row(widget: RECT, index: i32, logical: impl Fn(i32) -> i32) -> PortraitPreviewRow {
    portrait_preview_row_for_mode(widget, index, false, logical)
}

unsafe fn paint_vertical_style_preview(
    hdc: HDC, hwnd: HWND, preview: RECT, style: &ThemeStyle,
    minimal: bool, desktop: bool,
) {
    let (widget, detached_tab) = if desktop {
        let (widget, tab) = desktop_portrait_preview_bounds(preview, minimal, |v| scale(hwnd, v));
        (widget, Some(tab))
    } else {
        (portrait_preview_widget_bounds(preview, minimal, |v| scale(hwnd, v)), None)
    };
    let radius = scale(hwnd, i32::from(style.panel_corner_radius))
        .min(((widget.right - widget.left).min(widget.bottom - widget.top) / 2).max(0));
    if radius > 0 {
        fill_rounded_rect(hdc, widget, style.color(StyleColorTarget::PanelBackground), radius);
        draw_rounded_outline_rect(hdc, widget, style.color(StyleColorTarget::PanelBorder), radius, 1);
    } else {
        fill(hdc, widget, style.color(StyleColorTarget::PanelBackground));
        draw_outline_rect(hdc, widget, style.color(StyleColorTarget::PanelBorder));
    }

    if let Some(tab) = detached_tab {
        paint_preview_drag_tab(hdc, hwnd, tab, style);
    } else {
        // The embedded taskbar grip remains inside the taskbar body.
        let dot = scale(hwnd, 2).max(1);
        let dot_gap = scale(hwnd, 3);
        let grip_color = style.color(StyleColorTarget::DragHandle);
        for row in 0..2 {
            for col in 0..3 {
                let x = (widget.left + widget.right) / 2
                    - (3 * dot + 2 * dot_gap) / 2 + col * (dot + dot_gap);
                let y = widget.top + scale(hwnd, 5) + row * (dot + dot_gap);
                fill(hdc, RECT { left: x, top: y, right: x + dot, bottom: y + dot }, grip_color);
            }
        }
    }

    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
    let font = |size: i32, weight: i32| CreateFontW(
        -scale(hwnd, size).max(1), 0, 0, 0, weight, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32, ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, PCWSTR::from_raw(face.as_ptr()),
    );
    let label_font = font(10, FW_MEDIUM.0 as i32);
    // At a 50px taskbar width, a three-character percentage needs room
    // around its glyphs; a 9px font inside a 15px ring preserves that gap.
    let percent_font = font(if minimal { 12 } else { 9 }, FW_SEMIBOLD.0 as i32);
    let reset_font = font(9, FW_NORMAL.0 as i32);
    let old_font = SelectObject(hdc, label_font);
    let _ = SetBkMode(hdc, TRANSPARENT);

    for (index, (label, remaining, reset)) in [
        ("5H", 82.0_f32, "03:49"),
        ("7D", 37.0_f32, "10/22"),
    ].iter().enumerate() {
        if minimal {
            let center_y = widget.top + scale(hwnd, if desktop {
                if index == 0 { 22 } else { 52 }
            } else if index == 0 { 34 } else { 63 });
            let _ = SelectObject(hdc, percent_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
            draw_text(hdc, &format!("{remaining:.0}%"), RECT {
                left: widget.left + scale(hwnd, 2),
                top: center_y - scale(hwnd, 12),
                right: widget.right - scale(hwnd, 2),
                bottom: center_y + scale(hwnd, 12),
            }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        } else {
            let row = portrait_preview_row_for_mode(widget, index as i32, desktop, |v| scale(hwnd, v));
            let _ = SelectObject(hdc, label_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::QuotaType).to_colorref()));
            draw_text(hdc, label, row.label, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            paint_preview_remaining_ring(hdc, hwnd, row.center_x, row.center_y, *remaining, style);
            let _ = SelectObject(hdc, percent_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
            draw_text(hdc, &format!("{remaining:.0}%"), RECT {
                left: row.center_x - scale(hwnd, 14),
                top: row.center_y - scale(hwnd, 11),
                right: row.center_x + scale(hwnd, 14),
                bottom: row.center_y + scale(hwnd, 11),
            }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let _ = SelectObject(hdc, reset_font);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()));
            draw_text(hdc, reset, row.reset, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }

    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(label_font);
    let _ = DeleteObject(percent_font);
    let _ = DeleteObject(reset_font);
    paint_portrait_preview_tooltip(hdc, hwnd, preview, widget, style, minimal);
}



unsafe fn paint_taskbar_horizontal_preview_grip(
    hdc: HDC, hwnd: HWND, canvas: RECT, style: &ThemeStyle,
) {
    // The taskbar grip is embedded at the left edge; unlike the separate
    // desktop tab it uses the taskbar's original 2-column, 3-row dot shape.
    let dot = scale(hwnd, 2).max(1);
    let gap_x = scale(hwnd, 1).max(1);
    let gap_y = scale(hwnd, 2).max(1);
    let dots_w = 2 * dot + gap_x;
    let dots_h = 3 * dot + 2 * gap_y;
    let x0 = canvas.left + scale(hwnd, 6) - dots_w / 2;
    let y0 = (canvas.top + canvas.bottom - dots_h) / 2;
    for row in 0..3 {
        for col in 0..2 {
            let x = x0 + col * (dot + gap_x);
            let y = y0 + row * (dot + gap_y);
            fill(hdc, RECT { left: x, top: y, right: x + dot, bottom: y + dot },
                style.color(StyleColorTarget::DragHandle));
        }
    }
}

fn horizontal_preview_body(
    canvas: RECT, minimal: bool, desktop: bool, logical: impl Fn(i32) -> i32,
) -> RECT {
    let width = logical(if minimal { 56 } else { 184 }
        + if desktop { 0 } else { PREVIEW_TASKBAR_GRIP_W })
        .min(canvas.right - canvas.left - logical(if desktop { 8 } else { 4 }));
    let height = logical(if minimal { 40 } else { 42 });
    let tab_space = if desktop {
        logical(PREVIEW_DRAG_TAB_H + PREVIEW_DRAG_TAB_GAP)
    } else { 0 };
    let tooltip_space = logical(6 + PREVIEW_HORIZONTAL_TOOLTIP_H);
    let top = canvas.top + (canvas.bottom - canvas.top
        - (tab_space + height + tooltip_space)).max(0) / 2 + tab_space;
    let left = (canvas.left + canvas.right - width) / 2;
    RECT { left, top, right: left + width, bottom: top + height }
}

fn horizontal_preview_detached_tab(body: RECT, logical: impl Fn(i32) -> i32) -> RECT {
    let tab_w = logical(PREVIEW_DRAG_TAB_W);
    let tab_bottom = body.top - logical(PREVIEW_DRAG_TAB_GAP);
    RECT {
        left: (body.left + body.right - tab_w) / 2,
        top: tab_bottom - logical(PREVIEW_DRAG_TAB_H),
        right: (body.left + body.right + tab_w) / 2,
        bottom: tab_bottom,
    }
}

unsafe fn paint_preview_horizontal_minimal_text(
    hdc: HDC, hwnd: HWND, body: RECT, style: &ThemeStyle,
) {
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
    let row_height = (body.bottom - body.top) / 2;
    for (index, percent) in ["82%", "37%"].iter().enumerate() {
        let top = body.top + index as i32 * row_height;
        draw_text(hdc, percent, RECT {
            left: body.left + scale(hwnd, 2), top,
            right: body.right - scale(hwnd, 2), bottom: top + row_height,
        }, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    }
}

unsafe fn paint_horizontal_style_preview(
    hdc: HDC, hwnd: HWND, canvas: RECT, style: &ThemeStyle,
    minimal: bool, desktop: bool,
) {
    let body = horizontal_preview_body(canvas, minimal, desktop, |v| scale(hwnd, v));
    let radius = scale(hwnd, i32::from(style.panel_corner_radius))
        .min((body.bottom - body.top) / 2);
    if radius > 0 {
        fill_rounded_rect(hdc, body, style.color(StyleColorTarget::PanelBackground), radius);
        draw_rounded_outline_rect(hdc, body, style.color(StyleColorTarget::PanelBorder), radius, 1);
    } else {
        fill(hdc, body, style.color(StyleColorTarget::PanelBackground));
        draw_outline_rect(hdc, body, style.color(StyleColorTarget::PanelBorder));
    }
    if desktop {
        let tab = horizontal_preview_detached_tab(body, |v| scale(hwnd, v));
        paint_preview_drag_tab(hdc, hwnd, tab, style);
    } else {
        paint_taskbar_horizontal_preview_grip(hdc, hwnd, body, style);
    }
    // Both modes share the same data rows, progress palette and tooltip.
    // Taskbar mode alone reserves 16px inside the panel for the grip.
    let content_left = body.left
        + if desktop { 0 } else { scale(hwnd, PREVIEW_TASKBAR_GRIP_W) };
    let content = RECT { left: content_left, ..body };
    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
    let font = CreateFontW(
        -scale(hwnd, 10).max(1), 0, 0, 0, FW_MEDIUM.0 as i32, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32, ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32, PCWSTR::from_raw(face.as_ptr()),
    );
    let old_font = SelectObject(hdc, font);
    let _ = SetBkMode(hdc, TRANSPARENT);
    if minimal {
        paint_preview_horizontal_minimal_text(hdc, hwnd, content, style);
    } else {
        for (index, (label, percent, reset)) in [
            ("5H", "82%", "21:30"), ("7D", "37%", "10/22"),
        ].iter().enumerate() {
            let y = body.top + scale(hwnd, 3 + index as i32 * 19);
            let text_rect = |left: i32, right: i32| RECT {
                left, top: y, right, bottom: y + scale(hwnd, 18),
            };
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::QuotaType).to_colorref()));
            draw_text(hdc, label, text_rect(content.left + scale(hwnd, 9),
                content.left + scale(hwnd, 31)), DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            let progress = RECT {
                left: content.left + scale(hwnd, 34), top: y + scale(hwnd, 6),
                right: content.left + scale(hwnd, 86), bottom: y + scale(hwnd, 12),
            };
            let remaining = if index == 0 { 82 } else { 37 };
            let fill_rect = RECT { right: progress.left + (progress.right - progress.left)
                * remaining / 100, ..progress };
            let progress_radius = scale(hwnd, i32::from(style.progress_corner_radius))
                .min((progress.bottom - progress.top) / 2);
            let progress_color = style.color(if index == 0 {
                StyleColorTarget::ProgressHigh
            } else {
                StyleColorTarget::ProgressMedium
            });
            if progress_radius > 0 {
                fill_rounded_rect(hdc, progress,
                    style.color(StyleColorTarget::ProgressConsumed), progress_radius);
                fill_rounded_rect(hdc, fill_rect, progress_color,
                    progress_radius.min((fill_rect.right - fill_rect.left) / 2));
            } else {
                fill(hdc, progress, style.color(StyleColorTarget::ProgressConsumed));
                fill(hdc, fill_rect, progress_color);
            }
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::Remaining).to_colorref()));
            draw_text(hdc, percent, text_rect(content.left + scale(hwnd, 90),
                content.left + scale(hwnd, 124)), DT_CENTER | DT_VCENTER | DT_SINGLELINE);
            let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()));
            draw_text(hdc, reset, text_rect(content.left + scale(hwnd, 128),
                body.right - scale(hwnd, 6)), DT_RIGHT | DT_VCENTER | DT_SINGLELINE);
        }
    }

    let tip_width = scale(hwnd, 136).min(
        (canvas.right - canvas.left - scale(hwnd, 8)).max(scale(hwnd, 48)),
    );
    let tip_height = scale(hwnd, PREVIEW_HORIZONTAL_TOOLTIP_H);
    let tip = RECT {
        left: (canvas.left + canvas.right - tip_width) / 2,
        top: body.bottom + scale(hwnd, 6),
        right: (canvas.left + canvas.right + tip_width) / 2,
        bottom: body.bottom + scale(hwnd, 6) + tip_height,
    };
    let tooltip_radius = scale(hwnd, i32::from(style.tooltip_corner_radius))
        .min(tip_height / 2);
    if tooltip_radius > 0 {
        fill_rounded_rect(hdc, tip, style.color(StyleColorTarget::TooltipBackground), tooltip_radius);
        draw_rounded_outline_rect(hdc, tip, style.color(StyleColorTarget::TooltipBorder), tooltip_radius, 1);
    } else {
        fill(hdc, tip, style.color(StyleColorTarget::TooltipBackground));
        draw_outline_rect(hdc, tip, style.color(StyleColorTarget::TooltipBorder));
    }
    let _ = SetTextColor(hdc, COLORREF(style.color(StyleColorTarget::ResetTime).to_colorref()));
    let (date, time) = preview_reset_tooltip_lines(minimal);
    let first = RECT { bottom: tip.top + scale(hwnd, 18), ..tip };
    let second = RECT { top: tip.top + scale(hwnd, 18), ..tip };
    draw_text(hdc, &date, first, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    draw_text(hdc, &time, second, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    let _ = SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
}

#[allow(clippy::too_many_arguments)]
unsafe fn paint_style_preview_card(
    hdc: HDC,
    hwnd: HWND,
    r: RECT,
    style: &ThemeStyle,
    icon: SettingsIcon,
    label: &str,
    selected: bool,
    surface: Color,
    border: Color,
    accent: Color,
    primary: Color,
    vertical: bool,
    minimal: bool,
    desktop: bool,
) {
    fill_rounded_rect(hdc, r, surface, scale(hwnd, SETTINGS_CARD_RADIUS));
    draw_rounded_outline_rect(
        hdc,
        r,
        if selected { accent } else { border },
        scale(hwnd, SETTINGS_CARD_RADIUS),
        if selected { 2 } else { 1 },
    );
    let title_icon_size = scale(hwnd, 20);
    let title_icon_rect = RECT {
        left: r.left + scale(hwnd, 12),
        top: r.top + scale(hwnd, 10),
        right: r.left + scale(hwnd, 12) + title_icon_size,
        bottom: r.top + scale(hwnd, 10) + title_icon_size,
    };
    draw_settings_icon(hdc, hwnd, icon, title_icon_rect, primary, surface);

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        label,
        RECT {
            left: r.left + scale(hwnd, 42),
            top: r.top + scale(hwnd, 8),
            right: r.right - scale(hwnd, 12),
            bottom: r.top + scale(hwnd, 38),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let preview = RECT {
        left: r.left + scale(hwnd, 12),
        top: r.top + scale(hwnd, if vertical { 50 } else { 46 }),
        right: r.right - scale(hwnd, 12),
        bottom: if vertical { r.bottom - scale(hwnd, 53) } else { r.top + scale(hwnd, 152) },
    };
    // The preview rectangle is only a positioning region. The card itself
    // provides the backdrop; drawing a second frame here would surround the
    // grip, panel and tooltip with an unwanted inset outline.

    if vertical {
        paint_vertical_style_preview(hdc, hwnd, preview, style, minimal, desktop);
    } else {
        paint_horizontal_style_preview(hdc, hwnd, preview, style, minimal, desktop);
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
        let (left, top, width) = if vertical {
            let swatch_width = 26;
            let swatch_gap = 10;
            let total_width = 4 * swatch_width + 3 * swatch_gap;
            (
                (r.left + r.right - scale(hwnd, total_width)) / 2
                    + scale(hwnd, index as i32 * (swatch_width + swatch_gap)),
                r.bottom - scale(hwnd, 34),
                swatch_width,
            )
        } else {
            // Preserve the original horizontal preset-card color strips.
            (r.left + scale(hwnd, 14 + index as i32 * 48),
             r.top + scale(hwnd, 158), 34)
        };
        fill_rounded_rect(
            hdc,
            RECT {
                left, top,
                right: left + scale(hwnd, width),
                bottom: top + scale(hwnd, 14),
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

#[allow(clippy::too_many_arguments)]
unsafe fn paint_blur_edit_frame(
    hdc: HDC,
    hwnd: HWND,
    section: Section,
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
    let frame = blur_edit_frame_rect(hwnd, section);
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
        blur_suffix_rect(hwnd, section),
        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
    );
}

unsafe fn paint_corner_edit_frame(
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
    let frame = corner_edit_frame_rect(hwnd);
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
        "px",
        corner_suffix_rect(hwnd),
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

fn appearance_page_title(section: Section, language: LanguageId) -> String {
    let root = if language == LanguageId::SimplifiedChinese {
        "外观"
    } else {
        "Appearance"
    };
    if matches!(
        section,
        Section::Preset | Section::Panel | Section::Tooltip | Section::Text
            | Section::Progress | Section::Interaction
    ) {
        format!("{root} · {}", section_label(section, language))
    } else {
        root.to_string()
    }
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

/// The final blur row ends at logical Y=410; the editor begins at Y=432.
/// Render a small non-interactive hint inside that existing 22px gap.
unsafe fn paint_frosted_alpha_hint(
    hdc: HDC,
    hwnd: HWND,
    language: LanguageId,
    secondary: Color,
) {
    let label = if language == LanguageId::SimplifiedChinese {
        "仅当背景颜色含透明度（Alpha < 255）时，磨砂效果才会生效。"
    } else {
        "Frosted effect requires background transparency (Alpha < 255)."
    };
    let face = native_interop::wide_str(fonts::face(fonts::FontRole::Ui, Some(language)));
    let hint_font = CreateFontW(
        -scale(hwnd, 11),
        0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
        DEFAULT_CHARSET.0 as u32,
        OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
        PCWSTR::from_raw(face.as_ptr()),
    );
    let original = if !hint_font.0.is_null() {
        Some(SelectObject(hdc, hint_font))
    } else {
        None
    };
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        label,
        rect(hwnd, 218, 410, 940, 431),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
    if let Some(old_font) = original {
        let _ = SelectObject(hdc, old_font);
        let _ = DeleteObject(hint_font);
    }
}

fn row_label(row: EditorSelection, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match row {
        EditorSelection::CornerRadius => {
            if zh { "圆角半径" } else { "Corner radius" }
        }
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

unsafe fn draw_settings_icon(
    hdc: HDC,
    _hwnd: HWND,
    icon: SettingsIcon,
    rect: RECT,
    color: Color,
    _background: Color,
) {
    // All 32 glyphs are embedded SVG resources. The physical-pixel raster
    // cache preserves theme tint and DPI while Win32 performs alpha compositing.
    let _ = crate::svg_icons::draw(hdc, rect, icon as usize, color);
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_segment_with_icon(
    hdc: HDC,
    hwnd: HWND,
    rect: RECT,
    selected: bool,
    background: Color,
    foreground: Color,
    icon: SettingsIcon,
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

    let important_small_icon = matches!(
        icon,
        SettingsIcon::ThemeSystem | SettingsIcon::PositionTaskbar | SettingsIcon::PositionDesktop,
    );
    // The theme's gear needs a few more pixels than the much simpler
    // rectangular placement icons to keep its teeth distinguishable.
    let preferred_icon_size = if icon == SettingsIcon::ThemeSystem {
        22
    } else if important_small_icon {
        20
    } else {
        18
    };
    let icon_size = scale(hwnd, preferred_icon_size)
        .min((height - scale(hwnd, 8)).max(scale(hwnd, 12)));
    let gap = scale(hwnd, if important_small_icon { 5 } else { 7 });
    let text_width = text_width_px(hwnd, text).max(scale(hwnd, 12));
    let group_width = icon_size + gap + text_width;
    let group_left = rect.left + ((rect.right - rect.left - group_width) / 2).max(scale(hwnd, 6));
    let icon_rect = RECT {
        left: group_left,
        top: rect.top + (height - icon_size) / 2,
        right: group_left + icon_size,
        bottom: rect.top + (height - icon_size) / 2 + icon_size,
    };
    draw_settings_icon(hdc, hwnd, icon, icon_rect, foreground, background);

    let _ = SetTextColor(hdc, COLORREF(foreground.to_colorref()));
    draw_text(
        hdc,
        text,
        RECT {
            left: icon_rect.right + gap,
            top: rect.top,
            right: rect.right - scale(hwnd, 6),
            bottom: rect.bottom,
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
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
    // windows-rs exposes DrawTextW through a mutable UTF-16 slice. Passing an
    // empty Vec leaves the native call with a zero-length slice backed by a
    // dangling Vec pointer. The JSON validation status is intentionally cleared
    // when an invalid document becomes valid, so that exact transition can reach
    // this helper with an empty string. Skip the native call entirely for empty
    // labels/status text.
    if text.is_empty() {
        return;
    }
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let _ = DrawTextW(hdc, &mut wide, &mut rect, format);
}


#[cfg(test)]
mod ui_smoke_tests {
    use super::*;

    #[test]
    fn appearance_subpage_titles_do_not_overlap_the_shared_options_card() {
        for (section, zh, en) in [
            (Section::Preset, "外观 · 预设", "Appearance · Presets"),
            (Section::Panel, "外观 · 面板", "Appearance · Panel"),
            (Section::Tooltip, "外观 · 浮框", "Appearance · Tooltip"),
            (Section::Text, "外观 · 文字", "Appearance · Text"),
            (Section::Progress, "外观 · 进度条", "Appearance · Progress"),
            (Section::Interaction, "外观 · 交互", "Appearance · Interaction"),
        ] {
            assert_eq!(appearance_page_title(section, LanguageId::SimplifiedChinese), zh);
            assert_eq!(appearance_page_title(section, LanguageId::English), en);
        }
        let hwnd = HWND::default();
        let header = settings_page_title_rect(hwnd);
        let shared_card = settings_card_rect(hwnd, 58, 206);
        let first_editor_row = row_rect(hwnd, 0);
        assert!(header.bottom < shared_card.top,
            "page heading must be above the shared appearance options");
        assert!(first_editor_row.top > shared_card.bottom,
            "editor rows must start after the shared appearance options");
        assert_eq!(PRESET_CARD_TOP - shared_card.bottom, 20,
            "preset gallery should immediately follow the shared options without a redundant heading");
    }

    #[test]
    fn json_editor_visibility_depends_only_on_active_page_and_modal_state() {
        assert!(!json_editor_should_be_visible(Section::Preset, false));
        assert!(!json_editor_should_be_visible(Section::Json, true));
        assert!(json_editor_should_be_visible(Section::Json, false));
    }

    #[test]
    fn horizontal_preset_gallery_restores_three_across_and_custom_below() {
        let row = [
            ThemePreset::Classic, ThemePreset::Ocean, ThemePreset::Forest,
        ];
        let bounds: Vec<_> = row.iter().map(|p| {
            let index = match p {
                ThemePreset::Classic => 0,
                ThemePreset::Ocean => 1,
                ThemePreset::Forest => 2,
            };
            let left = 200 + index * 244;
            RECT { left, top: PRESET_CARD_TOP, right: left + 224, bottom: PRESET_CARD_TOP + 184 }
        }).collect();
        assert_eq!(bounds[2].right, 912);
        assert_eq!(bounds[0].top, bounds[2].top);
        let custom = RECT { left: 200, top: PRESET_CARD_TOP + 200,
            right: 424, bottom: PRESET_CARD_TOP + 384 };
        assert_eq!(custom.left, bounds[0].left);
        assert_eq!(custom.top - bounds[0].bottom, 16);
    }

    #[test]
    fn preset_gallery_is_four_tall_equal_cards_in_one_row() {
        let cards: Vec<_> = (0..4).map(|i| preset_gallery_card_rect(i, false)).collect();
        assert_eq!(cards[0].left, 200);
        assert_eq!(cards[3].right, 936);
        assert!(cards[3].right <= 940);
        for (i, card) in cards.iter().enumerate() {
            assert_eq!(card.right - card.left, PRESET_CARD_WIDTH);
            assert_eq!(card.bottom - card.top, PRESET_CARD_HEIGHT);
            assert_eq!(card.bottom, PRESET_CARD_TOP + PRESET_CARD_HEIGHT);
            if i > 0 {
                assert_eq!(card.left - cards[i - 1].right, PRESET_CARD_GAP);
                assert_eq!(card.top, cards[i - 1].top);
            }
        }
    }


    #[test]
    fn v23_vertical_preview_card_height_tracks_default_vs_minimal() {
        for i in 0..4 {
            let standard = preset_gallery_card_rect(i, false);
            let minimal = preset_gallery_card_rect(i, true);
            assert_eq!(standard.bottom - standard.top, 350);
            assert_eq!(minimal.bottom - minimal.top, 250);
            assert_eq!(standard.left, minimal.left);
            assert_eq!(standard.right, minimal.right);
        }
    }

    #[test]
    fn v23_detached_desktop_portrait_grip_is_outside_body() {
        for minimal in [false, true] {
            let canvas = RECT { left: 0, top: 0, right: 148,
                bottom: if minimal { 147 } else { 247 } };
            let (body, tab) = desktop_portrait_preview_bounds(canvas, minimal, |v| v);
            assert_eq!(body.top - tab.bottom, PREVIEW_DRAG_TAB_GAP);
            assert_eq!(tab.bottom - tab.top, PREVIEW_DRAG_TAB_H);
            assert_eq!(body.bottom - body.top,
                if minimal { DESKTOP_PREVIEW_MINIMAL_H } else { DESKTOP_PREVIEW_WIDGET_H });
            assert!(tab.left >= canvas.left && tab.right <= canvas.right);
            let tooltip = portrait_preview_tooltip_rect(canvas, body, |v| v);
            assert!(tooltip.top > body.bottom);
            assert!(tooltip.bottom <= canvas.bottom);
        }
    }

    #[test]
    fn v24_svg_glyph_ids_and_placement_shapes_remain_stable() {
        assert_eq!(SettingsIcon::ThemeSystem as i32, 8);
        assert_eq!(SettingsIcon::DirectionHorizontal as i32, 25);
        assert_eq!(SettingsIcon::DirectionVertical as i32, 26);
        assert_eq!(SettingsIcon::ToggleOn as i32, 27);
        assert_eq!(SettingsIcon::ToggleOff as i32, 28);
        assert_eq!(SettingsIcon::PositionTaskbar as i32, 29);
        assert_eq!(SettingsIcon::PositionDesktop as i32, 30);
        assert_eq!(DESKTOP_SUBMENU_ICON_SIZE, 18);
        let taskbar = include_str!("../assets/settings-icons/position-taskbar.svg");
        let desktop = include_str!("../assets/settings-icons/position-desktop.svg");
        assert!(taskbar.contains("y=\"12.8\""));
        assert!(desktop.contains("y=\"5.4\""));
        assert!(taskbar.contains("height=\"2.0\""));
        assert!(desktop.contains("height=\"6.4\""));
        assert!(taskbar.contains("M12 17v3.1"));
        assert!(desktop.contains("M12 17v3.1"));
        let horizontal = include_str!("../assets/settings-icons/direction-horizontal.svg");
        let vertical = include_str!("../assets/settings-icons/direction-vertical.svg");
        assert!(horizontal.contains("M5.8 12h12.4"));
        assert!(vertical.contains("M12 5.4v13.2"));
        let system = include_str!("../assets/settings-icons/theme-system.svg");
        assert!(system.contains("fill-rule=\"evenodd\""));
        let l = desktop_submenu_layout();
        assert_eq!(l.collapse_label.top, l.enabled.top);
        assert!(l.collapse_label.top > l.direction_label.bottom);
        assert_eq!(l.foot.left, APPEARANCE_BUTTON_LEFT + APPEARANCE_BUTTON_WIDTH + 4);
    }

    #[test]
    fn v24_28_desktop_submenu_fits_existing_card_and_includes_desktop_button() {
        let l = desktop_submenu_layout();
        let card = RECT { left: 200, top: 58, right: 940, bottom: 206 };
        for r in [l.well,l.foot,l.direction_label,l.collapse_label,
            l.horizontal,l.vertical,l.enabled,l.disabled] {
            assert!(r.left>=card.left && r.right<=card.right);
            assert!(r.top>=card.top && r.bottom<=card.bottom);
            assert!(r.right>r.left && r.bottom>r.top);
        }
        // The bottom foot surrounds Desktop, but never overlaps Taskbar.
        let desktop = RECT { left: 422, top: 162, right: 522, bottom: 196 };
        let taskbar = RECT { left: 314, top: 162, right: 414, bottom: 196 };
        assert!(l.foot.left < desktop.left && l.foot.right > desktop.right);
        assert!(l.foot.top < desktop.top && l.foot.bottom > desktop.bottom);
        assert_eq!(desktop.left - l.foot.left, 4,
            "Desktop must have a consistent left inset");
        assert_eq!(l.foot.left - taskbar.right, 4,
            "the inset must leave space after Taskbar");
        assert_eq!(desktop.left - taskbar.right, APPEARANCE_BUTTON_GAP,
            "position buttons must share the same column spacing");
        assert!(l.well.left > 630, "do not overlap the Light theme button");
        assert_eq!(l.well.right - l.well.left, 258,
            "desktop submenu width stays compact");
        assert_eq!(l.horizontal.left - l.direction_label.right, 8);
        assert_eq!(l.enabled.left - l.collapse_label.right, 8);
        assert_eq!(l.well.right - l.vertical.right, 16);
        assert_eq!(l.well.right - l.disabled.right, 16);
        assert!(l.direction_label.right <= l.horizontal.left);
        assert!(l.collapse_label.right <= l.enabled.left);
        assert!(l.foot.right > l.well.left && l.foot.top < l.well.bottom,
            "the two depressed regions must overlap to hide their seam");
        assert_eq!(submenu_background_alpha(0.0), 0.0);
        assert_eq!(submenu_background_alpha(1.0), 1.0);
        assert_eq!(submenu_content_offset(0.0), 8);
        assert_eq!(submenu_content_offset(1.0), 0);
        assert_eq!(PRESET_CARD_TOP, 226,
            "preset gallery starts immediately below the shared options card");
        for r in [l.horizontal,l.vertical,l.enabled,l.disabled] {
            assert!(r.left>=l.well.left && r.right<l.well.right);
        }
        assert!(l.horizontal.right<l.vertical.left);
        assert!(l.enabled.right<l.disabled.left);
        assert!(l.direction_label.right<l.horizontal.left);
        assert!(l.collapse_label.right<l.enabled.left);
        assert_eq!(l.horizontal.left, l.enabled.left);
        assert_eq!(l.horizontal.right, l.enabled.right);
        assert_eq!(l.vertical.left, l.disabled.left);
        assert_eq!(l.vertical.right, l.disabled.right);
        assert_eq!(l.horizontal.right - l.horizontal.left,
                   l.vertical.right - l.vertical.left);
        assert_eq!(l.vertical.left - l.horizontal.right, APPEARANCE_BUTTON_GAP);
    }

    #[test]
    fn appearance_buttons_share_aligned_columns_across_all_rows() {
        let a = appearance_button_rect(HWND::default(), 0, 70);
        let b = appearance_button_rect(HWND::default(), 1, 70);
        let c = appearance_button_rect(HWND::default(), 2, 70);
        assert_eq!(a.right - a.left, APPEARANCE_BUTTON_WIDTH);
        assert_eq!(b.right - b.left, APPEARANCE_BUTTON_WIDTH);
        assert_eq!(c.right - c.left, APPEARANCE_BUTTON_WIDTH);
        assert_eq!(b.left - a.right, APPEARANCE_BUTTON_GAP);
        assert_eq!(c.left - b.right, APPEARANCE_BUTTON_GAP);
        for top in [116, 162] {
            for column in 0..2 {
                let r = appearance_button_rect(HWND::default(), column, top);
                let reference = appearance_button_rect(HWND::default(), column, 70);
                assert_eq!((r.left, r.right), (reference.left, reference.right));
                assert_eq!(r.bottom - r.top, APPEARANCE_BUTTON_HEIGHT);
            }
        }
    }

    #[test]
    fn cached_card_key_ignores_taskbar_desktop_position_and_tracks_visual_state() {
        let hwnd = HWND::default();
        // GetDpiForWindow is not callable for a null test handle; compare
        // the stable key layout fields independently instead.
        let a = SubmenuCardKey {
            width: 980, height: 700, dpi: 96, dark: true, zh: true,
            theme: ThemeMode::Dark, layout: AppearancePreset::Default,
            direction: DesktopDirection::Horizontal, collapse: false,
        };
        let b = SubmenuCardKey { ..a };
        let c = SubmenuCardKey { direction: DesktopDirection::Vertical, ..a };
        assert!(a == b);
        assert!(a != c);
        let _ = hwnd;
    }

    #[test]
    fn submenu_frame_gate_and_metrics_percentile() {
        let start = Instant::now();
        let mut animation = DesktopSubmenuAnimation::initial(false);
        assert!(animation.retarget(true, start));
        assert!(animation.frame_due(start));
        assert!(!animation.frame_due(start + Duration::from_millis(8)));
        assert!(animation.frame_due(start + Duration::from_millis(16)));
        assert_eq!(submenu_p95(&[]), 0.0);
        assert_eq!(submenu_p95(&[10.0, 16.0, 20.0, 45.0]), 45.0);
        assert_eq!(submenu_p95(&[12.0, 14.0, 18.0]), 18.0);
    }

    #[test]
    fn v24_28_desktop_submenu_animation_can_reverse_without_jumps() {
        let start=Instant::now();
        let mut animation=DesktopSubmenuAnimation::initial(false);
        assert_eq!(animation.progress,0.0);
        assert!(!animation.hit_enabled());
        assert!(animation.retarget(true,start));
        animation.advance(start+Duration::from_millis(65));
        let partial=animation.progress;
        assert!(partial>0.0 && partial<1.0);
        assert!(!animation.hit_enabled());
        assert!(animation.retarget(false,start+Duration::from_millis(65)));
        assert!((animation.progress-partial).abs()<0.0001,
            "reversal must begin at the actual current interpolation");
        animation.advance(start+Duration::from_millis(300));
        assert_eq!(animation.progress,0.0);
        assert!(!animation.active());
        assert!(!animation.hit_enabled());
        let now=start+Duration::from_millis(300);
        assert!(animation.retarget(true,now));
        animation.advance(now+Duration::from_millis(600));
        assert_eq!(animation.progress,1.0);
        assert!(animation.hit_enabled());
        assert!(!animation.retarget(true,now+Duration::from_millis(601)),
            "snapshot refresh must not replay an already completed animation");
        assert!(animation.retarget(false,now+Duration::from_millis(602)));
        assert!(!animation.hit_enabled(),
            "close must disable secondary hit targets immediately");
        animation.advance(now+Duration::from_millis(1200));
        assert_eq!(animation.progress,0.0);
        assert_eq!(submenu_content_alpha(0.0),0.0);
        assert_eq!(submenu_content_alpha(1.0),1.0);
        assert_eq!(submenu_content_alpha(0.08), 0.0);
        assert_eq!(submenu_content_alpha(0.90), 1.0);
        assert!(submenu_content_alpha(0.50) > 0.0);
        assert!(submenu_content_alpha(0.50) < 1.0);
        assert!(submenu_content_alpha(0.80)>0.0);
    }

    #[test]
    fn compact_desktop_submenu_reveal_and_reverse_stay_continuous() {
        let layout = desktop_submenu_layout();
        assert!(layout.well.left < layout.well.right);
        assert!(layout.foot.left < layout.well.left);
        assert_eq!(submenu_background_alpha(0.0), 0.0);
        assert_eq!(submenu_background_alpha(1.0), 1.0);
        for step in 1..=10 {
            let prev = (step - 1) as f32 / 10.0;
            let next = step as f32 / 10.0;
            assert!(submenu_background_alpha(next) >= submenu_background_alpha(prev));
            assert!(submenu_content_alpha(next) >= submenu_content_alpha(prev));
            assert!(submenu_content_offset(next) <= submenu_content_offset(prev));
        }
        let start = Instant::now();
        let mut state = DesktopSubmenuAnimation::initial(false);
        state.retarget(true, start);
        state.advance(start + Duration::from_millis(100));
        let mid = state.progress;
        assert!(mid > 0.0 && mid < 1.0);
        state.retarget(false, start + Duration::from_millis(100));
        assert!((state.progress - mid).abs() < 0.0001);
        assert_eq!(state.duration_ms,
            (DESKTOP_SUBMENU_CLOSE_DURATION_MS as f32 * mid).round().max(72.0) as u64);
        state.advance(start + Duration::from_millis(400));
        assert_eq!(state.progress, 0.0);
        assert!(!state.hit_enabled());
    }

    #[test]
    fn preview_tooltip_templates_match_runtime_content_in_all_layouts() {
        for (minimal, expected) in [
            (false, "YYYY-MM-DD HH:mm"),
            (true, "5H · YYYY-MM-DD HH:mm"),
        ] {
            let (first_line, second_line) = preview_reset_tooltip_lines(minimal);
            assert_eq!(format!("{first_line} {second_line}"), expected);
            assert_eq!(second_line, "HH:mm");
            assert!(!first_line.ends_with(' '));
        }
        // Both orientations and both display positions call the exact same
        // template helper; dimensions only affect where the break is drawn.
    }

    #[test]
    fn v23_horizontal_minimal_preview_is_content_sized() {
        let canvas = RECT { left: 0, top: 0, right: 200, bottom: 106 };
        for minimal in [false, true] {
            let desktop = horizontal_preview_body(canvas, minimal, true, |v| v);
            let taskbar = horizontal_preview_body(canvas, minimal, false, |v| v);
            let content = RECT { left: taskbar.left + PREVIEW_TASKBAR_GRIP_W, ..taskbar };
            assert_eq!(desktop.right - desktop.left, if minimal { 56 } else { 184 });
            assert_eq!(taskbar.right - taskbar.left, if minimal { 72 } else { 196 });
            assert_eq!(taskbar.right - content.left, if minimal { 56 } else { 180 });
            assert_eq!(taskbar.bottom - taskbar.top, desktop.bottom - desktop.top);
            assert_eq!(taskbar.bottom - taskbar.top, if minimal { 40 } else { 42 });
            for body in [desktop, taskbar] {
                assert!(body.top >= canvas.top);
                assert!(body.bottom + 6 + PREVIEW_HORIZONTAL_TOOLTIP_H <= canvas.bottom);
            }
            let tab = horizontal_preview_detached_tab(desktop, |v| v);
            assert!(tab.top >= canvas.top);
            assert_eq!(desktop.top - tab.bottom, PREVIEW_DRAG_TAB_GAP);
            assert_eq!(tab.bottom - tab.top, PREVIEW_DRAG_TAB_H);
            assert!(tab.left >= desktop.left && tab.right <= desktop.right);
        }
    }

    #[test]
    fn v23_desktop_preview_tab_gaps_scale_with_dpi() {
        let portrait = RECT { left: 0, top: 0, right: 296, bottom: 494 };
        let (body, tab) = desktop_portrait_preview_bounds(portrait, false, |v| v * 2);
        assert_eq!(body.top - tab.bottom, PREVIEW_DRAG_TAB_GAP * 2);
        assert_eq!(tab.bottom - tab.top, PREVIEW_DRAG_TAB_H * 2);
        let landscape = RECT { left: 0, top: 0, right: 400, bottom: 212 };
        let body = horizontal_preview_body(landscape, false, true, |v| v * 2);
        let tab = horizontal_preview_detached_tab(body, |v| v * 2);
        assert_eq!(body.top - tab.bottom, PREVIEW_DRAG_TAB_GAP * 2);
        assert_eq!(tab.bottom - tab.top, PREVIEW_DRAG_TAB_H * 2);
        assert!(body.bottom + 2 * (6 + PREVIEW_HORIZONTAL_TOOLTIP_H) <= landscape.bottom);
    }
    #[test]
    fn portrait_preview_tooltip_is_visible_without_covering_component() {
        let canvas = RECT { left: 0, top: 0, right: 148, bottom: 276 };
        for minimal in [false, true] {
            let widget = portrait_preview_widget_bounds(canvas, minimal, |v| v);
            let popup = portrait_preview_tooltip_rect(canvas, widget, |v| v);
            assert!(popup.top > widget.bottom);
            assert!(popup.bottom <= canvas.bottom);
            assert!(popup.left >= canvas.left && popup.right <= canvas.right);
            assert_eq!(popup.right - popup.left, 128);
            assert_eq!(popup.bottom - popup.top, 40);
        }
    }

    #[test]
    fn portrait_preview_matches_taskbar_aspect_ratio_and_label_clearances() {
        let canvas = RECT { left: 0, top: 0, right: 148, bottom: 276 };
        let widget = portrait_preview_widget_bounds(canvas, false, |v| v);
        assert_eq!(widget.right - widget.left, 50);
        assert_eq!(widget.bottom - widget.top, 168);
        assert_eq!(widget.left, 49);
        // The widget + tooltip move up as one block, reducing unused space.
        assert_eq!(widget.top, 20);
        let popup = portrait_preview_tooltip_rect(canvas, widget, |v| v);
        assert_eq!(popup.top, 195);
        assert!(popup.bottom <= canvas.bottom);
        let first = portrait_preview_row(widget, 0, |v| v);
        let second = portrait_preview_row(widget, 1, |v| v);
        let ring_extent = 17;
        assert!(first.label.bottom < first.center_y - ring_extent);
        assert!(first.center_y + ring_extent < first.reset.top);
        assert!(first.reset.bottom < second.label.top);
        assert!(second.label.bottom < second.center_y - ring_extent);
        assert!(second.center_y + ring_extent < second.reset.top);
        assert!(second.reset.bottom <= widget.bottom);
        let compact = portrait_preview_widget_bounds(canvas, true, |v| v);
        assert_eq!(compact.right - compact.left, 50);
        assert_eq!(compact.bottom - compact.top, 82);
        assert_eq!(compact.left, widget.left);
        assert!(compact.top > widget.top);
        let large_canvas = RECT { left: 0, top: 0, right: 296, bottom: 552 };
        let scaled = portrait_preview_widget_bounds(large_canvas, false, |v| v * 2);
        assert_eq!(scaled.right - scaled.left, 100);
        assert_eq!(scaled.bottom - scaled.top, 336);
        assert_eq!(scaled.top, 40);
        let last = portrait_preview_row(scaled, 1, |v| v * 2);
        assert!(last.reset.bottom <= scaled.bottom);
    }

    #[test]
    fn portrait_preview_ring_has_safe_clearance_for_larger_radius() {
        // The type/reset bands cannot touch the 15px radius + 1.5px stroke.
        let widget = RECT { left: 0, top: 0, right: 50, bottom: 168 };
        let first = portrait_preview_row(widget, 0, |v| v);
        let second = portrait_preview_row(widget, 1, |v| v);
        for row in [first, second] {
            assert!(row.label.bottom < row.center_y - 16);
            assert!(row.center_y + 17 < row.reset.top);
            assert!(row.center_x - 18 >= widget.left);
            assert!(row.center_x + 18 <= widget.right);
        }
        assert!(first.reset.bottom < second.label.top);
        // The 4x4 antialiased renderer retains the 6 o'clock gap at 100%.
        let bg = Color::rgba(18, 21, 25, 255);
        let track = Color::rgba(41, 52, 63, 255);
        let accent = Color::rgba(5, 206, 230, 255);
        let background_pixel = 0x00121519;
        assert_eq!(preview_ring_pixel(18, 34, 36, 15.0, 3.0, 100.0,
            bg, track, accent), background_pixel);
        assert_ne!(preview_ring_pixel(18, 3, 36, 15.0, 3.0, 100.0,
            bg, track, accent), background_pixel);
    }

    #[test]
    fn vertical_preset_preview_ring_has_bottom_gap_and_remaining_quota_palette() {
        let (start, end, progress) = preview_ring_angles(82.0);
        assert!((start - (std::f32::consts::PI + std::f32::consts::PI / 10.0)).abs() < 0.0001);
        assert!((end - (std::f32::consts::PI * 3.0 - std::f32::consts::PI / 10.0)).abs() < 0.0001);
        assert!(progress > start && progress < end);
        assert_eq!(preview_ring_angles(0.0).2, start);
        assert_eq!(preview_ring_angles(100.0).2, end);
        assert_eq!(preview_remaining_color(82.0), StyleColorTarget::ProgressHigh);
        assert_eq!(preview_remaining_color(37.0), StyleColorTarget::ProgressMedium);
        assert_eq!(preview_remaining_color(12.0), StyleColorTarget::ProgressLow);
        // A true 100% ring retains an opaque-free gap at six o'clock.
        let bg = Color::rgba(18, 21, 25, 255);
        let track = Color::rgba(41, 52, 63, 255);
        let accent = Color::rgba(5, 206, 230, 255);
        let bg_pixel = (18u32 << 16) | (21u32 << 8) | 25u32;
        assert_eq!(preview_ring_pixel(18, 31, 36, 13.0, 3.0, 100.0, bg, track, accent), bg_pixel);
        assert_ne!(preview_ring_pixel(18, 5, 36, 13.0, 3.0, 100.0, bg, track, accent), bg_pixel);
        // Supersampling produces partially covered edge pixels, not only
        // abrupt foreground/background transitions.
        let pixels: Vec<u32> = (0..36).flat_map(|y| (0..36).map(move |x|
            preview_ring_pixel(x, y, 36, 13.0, 3.0, 82.0, bg, track, accent)
        )).collect();
        assert!(pixels.iter().any(|p| *p != bg_pixel && *p != 0x0005CEE6 && *p != 0x0029343F));
    }

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
                display_position: DisplayPosition::Taskbar,
                desktop_direction: DesktopDirection::Horizontal,
                desktop_auto_collapse: true,
                vertical_taskbar_mode: false,
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
                        position: "taskbar".into(),
                        desktop_direction: "horizontal".into(),
                        desktop_auto_collapse: true,
                        dark: EditableThemeStyle::from_theme_style(&ThemeStyle::dark_default()),
                        light: EditableThemeStyle::from_theme_style(&ThemeStyle::light_default()),
                    },
                },
            };

            let reopen_snapshot = snapshot.clone();
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
            let json_page_surface = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().unwrap().json_page_surface.to_hwnd()
            };
            assert!(IsWindow(json_page_surface).as_bool());
            for (child, label) in [
                (json_page_surface, "JSON content pane"),
                (json_edit, "RichEdit editor"),
            ] {
                let style = GetWindowLongW(child, GWL_STYLE) as u32;
                assert_ne!(
                    style & WS_CLIPSIBLINGS.0, 0,
                    "{label} must clip overlapping sibling pixels"
                );
            }
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            assert_ne!(
                ex_style & WS_EX_COMPOSITED.0,
                0,
                "the parent must buffer GDI and RichEdit children as one frame"
            );
            for _ in 0..3 {
                set_section(Section::Json);
                assert!(
                    IsWindowVisible(json_page_surface).as_bool(),
                    "opaque JSON content pane must cover the old appearance page"
                );
                assert!(
                    IsWindowVisible(json_edit).as_bool(),
                    "RichEdit must be visible on top of the JSON content pane"
                );
                // A visible RichEdit can still be 100% obscured by a sibling.
                // Ensure RichEdit is the topmost child, not merely WS_VISIBLE.
                let top_child = GetWindow(hwnd, GW_CHILD)
                    .expect("settings window should have a topmost child");
                assert_eq!(
                    top_child, json_edit,
                    "opaque JSON surface must remain below the RichEdit HWND"
                );
                assert!(
                    GetWindowTextLengthW(json_edit) > 0,
                    "JSON RichEdit must retain the generated configuration text"
                );
                // Repainting the opaque pane must not change the z-order.
                let _ = RedrawWindow(
                    json_page_surface, None, HRGN::default(),
                    RDW_INVALIDATE | RDW_UPDATENOW,
                );
                assert_eq!(
                    GetWindow(hwnd, GW_CHILD).expect("top child after repaint"),
                    json_edit,
                    "a page repaint must not obscure RichEdit"
                );
                set_section(Section::Preset);
                assert!(
                    !IsWindowVisible(json_page_surface).as_bool(),
                    "opaque JSON pane must be hidden on preset page"
                );
                assert!(
                    !IsWindowVisible(json_edit).as_bool(),
                    "RichEdit must be hidden whenever the preset page is active"
                );
                let _ = UpdateWindow(hwnd);
            }

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

            // A second opening reuses the HWND and must not discard edits.
            open_or_focus(HWND::default(), reopen_snapshot);
            {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let s = state.as_ref().unwrap();
                assert_eq!(s.hwnd.to_hwnd(), hwnd);
                assert_eq!(s.snapshot.active_style.panel_background, "#12345678");
                assert_eq!(s.section, Section::Panel);
            }

            let (_, _, _, _, show_rgba, show_blur, show_corner) =
                editor_layout_snapshot().unwrap();
            assert!(show_rgba);
            assert!(show_blur);
            assert!(show_corner);

            select_editor(EditorSelection::Blur);
            let (_, _, _, _, show_rgba, show_blur, show_corner) =
                editor_layout_snapshot().unwrap();
            assert!(!show_rgba, "blur selection must clear the lower RGBA editor");
            assert!(show_blur, "blur control must remain visible inline");
            assert!(show_corner, "corner radius input must remain visible inline");

            let _ = DestroyWindow(hwnd);

            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            assert!(state.is_none(), "style settings state must be released on destroy");
        }
    }
}
