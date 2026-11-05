use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use std::{fs, path::PathBuf};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
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
    DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::appearance::{AppearancePreset, DesktopDirection, DisplayPosition};
use crate::fonts;
use crate::localization::LanguageId;
use crate::native_interop::{self, Color, WM_APP};
use crate::window_common::{load_embedded_app_icons, set_window_caption_icons, SendHwnd};
use crate::settings_model::{parse_jsonc, EditableSettings, EditableThemeStyle};
mod preset_preview;
mod json_support;
mod json_scroll;
mod input_events;
mod command_events;
mod timer_events;
mod lifecycle_events;
mod render_primitives;
mod json_paint;
mod preset_gallery_paint;
mod appearance_paint;
mod editor_paint;
mod general_page_paint;
mod navigation_paint;
mod layout_geometry;
mod editor_geometry;
mod json_geometry;
mod color_codec;
use self::preset_preview::*;
use self::json_support::*;
use self::json_scroll::*;
use self::input_events::*;
use self::command_events::*;
use self::timer_events::*;
use self::lifecycle_events::*;
use self::render_primitives::*;
use self::json_paint::*;
use self::preset_gallery_paint::*;
use self::appearance_paint::*;
use self::editor_paint::*;
use self::general_page_paint::*;
use self::navigation_paint::*;
use self::layout_geometry::*;
use self::editor_geometry::*;
use self::json_geometry::*;
pub use self::color_codec::{decode_color_target, unpack_color};
use self::color_codec::{encode_color_target, pack_color, parse_hex_input, target_from_hex_control_id};

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
// Asynchronously refresh the expensive Preset gallery after menu animation,
// never while its top-card frames are being composed.
const WM_SETTINGS_PRESET_GALLERY_REFRESH: u32 = WM_APP + 146;
const WM_SETTINGS_PREWARM_GALLERY: u32 = WM_APP + 147;
const WM_SETTINGS_MULTIMEDIA_ANIMATION_TICK: u32 = WM_APP + 148;
// Interactive paint requests are posted (not generated WM_PAINT messages)
// to prevent low-priority paint starvation under continuous user input.
const WM_SETTINGS_PRESENT_INTERACTIVE_FRAME: u32 = WM_APP + 149;
// WM_TIMER is a low-priority generated message. A periodic multimedia timer
// posts one coalesced message to the UI queue for a steadier ~60Hz cadence.
const DESKTOP_SUBMENU_FRAME_MS: u32 = 16;
const DESKTOP_SUBMENU_MIN_FRAME_MS: u64 = 15;
const DESKTOP_SUBMENU_MEDIA_MIN_FRAME_MS: u64 = 8;
const TIME_PERIODIC_FLAG: u32 = 0x0001;
const TIME_KILL_SYNCHRONOUS_FLAG: u32 = 0x0100;
const DESKTOP_SUBMENU_DURATION_MS: u64 = 220;
const DESKTOP_SUBMENU_CLOSE_DURATION_MS: u64 = 190;
const DESKTOP_SUBMENU_ICON_SIZE: i32 = 18;
const JSON_ACTION_TIMER_ID: usize = 0x4A53;
const JSON_SCROLLBAR_TIMER_ID: usize = 0x4A54;
// Keep the latest drag pointer until the next 60 Hz display interval.
const JSON_SCROLL_DRAG_FRAME_TIMER_ID: usize = 0x4A5A;
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
    #[allow(dead_code)] // Keep SVG icon IDs stable for saved resource maps.
    ToggleOn,
    #[allow(dead_code)]
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
    LanguageOption(usize),
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
    final_frame_pending: bool,
}
impl DesktopSubmenuAnimation {
    fn initial(desktop: bool) -> Self {
        let progress = if desktop { 1.0 } else { 0.0 };
        Self {
            progress, from: progress, target: progress,
            started: Instant::now(), duration_ms: DESKTOP_SUBMENU_DURATION_MS,
            last_frame: None,
            final_frame_pending: false,
        }
    }
    fn active(&self) -> bool {
        (self.progress - self.target).abs() > 0.0001
    }
    fn frame_due_with_gate(&mut self, now: Instant, min_interval_ms: u64) -> bool {
        if !self.active() {
            return false;
        }
        if self.last_frame.is_some_and(|last| {
            now.saturating_duration_since(last) < Duration::from_millis(min_interval_ms)
        }) {
            return false;
        }
        self.last_frame = Some(now);
        true
    }
    fn tick_for_paint(&mut self, now: Instant) -> (bool, bool) {
        self.tick_for_paint_with_gate(now, DESKTOP_SUBMENU_MIN_FRAME_MS)
    }
    fn tick_for_paint_with_gate(
        &mut self, now: Instant, min_interval_ms: u64,
    ) -> (bool, bool) {
        let due = self.frame_due_with_gate(now, min_interval_ms);
        if due {
            self.advance(now);
            if !self.active() {
                // Preserve the cached fast path on the terminal frame: the
                // next WM_PAINT must still use the pre-rendered endpoints.
                self.final_frame_pending = true;
            }
        }
        (self.active(), due)
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
        self.final_frame_pending = false;
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsPaintCause {
    Unknown,
    AnimationTimer,
    PositionSync,
    FullSync,
    GalleryRefresh,
    MouseCaptureChanged,
    MouseReleased,
    MousePressed,
    MouseHover,
}

fn mark_settings_paint_cause(cause: SettingsPaintCause) {
    SETTINGS_PAINT_CAUSE.with(|cell| cell.set(cause));
}

// WM_PAINT is generated only when the thread's queue has no higher-priority
// messages. Persistent widget updates can leave the Settings parent dirty
// without presenting it, while its hit testing continues to work. An
// explicitly posted, coalesced frame calls UpdateWindow on the UI thread.
// Only interactive changes use this path; JSON's own child scroll rendering
// and full-page synchronous redraws retain their existing behavior.
fn settings_interactive_frame_due(last: Option<Instant>, now: Instant, throttle: bool) -> bool {
    !throttle || last.is_none_or(|at| {
        now.saturating_duration_since(at) >= Duration::from_millis(16)
    })
}

fn queue_settings_interactive_frame(hwnd: HWND, throttle: bool) {
    let now = Instant::now();
    let queued = SETTINGS_INTERACTIVE_FRAME_QUEUED.with(|pending| {
        if pending.get() {
            return false;
        }
        let due = SETTINGS_LAST_INTERACTIVE_FRAME.with(|last| {
            settings_interactive_frame_due(last.get(), now, throttle)
        });
        if !due {
            return false;
        }
        pending.set(true);
        true
    });
    if !queued {
        return;
    }
    if unsafe {
        PostMessageW(hwnd, WM_SETTINGS_PRESENT_INTERACTIVE_FRAME, WPARAM(0), LPARAM(0))
    }.is_ok() {
        SETTINGS_LAST_INTERACTIVE_FRAME.with(|last| last.set(Some(now)));
    } else {
        SETTINGS_INTERACTIVE_FRAME_QUEUED.with(|pending| pending.set(false));
    }
}

static SUBMENU_MEDIA_TICK_QUEUED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static SUBMENU_MULTIMEDIA_TIMER_ID: Cell<u32> = const { Cell::new(0) };
    static PRECISE_ANIMATION_TIMER: RefCell<bool> = const { RefCell::new(false) };
    static SETTINGS_PAINT_CAUSE: Cell<SettingsPaintCause> =
        const { Cell::new(SettingsPaintCause::Unknown) };
    static SETTINGS_INTERACTIVE_FRAME_QUEUED: Cell<bool> = const { Cell::new(false) };
    static SETTINGS_LAST_INTERACTIVE_FRAME: Cell<Option<Instant>> =
        const { Cell::new(None) };
}

type MultimediaTimerCallback = unsafe extern "system" fn(u32, u32, usize, usize, usize);

#[link(name = "winmm")]
unsafe extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
    fn timeSetEvent(
        delay: u32, resolution: u32, callback: Option<MultimediaTimerCallback>,
        user: usize, flags: u32,
    ) -> u32;
    fn timeKillEvent(timer_id: u32) -> u32;
}

// The multimedia callback is NOT on the UI thread. Do not touch STATE,
// window DCs, or thread-local rendering caches here; only post a single
// pending animation message. A queued flag prevents unbounded message buildup.
unsafe extern "system" fn submenu_multimedia_timer_callback(
    _timer_id: u32, _message: u32, user: usize, _dw1: usize, _dw2: usize,
) {
    if SUBMENU_MEDIA_TICK_QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    if PostMessageW(
        HWND(user as *mut _), WM_SETTINGS_MULTIMEDIA_ANIMATION_TICK, WPARAM(0), LPARAM(0),
    ).is_err() {
        SUBMENU_MEDIA_TICK_QUEUED.store(false, Ordering::Release);
    }
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

fn begin_submenu_frame_clock(hwnd: HWND) {
    begin_precise_animation_timer();
    let active = SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| cell.get() != 0);
    if active { return; }
    // Windows multimedia periodic timers are more suitable for frame cadence
    // than synthesized low-priority WM_TIMER messages.
    let id = unsafe {
        timeSetEvent(
            DESKTOP_SUBMENU_FRAME_MS, 1, Some(submenu_multimedia_timer_callback),
            hwnd.0 as usize, TIME_PERIODIC_FLAG | TIME_KILL_SYNCHRONOUS_FLAG,
        )
    };
    if id != 0 {
        SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| cell.set(id));
        unsafe { let _ = KillTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID); }
    } else {
        // On constrained or older systems, retain the standard Win32 timer.
        unsafe {
            let _ = SetTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID, DESKTOP_SUBMENU_FRAME_MS, None);
        }
    }
}

fn end_submenu_frame_clock(hwnd: HWND) {
    SUBMENU_MULTIMEDIA_TIMER_ID.with(|cell| {
        let id = cell.replace(0);
        if id != 0 {
            // The synchronous flag guarantees the callback has finished.
            unsafe { let _ = timeKillEvent(id); }
        }
    });
    SUBMENU_MEDIA_TICK_QUEUED.store(false, Ordering::Release);
    unsafe { let _ = KillTimer(hwnd, DESKTOP_SUBMENU_TIMER_ID); }
    end_precise_animation_timer();
}

unsafe fn drive_submenu_frame_tick(hwnd: HWND, multimedia: bool) {
    let now = Instant::now();
    let (active, rendered) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            if multimedia {
                s.desktop_submenu.tick_for_paint_with_gate(
                    now, DESKTOP_SUBMENU_MEDIA_MIN_FRAME_MS,
                )
            } else {
                s.desktop_submenu.tick_for_paint(now)
            }
        } else { (false, false) }
    };
    if !active {
        end_submenu_frame_clock(hwnd);
    }
    if rendered {
        mark_settings_paint_cause(SettingsPaintCause::AnimationTimer);
        let _ = InvalidateRect(hwnd, Some(&settings_card_rect(hwnd, 58, 206)), false);
        queue_settings_interactive_frame(hwnd, active);
    }
}

/// Fixed 96-DPI geometry for an independent inset on the right of the
/// appearance option groups. It never touches the Desktop position button.
#[derive(Clone, Copy, Debug)]
struct DesktopSubmenuLayout {
    well: RECT,
    title: RECT,
    direction_label: RECT,
    collapse_label: RECT,
    horizontal: RECT,
    vertical: RECT,
    collapse_switch: RECT,
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
    preset_gallery_refresh_pending: bool,
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


// Choose the screen from the *visible widget*, not from the stable service
// HWND (which can remain on the original taskbar after replica handoffs).
// The work area excludes taskbar reservations. Negative virtual-desktop
// coordinates are valid for monitors left/above the primary display.


fn settings_work_area_for_widget(widget: HWND) -> Option<RECT> {
    if widget.0.is_null() || !unsafe { IsWindow(widget).as_bool() } {
        return None;
    }
    unsafe {
        let monitor = MonitorFromWindow(widget, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).as_bool().then_some(info.rcWork)
    }
}

pub fn open_or_focus(parent: HWND, snapshot: StyleWindowSnapshot) {
    // The owner is a stable message endpoint, which can be on a different
    // monitor from the live component after a taskbar replica handoff.
    let widget = crate::window::settings_placement_anchor().unwrap_or(parent);
    open_or_focus_on_widget(parent, widget, snapshot);
}

/// The message owner stays stable while taskbar replicas and the desktop
/// widget can move among monitors. Use the current widget only as an anchor.
pub fn open_or_focus_on_widget(parent: HWND, widget: HWND, snapshot: StyleWindowSnapshot) {
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
        // Other settings pages use whole-window composition; the JSON page
        // disables it after its buffered transition to avoid RichEdit latency.
        let widget_work_area = settings_work_area_for_widget(widget);
        let hwnd = match CreateWindowExW(
            settings_window_ex_style(),
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

        set_window_caption_icons(hwnd, large_icon, small_icon);

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
                preset_gallery_refresh_pending: false,
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
    let (hwnd, gallery_changed) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else { return };
        if s.snapshot.vertical_taskbar_mode == vertical { return; }
        let previous = preview_vertical_for_snapshot(&s.snapshot);
        s.snapshot.vertical_taskbar_mode = vertical;
        let gallery_changed = s.section == Section::Preset
            && previous != preview_vertical_for_snapshot(&s.snapshot);
        if gallery_changed { s.preset_gallery_refresh_pending = true; }
        (s.hwnd.to_hwnd(), gallery_changed)
    };
    if gallery_changed {
        // The parent calls this during position changes before sync(). Posting
        // lets sync() start its animation before the gallery is invalidated.
        unsafe {
            let _ = PostMessageW(hwnd, WM_SETTINGS_PRESET_GALLERY_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

// Persisted position is mirrored both in display_position and JSON settings.
// Normalize both fields before deciding whether other UI sections changed.
fn position_is_only_snapshot_change(
    old: &StyleWindowSnapshot, new: &StyleWindowSnapshot,
) -> bool {
    if old.display_position == new.display_position {
        return false;
    }
    let mut normalized = new.editable_settings.clone();
    normalized.appearance.position.clone_from(&old.editable_settings.appearance.position);
    old.language == new.language
        && old.theme_mode == new.theme_mode
        && old.is_dark == new.is_dark
        && old.appearance_preset == new.appearance_preset
        && old.desktop_direction == new.desktop_direction
        && old.desktop_auto_collapse == new.desktop_auto_collapse
        // Vertical taskbar preview orientation only affects the lower Preset
        // gallery, which is refreshed separately after the card animation.
        && old.active_style == new.active_style
        && old.editable_settings == normalized
}

pub fn sync(snapshot: StyleWindowSnapshot) {
    let (hwnd, section, json_dirty, json_apply_pending, submenu_animated, card_only, gallery_changed) = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        let changed_position = s.snapshot.display_position != snapshot.display_position;
        let previous_preview = preview_vertical_for_snapshot(&s.snapshot);
        let gallery_changed = s.section == Section::Preset
            && (changed_position
                || previous_preview != preview_vertical_for_snapshot(&snapshot));
        if gallery_changed { s.preset_gallery_refresh_pending = true; }
        let card_only = is_appearance_section(s.section)
            && position_is_only_snapshot_change(&s.snapshot, &snapshot);
        let submenu_animated = if changed_position {
            s.desktop_submenu.retarget(
                snapshot.display_position == DisplayPosition::Desktop,
                Instant::now(),
            )
        } else { false };
        s.snapshot = snapshot;
        if !card_only {
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
        }
        (
            s.hwnd.to_hwnd(),
            s.section,
            s.json_dirty,
            s.pending_json_action == Some(JsonAction::Apply),
            submenu_animated,
            card_only,
            gallery_changed,
        )
    };
    if gallery_changed {
        unsafe {
            let _ = PostMessageW(hwnd, WM_SETTINGS_PRESET_GALLERY_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
    if submenu_animated {
        // Retargeting mid-animation reuses the same periodic timer.
        begin_submenu_frame_clock(hwnd);
    }
    if card_only {
        mark_settings_paint_cause(SettingsPaintCause::PositionSync);
        unsafe { invalidate_appearance_card_or_window(hwnd, section, true); }
        return;
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
    mark_settings_paint_cause(SettingsPaintCause::FullSync);
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

const SETTINGS_CONTENT_LEFT: i32 = 200;
const SETTINGS_CONTENT_RIGHT: i32 = 940;
const SETTINGS_CARD_RADIUS: i32 = 10;
const SETTINGS_CARD_INSET_X: i32 = 18;
const SETTINGS_CHOICE_LABEL_RIGHT: i32 = 338;
const SETTINGS_EDITOR_HEIGHT: i32 = 164;
const SETTINGS_EDITOR_CHANNEL_GAP: i32 = 30;
















fn settings_window_ex_style() -> WINDOW_EX_STYLE {
    WS_EX_APPWINDOW | WS_EX_COMPOSITED
}

// Keep other settings pages buffered. JSON has its own opaque child surface,
// so compositing its RichEdit subtree adds latency without visual benefit.
fn settings_composition_for_section(section: Section) -> bool {
    section != Section::Json
}

fn sync_settings_window_composition(hwnd: HWND, section: Section) {
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
















// Read the edit's actual selected font, not an assumed 96-DPI line height.
// This also handles a popup moved between monitors with different DPI and
// Windows font metrics without relying on EM_SETRECT (multiline-only).
unsafe fn single_line_edit_font_height(edit: HWND, fallback: i32) -> i32 {
    let dc = GetDC(edit);
    if dc.0.is_null() {
        return fallback.max(1);
    }
    let selected = SendMessageW(edit, WM_GETFONT, WPARAM(0), LPARAM(0));
    let old_font = if selected.0 != 0 {
        Some(SelectObject(dc, HGDIOBJ(selected.0 as *mut _)))
    } else {
        None
    };
    let mut metrics = TEXTMETRICW::default();
    let measured = GetTextMetricsW(dc, &mut metrics).as_bool();
    if let Some(old_font) = old_font {
        let _ = SelectObject(dc, old_font);
    }
    let _ = ReleaseDC(edit, dc);
    if measured && metrics.tmHeight > 0 {
        metrics.tmHeight.saturating_add(metrics.tmExternalLeading.max(0))
    } else {
        fallback.max(1)
    }
}



// Appearance-option hover, press, and position changes are confined to the
// shared top card; avoid invalidating every expensive settings page element.




unsafe fn invalidate_appearance_card_or_window(
    hwnd: HWND, section: Section, card_only: bool,
) {
    if is_appearance_section(section) && card_only {
        let _ = InvalidateRect(hwnd, Some(&settings_card_rect(hwnd, 58, 206)), false);
    } else {
        let _ = InvalidateRect(hwnd, None, false);
    }
    if section != Section::Json {
        queue_settings_interactive_frame(hwnd, false);
    }
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
        // All numeric/blur/corner children use the same UI font. Measure once
        // per relayout, then center each native single-line EDIT in its frame.
        let font_height = single_line_edit_font_height(numeric_edits[0].to_hwnd(), scale(hwnd, 14));
        let text_padding = scale(hwnd, 4);
        for (index, edit) in numeric_edits.iter().enumerate() {
            let r = centered_single_line_edit_rect(
                numeric_edit_rect(hwnd, section, index), font_height, text_padding,
            );
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

        let blur_rect = centered_single_line_edit_rect(
            blur_edit_rect(hwnd, section), font_height, text_padding,
        );
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

        let corner_rect = centered_single_line_edit_rect(
            corner_edit_rect(hwnd), font_height, text_padding,
        );
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
        WM_MOUSEWHEEL => {
            // If Windows delivers a wheel message directly to the opaque
            // JSON pane, use the same parent routing as wheel messages that
            // hit the empty scrollbar strip.
            let owner = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.hwnd.to_hwnd())
            };
            if let Some(owner) = owner {
                SendMessageW(owner, msg, wparam, lparam)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_PAINT => {
            paint_json_page_surface(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
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
            invalidate_json_scroll_layout_cache();
            let _ = SetTimer(hwnd, JSON_SCROLLBAR_TIMER_ID, 120, None);
        } else {
            let _ = KillTimer(hwnd, JSON_SCROLLBAR_TIMER_ID);
            invalidate_json_scroll_layout_cache();
        }
    }
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
        // The dialog was dismissed without selecting a file. The pending
        // action has already ended; do not leave a stale "Importing" status.
        set_json_status(String::new());
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
        // Discard the temporary "Exporting" status on file-dialog dismissal.
        set_json_status(String::new());
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
        let font_height = single_line_edit_font_height(hex_edits[0].to_hwnd(), scale(hwnd, 14));
        let text_padding = scale(hwnd, 4);
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
                let r = centered_single_line_edit_rect(
                    hex_edit_rect(hwnd, row_index), font_height, text_padding,
                );
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
        // Panel blur previews invoke render_layered() in the parent window,
        // unlike Tooltip blur previews. The native EDIT has already delivered
        // EN_CHANGE and the model is correct, but a normal invalidation can
        // leave both the focused glyphs and slider visually stale while the
        // composited Settings parent continues handling updates.
        //
        // Flush the EDIT separately and post a coalesced Settings presentation
        // after this command finishes. Do NOT call sync_blur_edit() here: that
        // would rewrite partially typed text and reset the caret/selection.
        // No STATE mutex may be held during these synchronous Win32 calls.
        let _ = RedrawWindow(
            edit, None, HRGN::default(),
            RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
        );
        let _ = InvalidateRect(hwnd, None, false);
        queue_settings_interactive_frame(hwnd, false);
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

    let (section, json_dirty, json_status_path, language, json_action_pending) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let s = state.as_ref()?;
        (s.section, s.json_dirty, s.json_status_path.is_some(),
            s.snapshot.language, s.pending_json_action.is_some())
    };

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
        let submenu = {
            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            state.as_ref().and_then(|s| (
                s.snapshot.display_position == DisplayPosition::Desktop
                    && s.desktop_submenu.hit_enabled()
            ).then_some(s.snapshot.desktop_auto_collapse))
        };
        if let Some(collapsed) = submenu {
            if pt_in_rect(desktop_auto_collapse_rect(hwnd), x, y) {
                return Some(HitTarget::DesktopAutoCollapse(!collapsed));
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
        for index in 0..language_option_count() {
            if pt_in_rect(language_option_rect(hwnd, index), x, y) {
                return Some(HitTarget::LanguageOption(index));
            }
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
        HitTarget::LanguageOption(index) => {
            {
                let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(s) = state.as_mut() {
                    s.snapshot.editable_settings.general.language = language_code_for_index(index);
                }
            }
            send_parent(WM_SETTINGS_LANGUAGE_CHANGE, index, 0);
        }
    }
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
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
    // Restore buffering before hiding JSON and drawing a non-JSON page.
    // Entering JSON takes the inverse path: the opaque pane and RichEdit
    // are first shown together, then composition is disabled after painting.
    if current_section == Section::Json && section != Section::Json {
        sync_settings_window_composition(hwnd, section);
    }
    layout_numeric_edits(hwnd);
    layout_hex_edits(hwnd);
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
    // Finish the one-time page transition while composited, preventing
    // remnants from the previous page beneath the RichEdit sibling.
    redraw_settings_window(hwnd);
    if section == Section::Json {
        sync_settings_window_composition(hwnd, section);
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
        // WM_SETTEXT synchronously triggers EN_CHANGE. Blur's handler already
        // updates state, redraws EDIT and queues a slider frame. Other numeric
        // controls keep their existing synchronization and repaint behavior.
        set_edit_text(edit, next);
        match target {
            WheelNumericTarget::Color(_) => sync_numeric_edits(),
            WheelNumericTarget::Blur => return,
            WheelNumericTarget::Corner => sync_corner_edit(),
        }
        unsafe {
            let _ = RedrawWindow(
                edit, None, HRGN::default(),
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );
        }
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
    // A drag can keep the message queue nonempty, starving generated WM_PAINT.
    // Cap active drag presentation at ~60 fps, then flush the final value on
    // WM_LBUTTONUP via invalidate_appearance_card_or_window.
    queue_settings_interactive_frame(hwnd, true);
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
        x if is_settings_scheduled_message(x) => {
            handle_settings_scheduled_message(hwnd, msg, wparam, lparam)
        }
        x if is_settings_input_message(x) => {
            handle_settings_input_message(hwnd, msg, wparam, lparam)
        }
        WM_COMMAND => handle_settings_command(hwnd, msg, wparam, lparam),
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
        x if is_settings_lifecycle_message(x) => {
            handle_settings_lifecycle_message(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// Reuse the offscreen DC and bitmap between WM_PAINT frames. Previously
// every 16 ms tick allocated and released a full-window compatible bitmap.
struct SettingsPaintBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
    // Only reuse the retained pixels outside the card when they were painted
    // for the current section; a new bitmap contains undefined pixels.
    painted_section: Cell<Option<Section>>,
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
        Some(Self { dc, bitmap, previous_bitmap, width, height,
            painted_section: Cell::new(None) })
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
            direction: snapshot.desktop_direction,
            collapse: snapshot.desktop_auto_collapse,
        }
    }
}

struct SubmenuCardFrames {
    key: SubmenuCardKey,
    closed: SettingsPaintBuffer,
    open: SettingsPaintBuffer,
}

// The preview gallery is expensive to rasterize (ring anti-aliasing and
// composited thumbnails). Pre-render both position variants outside the
// animation and copy the correct view when the deferred gallery refresh runs.
#[derive(Clone, PartialEq, Eq)]
struct PresetGalleryKey {
    width: i32,
    height: i32,
    dpi: u32,
    dark: bool,
    zh: bool,
    layout: AppearancePreset,
    desktop_direction: DesktopDirection,
    taskbar_vertical: bool,
    active_style: ThemeStyle,
}

impl PresetGalleryKey {
    fn new(hwnd: HWND, width: i32, height: i32, snapshot: &StyleWindowSnapshot) -> Self {
        Self {
            width, height, dpi: unsafe { GetDpiForWindow(hwnd) },
            dark: snapshot.is_dark,
            zh: snapshot.language == LanguageId::SimplifiedChinese,
            layout: snapshot.appearance_preset,
            desktop_direction: snapshot.desktop_direction,
            taskbar_vertical: snapshot.vertical_taskbar_mode,
            active_style: snapshot.active_style.clone(),
        }
    }
}

struct PresetGalleryFrames {
    key: PresetGalleryKey,
    taskbar: SettingsPaintBuffer,
    desktop: SettingsPaintBuffer,
}



fn preset_gallery_cache_ready(
    hwnd: HWND, width: i32, height: i32, snapshot: &StyleWindowSnapshot,
) -> bool {
    let key = PresetGalleryKey::new(hwnd, width, height, snapshot);
    PRESET_GALLERY_FRAMES.with(|cell| {
        cell.borrow().as_ref().is_some_and(|frames| frames.key == key)
    })
}

unsafe fn build_preset_gallery_cache(
    screen: HDC, hwnd: HWND, width: i32, height: i32,
    snapshot: &StyleWindowSnapshot, font: isize,
) -> bool {
    let key = PresetGalleryKey::new(hwnd, width, height, snapshot);
    PRESET_GALLERY_FRAMES.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.as_ref().is_some_and(|frames| frames.key == key) {
            return true;
        }
        let Some(taskbar) = SettingsPaintBuffer::create(screen, width, height) else {
            return false;
        };
        let Some(desktop) = SettingsPaintBuffer::create(screen, width, height) else {
            return false;
        };
        let background = if snapshot.is_dark {
            Color::from_hex("#1F2125FF")
        } else {
            Color::from_hex("#E9EEF4FF")
        };
        let card = if snapshot.is_dark {
            Color::from_hex("#292C31FF")
        } else { Color::from_hex("#FFFFFFFF") };
        let hover = if snapshot.is_dark {
            Color::from_hex("#343840FF")
        } else { Color::from_hex("#DCE5EFFF") };
        let pressed = if snapshot.is_dark {
            Color::from_hex("#414751FF")
        } else { Color::from_hex("#CBD7E4FF") };
        let border = if snapshot.is_dark {
            Color::from_hex("#454A52FF")
        } else { Color::from_hex("#C1CCD8FF") };
        let primary = if snapshot.is_dark {
            Color::from_hex("#F2F3F5FF")
        } else { Color::from_hex("#202124FF") };
        let palette = || PresetGalleryPalette {
            card, card_hover: hover, card_pressed: pressed,
            border, accent: Color::from_hex("#4C8DFFFF"), primary,
        };
        for (frame, position) in [
            (&taskbar, DisplayPosition::Taskbar),
            (&desktop, DisplayPosition::Desktop),
        ] {
            let mut view = snapshot.clone();
            view.display_position = position;
            let region = preset_gallery_region(hwnd, width, height);
            fill(frame.dc, region, background);
            let previous = SelectObject(frame.dc, HGDIOBJ(font as *mut _));
            let _ = SetBkMode(frame.dc, TRANSPARENT);
            paint_preset_gallery(frame.dc, hwnd, &view, None, None, palette());
            SelectObject(frame.dc, previous);
        }
        *cache = Some(PresetGalleryFrames { key, taskbar, desktop });
        true
    })
}

fn request_preset_gallery_warmup(hwnd: HWND) {
    let already_requested = SETTINGS_GALLERY_WARM_REQUESTED.with(|cell| {
        cell.replace(true)
    });
    if !already_requested
        && unsafe {
            PostMessageW(hwnd, WM_SETTINGS_PREWARM_GALLERY, WPARAM(0), LPARAM(0))
        }.is_err()
    {
        SETTINGS_GALLERY_WARM_REQUESTED.with(|cell| cell.set(false));
    }
}

unsafe fn copy_cached_preset_gallery(
    target: HDC, hwnd: HWND, width: i32, height: i32,
    snapshot: &StyleWindowSnapshot, dirty: RECT,
) -> bool {
    let key = PresetGalleryKey::new(hwnd, width, height, snapshot);
    PRESET_GALLERY_FRAMES.with(|cell| {
        let cache = cell.borrow();
        let Some(frames) = cache.as_ref().filter(|frames| frames.key == key) else {
            return false;
        };
        let source = if snapshot.display_position == DisplayPosition::Desktop {
            &frames.desktop
        } else { &frames.taskbar };
        BitBlt(target, dirty.left, dirty.top,
            dirty.right - dirty.left, dirty.bottom - dirty.top,
            source.dc, dirty.left, dirty.top, SRCCOPY).is_ok()
    })
}

thread_local! {
    static SETTINGS_PAINT_BUFFER: RefCell<Option<SettingsPaintBuffer>> =
        const { RefCell::new(None) };
    static SUBMENU_CARD_FRAMES: RefCell<Option<SubmenuCardFrames>> =
        const { RefCell::new(None) };
    static PRESET_GALLERY_FRAMES: RefCell<Option<PresetGalleryFrames>> =
        const { RefCell::new(None) };
    static SETTINGS_GALLERY_REFRESH: Cell<bool> = const { Cell::new(false) };
    static SETTINGS_GALLERY_WARM_REQUESTED: Cell<bool> = const { Cell::new(false) };
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
        if copied.is_err() { return false; }
        let opacity = (progress.clamp(0.0, 1.0) * 255.0).round() as u8;
        opacity == 0 || native_interop::alpha_blend_cached_card(
            dest, frames.open.dc, card_bounds, opacity)
    })
}

fn finish_submenu_terminal_paint(hwnd: HWND) {
    let needs_gallery = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else { return };
        s.desktop_submenu.final_frame_pending = false;
        s.section == Section::Preset && s.preset_gallery_refresh_pending
    };
    if needs_gallery {
        unsafe {
            let _ = PostMessageW(
                hwnd, WM_SETTINGS_PRESET_GALLERY_REFRESH, WPARAM(0), LPARAM(0),
            );
        }
    }
}

// A position-only sync can have its card rect coalesced with a prior
// invalid region by Win32. The rest of the retained backing bitmap is still
// correct for this section; avoid a full page redraw solely because rcPaint
// extends beyond the animated card. Never reuse uninitialized or other-page
// pixels, or a genuine gallery/other-settings refresh.
fn can_reuse_coalesced_position_paint(
    section: Section,
    painted_section: Option<Section>,
    cause: SettingsPaintCause,
    submenu_only: bool,
    animating: bool,
    gallery_refresh: bool,
) -> bool {
    !submenu_only
        && animating
        && !gallery_refresh
        && is_appearance_section(section)
        && painted_section == Some(section)
        && cause == SettingsPaintCause::PositionSync
}

unsafe fn paint(hwnd: HWND) {
    let paint_cause = SETTINGS_PAINT_CAUSE.with(|cell| cell.replace(SettingsPaintCause::Unknown));
    let gallery_refresh = SETTINGS_GALLERY_REFRESH.with(|cell| cell.replace(false));
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
        desktop_submenu_final_frame_pending,
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
            s.desktop_submenu.final_frame_pending,
        )
    };
    // WM_TIMER dirties exactly the shared top card. Keep all drawing outside
    // that card intact in the cached buffer, skipping expensive preset previews.
    let dirty = ps.rcPaint;
    let card_bounds = settings_card_rect(hwnd, 58, 206);
    let submenu_only = is_appearance_section(section)
        && dirty.left >= card_bounds.left && dirty.top >= card_bounds.top
        && dirty.right <= card_bounds.right && dirty.bottom <= card_bounds.bottom
        && dirty.left < dirty.right && dirty.top < dirty.bottom;

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

    // Deferred Preset-gallery refresh must not trigger a full-page paint.
    // Both position variants were rasterized while the settings page was idle.
    let gallery_bounds = preset_gallery_region(hwnd, width, height);
    let gallery_only = section == Section::Preset
        && dirty.left >= gallery_bounds.left && dirty.top >= gallery_bounds.top
        && dirty.right <= gallery_bounds.right && dirty.bottom <= gallery_bounds.bottom
        && dirty.left < dirty.right && dirty.top < dirty.bottom;
    let preview_interacting = matches!(hovered, Some(HitTarget::Preset(_)))
        || matches!(pressed, Some(HitTarget::Preset(_)));
    if gallery_refresh && gallery_only && !desktop_submenu_active
        && !desktop_submenu_final_frame_pending && !discard_pending
        && !preview_interacting
        && copy_cached_preset_gallery(hdc, hwnd, width, height, &snapshot, dirty)
    {
        let _ = BitBlt(screen_hdc, dirty.left, dirty.top,
            dirty.right - dirty.left, dirty.bottom - dirty.top,
            hdc, dirty.left, dirty.top, SRCCOPY);
        let _ = EndPaint(hwnd, &ps);
        return;
    }

    // Keep the non-card pixels from the already painted backbuffer if Win32
    // merges a pure position-sync invalidation with a larger pending region.
    // The lower Preset gallery has its own deferred refresh.
    let coalesced_position = can_reuse_coalesced_position_paint(
        section, backbuffer.painted_section.get(), paint_cause, submenu_only,
        desktop_submenu_active, gallery_refresh,
    );
    // Animation normally touches only the shared card; both the exact card
    // invalidation and a safe coalesced position-only invalidation use blits.
    if (submenu_only || coalesced_position)
        && (desktop_submenu_active || desktop_submenu_final_frame_pending)
        && !discard_pending && composite_cached_submenu_card(
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
        if desktop_submenu_final_frame_pending {
            finish_submenu_terminal_paint(hwnd);
        }
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
    // Prepare both endpoint images while the page is idle. The next click
    // can then start the animation without blocking on SVG/Direct2D rendering.
    // This runs after presenting the normal frame, so cache preparation only
    // touches the offscreen DC and cannot overwrite what the user sees.
    if !submenu_only && !desktop_submenu_active && !discard_pending &&
        !matches!(section, Section::General | Section::Json)
    {
        let cache_key = SubmenuCardKey::new(hwnd, width, height, &snapshot);
        let needs_warmup = SUBMENU_CARD_FRAMES.with(|cell| {
            cell.borrow().as_ref().is_none_or(|f| f.key != cache_key)
        });
        if needs_warmup {
            let _ = composite_cached_submenu_card(
                screen_hdc, hdc, hwnd, width, height, &snapshot, section, editor,
                font, card, card_hover, card_pressed, track_background,
                accent, accent_hover, accent_pressed, primary, secondary,
                desktop_submenu_progress,
            );
        }
    }
    // Prepare gallery variants via a deferred idle message. No costly
    // gallery rasterization is performed from the 60Hz animation paint path.
    if section == Section::Preset && !desktop_submenu_active
        && !desktop_submenu_final_frame_pending && !discard_pending
        && !preset_gallery_cache_ready(hwnd, width, height, &snapshot)
    {
        request_preset_gallery_warmup(hwnd);
    }
    // Only a full draw establishes valid retained pixels for this page;
    // coalesced partial paint reuse requires this exact painted section.
    if !submenu_only {
        backbuffer.painted_section.set(Some(section));
    }
    let _ = EndPaint(hwnd, &ps);
    if desktop_submenu_final_frame_pending {
        finish_submenu_terminal_paint(hwnd);
    }
    });
}


#[cfg(test)]
mod ui_smoke_tests {
    use super::*;
    #[test]
    fn single_line_edits_stay_vertically_centered_across_font_metrics_and_dpi() {
        // Pixel-space layout works equally for 96, 144 and 192 DPI.
        for (top, bottom, font_height, padding) in [
            (0, 24, 17, 4),
            (8, 38, 21, 5),
            (12, 48, 29, 6),
            (20, 68, 43, 8),
            (0, 20, 25, 4), // Oversized fonts must not exceed the frame.
        ] {
            let bounds = RECT { left: 11, top, right: 123, bottom };
            let actual = centered_single_line_edit_rect(bounds, font_height, padding);
            assert_eq!((actual.left, actual.right), (bounds.left, bounds.right));
            assert!(actual.top >= bounds.top && actual.bottom <= bounds.bottom);
            assert!(
                ((actual.top - bounds.top) - (bounds.bottom - actual.bottom)).abs() <= 1,
                "native edit window must be centered to within one pixel",
            );
        }
        let zero = RECT { left: 0, top: 10, right: 50, bottom: 10 };
        assert_eq!(centered_single_line_edit_rect(zero, 14, 4).top, 10);
        assert_eq!(centered_single_line_edit_rect(zero, 14, 4).bottom, 10);
    }

    #[test]
    fn appearance_submenu_color_mix_clamps_and_preserves_rgba_endpoints() {
        let a = Color::rgba(12, 34, 56, 78);
        let b = Color::rgba(210, 180, 150, 120);
        let rgba = |v: Color| (v.r, v.g, v.b, v.a);
        assert_eq!(rgba(submenu_mix(a, b, -0.5)), rgba(a));
        assert_eq!(rgba(submenu_mix(a, b, 0.0)), rgba(a));
        assert_eq!(rgba(submenu_mix(a, b, 1.0)), rgba(b));
        assert_eq!(rgba(submenu_mix(a, b, 1.5)), rgba(b));
        let midpoint = submenu_mix(a, b, 0.5);
        assert_eq!(rgba(midpoint), (111, 107, 103, 99));
    }

    #[test]
    fn settings_coordinate_helpers_keep_signed_points_and_half_open_hits() {
        let hwnd = HWND::default();
        assert_eq!(window_dpi(hwnd), 96);
        assert_eq!(scale(hwnd, 44), 44);
        // Retain the existing signed-division rounding, while testing hit
        // rectangle boundaries with nonnegative screen-space coordinates.
        assert_eq!(scale(hwnd, -12), -11);
        let region = rect(hwnd, 12, 5, 32, 25);
        assert_eq!((region.left, region.top, region.right, region.bottom), (12, 5, 32, 25));
        let packed = |x: i16, y: i16| {
            LPARAM((((y as u16 as u32) << 16) | (x as u16 as u32)) as isize)
        };
        assert_eq!(point_from_lparam(packed(-50, 320)), (-50, 320));
        assert_eq!(point_from_lparam(packed(210, -12)), (210, -12));
        assert!(pt_in_rect(region, 12, 5));
        assert!(pt_in_rect(region, 31, 24));
        assert!(!pt_in_rect(region, 32, 24));
        assert!(!pt_in_rect(region, 31, 25));
    }

    #[test]
    fn settings_relocated_helpers_preserve_original_boundaries_and_status() {
        let screen = RECT { left: -40, top: -30, right: 60, bottom: 70 };
        assert_eq!(settings_centered_origin(screen, 20, 20), (0, 10));
        assert_eq!(settings_centered_origin(screen, 200, 200), (-40, -30));

        let track = RECT { left: -20, top: 0, right: 80, bottom: 12 };
        assert_eq!(slider_value_from_x(track, -25, 100), 0);
        assert_eq!(slider_value_from_x(track, 30, 100), 50);
        assert_eq!(slider_value_from_x(track, 150, 100), 100);

        assert!(json_editor_should_be_visible(Section::Json, false));
        assert!(!json_editor_should_be_visible(Section::Json, true));
        assert!(!json_editor_should_be_visible(Section::General, false));
        assert_eq!(
            json_action_running_text(JsonAction::Import, LanguageId::SimplifiedChinese),
            "◌ 导入中"
        );
        assert_eq!(
            json_action_running_text(JsonAction::Export, LanguageId::English),
            "◌ Exporting"
        );
        assert!(is_appearance_section(Section::Panel));
        assert!(!is_appearance_section(Section::Json));
        assert!(card_only_hover_change(
            Section::Panel,
            Some(HitTarget::Theme(ThemeMode::Dark)),
            Some(HitTarget::Layout(AppearancePreset::Minimal)),
        ));
        assert!(!card_only_hover_change(
            Section::Json,
            Some(HitTarget::Theme(ThemeMode::Dark)),
            Some(HitTarget::Layout(AppearancePreset::Minimal)),
        ));
    }

    #[test]
    fn hex_input_helpers_keep_partial_and_full_color_behavior() {
        assert!(parse_hex_input("#").unwrap().is_none());
        assert!(parse_hex_input("ABCD").unwrap().is_none());
        assert_eq!(parse_hex_input("#010203").unwrap().map(|c| (c.r,c.g,c.b,c.a)), Some((1,2,3,255)));
        assert!(parse_hex_input("#123456789").is_err());
        assert!(parse_hex_input("nocolor").is_err());
        assert_eq!(target_from_hex_control_id(ID_EDIT_HEX_BASE), Some(StyleColorTarget::PanelBackground));
        assert_eq!(target_from_hex_control_id(ID_EDIT_HEX_BASE + 12), Some(StyleColorTarget::TooltipBorder));
        assert_eq!(target_from_hex_control_id(ID_EDIT_HEX_BASE - 1), None);
    }

    #[test]
    fn settings_color_codec_round_trips_all_targets_and_rgba_channels() {
        for (index, target) in COLOR_TARGETS.iter().copied().enumerate() {
            assert_eq!(decode_color_target(index), Some(target));
            assert_eq!(encode_color_target(target), index);
        }
        assert_eq!(decode_color_target(COLOR_TARGETS.len()), None);
        assert_eq!(decode_color_target(usize::MAX), None);
        for (r, g, b, a) in [(0, 0, 0, 0), (255, 255, 255, 255), (1, 42, 128, 250)] {
            let original = Color::rgba(r, g, b, a);
            let decoded = unpack_color(pack_color(original));
            assert_eq!((decoded.r, decoded.g, decoded.b, decoded.a), (r, g, b, a));
        }
    }

    #[test]
    fn settings_icons_preserve_theme_layout_preset_and_json_mappings() {
        assert_eq!(theme_icon(ThemeMode::System), SettingsIcon::ThemeSystem);
        assert_eq!(theme_icon(ThemeMode::Dark), SettingsIcon::ThemeDark);
        assert_eq!(theme_icon(ThemeMode::Light), SettingsIcon::ThemeLight);
        assert_eq!(layout_icon(AppearancePreset::Default), SettingsIcon::LayoutDefault);
        assert_eq!(layout_icon(AppearancePreset::Minimal), SettingsIcon::LayoutMinimal);
        for (preset, dark_icon, light_icon) in [
            (ThemePreset::Classic, SettingsIcon::PresetClassic, SettingsIcon::PresetCloud),
            (ThemePreset::Ocean, SettingsIcon::PresetOcean, SettingsIcon::PresetBay),
            (ThemePreset::Forest, SettingsIcon::PresetForest, SettingsIcon::PresetWheat),
        ] {
            assert_eq!(preset_icon(preset, true), dark_icon);
            assert_eq!(preset_icon(preset, false), light_icon);
        }
        for (action, icon) in [
            (JsonAction::Reload, SettingsIcon::JsonReload),
            (JsonAction::Format, SettingsIcon::JsonFormat),
            (JsonAction::Import, SettingsIcon::JsonImport),
            (JsonAction::Export, SettingsIcon::JsonExport),
            (JsonAction::Apply, SettingsIcon::JsonApply),
        ] {
            assert_eq!(json_action_icon(action), icon);
        }
    }

    #[test]
    fn editor_labels_preserve_localized_frosted_and_progress_values() {
        for (row, zh, en) in [
            (EditorSelection::Blur, "磨砂强度", "Frosted intensity"),
            (EditorSelection::TooltipBlur, "磨砂强度", "Frosted intensity"),
            (EditorSelection::CornerRadius, "圆角半径", "Corner radius"),
            (EditorSelection::Color(StyleColorTarget::ProgressConsumed), "已消耗部分", "Consumed"),
        ] {
            assert_eq!(row_label(row, LanguageId::SimplifiedChinese), zh);
            assert_eq!(row_label(row, LanguageId::English), en);
        }
    }

    #[test]
    fn preset_gallery_retains_vertical_and_horizontal_card_sizes() {
        let hwnd = HWND::default();
        let full = preset_card_rect_for_orientation(hwnd, ThemePreset::Classic, true, false);
        let compact = preset_card_rect_for_orientation(hwnd, ThemePreset::Classic, true, true);
        let horizontal = preset_card_rect_for_orientation(hwnd, ThemePreset::Classic, false, false);
        assert_eq!(full.bottom - full.top, PRESET_CARD_HEIGHT);
        assert_eq!(compact.bottom - compact.top, PRESET_CARD_MINIMAL_HEIGHT);
        assert_eq!(full.top, PRESET_CARD_TOP);
        assert_eq!(full.left, compact.left);
        assert_eq!(horizontal.bottom - horizontal.top, 184);
    }

    #[test]
    fn preset_gallery_labels_keep_each_theme_and_language_mapping() {
        use LanguageId::{English, SimplifiedChinese};
        for (preset, dark_zh, dark_en, light_zh, light_en) in [
            (ThemePreset::Classic, "石墨", "Graphite", "云瓷", "Cloud Porcelain"),
            (ThemePreset::Ocean, "深海", "Deep Sea", "晴湾", "Clear Bay"),
            (ThemePreset::Forest, "松影", "Pine Shade", "麦光", "Wheat Glow"),
        ] {
            assert_eq!(preset_label(preset, true, SimplifiedChinese), dark_zh);
            assert_eq!(preset_label(preset, true, English), dark_en);
            assert_eq!(preset_label(preset, false, SimplifiedChinese), light_zh);
            assert_eq!(preset_label(preset, false, English), light_en);
        }
    }

    #[test]
    fn control_border_brightness_threshold_is_preserved() {
        let dark_border = Color::from_hex("#4B5360FF").to_colorref();
        let light_border = Color::from_hex("#B8C3CFFF").to_colorref();
        // Sum of RGB channels: 383 remains on the dark path, 384 on light.
        assert_eq!(
            subtle_control_border(Color::from_hex("#7F8080FF")).to_colorref(),
            dark_border
        );
        assert_eq!(
            subtle_control_border(Color::from_hex("#808080FF")).to_colorref(),
            light_border
        );
        assert_eq!(
            subtle_control_border(Color::from_hex("#000000FF")).to_colorref(),
            dark_border
        );
        assert_eq!(
            subtle_control_border(Color::from_hex("#FFFFFFFF")).to_colorref(),
            light_border
        );
    }

    #[test]
    fn settings_lifecycle_router_matches_size_dpi_close_and_destroy_only() {
        for msg in [WM_DPICHANGED, WM_SIZE, WM_GETMINMAXINFO, WM_CLOSE, WM_DESTROY] {
            assert!(is_settings_lifecycle_message(msg),
                "missing lifecycle message: {msg:#X}");
            assert!(!is_settings_input_message(msg),
                "lifecycle message overlapped input router: {msg:#X}");
            assert!(!is_settings_scheduled_message(msg),
                "lifecycle message overlapped scheduled router: {msg:#X}");
        }
        for msg in [WM_PAINT, WM_ERASEBKGND, WM_TIMER, WM_COMMAND, WM_MOUSEMOVE, WM_CTLCOLOREDIT] {
            assert!(!is_settings_lifecycle_message(msg),
                "unrelated message routed to lifecycle handler: {msg:#X}");
        }
    }

    #[test]
    fn settings_scheduled_message_routing_excludes_regular_window_events() {
        for message in [
            WM_SETTINGS_PREWARM_GALLERY,
            WM_SETTINGS_PRESET_GALLERY_REFRESH,
            WM_SETTINGS_MULTIMEDIA_ANIMATION_TICK,
            WM_SETTINGS_PRESENT_INTERACTIVE_FRAME,
            WM_TIMER,
        ] {
            assert!(is_settings_scheduled_message(message),
                "scheduled message must be dispatched: {message:#X}");
        }
        for message in [
            WM_PAINT, WM_SIZE, WM_COMMAND, WM_MOUSEMOVE, WM_DESTROY, WM_CLOSE,
        ] {
            assert!(!is_settings_scheduled_message(message),
                "unrelated message must not be dispatched to timer handler: {message:#X}");
        }
    }

    #[test]
    fn settings_timer_ids_do_not_overlap() {
        let ids = [
            DESKTOP_SUBMENU_TIMER_ID,
            JSON_ACTION_TIMER_ID,
            JSON_VALIDATION_TIMER_ID,
            JSON_ERROR_SHAKE_TIMER_ID,
            JSON_SCROLL_DRAG_FRAME_TIMER_ID,
            JSON_SCROLLBAR_TIMER_ID,
        ];
        for (index, id) in ids.iter().enumerate() {
            assert_ne!(*id, 0, "timer ID must not be zero");
            assert!(!ids[..index].contains(id), "duplicate timer ID: {id:#X}");
        }
    }

    #[test]
    fn settings_command_packs_edit_notification_words_without_sign_extension() {
        assert_eq!(decode_settings_command(WPARAM(0)), (0, 0));
        assert_eq!(
            decode_settings_command(WPARAM((usize::from(EN_CHANGE_CODE) << 16) | usize::from(ID_EDIT_JSON))),
            (ID_EDIT_JSON, EN_CHANGE_CODE)
        );
        assert_eq!(
            decode_settings_command(WPARAM((usize::from(EN_SETFOCUS_CODE) << 16) | usize::from(ID_EDIT_R))),
            (ID_EDIT_R, EN_SETFOCUS_CODE)
        );
        assert_eq!(
            decode_settings_command(WPARAM((usize::from(EN_KILLFOCUS_CODE) << 16) | usize::from(ID_EDIT_BLUR))),
            (ID_EDIT_BLUR, EN_KILLFOCUS_CODE)
        );
        assert_eq!(
            decode_settings_command(WPARAM(usize::MAX)),
            (u16::MAX, u16::MAX)
        );
    }

    
    #[test]
    fn settings_mouse_dispatch_preserves_all_input_and_capture_messages() {
        for message in [
            WM_MOUSEWHEEL, WM_SETCURSOR, WM_LBUTTONDOWN, WM_MOUSEMOVE,
            WM_MOUSELEAVE_MSG, WM_LBUTTONUP, WM_CANCELMODE, WM_CAPTURECHANGED,
        ] {
            assert!(is_settings_input_message(message), "missing input message: {message:#X}");
        }
        for message in [WM_COMMAND, WM_TIMER, WM_PAINT, WM_SIZE, WM_DESTROY] {
            assert!(!is_settings_input_message(message),
                "unrelated message routed to mouse input: {message:#X}");
        }
    }


    #[test]
    fn appearance_button_hit_areas_keep_joined_edges() {
        let hwnd = HWND::default();
        let a = theme_rect(hwnd, ThemeMode::System);
        let b = theme_rect(hwnd, ThemeMode::Dark);
        let c = theme_rect(hwnd, ThemeMode::Light);
        assert_eq!(a.right, b.left);
        assert_eq!(b.right, c.left);
        assert_eq!(a.left, appearance_group_rect(hwnd, 70, 3).left);
        assert_eq!(c.right, appearance_group_rect(hwnd, 70, 3).right);
        assert_eq!(layout_rect(hwnd, AppearancePreset::Default).right,
                   layout_rect(hwnd, AppearancePreset::Minimal).left);
        assert_eq!(position_rect(hwnd, DisplayPosition::Taskbar).right,
                   position_rect(hwnd, DisplayPosition::Desktop).left);
    }

    #[test]
    fn editor_slider_hit_regions_keep_blur_rows_aligned() {
        let hwnd = HWND::default();
        assert_eq!(rows(Section::Panel).len(), 4);
        assert_eq!(rows(Section::Tooltip).len(), 4);
        assert_eq!(rows(Section::Progress).len(), 5);
        assert_eq!(rows(Section::Interaction).len(), 1);
        assert!(rows(Section::Json).is_empty());
        for section in [Section::Panel, Section::Tooltip] {
            assert_eq!(blur_row_index(section), Some(3));
            let track = blur_slider_track_rect(hwnd, section);
            let hit = blur_slider_hit_rect(hwnd, section);
            let frame = blur_edit_frame_rect(hwnd, section);
            let suffix = blur_suffix_rect(hwnd, section);
            assert!(hit.left < track.left && hit.right > track.right);
            assert!(hit.top < track.top && hit.bottom > track.bottom);
            assert!(suffix.left >= frame.left && suffix.right <= frame.right);
        }
    }

    #[test]
    fn general_choice_groups_are_contiguous_and_match_click_areas() {
        let hwnd = HWND::default(); // logical 96-DPI fallback
        for group in [general_refresh_group_rect(hwnd), general_alert_group_rect(hwnd)] {
            let all: Vec<_> = (0..4).map(|i| general_segment_rect(group, i, 4)).collect();
            assert_eq!(all.first().unwrap().left, group.left);
            assert_eq!(all.last().unwrap().right, group.right);
            for pair in all.windows(2) {
                assert_eq!(pair[0].right, pair[1].left, "segmented controls cannot have gaps");
                assert_eq!(pair[0].top, pair[1].top);
                assert_eq!(pair[0].bottom, pair[1].bottom);
            }
            assert!(all.iter().all(|r| r.right > r.left));
        }
        assert_eq!(
            general_refresh_rect(hwnd, 300_000),
            general_segment_rect(general_refresh_group_rect(hwnd), 1, 4)
        );
        assert_eq!(
            general_alert_rect(hwnd, 20),
            general_segment_rect(general_alert_group_rect(hwnd), 2, 4)
        );
    }

    #[test]
    fn general_usage_and_application_share_equal_non_overlapping_cards() {
        let hwnd = HWND::default();
        let usage = general_usage_card_rect(hwnd);
        let app = general_application_card_rect(hwnd);
        assert_eq!(usage.right - usage.left, app.right - app.left);
        assert_eq!(app.left - usage.right, 16);
        assert_eq!(usage.top, app.top);
        assert_eq!(usage.bottom, app.bottom);
        for r in [
            general_usage_rect(hwnd, false),
            general_usage_rect(hwnd, true),
        ] {
            assert!(r.left >= usage.left && r.right <= usage.right);
            assert!(r.top >= usage.top && r.bottom <= usage.bottom);
        }
        let startup = general_startup_rect(hwnd);
        let language = general_language_group_rect(hwnd);
        for r in [startup, language] {
            assert!(r.left >= app.left && r.right <= app.right);
            assert!(r.top >= app.top && r.bottom <= app.bottom);
        }
        assert!(general_usage_rect(hwnd, false).bottom <= general_usage_rect(hwnd, true).top);
        assert_eq!(language_option_count(), 2);
        assert_eq!(language_option_rect(hwnd, 0).right, language_option_rect(hwnd, 1).left);
        assert_eq!(language_option_rect(hwnd, 0).left, language.left);
        assert_eq!(language_option_rect(hwnd, 1).right, language.right);
    }


    #[test]
    fn interactive_paint_frame_throttles_drag_but_never_drops_release() {
        let now = Instant::now();
        assert!(settings_interactive_frame_due(None, now, true));
        assert!(!settings_interactive_frame_due(Some(now), now + Duration::from_millis(8), true));
        assert!(settings_interactive_frame_due(Some(now), now + Duration::from_millis(16), true));
        assert!(settings_interactive_frame_due(Some(now), now, false));
    }


    #[test]
    fn settings_navigation_icons_labels_and_hit_regions_are_consistent() {
        for (section, icon, zh, en) in [
            (Section::General, SettingsIcon::General, "常规", "General"),
            (Section::Preset, SettingsIcon::Preset, "预设", "Presets"),
            (Section::Panel, SettingsIcon::Panel, "面板", "Panel"),
            (Section::Tooltip, SettingsIcon::Tooltip, "浮框", "Tooltip"),
            (Section::Text, SettingsIcon::Text, "文字", "Text"),
            (Section::Progress, SettingsIcon::Progress, "进度条", "Progress"),
            (Section::Interaction, SettingsIcon::Interaction, "交互", "Interaction"),
            (Section::Json, SettingsIcon::Json, "JSON 配置", "JSON config"),
        ] {
            assert_eq!(navigation_icon(section), icon);
            assert_eq!(section_label(section, LanguageId::SimplifiedChinese), zh);
            assert_eq!(section_label(section, LanguageId::English), en);
            assert_eq!(navigation_text_inset(section), 44);
            let rect = section_rect(HWND::default(), section);
            assert!(rect.right > rect.left && rect.bottom > rect.top);
        }
    }

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
    fn unsaved_json_confirmation_buttons_keep_distinct_hit_rects() {
        // Both button painter and hit-test routing must use these unchanged
        // shared geometries (including the padding and inter-button gap).
        let hwnd = HWND::default();
        let dialog = discard_dialog_rect(hwnd);
        let discard = discard_dialog_button_rect(hwnd, true);
        let keep = discard_dialog_button_rect(hwnd, false);
        assert_eq!(discard.top, keep.top);
        assert_eq!(discard.bottom, keep.bottom);
        assert_eq!(discard.right + scale(hwnd, 12), keep.left);
        assert!(discard.left >= dialog.left && keep.right <= dialog.right);
        assert!(discard.top >= dialog.top && discard.bottom <= dialog.bottom);
        assert!(discard.right < keep.left, "discard and keep hit targets must not overlap");
    }

    #[test]
    fn settings_window_centers_in_widget_monitor_work_area() {
        let primary = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
        assert_eq!(settings_centered_origin(primary, 980, 700), (470, 170));
        let left_monitor = RECT {
            left: -1920, top: -120, right: 0, bottom: 920,
        };
        assert_eq!(settings_centered_origin(left_monitor, 980, 700), (-1450, 50));
        let above_monitor = RECT {
            left: 0, top: -1080, right: 1920, bottom: -40,
        };
        assert_eq!(settings_centered_origin(above_monitor, 980, 700), (470, -910));
        // A monitor smaller than the window must keep its top-left controls
        // accessible instead of positioning the caption outside the work area.
        let small = RECT { left: -1366, top: 0, right: 0, bottom: 650 };
        assert_eq!(settings_centered_origin(small, 980, 700), (-1173, 0));
        let narrow = RECT { left: 1920, top: 80, right: 2720, bottom: 680 };
        assert_eq!(settings_centered_origin(narrow, 980, 700), (1920, 80));
    }

    #[test]
    fn json_scroll_composition_keeps_optimized_default() {
        assert_ne!(
            settings_window_ex_style().0 & WS_EX_COMPOSITED.0, 0,
            "initial settings window must retain whole-window composition"
        );
        for section in [Section::General, Section::Preset, Section::Panel, Section::Tooltip] {
            assert!(settings_composition_for_section(section),
                "non-JSON settings pages must retain whole-window composition");
        }
        assert!(!settings_composition_for_section(Section::Json),
            "JSON editor must use the uncomposited optimized scroll path");
    }

    #[test]
    fn json_scroll_frame_pacing_follows_display_refresh_rate() {
        assert_eq!(json_scroll_target_hz(0), 60);
        assert_eq!(json_scroll_target_hz(1), 60);
        assert_eq!(json_scroll_target_hz(59), 60);
        assert_eq!(json_scroll_target_hz(60), 60);
        assert_eq!(json_scroll_target_hz(61), 60);
        assert_eq!(json_scroll_target_hz(75), 75);
        assert_eq!(json_scroll_target_hz(120), 120);
        assert_eq!(json_scroll_target_hz(144), 144);
        assert_eq!(json_scroll_target_hz(360), 240);
        let now = Instant::now();
        assert!(json_scroll_frame_due(None, now, 144));
        assert!(!json_scroll_frame_due(Some(now), now + Duration::from_millis(5), 144));
        assert!(json_scroll_frame_due(Some(now), now + Duration::from_millis(8), 144));
        assert!(!json_scroll_frame_due(Some(now), now + Duration::from_millis(8), 60));
        assert!(json_scroll_frame_due(Some(now), now + Duration::from_millis(17), 60));
        assert_eq!(json_scroll_frame_interval(120), Duration::from_micros(8333));
        assert_eq!(json_scroll_frame_interval(59), Duration::from_micros(16666));
        assert!(json_scroll_drag_should_apply(None, now, 60, false));
        assert!(!json_scroll_drag_should_apply(
            Some(now), now + Duration::from_millis(8), 60, false,
        ));
        assert!(json_scroll_drag_should_apply(
            Some(now), now + Duration::from_millis(16), 60, false,
        ), "an 8 ms input source must not collapse 60 Hz into 42 Hz");
        assert!(json_scroll_drag_should_apply(
            Some(now), now + Duration::from_millis(17), 60, false,
        ));
        assert!(json_scroll_drag_should_apply(
            Some(now), now + Duration::from_millis(8), 60, true,
        ), "mouse release must flush the most recent drag pointer");
    }

    #[test]
    fn json_scroll_pacing_handles_high_refresh_and_clock_boundaries() {
        let now = Instant::now();
        assert_eq!(json_scroll_target_hz(20), 30);
        assert_eq!(json_scroll_target_hz(241), 240);
        assert!(!json_scroll_frame_due(Some(now), now, 240));
        assert!(json_scroll_frame_due(
            Some(now), now + Duration::from_micros(4166), 240,
        ));
        assert!(!json_scroll_drag_should_apply(
            Some(now), now + Duration::from_millis(1), 240, false,
        ));
        assert!(json_scroll_drag_should_apply(
            Some(now), now + Duration::from_millis(3), 240, false,
        ));
        assert!(json_scroll_drag_should_apply(Some(now), now, 240, true));
        assert!(!json_scroll_drag_should_apply(
            Some(now + Duration::from_millis(1)), now, 120, false,
        ));
    }

    #[test]
    fn json_drag_direct_pixel_target_is_bounded_and_relative() {
        assert_eq!(json_direct_drag_pixel_target(360, 20, 42, 18, 200, 24), Some(756));
        assert_eq!(json_direct_drag_pixel_target(360, 20, 0, 18, 200, 24), Some(0));
        assert_eq!(
            json_direct_drag_pixel_target(2700, 150, 199, 18, 200, 24),
            Some((200 - 24) * 18),
        );
        // RichEdit's pixel point is limited to 16-bit values; longer JSON
        // documents must use the original, unbounded EM_LINESCROLL path.
        assert_eq!(json_direct_drag_pixel_target(0, 0, 9999, 18, 20000, 24), None);
        assert_eq!(json_direct_drag_pixel_target(0, 0, 2, 0, 200, 24), None);
        assert_eq!(json_direct_drag_pixel_target(0, 0, 0, 18, 10, 24), None);
    }

    #[test]
    fn json_scroll_layout_cache_invalidates_on_document_or_viewport_change() {
        let cache = JsonScrollLayoutCache {
            edit: 100, font: 12, viewport_height: 430,
            viewport_width: 600, line_height: 18, total_lines: 200,
        };
        assert!(cache.matches(100, 12, 600, 430));
        assert!(!cache.matches(100, 12, 620, 430),
            "word wrapping changes when editor width changes");
        assert!(!cache.matches(100, 12, 600, 450),
            "visible line count changes with the editor height");
        assert!(!cache.matches(100, 14, 600, 430),
            "font changes invalidate the measurement");
        assert!(!cache.matches(101, 12, 600, 430),
            "never reuse a prior RichEdit window's line count");
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
        assert_eq!(l.collapse_label.top, l.collapse_switch.top);
        assert!(l.collapse_label.top > l.direction_label.bottom);
        assert!(l.title.top >= l.well.top && l.title.bottom < l.direction_label.top);
    }

    #[test]
    fn independent_desktop_submenu_fits_beside_all_appearance_groups() {
        let l = desktop_submenu_layout();
        let card = RECT { left: 200, top: 58, right: 940, bottom: 206 };
        for r in [
            l.well, l.title, l.direction_label, l.collapse_label,
            l.horizontal, l.vertical, l.collapse_switch,
        ] {
            assert!(r.left >= card.left && r.right <= card.right);
            assert!(r.top >= card.top && r.bottom <= card.bottom);
            assert!(r.right > r.left && r.bottom > r.top);
        }
        let taskbar = appearance_button_rect(HWND::default(), 0, 162);
        let desktop = appearance_button_rect(HWND::default(), 1, 162);
        assert_eq!(taskbar.right, desktop.left);
        assert!(l.well.left >= appearance_group_rect(HWND::default(), 70, 3).right + 14);
        assert!(l.well.left > desktop.right);
        assert_eq!(l.well.right, card.right - 16);
        assert_eq!(l.well.bottom, card.bottom - 8);
        assert!(l.title.bottom < l.horizontal.top);
        assert!(l.title.left >= l.well.left && l.title.right <= l.well.right);
        assert_eq!(l.horizontal.left - l.direction_label.right, 6);
        assert_eq!(l.well.right - l.vertical.right, 20);
        assert_eq!(l.well.right - l.collapse_switch.right, 20);
        assert!(l.collapse_label.right <= l.collapse_switch.left);
        assert_eq!(l.horizontal.right, l.vertical.left);
        assert_eq!(l.horizontal.right - l.horizontal.left, l.vertical.right - l.vertical.left);
        assert_eq!(PRESET_CARD_TOP, 226);
    }

    #[test]
    fn appearance_button_hit_rects_join_without_gaps() {
        let hwnd = HWND::default();
        let themes: Vec<_> = (0..3).map(|i| appearance_button_rect(hwnd, i, 70)).collect();
        let layouts: Vec<_> = (0..2).map(|i| appearance_button_rect(hwnd, i, 116)).collect();
        let positions: Vec<_> = (0..2).map(|i| appearance_button_rect(hwnd, i, 162)).collect();
        for (top, group) in [(70, &themes), (116, &layouts), (162, &positions)] {
            let bounds = appearance_group_rect(hwnd, top, group.len());
            assert_eq!(group[0].left, bounds.left);
            assert_eq!(group.last().unwrap().right, bounds.right);
            for pair in group.windows(2) {
                assert_eq!(pair[0].right, pair[1].left);
                assert_eq!(pair[0].top, pair[1].top);
                assert_eq!(pair[0].bottom, pair[1].bottom);
            }
            assert!(group.iter().all(|r| r.bottom - r.top == APPEARANCE_BUTTON_HEIGHT));
        }
        assert_eq!(theme_rect(hwnd, ThemeMode::Light), themes[2]);
        assert_eq!(layout_rect(hwnd, AppearancePreset::Minimal), layouts[1]);
        assert_eq!(position_rect(hwnd, DisplayPosition::Desktop), positions[1]);
        let h = desktop_direction_rect(hwnd, DesktopDirection::Horizontal);
        let v = desktop_direction_rect(hwnd, DesktopDirection::Vertical);
        assert_eq!(h.right, v.left);
        assert_eq!(desktop_auto_collapse_rect(hwnd),
                   scaled_rect(hwnd, desktop_submenu_layout().collapse_switch));
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
        let d = SubmenuCardKey { collapse: true, ..a };
        assert!(a == b);
        assert!(a != c);
        assert!(a != d, "the cached Desktop submenu must repaint when collapse changes");
        let _ = hwnd;
    }

    #[test]
    fn appearance_card_hover_and_press_invalidation_is_scoped() {
        assert!(card_only_hover_change(
            Section::Preset, Some(HitTarget::Position(DisplayPosition::Desktop)), None,
        ));
        assert!(card_only_hover_change(
            Section::Preset, Some(HitTarget::Position(DisplayPosition::Taskbar)),
            Some(HitTarget::Position(DisplayPosition::Desktop)),
        ));
        assert!(!card_only_hover_change(
            Section::Preset, Some(HitTarget::Section(Section::Preset)),
            Some(HitTarget::Position(DisplayPosition::Desktop)),
        ));
        assert!(!card_only_hover_change(
            Section::Json, None, Some(HitTarget::Position(DisplayPosition::Desktop)),
        ));
        assert!(!card_only_hover_change(Section::Preset, None, None));
    }

    #[test]
    fn preset_gallery_cache_key_changes_with_rendered_state() {
        let base = PresetGalleryKey {
            width: 980, height: 700, dpi: 96, dark: true, zh: true,
            layout: AppearancePreset::Default,
            desktop_direction: DesktopDirection::Horizontal,
            taskbar_vertical: false,
            active_style: ThemeStyle::dark_default(),
        };
        let same = base.clone();
        assert!(base == same,
            "the same gallery cache must be reusable for position changes");
        let vertical = PresetGalleryKey {
            taskbar_vertical: true, ..base.clone()
        };
        assert!(base != vertical, "preview orientation invalidates the cache");
        let light = PresetGalleryKey { dark: false, ..base.clone() };
        assert!(base != light, "theme changes invalidate the cache");
        let resized = PresetGalleryKey { width: 1000, ..base.clone() };
        assert!(base != resized, "DPI/layout size changes invalidate the cache");
    }

    #[test]
    fn position_sync_coalesced_dirty_uses_only_valid_retained_page() {
        let section = Section::Preset;
        assert!(can_reuse_coalesced_position_paint(
            section, Some(section), SettingsPaintCause::PositionSync,
            false, true, false,
        ));
        // A just-created buffer or one painted for another page is unsafe.
        assert!(!can_reuse_coalesced_position_paint(
            section, None, SettingsPaintCause::PositionSync,
            false, true, false,
        ));
        assert!(!can_reuse_coalesced_position_paint(
            section, Some(Section::General), SettingsPaintCause::PositionSync,
            false, true, false,
        ));
        // Real content refresh, user interaction, or an idle state must
        // remain eligible for normal complete painting.
        assert!(!can_reuse_coalesced_position_paint(
            section, Some(section), SettingsPaintCause::FullSync,
            false, true, false,
        ));
        assert!(!can_reuse_coalesced_position_paint(
            section, Some(section), SettingsPaintCause::PositionSync,
            false, true, true,
        ));
        assert!(!can_reuse_coalesced_position_paint(
            section, Some(section), SettingsPaintCause::PositionSync,
            false, false, false,
        ));
    }

    #[test]
    fn submenu_terminal_frame_keeps_cache_eligibility_and_retargets_cleanly() {
        let start = Instant::now();
        let mut animation = DesktopSubmenuAnimation::initial(false);
        assert!(animation.retarget(true, start));
        let (active, due) = animation.tick_for_paint(start + Duration::from_millis(225));
        assert!(due);
        assert!(!active);
        assert_eq!(animation.progress, 1.0);
        assert!(animation.final_frame_pending);
        // WM_PAINT composites this last frame, then clears the pending flag.
        animation.final_frame_pending = false;
        assert!(!animation.final_frame_pending);
        assert!(animation.retarget(false, start + Duration::from_millis(230)));
        assert!(!animation.final_frame_pending);
        let (active, due) = animation.tick_for_paint(start + Duration::from_millis(425));
        assert!(due);
        assert!(!active);
        assert_eq!(animation.progress, 0.0);
        assert!(animation.final_frame_pending);
    }

    #[test]
    fn multimedia_tick_does_not_skip_adjacent_16ms_frames() {
        let start = Instant::now();
        let mut animation = DesktopSubmenuAnimation::initial(false);
        assert!(animation.retarget(true, start));
        // Callback jitter around 16ms must not trigger the old 15ms
        // gate and accidentally skip an entire 16ms display interval.
        let mut drawn = 0;
        for ms in [15, 29, 45, 60, 76, 91, 107, 122] {
            let (active, due) = animation.tick_for_paint_with_gate(
                start + Duration::from_millis(ms),
                DESKTOP_SUBMENU_MEDIA_MIN_FRAME_MS,
            );
            assert!(active);
            if due { drawn += 1; }
        }
        assert_eq!(drawn, 8);
        // Duplicate UI tick messages in the same millisecond are rejected.
        assert!(!animation.frame_due_with_gate(
            start + Duration::from_millis(122), DESKTOP_SUBMENU_MEDIA_MIN_FRAME_MS,
        ));
    }

    #[test]
    fn submenu_frame_gate_keeps_animation_cadence() {
        let start = Instant::now();
        let mut animation = DesktopSubmenuAnimation::initial(false);
        assert!(animation.retarget(true, start));
        assert!(animation.frame_due_with_gate(start, DESKTOP_SUBMENU_MIN_FRAME_MS));
        assert!(!animation.frame_due_with_gate(start + Duration::from_millis(8), DESKTOP_SUBMENU_MIN_FRAME_MS));
        assert!(animation.frame_due_with_gate(start + Duration::from_millis(16), DESKTOP_SUBMENU_MIN_FRAME_MS));
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
    fn independent_desktop_submenu_fade_and_reverse_stay_continuous() {
        let layout = desktop_submenu_layout();
        assert!(layout.well.left < layout.well.right);
        assert!(layout.title.bottom < layout.direction_label.top);
        assert!(layout.well.left > APPEARANCE_BUTTON_LEFT + APPEARANCE_BUTTON_WIDTH * 3);
        assert_eq!(submenu_content_alpha(0.0), 0.0);
        assert_eq!(submenu_content_alpha(1.0), 1.0);
        for step in 1..=10 {
            let prev = (step - 1) as f32 / 10.0;
            let next = step as f32 / 10.0;
            assert!(submenu_content_alpha(next) >= submenu_content_alpha(prev));
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
    fn desktop_submenu_has_title_joined_direction_and_single_collapse_switch() {
        let l = desktop_submenu_layout();
        for r in [
            l.title, l.direction_label, l.collapse_label, l.horizontal,
            l.vertical, l.collapse_switch,
        ] {
            assert!(r.top >= l.well.top && r.bottom <= l.well.bottom);
            assert!(r.left >= l.well.left && r.right <= l.well.right);
        }
        assert_eq!(l.horizontal.right, l.vertical.left);
        assert!(l.horizontal.bottom < l.collapse_switch.top);
        assert_eq!(l.direction_label.top, l.horizontal.top);
        assert_eq!(l.collapse_label.top, l.collapse_switch.top);
        assert!(l.title.bottom < l.direction_label.top);
        assert_eq!(l.well, RECT { left: 652, top: 66, right: 924, bottom: 198 });
        assert_eq!(l.horizontal.top, 106);
        assert_eq!(l.collapse_switch.top, 152);
        assert!(l.well.left > appearance_group_rect(HWND::default(), 70, 3).right);
        assert!(l.well.top < l.title.top && l.collapse_switch.bottom < l.well.bottom);
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

            // A taskbar/desktop switch may update runtime preview orientation,
            // but only the lower gallery depends on that field.
            let mut switched_position = snapshot.clone();
            switched_position.display_position = DisplayPosition::Desktop;
            switched_position.vertical_taskbar_mode = true;
            switched_position.editable_settings.appearance.position = "desktop".into();
            assert!(position_is_only_snapshot_change(&snapshot, &switched_position));
            switched_position.theme_mode = ThemeMode::Light;
            assert!(!position_is_only_snapshot_change(&snapshot, &switched_position));

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
                assert_eq!(
                    GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED.0, 0,
                    "JSON RichEdit must scroll without whole-tree composition"
                );
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
                // Exercise the narrow, uncomposited scrollbar paint path.
                // Dragging must use a retained GDI backbuffer, not erase the
                // live track before drawing its thumb.
                let track = json_scrollbar_track_rect(hwnd);
                let hover_x = (track.left + track.right) / 2;
                let hover_y = (track.top + track.bottom) / 2;
                assert_eq!(
                    json_scrollbar_wheel_target(hwnd, hover_x, hover_y),
                    Some(json_edit),
                    "mouse wheel over JSON scrollbar must target RichEdit"
                );
                assert_eq!(
                    json_scrollbar_wheel_target(hwnd, track.right + scale(hwnd, 6), hover_y),
                    None,
                    "wheel outside the scrollbar must not be redirected"
                );
                let left = scale(hwnd, 180);
                let track_local = RECT {
                    left: track.left - left,
                    top: track.top,
                    right: track.right - left,
                    bottom: track.bottom,
                };
                let _ = InvalidateRect(json_page_surface, Some(&track_local), false);
                let _ = UpdateWindow(json_page_surface);
                // UpdateWindow can coalesce the dirty track with a pending
                // whole-pane redraw on headless CI runners. Independently
                // exercise the atomic paint helper with an actual window DC.
                let screen = GetDC(json_page_surface);
                assert!(!screen.0.is_null());
                let is_dark = {
                    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    state.as_ref().unwrap().snapshot.is_dark
                };
                let painted = paint_json_scrollbar_buffered(
                    screen, hwnd, is_dark, left, track,
                );
                let _ = ReleaseDC(json_page_surface, screen);
                assert!(painted, "the narrow scrollbar must support double-buffered painting");
                JSON_SCROLLBAR_PAINT_BUFFER.with(|slot| {
                    let cache = slot.borrow();
                    let buffer = cache.as_ref().expect("scrollbar paint buffer must be retained");
                    assert_eq!(buffer.width, track.right - track.left);
                    assert_eq!(buffer.height, track.bottom - track.top);
                });
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
                assert_eq!(
                    json_scrollbar_wheel_target(hwnd, hover_x, hover_y),
                    None,
                    "other settings pages must not route wheel events to hidden JSON edit"
                );
                assert_ne!(
                    GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_COMPOSITED.0, 0,
                    "non-JSON pages must restore whole-window composition"
                );
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

            // Regression: wheel over the focused blur EDIT used to move the
            // slider/live preview while the native text still showed "0".
            // Exercise repeated increments and decrements with the real
            // HWND and ensure both the model and displayed text stay in step.
            let (blur_edit, start_blur) = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                let s = state.as_ref().unwrap();
                (s.blur_edit.to_hwnd(), s.snapshot.active_style.panel_frosted_strength)
            };
            let _ = SetFocus(blur_edit);
            let expected_up = start_blur.saturating_add(3).min(FROSTED_STRENGTH_MAX);
            for _ in 0..3 {
                adjust_numeric_by_wheel(WheelNumericTarget::Blur, 120);
            }
            let actual_up = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().unwrap().snapshot.active_style.panel_frosted_strength
            };
            assert_eq!(actual_up, expected_up, "wheel must update blur strength");
            assert_eq!(
                read_edit_value(blur_edit),
                Some(u16::from(actual_up)),
                "focused native EDIT text must match the blur slider/model",
            );
            for _ in 0..3 {
                adjust_numeric_by_wheel(WheelNumericTarget::Blur, -120);
            }
            let actual_down = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().unwrap().snapshot.active_style.panel_frosted_strength
            };
            assert_eq!(actual_down, start_blur);
            assert_eq!(read_edit_value(blur_edit), Some(u16::from(actual_down)));

            // Direct typing is distinct from wheel: EN_CHANGE must update
            // model + slider and schedule a frame even if Panel's live preview
            // is busy composing the desktop widget. Compare the already-good
            // Tooltip path using exactly the same native EDIT.
            for (section, typed_value) in [(Section::Panel, 27), (Section::Tooltip, 43)] {
                set_section(section);
                let blur_edit = {
                    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    state.as_ref().unwrap().blur_edit.to_hwnd()
                };
                let _ = SetFocus(blur_edit);
                SETTINGS_INTERACTIVE_FRAME_QUEUED.with(|pending| pending.set(false));
                set_edit_text(blur_edit, typed_value);
                let stored = {
                    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                    let s = state.as_ref().unwrap();
                    if section == Section::Tooltip {
                        s.snapshot.active_style.tooltip_frosted_strength()
                    } else {
                        s.snapshot.active_style.panel_frosted_strength
                    }
                };
                assert_eq!(stored, typed_value);
                assert_eq!(read_edit_value(blur_edit), Some(u16::from(typed_value)));
                assert!(
                    SETTINGS_INTERACTIVE_FRAME_QUEUED.with(|pending| pending.get()),
                    "typing blur must request an explicit frame for both pages",
                );
                let _ = SendMessageW(
                    hwnd, WM_SETTINGS_PRESENT_INTERACTIVE_FRAME, WPARAM(0), LPARAM(0),
                );
                assert_eq!(read_edit_value(blur_edit), Some(u16::from(typed_value)));
            }

            let _ = DestroyWindow(hwnd);

            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            assert!(state.is_none(), "style settings state must be released on destroy");
        }
    }
}
