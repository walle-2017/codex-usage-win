use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
use windows::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows::Win32::UI::Controls::InitCommonControls;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TRACKMOUSEEVENT, TME_LEAVE,
};
use windows::Win32::UI::Shell::{ExtractIconExW, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::appearance::{self, AppearancePreset, DesktopDirection, DisplayPosition};
use crate::diagnose;
use crate::fonts;
use crate::localization::{self, LanguageId, Strings};
use crate::models::AppUsageData;
use crate::native_interop::{
    self, Color, TIMER_COUNTDOWN, TIMER_POLL, TIMER_RESET_POLL, WM_APP_TASKBAR_LAYOUT_UPDATED,
    WM_APP_TRAY, WM_APP_USAGE_UPDATED,
};
use crate::poller;
use crate::popup_menu::{self, PopupItem};
use crate::settings_model::{
    EditableAppearance, EditableGeneral, EditableSettings, EditableThemeStyle, EditableUsage,
    EDITABLE_SETTINGS_SCHEMA_VERSION,
};
use crate::style::{
    StyleColorTarget, StyleSettings, ThemeMode, ThemePreset, ThemeStyle, FROSTED_STRENGTH_MAX,
};
use crate::style_window;
use crate::theme;
use crate::tray_icon;
use crate::updater;

mod taskbar_replica;
mod blur_backdrop;
use self::taskbar_replica::*;
use self::blur_backdrop::*;

/// Wrapper to make HWND sendable across threads (safe for PostMessage usage)
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

/// Shared application state
struct AppState {
    hwnd: SendHwnd,
    taskbar_hwnd: Option<HWND>,
    tray_notify_hwnd: Option<HWND>,
    win_event_hook: Option<HWINEVENTHOOK>,
    is_dark: bool,
    embedded: bool,
    language_override: Option<LanguageId>,
    language: LanguageId,
    appearance_preset: AppearancePreset,
    effective_appearance_preset: AppearancePreset,
    display_position: DisplayPosition,
    desktop_direction: DesktopDirection,
    hidden_for_taskbar_space: bool,
    taskbar_space_warning_shown: bool,
    floating_mode: bool,
    floating_hwnd: Option<SendHwnd>,
    parked_taskbar_foreground: Option<SendHwnd>,
    floating_x: i32,
    floating_y: i32,
    floating_edge: Option<FloatingEdge>,
    floating_collapsed: bool,
    // 0 = hidden edge tab, 100 = fully expanded; updated by the popup timer.
    floating_reveal: u8,
    // Desktop-only: undocked grip is hidden until the pointer hovers the widget.
    floating_grip_alpha: u8,
    floating_slide_from: u8,
    floating_slide_target: u8,
    floating_slide_started: Instant,
    floating_fade_from: u8,
    floating_fade_target: u8,
    floating_fade_started: Instant,
    floating_grip_hover: bool,
    // A fresh edge drop must finish retracting and clear the pointer from its
    // visible tab before mouse hover is allowed to reveal the popup again.
    floating_first_collapse_guard: bool,
    theme_mode: ThemeMode,
    styles: StyleSettings,
    composition_blur_active: bool,
    small_taskbar_mode: bool,
    small_show_weekly: bool,
    minimal_hover_target: Option<MinimalHoverTarget>,
    drag_handle_hovered: bool,

    codex_session_percent: f64,
    codex_session_text: String,
    codex_weekly_percent: f64,
    codex_weekly_text: String,
    show_session_window: bool,
    show_weekly_window: bool,
    alert_threshold_percent: u8,
    notified_quota_windows: BTreeSet<String>,
    data: Option<AppUsageData>,

    poll_interval_ms: u32,
    retry_count: u32,
    force_notify_auth_error: bool,
    auth_error_paused_polling: bool,
    auth_watch_mode: poller::CredentialWatchMode,
    auth_watch_snapshot: poller::CredentialWatchSnapshot,
    last_poll_ok: bool,
    available_update_version: Option<String>,

    taskbar_index: usize,
    taskbar_monitor: Option<String>,
    taskbar_left_offset: i32,
    taskbar_top_offset: i32,
    vertical_taskbar_mode: bool,
    vertical_widget_width: i32,
    legacy_tray_offset: Option<i32>,
    dragging: bool,
    drag_anchor_logical_x: i32,
    drag_anchor_logical_y: i32,
    // Keep desktop grip's physical grab offset constant across DPI changes.
    floating_drag_anchor_x: i32,
    floating_drag_anchor_y: i32,
    drag_reparenting: bool,
    failed_drag_target: Option<HWND>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FloatingEdge { Left, Right, Top, Bottom }

impl FloatingEdge {
    fn name(self) -> &'static str {
        match self { Self::Left => "left", Self::Right => "right", Self::Top => "top", Self::Bottom => "bottom" }
    }
    fn parse(value: &str) -> Option<Self> {
        match value { "left" => Some(Self::Left), "right" => Some(Self::Right),
            "top" => Some(Self::Top), "bottom" => Some(Self::Bottom), _ => None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MinimalHoverTarget {
    Session,
    Weekly,
}

const RETRY_BASE_MS: u32 = 30_000; // 30 seconds

const POLL_1_MIN: u32 = 60_000;
const POLL_5_MIN: u32 = 300_000;
const POLL_15_MIN: u32 = 900_000;
const POLL_1_HOUR: u32 = 3_600_000;

// Menu item IDs for update frequency
const IDM_FREQ_1MIN: u16 = 10;
const IDM_FREQ_5MIN: u16 = 11;
const IDM_FREQ_15MIN: u16 = 12;
const IDM_FREQ_1HOUR: u16 = 13;
const IDM_START_WITH_WINDOWS: u16 = 20;
const IDM_RESET_POSITION: u16 = 30;
const IDM_LANG_SYSTEM: u16 = 40;
const IDM_LANG_ENGLISH: u16 = 41;
const IDM_LANG_DUTCH: u16 = 42;
const IDM_LANG_SPANISH: u16 = 43;
const IDM_LANG_FRENCH: u16 = 44;
const IDM_LANG_GERMAN: u16 = 45;
const IDM_LANG_JAPANESE: u16 = 46;
const IDM_LANG_KOREAN: u16 = 47;
const IDM_LANG_TRADITIONAL_CHINESE: u16 = 48;
const IDM_LANG_RUSSIAN: u16 = 49;
const IDM_LANG_PORTUGUESE_BRAZIL: u16 = 50;
const IDM_LANG_SIMPLIFIED_CHINESE: u16 = 51;
const IDM_CHECK_UPDATE: u16 = 60;
const IDM_OPEN_RELEASES: u16 = 61;
const IDM_SHOW_SESSION_WINDOW: u16 = 71;
const IDM_SHOW_WEEKLY_WINDOW: u16 = 72;
const IDM_ALERT_OFF: u16 = 80;
const IDM_ALERT_10: u16 = 81;
const IDM_ALERT_20: u16 = 82;
const IDM_ALERT_30: u16 = 83;

const IDM_LAYOUT_DEFAULT: u16 = 91;
const IDM_LAYOUT_MINIMAL: u16 = 92;
const IDM_LAYOUT_ADAPTIVE: u16 = 97;
const IDM_THEME_SYSTEM: u16 = 93;
const IDM_THEME_DARK: u16 = 94;
const IDM_THEME_LIGHT: u16 = 95;
const IDM_STYLE_SETTINGS: u16 = 96;

const IDM_STYLE_PANEL_BACKGROUND: u16 = 100;
const IDM_STYLE_PANEL_BORDER: u16 = 101;
const IDM_STYLE_PANEL_BLUR: u16 = 102;
const IDM_STYLE_QUOTA_TYPE: u16 = 103;
const IDM_STYLE_REMAINING: u16 = 104;
const IDM_STYLE_RESET_TIME: u16 = 105;
const IDM_STYLE_ERROR: u16 = 106;
const IDM_STYLE_PROGRESS_HIGH: u16 = 107;
const IDM_STYLE_PROGRESS_MEDIUM: u16 = 108;
const IDM_STYLE_PROGRESS_LOW: u16 = 109;
const IDM_STYLE_PROGRESS_CONSUMED: u16 = 110;
const IDM_STYLE_DRAG_HANDLE: u16 = 111;
const IDM_STYLE_RESET_CURRENT: u16 = 112;

const TBM_GETPOS_MSG: u32 = WM_USER;
const TBM_SETPOS_MSG: u32 = WM_USER + 5;
const TBM_SETRANGE_MSG: u32 = WM_USER + 6;
const TB_ENDTRACK_CODE: u16 = 8;
/// Keep the visible panel border at one physical pixel even at high DPI.
const PANEL_BORDER_WIDTH_PX: i32 = 1;
/// Layered windows treat fully transparent pixels as mouse-pass-through.
/// Keep an imperceptible alpha on visually transparent panel pixels so the
/// component remains draggable/right-clickable even when its background is 0 alpha.
const MIN_INTERACTIVE_ALPHA: u8 = 1;
const FROSTED_MAX_BLUR_PX: f32 = 20.0;
const STYLE_PREVIEW_FRAME_MS: u64 = 16;
const DRAG_FRAME_MS: u64 = 8;
const TASKBAR_LAYOUT_POLL_MS: u32 = 1_000;
const TASKBAR_CONTROL_MARGIN_LOGICAL: i32 = 4;

const GITHUB_RELEASES_URL: &str =
    "https://github.com/walle-2017/codex-usage-win/releases";
const WM_DPICHANGED_MSG: u32 = 0x02E0;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const MINIMAL_TOOLTIP_CLASS: &str = "CodexUsageMinimalTooltip";
const WM_APP_FOCUS_EXISTING_SETTINGS: u32 = native_interop::WM_APP + 140;
const TRAY_ICON_UPDATE_REPOSITION_SUPPRESS_MS: u64 = 750;

/// How often the watchdog thread polls for an explorer.exe restart (which
/// recreates the taskbar and wipes our tray-icon registration).
const TASKBAR_WATCH_INTERVAL_SECS: u64 = 2;

static SUPPRESS_TRAY_REPOSITION_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlurBackdropParams {
    blur_bits: u32,
    tint: Color,
}

static BLUR_BACKDROP_HWND: Mutex<Option<SendHwnd>> = Mutex::new(None);
static BLUR_BACKDROP_CONTEXT: Mutex<Option<usize>> = Mutex::new(None);
static BLUR_BACKDROP_PARAMS: Mutex<Option<BlurBackdropParams>> = Mutex::new(None);

// Embedded blur siblings stay tied to their original Explorer taskbar.
// Reuse them after a round trip instead of creating a new cross-process child
// while the Start menu is holding that taskbar active.
struct ParkedBlurBackdrop {
    parent: SendHwnd,
    hwnd: SendHwnd,
    context: usize,
    params: Option<BlurBackdropParams>,
}
static PARKED_BLUR_BACKDROPS: Mutex<Vec<ParkedBlurBackdrop>> = Mutex::new(Vec::new());
static LAST_STYLE_PREVIEW_RENDER: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_DRAG_FRAME: Mutex<Option<Instant>> = Mutex::new(None);
static MINIMAL_TOOLTIP_HWND: Mutex<Option<SendHwnd>> = Mutex::new(None);
static MINIMAL_TOOLTIP_TEXT: Mutex<String> = Mutex::new(String::new());
static TOOLTIP_BLUR_HWND: Mutex<Option<SendHwnd>> = Mutex::new(None);
static TOOLTIP_BLUR_CONTEXT: Mutex<Option<usize>> = Mutex::new(None);
static TOOLTIP_BLUR_PARAMS: Mutex<Option<BlurBackdropParams>> = Mutex::new(None);

#[derive(Clone, Debug)]
struct TaskbarLayoutSnapshot {
    taskbar_hwnd_raw: isize,
    controls: Vec<RECT>,
}

static TASKBAR_LAYOUT_CACHE: Mutex<Vec<TaskbarLayoutSnapshot>> = Mutex::new(Vec::new());

#[derive(Clone, Copy)]
struct ColorEditorState {
    hwnd: SendHwnd,
    theme_is_dark: bool,
    target: StyleColorTarget,
    sliders: [SendHwnd; 4],
    value_labels: [SendHwnd; 4],
    hex_label: SendHwnd,
}

#[derive(Clone, Copy)]
struct BlurEditorState {
    hwnd: SendHwnd,
    theme_is_dark: bool,
    slider: SendHwnd,
    value_label: SendHwnd,
}

static COLOR_EDITOR_STATE: Mutex<Option<ColorEditorState>> = Mutex::new(None);
static BLUR_EDITOR_STATE: Mutex<Option<BlurEditorState>> = Mutex::new(None);

/// Current system DPI (96 = 100% scaling, 144 = 150%, 192 = 200%, etc.)
static CURRENT_DPI: AtomicU32 = AtomicU32::new(96);

/// Scale a base pixel value (designed at 96 DPI) to the current DPI.
fn sc(px: i32) -> i32 {
    let dpi = CURRENT_DPI.load(Ordering::Relaxed);
    (px as f64 * dpi as f64 / 96.0).round() as i32
}

fn gdi_text_quality() -> u32 {
    ANTIALIASED_QUALITY.0 as u32
}

/// Re-query the monitor DPI for our window and update the cached value.
/// Uses GetDpiForWindow which returns the live DPI (unlike GetDpiForSystem
/// which is cached at process startup and never changes).
fn refresh_dpi() {
    let dpi_source = {
        let state = lock_state();
        state.as_ref().map(|s| {
            // During a cross-monitor popup move, the foreground HWND can report
            // the old monitor DPI for a short period. The selected taskbar is
            // already authoritative for layout, so prefer its DPI.
            if s.floating_mode { s.hwnd.to_hwnd() }
            else { s.taskbar_hwnd.unwrap_or_else(|| s.hwnd.to_hwnd()) }
        })
    };
    if let Some(hwnd) = dpi_source {
        let dpi = unsafe { GetDpiForWindow(hwnd) };
        if dpi > 0 {
            CURRENT_DPI.store(dpi, Ordering::Relaxed);
        }
    }
}

/// Spacing below which two relaunches are treated as a storm (e.g. explorer.exe
/// crash-looping); when detected we back off instead of spawning in a tight loop.
const RELAUNCH_THROTTLE_SECS: u64 = 10;
const RELAUNCH_BACKOFF_SECS: u64 = 30;
/// Environment flag set on a relaunched child so it waits for the previous
/// instance's single-instance mutex instead of exiting immediately.
const ENV_RELAUNCH: &str = "CODEX_USAGE_RELAUNCH";
/// Unix timestamp (seconds) of the relaunch that spawned this process, passed to
/// the child so it can detect a relaunch storm.
const ENV_LAST_RELAUNCH_UNIX: &str = "CODEX_USAGE_LAST_RELAUNCH_UNIX";

/// Relaunch the widget as a fresh process after explorer.exe has restarted.
///
/// When the shell restarts it destroys our embedded child window outright (the
/// window is gone, not merely orphaned - `IsWindow` returns false) and leaves
/// the UI thread parked in `GetMessage` with no window to recreate in place.
/// Spawning a clean new process - which re-embeds into the freshly created
/// taskbar - and exiting this one is the robust recovery. The child is flagged
/// via `ENV_RELAUNCH` so it waits for this instance's single-instance mutex to
/// be released before taking over (see the guard in `run`).
fn relaunch_self() {
    // Back off if we are relaunching very soon after the relaunch that spawned
    // us: that signals the shell is crash-looping, not a one-off restart.
    let now = now_unix_secs();
    let last = std::env::var(ENV_LAST_RELAUNCH_UNIX)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    if last != 0 && now.saturating_sub(last) < RELAUNCH_THROTTLE_SECS {
        diagnose::log("relaunch storm detected; backing off before relaunching");
        std::thread::sleep(Duration::from_secs(RELAUNCH_BACKOFF_SECS));
    }

    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            diagnose::log_error("watchdog: unable to resolve current executable", error);
            return;
        }
    };

    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|arg| !updater::is_internal_update_arg(arg))
        .collect();
    match std::process::Command::new(exe)
        .args(&args)
        .env(ENV_RELAUNCH, "1")
        .env(ENV_LAST_RELAUNCH_UNIX, now.to_string())
        .spawn()
    {
        Ok(_) => {
            diagnose::log("watchdog: relaunched fresh instance, exiting old one");
            std::process::exit(0);
        }
        Err(error) => {
            diagnose::log_error("watchdog: unable to spawn relaunched instance", error);
        }
    }
}

/// Detect explorer.exe restarts and recover from them.
///
/// Once explorer destroys the taskbar, our embedded child window is destroyed
/// and the UI message loop is dead, so recovery cannot happen in-process. This
/// dedicated thread (independent of the dead message loop) polls the taskbar
/// handle and, when it changes, relaunches the widget as a fresh process.
fn spawn_taskbar_watchdog() {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(TASKBAR_WATCH_INTERVAL_SECS));
        let stored = {
            let state = lock_state();
            state
                .as_ref()
                .and_then(|s| s.taskbar_hwnd.map(|hwnd| (hwnd, s.taskbar_monitor.clone())))
        };
        let Some((old, preferred_monitor)) = stored else {
            continue;
        };

        // Do not treat one failed EnumWindows pass as an Explorer restart.
        // Opening Start / shell flyouts can transiently make the taskbar absent
        // from our top-level enumeration even though the persisted HWND is still
        // alive and still has a taskbar class.
        if native_interop::is_taskbar_window(old) {
            continue;
        }

        let taskbars = native_interop::find_taskbars();

        if let Some(preferred_monitor) = preferred_monitor.as_deref() {
            if let Some(replacement) = taskbars
                .iter()
                .find(|taskbar| taskbar.monitor_device.as_deref() == Some(preferred_monitor))
            {
                diagnose::log(format!(
                    "watchdog: taskbar recreated on monitor={} old={:?} new={:?} -> relaunching",
                    preferred_monitor, old.0, replacement.hwnd.0
                ));
                relaunch_self();
            } else {
                diagnose::log(format!(
                    "watchdog: taskbar missing on monitor={}; waiting instead of switching monitors",
                    preferred_monitor
                ));
            }
        } else if let Some(replacement) = taskbars.first() {
            diagnose::log(format!(
                "watchdog: legacy taskbar identity changed old={:?} new={:?} -> relaunching",
                old.0, replacement.hwnd.0
            ));
            relaunch_self();
        }
    });
}

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

unsafe impl Send for AppState {}

static STATE: Mutex<Option<AppState>> = Mutex::new(None);

/// Lock STATE safely, recovering from poisoned mutex
fn lock_state() -> MutexGuard<'static, Option<AppState>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

const SETTINGS_DIR: &str = "CodexUsage";
const LEGACY_SETTINGS_DIR: &str = "ClaudeCodeUsageMonitor";

fn appdata_path(directory: &str) -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata).join(directory).join("settings.json")
}

fn settings_path() -> PathBuf {
    appdata_path(SETTINGS_DIR)
}

fn legacy_settings_path() -> PathBuf {
    appdata_path(LEGACY_SETTINGS_DIR)
}

#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    tray_offset: i32,
    #[serde(default)]
    taskbar_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    taskbar_left_offset: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    taskbar_top_offset: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    floating_x: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    floating_y: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    floating_edge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    taskbar_monitor: Option<String>,
    #[serde(default = "default_poll_interval")]
    poll_interval_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(default)]
    appearance_preset: AppearancePreset,
    #[serde(default)]
    display_position: DisplayPosition,
    #[serde(default)]
    desktop_direction: DesktopDirection,
    #[serde(default)]
    theme_mode: ThemeMode,
    #[serde(default)]
    styles: StyleSettings,
    #[serde(default = "default_show_usage_window")]
    show_session_window: bool,
    #[serde(default = "default_show_usage_window")]
    show_weekly_window: bool,
    #[serde(default)]
    alert_threshold_percent: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notified_quota_windows: Vec<String>,
}

impl Default for SettingsFile {
    fn default() -> Self {
        Self {
            tray_offset: 0,
            taskbar_index: 0,
            taskbar_left_offset: None,
            taskbar_top_offset: None,
            floating_x: None,
            floating_y: None,
            floating_edge: None,
            taskbar_monitor: None,
            poll_interval_ms: default_poll_interval(),
            language: None,
            appearance_preset: AppearancePreset::Default,
            display_position: DisplayPosition::Taskbar,
            desktop_direction: DesktopDirection::Horizontal,
            theme_mode: ThemeMode::System,
            styles: StyleSettings::default(),
            show_session_window: true,
            show_weekly_window: true,
            alert_threshold_percent: 0,
            notified_quota_windows: Vec::new(),
        }
    }
}

fn default_poll_interval() -> u32 {
    POLL_15_MIN
}

fn default_show_usage_window() -> bool {
    true
}

fn load_settings() -> SettingsFile {
    let current_path = settings_path();
    let legacy_path = legacy_settings_path();
    let (settings, migrated) = load_settings_from_paths(&current_path, &legacy_path)
        .unwrap_or_else(|| (SettingsFile::default(), false));
    let settings = normalize_settings(settings);
    if migrated {
        save_settings(&settings);
        diagnose::log(format!(
            "migrated settings from {} to {}",
            legacy_path.display(),
            current_path.display()
        ));
    }
    settings
}

fn load_settings_from_paths(
    current_path: &std::path::Path,
    legacy_path: &std::path::Path,
) -> Option<(SettingsFile, bool)> {
    if let Ok(content) = std::fs::read_to_string(current_path) {
        return serde_json::from_str(&content)
            .ok()
            .map(|settings| (settings, false));
    }

    let content = std::fs::read_to_string(legacy_path).ok()?;
    serde_json::from_str(&content)
        .ok()
        .map(|settings| (settings, true))
}

fn normalize_settings(mut settings: SettingsFile) -> SettingsFile {
    if !settings.show_session_window && !settings.show_weekly_window {
        settings.show_session_window = true;
    }
    if !matches!(settings.alert_threshold_percent, 0 | 10 | 20 | 30) {
        settings.alert_threshold_percent = 0;
    }
    settings.notified_quota_windows.sort();
    settings.notified_quota_windows.dedup();
    settings.styles.normalize();
    settings.appearance_preset = settings.appearance_preset.configured();
    settings.styles.clamp_corner_radii(settings.appearance_preset);
    settings
}

fn save_settings(settings: &SettingsFile) {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

fn save_state_settings() {
    // Serialize a snapshot, then release STATE before performing file I/O.
    // Switching desktop orientation must never block state access on the
    // settings file (e.g. if antivirus scans it simultaneously).
    let snapshot = {
        let state = lock_state();
        state.as_ref().map(|s| SettingsFile {
            // Legacy fields remain serialized for backward compatibility, but
            // new versions use an absolute left-edge offset plus monitor identity.
            tray_offset: 0,
            taskbar_index: s.taskbar_index,
            taskbar_left_offset: Some(s.taskbar_left_offset),
            taskbar_top_offset: Some(s.taskbar_top_offset),
            floating_x: Some(s.floating_x),
            floating_y: Some(s.floating_y),
            floating_edge: s.floating_edge.map(|e| e.name().to_string()),
            taskbar_monitor: s.taskbar_monitor.clone(),
            poll_interval_ms: s.poll_interval_ms,
            language: s
                .language_override
                .map(|language| language.code().to_string()),
            appearance_preset: s.appearance_preset.configured(),
            display_position: s.display_position,
            desktop_direction: s.desktop_direction,
            theme_mode: s.theme_mode,
            styles: s.styles.clone(),
            show_session_window: s.show_session_window,
            show_weekly_window: s.show_weekly_window,
            alert_threshold_percent: s.alert_threshold_percent,
            notified_quota_windows: s.notified_quota_windows.iter().cloned().collect(),
        })
    };
    if let Some(snapshot) = snapshot {
        save_settings(&snapshot);
    }
}

fn format_precise_reset_time(resets_at: Option<SystemTime>) -> Option<String> {
    let local = native_interop::system_time_to_local(resets_at?)?;
    Some(format_local_system_time(local))
}

fn format_local_system_time(local: SYSTEMTIME) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        local.wYear, local.wMonth, local.wDay, local.wHour, local.wMinute
    )
}

fn service_tooltip(
    service: &str,
    session_text: &str,
    weekly_text: &str,
    show_session_window: bool,
    show_weekly_window: bool,
) -> String {
    let mut parts = Vec::new();
    if show_session_window {
        parts.push(format!("5H {session_text}"));
    }
    if show_weekly_window {
        parts.push(format!("7D {weekly_text}"));
    }
    format!("{service}: {}", parts.join(" | "))
}

struct QuotaAlert {
    kind: tray_icon::TrayIconKind,
    title: String,
    message: String,
}

fn collect_low_quota_alerts(state: &mut AppState, data: &AppUsageData) -> Vec<QuotaAlert> {
    let threshold = state.alert_threshold_percent;
    if threshold == 0 {
        return Vec::new();
    }

    let mut alerts = Vec::new();
    if let Some(usage) = data.codex.as_ref() {
        let strings = state.language.strings();
        append_provider_alerts(
            &mut alerts,
            &mut state.notified_quota_windows,
            threshold,
            state.language,
            tray_icon::TrayIconKind::Codex,
            "codex",
            strings.codex_model,
            usage,
            strings,
        );
    }
    alerts
}

fn notify_quota_alerts(hwnd: HWND, alerts: &[QuotaAlert]) {
    let hwnd = taskbar_replica_service_hwnd(hwnd);
    let Some(first) = alerts.first() else {
        return;
    };
    let message = alerts
        .iter()
        .map(|alert| alert.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    tray_icon::notify_info(hwnd, first.kind, &first.title, &message);
}

#[allow(clippy::too_many_arguments)]
fn append_provider_alerts(
    alerts: &mut Vec<QuotaAlert>,
    notified: &mut BTreeSet<String>,
    threshold: u8,
    language: LanguageId,
    kind: tray_icon::TrayIconKind,
    provider_key: &str,
    provider_label: &str,
    usage: &crate::models::UsageData,
    strings: Strings,
) {
    append_quota_alert(
        alerts,
        notified,
        threshold,
        language,
        kind,
        provider_key,
        provider_label,
        "session",
        strings.session_window,
        &usage.session,
    );
    append_quota_alert(
        alerts,
        notified,
        threshold,
        language,
        kind,
        provider_key,
        provider_label,
        "weekly",
        strings.weekly_window,
        &usage.weekly,
    );
}

#[allow(clippy::too_many_arguments)]
fn append_quota_alert(
    alerts: &mut Vec<QuotaAlert>,
    notified: &mut BTreeSet<String>,
    threshold: u8,
    language: LanguageId,
    kind: tray_icon::TrayIconKind,
    provider_key: &str,
    provider_label: &str,
    window_key: &str,
    window_label: &str,
    section: &crate::models::UsageSection,
) {
    let prefix = format!("{provider_key}:{window_key}:");
    let reset_key = section
        .resets_at
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_secs().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let key = format!("{prefix}{reset_key}");
    notified.retain(|existing| !existing.starts_with(&prefix) || existing == &key);

    let remaining = poller::remaining_percentage(section.percentage).round() as u8;
    if remaining > threshold || !notified.insert(key) {
        return;
    }

    let reset = format_precise_reset_time(section.resets_at);
    let (title, message) = if language == LanguageId::SimplifiedChinese {
        (
            format!("{provider_label} 额度"),
            format!(
                "{window_label} 剩余 {remaining}% · {} 重置",
                reset.unwrap_or_else(|| "未知".to_string())
            ),
        )
    } else {
        (
            format!("{provider_label} quota"),
            format!(
                "{window_label} {remaining}% remaining · reset {}",
                reset.unwrap_or_else(|| "unknown".to_string())
            ),
        )
    };
    alerts.push(QuotaAlert {
        kind,
        title,
        message,
    });
}

fn full_usage_line(
    section: &crate::models::UsageSection,
    language: LanguageId,
    strings: Strings,
    window: poller::UsageWindowKind,
) -> String {
    poller::format_line(
        section,
        strings,
        language == LanguageId::SimplifiedChinese,
        window,
    )
}

fn tray_icon_data_from_state() -> Option<tray_icon::TrayIconData> {
    let state = lock_state();
    let s = state.as_ref()?;
    if !s.last_poll_ok {
        return Some(tray_icon::TrayIconData {
            tooltip: s.language.strings().window_title.to_string(),
        });
    }

    let strings = s.language.strings();
    let usage = s.data.as_ref()?.codex.as_ref()?;
    let session = full_usage_line(
        &usage.session,
        s.language,
        strings,
        poller::UsageWindowKind::Session,
    );
    let weekly = full_usage_line(
        &usage.weekly,
        s.language,
        strings,
        poller::UsageWindowKind::Weekly,
    );
    Some(tray_icon::TrayIconData {
        tooltip: service_tooltip(
            strings.codex_model,
            &session,
            &weekly,
            s.show_session_window,
            s.show_weekly_window,
        ),
    })
}

fn sync_tray_icons(hwnd: HWND) {
    let icon = tray_icon_data_from_state();
    tray_icon::sync(taskbar_replica_service_hwnd(hwnd), icon.as_ref());
}

fn select_preferred_taskbar(
    taskbars: &[native_interop::TaskbarWindow],
    preferred_monitor: Option<&str>,
    fallback_index: usize,
) -> Option<(usize, native_interop::TaskbarWindow)> {
    if let Some(preferred_monitor) = preferred_monitor {
        if let Some((index, taskbar)) = taskbars
            .iter()
            .enumerate()
            .find(|(_, taskbar)| taskbar.monitor_device.as_deref() == Some(preferred_monitor))
        {
            return Some((index, taskbar.clone()));
        }
    }
    if taskbars.is_empty() {
        None
    } else {
        let index = fallback_index.min(taskbars.len() - 1);
        Some((index, taskbars[index].clone()))
    }
}

fn attach_to_taskbar_window(
    hwnd: HWND,
    index: usize,
    taskbar: &native_interop::TaskbarWindow,
) -> bool {
    diagnose::log_lazy(|| format!(
        "taskbar selected index={index} monitor={} hwnd={:?} rect=({}, {}, {}, {})",
        taskbar.monitor_device.as_deref().unwrap_or("<unknown>"),
        taskbar.hwnd,
        taskbar.rect.left,
        taskbar.rect.top,
        taskbar.rect.right,
        taskbar.rect.bottom
    ));

    if let Some(switched) = try_taskbar_replica_handoff(hwnd, index, taskbar) {
        return switched;
    }

    if !native_interop::embed_in_taskbar(hwnd, taskbar.hwnd) {
        diagnose::log(format!(
            "taskbar switch aborted: target hwnd={:?} monitor={} (window parent did not change)",
            taskbar.hwnd,
            taskbar.monitor_device.as_deref().unwrap_or("<unknown>")
        ));
        return false;
    }

    // Only release the old event hook after the window actually moves.
    let old_hook = {
        let mut state = lock_state();
        state.as_mut().and_then(|s| s.win_event_hook.take())
    };
    if let Some(hook) = old_hook {
        native_interop::unhook_win_event(hook);
    }

    let tray_notify = native_interop::find_child_window(taskbar.hwnd, "TrayNotifyWnd");
    let hook = tray_notify.and_then(|tray_hwnd| {
        let thread_id = native_interop::get_window_thread_id(tray_hwnd);
        native_interop::set_tray_event_hook(thread_id, on_tray_location_changed)
    });

    let mut state = lock_state();
    if let Some(s) = state.as_mut() {
        s.taskbar_hwnd = Some(taskbar.hwnd);
        s.tray_notify_hwnd = tray_notify;
        s.win_event_hook = hook;
        s.taskbar_index = index;
        s.taskbar_monitor = taskbar.monitor_device.clone();
        s.embedded = true;
    }
    drop(state);
    true
}

fn attach_to_preferred_taskbar(
    hwnd: HWND,
    preferred_monitor: Option<&str>,
    fallback_index: usize,
) -> bool {
    let taskbars = native_interop::find_taskbars();
    let Some((index, taskbar)) =
        select_preferred_taskbar(&taskbars, preferred_monitor, fallback_index)
    else {
        diagnose::log("taskbar not found; using fallback popup window");
        return false;
    };
    if preferred_monitor.is_some()
        && taskbar.monitor_device.as_deref() != preferred_monitor
    {
        diagnose::log(format!(
            "preferred monitor {} unavailable; using fallback monitor {}",
            preferred_monitor.unwrap_or("<unknown>"),
            taskbar.monitor_device.as_deref().unwrap_or("<unknown>")
        ));
    }
    attach_to_taskbar_window(hwnd, index, &taskbar)
}

// A Start-menu activation can temporarily omit the *previous* taskbar from
// EnumWindows even though its original (now parked) widget is still parented
// to that live Explorer taskbar. The current-only fallback below cannot
// recover that return direction. Reuse existing HWNDs as read-only locators;
// never create, detach, reparent, show, or raise any window in this code.
thread_local! {
    static LAST_RECOVERED_REPLICA_TARGET: std::cell::Cell<isize> =
        const { std::cell::Cell::new(0) };
}

fn replica_taskbar_targets_from_existing_windows() -> Vec<native_interop::TaskbarWindow> {
    let primary = TASKBAR_PRIMARY_WINDOW.with(|cell| HWND(cell.get() as *mut _));
    let replicas = TASKBAR_REPLICA_WINDOWS.with(|cache| {
        cache.borrow().iter().map(|slot| {
            (slot.child.to_hwnd(), Some(slot.parent.to_hwnd()))
        }).collect::<Vec<_>>()
    });
    let mut result = Vec::new();
    for (child, expected_parent) in std::iter::once((primary, None)).chain(replicas) {
        unsafe {
            if child == HWND::default() || !IsWindow(child).as_bool() {
                continue;
            }
            let parent = GetAncestor(child, GA_PARENT);
            if expected_parent.is_some_and(|expected| expected != parent)
                || !native_interop::is_taskbar_window(parent)
                || result.iter().any(|t: &native_interop::TaskbarWindow| t.hwnd == parent)
            {
                continue;
            }
            let Some(rect) = native_interop::get_taskbar_rect(parent)
                .or_else(|| native_interop::get_window_rect_safe(parent)) else {
                continue;
            };
            if rect.right <= rect.left || rect.bottom <= rect.top {
                continue;
            }
            result.push(native_interop::TaskbarWindow {
                hwnd: parent, rect,
                monitor_device: native_interop::monitor_device_name(parent),
            });
        }
    }
    result
}

fn point_is_in_taskbar_rect(pt: POINT, rect: RECT) -> bool {
    pt.x >= rect.left && pt.x < rect.right && pt.y >= rect.top && pt.y < rect.bottom
}

/// When the pointer leaves all taskbars during a captured drag, keep using
/// the already-embedded foreground's actual Explorer parent. This is a
/// placement fallback only: cross-taskbar handoff still requires a real hit
/// in taskbar_at_point().
fn current_drag_taskbar() -> Option<(usize, native_interop::TaskbarWindow)> {
    let (index, parent, foreground, monitor_device, embedded) = {
        let state = lock_state();
        let s = state.as_ref()?;
        (s.taskbar_index, s.taskbar_hwnd?, s.hwnd.to_hwnd(), s.taskbar_monitor.clone(), s.embedded)
    };
    if !embedded || !native_interop::is_taskbar_window(parent) {
        return None;
    }
    unsafe {
        if !IsWindow(foreground).as_bool() || GetAncestor(foreground, GA_PARENT) != parent {
            return None;
        }
    }
    let rect = native_interop::get_taskbar_rect(parent)
        .or_else(|| native_interop::get_window_rect_safe(parent))?;
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return None;
    }
    Some((index, native_interop::TaskbarWindow {
        hwnd: parent,
        rect,
        monitor_device,
    }))
}

fn taskbar_for_drag_point(pt: POINT) -> Option<(usize, native_interop::TaskbarWindow)> {
    taskbar_at_point(pt).or_else(current_drag_taskbar)
}

fn taskbar_at_point(pt: POINT) -> Option<(usize, native_interop::TaskbarWindow)> {
    let mut enumerated = native_interop::find_taskbars();
    if let Some((index, found)) = enumerated.iter().enumerate()
        .find(|(_, taskbar)| point_is_in_taskbar_rect(pt, taskbar.rect))
    {
        return Some((index, found.clone()));
    }

    // On enumeration miss, resolve parked primary/replica HWNDs using their
    // actual Explorer parents. No edge tolerance or z-order edits.
    let existing = replica_taskbar_targets_from_existing_windows();
    for target in existing {
        if !enumerated.iter().any(|item| item.hwnd == target.hwnd) {
            enumerated.push(target);
        }
    }
    enumerated.sort_by_key(|item| (
        item.rect.top, item.rect.left, item.rect.bottom, item.rect.right
    ));
    if let Some((index, found)) = enumerated.iter().enumerate()
        .find(|(_, taskbar)| point_is_in_taskbar_rect(pt, taskbar.rect))
    {
        LAST_RECOVERED_REPLICA_TARGET.with(|last| {
            if last.replace(found.hwnd.0 as isize) != found.hwnd.0 as isize {
                diagnose::log_lazy(|| format!(
                    "taskbar replica recovered missed target from parked HWND parent={:?} monitor={} cursor=({}, {}) rect=({}, {}, {}, {})",
                    found.hwnd,
                    found.monitor_device.as_deref().unwrap_or("<unknown>"),
                    pt.x, pt.y, found.rect.left, found.rect.top,
                    found.rect.right, found.rect.bottom
                ));
            }
        });
        return Some((index, found.clone()));
    }

    // Opening Start or another shell surface can transiently omit the taskbar
    // from EnumWindows. Dragging must still work on the current taskbar while
    // its persisted HWND/class remains valid.
    let (index, hwnd, monitor_device) = {
        let state = lock_state();
        let state = state.as_ref()?;
        (
            state.taskbar_index,
            state.taskbar_hwnd?,
            state.taskbar_monitor.clone(),
        )
    };

    if !native_interop::is_taskbar_window(hwnd) {
        return None;
    }

    let rect = native_interop::get_taskbar_rect(hwnd)
        .or_else(|| native_interop::get_window_rect_safe(hwnd))?;
    if !point_is_in_taskbar_rect(pt, rect) {
        return None;
    }

    Some((
        index,
        native_interop::TaskbarWindow {
            hwnd,
            rect,
            monitor_device,
        },
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HorizontalSpan {
    left: i32,
    right: i32,
}

impl HorizontalSpan {
    fn width(self) -> i32 {
        (self.right - self.left).max(0)
    }
}

/// Explorer taskbars can be docked at the left or right edge of a monitor.
/// Orientation is determined from geometry, including secondary taskbars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskbarAxis {
    Horizontal,
    Vertical,
}

fn taskbar_axis(rect: RECT) -> TaskbarAxis {
    if rect.bottom > rect.top
        && rect.right > rect.left
        && rect.bottom - rect.top > rect.right - rect.left
    {
        TaskbarAxis::Vertical
    } else {
        TaskbarAxis::Horizontal
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VerticalSpan {
    top: i32,
    bottom: i32,
}

impl VerticalSpan {
    fn height(self) -> i32 {
        (self.bottom - self.top).max(0)
    }
}

fn vertical_widget_width(taskbar_rect: RECT) -> Option<i32> {
    let available = taskbar_rect.right - taskbar_rect.left - sc(VERTICAL_WIDGET_INSET) * 2;
    (available >= sc(VERTICAL_WIDGET_MIN_WIDTH))
        .then(|| available.min(sc(VERTICAL_WIDGET_MAX_WIDTH)))
}

fn free_vertical_spans_from_occupied(
    taskbar_rect: RECT,
    tray_top: i32,
    occupied_rects: &[RECT],
    margin: i32,
) -> Vec<VerticalSpan> {
    let start = taskbar_rect.top;
    let end = tray_top.clamp(start, taskbar_rect.bottom);
    if end <= start {
        return Vec::new();
    }
    let margin = margin.max(0);
    let mut occupied: Vec<VerticalSpan> = occupied_rects
        .iter()
        .filter(|rect| rect.right > taskbar_rect.left && rect.left < taskbar_rect.right)
        .filter_map(|rect| {
            let top = (rect.top - margin).max(start);
            let bottom = (rect.bottom + margin).min(end);
            (bottom > top).then_some(VerticalSpan { top, bottom })
        })
        .collect();
    occupied.sort_by_key(|span| (span.top, span.bottom));
    let mut merged: Vec<VerticalSpan> = Vec::new();
    for span in occupied {
        if let Some(last) = merged.last_mut() {
            if span.top <= last.bottom {
                last.bottom = last.bottom.max(span.bottom);
                continue;
            }
        }
        merged.push(span);
    }
    let mut free = Vec::new();
    let mut cursor = start;
    for span in merged {
        if span.top > cursor {
            free.push(VerticalSpan { top: cursor, bottom: span.top });
        }
        cursor = cursor.max(span.bottom);
    }
    if cursor < end {
        free.push(VerticalSpan { top: cursor, bottom: end });
    }
    free
}

fn taskbar_free_vertical_spans(
    taskbar_hwnd: HWND,
    rect: RECT,
) -> Option<Vec<VerticalSpan>> {
    let controls = cached_taskbar_controls(taskbar_hwnd)?;
    if controls.is_empty() {
        diagnose::log("vertical taskbar layout cache is empty; failing closed");
        return None;
    }
    let tray_top = native_interop::find_child_window(taskbar_hwnd, "TrayNotifyWnd")
        .and_then(native_interop::get_window_rect_safe)
        .filter(|tray| tray.top >= rect.top && tray.top < rect.bottom)
        .map(|tray| tray.top)
        .unwrap_or(rect.bottom);
    Some(free_vertical_spans_from_occupied(
        rect, tray_top, &controls, sc(TASKBAR_CONTROL_MARGIN_LOGICAL),
    ))
}

fn nearest_free_top_offset(
    taskbar_rect: RECT,
    free_spans: &[VerticalSpan],
    widget_height: i32,
    desired_top_offset: i32,
) -> Option<i32> {
    free_spans
        .iter()
        .copied()
        .filter(|span| span.height() >= widget_height)
        .map(|span| {
            let min_top = span.top - taskbar_rect.top;
            let max_top = span.bottom - taskbar_rect.top - widget_height;
            let offset = desired_top_offset.clamp(min_top, max_top);
            ((offset - desired_top_offset).abs(), offset)
        })
        .min_by_key(|(distance, offset)| (*distance, *offset))
        .map(|(_, offset)| offset)
}

fn drag_top_from_cursor(taskbar_rect: RECT, pt: POINT, anchor_px: i32) -> i32 {
    pt.y - taskbar_rect.top - anchor_px
}

fn drag_top_for_pointer(rect: RECT, pt: POINT, anchor_px: i32, height: i32) -> i32 {
    let desired = drag_top_from_cursor(rect, pt, anchor_px);
    if point_is_in_taskbar_rect(pt, rect) {
        desired
    } else {
        clamp_left_offset(desired, (rect.bottom - rect.top - height).max(0))
    }
}

fn tray_left_for_taskbar(taskbar_hwnd: HWND, taskbar_rect: RECT) -> i32 {
    let mut tray_left = taskbar_rect.right;
    if let Some(tray_hwnd) = native_interop::find_child_window(taskbar_hwnd, "TrayNotifyWnd") {
        if let Some(tray_rect) = native_interop::get_window_rect_safe(tray_hwnd) {
            tray_left = tray_rect.left;
        }
    }
    tray_left
}

fn max_left_offset_for_taskbar(
    taskbar_hwnd: HWND,
    taskbar_rect: RECT,
    widget_width: i32,
) -> i32 {
    let tray_left = tray_left_for_taskbar(taskbar_hwnd, taskbar_rect);
    (tray_left - taskbar_rect.left - widget_width).max(0)
}

fn clamp_left_offset(left_offset: i32, max_left: i32) -> i32 {
    left_offset.clamp(0, max_left.max(0))
}

fn legacy_left_offset_from_max(max_left: i32, tray_offset: i32) -> i32 {
    let max_left = max_left.max(0);
    max_left - tray_offset.clamp(0, max_left)
}

fn legacy_left_offset(
    taskbar_hwnd: HWND,
    taskbar_rect: RECT,
    widget_width: i32,
    tray_offset: i32,
) -> i32 {
    legacy_left_offset_from_max(
        max_left_offset_for_taskbar(taskbar_hwnd, taskbar_rect, widget_width),
        tray_offset,
    )
}

fn free_spans_from_occupied(
    taskbar_rect: RECT,
    tray_left: i32,
    occupied_rects: &[RECT],
    margin: i32,
) -> Vec<HorizontalSpan> {
    let start = taskbar_rect.left;
    let end = tray_left.clamp(taskbar_rect.left, taskbar_rect.right);
    if end <= start {
        return Vec::new();
    }

    let margin = margin.max(0);
    let mut occupied: Vec<HorizontalSpan> = occupied_rects
        .iter()
        .filter(|rect| rect.bottom > taskbar_rect.top && rect.top < taskbar_rect.bottom)
        .filter_map(|rect| {
            let left = (rect.left - margin).max(start);
            let right = (rect.right + margin).min(end);
            (right > left).then_some(HorizontalSpan { left, right })
        })
        .collect();
    occupied.sort_by_key(|span| (span.left, span.right));

    let mut merged: Vec<HorizontalSpan> = Vec::new();
    for span in occupied {
        if let Some(last) = merged.last_mut() {
            if span.left <= last.right {
                last.right = last.right.max(span.right);
                continue;
            }
        }
        merged.push(span);
    }

    let mut free = Vec::new();
    let mut cursor = start;
    for span in merged {
        if span.left > cursor {
            free.push(HorizontalSpan {
                left: cursor,
                right: span.left,
            });
        }
        cursor = cursor.max(span.right);
    }
    if cursor < end {
        free.push(HorizontalSpan {
            left: cursor,
            right: end,
        });
    }
    free
}

fn cached_taskbar_controls(taskbar_hwnd: HWND) -> Option<Vec<RECT>> {
    let raw = taskbar_hwnd.0 as isize;
    let cache = TASKBAR_LAYOUT_CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    cache
        .iter()
        .find(|snapshot| snapshot.taskbar_hwnd_raw == raw)
        .map(|snapshot| snapshot.controls.clone())
}

fn taskbar_free_spans(taskbar_hwnd: HWND, taskbar_rect: RECT) -> Option<Vec<HorizontalSpan>> {
    let controls = cached_taskbar_controls(taskbar_hwnd)?;
    if controls.is_empty() {
        diagnose::log("taskbar free-space cache has no actionable controls; failing closed");
        return None;
    }
    let tray_left = tray_left_for_taskbar(taskbar_hwnd, taskbar_rect);
    Some(free_spans_from_occupied(
        taskbar_rect,
        tray_left,
        &controls,
        sc(TASKBAR_CONTROL_MARGIN_LOGICAL),
    ))
}

fn same_control_rects(left: &[RECT], right: &[RECT]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(a, b)| {
            a.left == b.left
                && a.top == b.top
                && a.right == b.right
                && a.bottom == b.bottom
        })
}

fn merge_taskbar_layout_cache(
    cache: &mut Vec<TaskbarLayoutSnapshot>,
    active_taskbars: &BTreeSet<isize>,
    successful_updates: Vec<TaskbarLayoutSnapshot>,
) -> bool {
    let previous_len = cache.len();
    cache.retain(|snapshot| active_taskbars.contains(&snapshot.taskbar_hwnd_raw));
    let mut changed = cache.len() != previous_len;

    for update in successful_updates {
        if let Some(existing) = cache
            .iter_mut()
            .find(|snapshot| snapshot.taskbar_hwnd_raw == update.taskbar_hwnd_raw)
        {
            if !same_control_rects(&existing.controls, &update.controls) {
                existing.controls = update.controls;
                changed = true;
            }
        } else {
            cache.push(update);
            changed = true;
        }
    }

    changed
}

fn preserve_valid_cached_taskbars<F>(
    active_taskbars: &mut BTreeSet<isize>,
    cache: &[TaskbarLayoutSnapshot],
    mut is_valid_taskbar: F,
) where
    F: FnMut(isize) -> bool,
{
    for snapshot in cache {
        if is_valid_taskbar(snapshot.taskbar_hwnd_raw) {
            active_taskbars.insert(snapshot.taskbar_hwnd_raw);
        }
    }
}

fn spawn_taskbar_layout_worker(hwnd: HWND) {
    let target = SendHwnd::from_hwnd(hwnd);
    std::thread::spawn(move || {
        // Track only failure-state transitions so a persistent Explorer/UIA
        // condition does not flood the runtime log once per second.
        let mut last_failures: BTreeMap<isize, Option<i32>> = BTreeMap::new();

        loop {
            let taskbars = native_interop::find_taskbars();
            let mut active_taskbars: BTreeSet<isize> = taskbars
                .iter()
                .map(|taskbar| taskbar.hwnd.0 as isize)
                .collect();

            // EnumWindows is not a reliable destruction signal for the taskbar:
            // Start/shell transitions can transiently omit a still-valid taskbar.
            // Preserve last-known-good snapshots while their HWND/class are valid.
            {
                let cache = TASKBAR_LAYOUT_CACHE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                preserve_valid_cached_taskbars(
                    &mut active_taskbars,
                    &cache,
                    |raw| native_interop::is_taskbar_window(HWND(raw as *mut _)),
                );
            }

            let mut successful_updates = Vec::new();

            for taskbar in taskbars {
                let raw = taskbar.hwnd.0 as isize;
                match native_interop::taskbar_control_rects(taskbar.hwnd) {
                    Ok(controls) if !controls.is_empty() => {
                        if last_failures.remove(&raw).is_some() {
                            diagnose::log(format!(
                                "taskbar UI Automation query recovered hwnd={:?} controls={}",
                                taskbar.hwnd,
                                controls.len()
                            ));
                        }
                        successful_updates.push(TaskbarLayoutSnapshot {
                            taskbar_hwnd_raw: raw,
                            controls,
                        });
                    }
                    Ok(_) => {
                        if last_failures.get(&raw) != Some(&None) {
                            diagnose::log(format!(
                                "taskbar UI Automation returned zero actionable controls hwnd={:?}; preserving last-known-good layout",
                                taskbar.hwnd
                            ));
                            last_failures.insert(raw, None);
                        }
                    }
                    Err(status) => {
                        let state = Some(status);
                        if last_failures.get(&raw) != Some(&state) {
                            diagnose::log(format!(
                                "taskbar UI Automation query failed hwnd={:?} hresult=0x{:08X}; preserving last-known-good layout",
                                taskbar.hwnd,
                                status as u32
                            ));
                            last_failures.insert(raw, state);
                        }
                    }
                }
            }

            last_failures.retain(|raw, _| active_taskbars.contains(raw));

            let changed = {
                let mut cache = TASKBAR_LAYOUT_CACHE
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                merge_taskbar_layout_cache(&mut cache, &active_taskbars, successful_updates)
            };

            if changed {
                unsafe {
                    let _ = PostMessageW(
                        target.to_hwnd(),
                        WM_APP_TASKBAR_LAYOUT_UPDATED,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
            }

            std::thread::sleep(Duration::from_millis(u64::from(TASKBAR_LAYOUT_POLL_MS)));
        }
    });
}

fn nearest_free_left_offset(
    taskbar_rect: RECT,
    free_spans: &[HorizontalSpan],
    widget_width: i32,
    desired_left_offset: i32,
) -> Option<i32> {
    let mut best: Option<(i32, i32)> = None;
    for span in free_spans.iter().copied().filter(|span| span.width() >= widget_width) {
        let min_left = span.left - taskbar_rect.left;
        let max_left = span.right - taskbar_rect.left - widget_width;
        let candidate = desired_left_offset.clamp(min_left, max_left);
        let distance = (candidate - desired_left_offset).abs();
        if best.map(|(best_distance, _)| distance < best_distance).unwrap_or(true) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

fn placement_for_free_spans(
    configured_preset: AppearancePreset,
    language: LanguageId,
    taskbar_rect: RECT,
    free_spans: &[HorizontalSpan],
    desired_left_offset: i32,
) -> Option<(AppearancePreset, i32, i32)> {
    let choice = configured_preset.configured();
    let preset_width = |preset| total_widget_width_for_preset(language, preset);
    let place = |preset| {
        let width = preset_width(preset);
        nearest_free_left_offset(taskbar_rect, free_spans, width, desired_left_offset)
            .map(|left| (preset, width, left))
    };
    match choice {
        AppearancePreset::Minimal => place(AppearancePreset::Minimal),
        AppearancePreset::Default => place(AppearancePreset::Default)
            .or_else(|| place(AppearancePreset::Minimal)),
        AppearancePreset::Adaptive => unreachable!(),
    }
}
fn drag_anchor_px_for_dpi(logical_x: i32, dpi: u32) -> i32 {
    let dpi = dpi.max(1);
    (logical_x as f64 * dpi as f64 / 96.0).round() as i32
}

fn drag_left_from_cursor(taskbar_rect: RECT, pt: POINT, anchor_px: i32) -> i32 {
    pt.x - taskbar_rect.left - anchor_px
}

/// Outside the taskbar, use the pointer's X position but keep the entire
/// widget within the current taskbar. In-taskbar movement is unchanged.
fn drag_left_for_pointer(
    taskbar_rect: RECT,
    pt: POINT,
    anchor_px: i32,
    widget_width: i32,
) -> i32 {
    let desired = drag_left_from_cursor(taskbar_rect, pt, anchor_px);
    if point_is_in_taskbar_rect(pt, taskbar_rect) {
        desired
    } else {
        let taskbar_width = taskbar_rect.right - taskbar_rect.left;
        clamp_left_offset(desired, (taskbar_width - widget_width).max(0))
    }
}

fn left_offset_for_drag_left(
    _taskbar_hwnd: HWND,
    taskbar_rect: RECT,
    widget_width: i32,
    drag_left: i32,
) -> i32 {
    let taskbar_width = taskbar_rect.right - taskbar_rect.left;
    clamp_left_offset(drag_left, (taskbar_width - widget_width).max(0))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn refresh_usage_texts(state: &mut AppState) {
    if !state.last_poll_ok {
        return;
    }
    let Some(codex) = state.data.as_ref().and_then(|data| data.codex.as_ref()) else {
        return;
    };
    let preset = effective_appearance_preset(state);
    state.codex_session_text = appearance::taskbar_line(
        preset,
        state.language,
        &codex.session,
        poller::UsageWindowKind::Session,
    );
    state.codex_weekly_text = appearance::taskbar_line(
        preset,
        state.language,
        &codex.weekly,
        poller::UsageWindowKind::Weekly,
    );
}

fn manual_update_required_message(language: LanguageId, version: &str) -> String {
    match language {
        LanguageId::English => format!(
            "Version v{version} is available, but the program or release asset name has changed. Automatic update is unavailable. Open GitHub Releases and update manually."
        ),
        LanguageId::Dutch => format!(
            "Versie v{version} is beschikbaar, maar de programma- of releasebestandsnaam is gewijzigd. Automatisch bijwerken is niet mogelijk. Open GitHub Releases en werk handmatig bij."
        ),
        LanguageId::Spanish => format!(
            "La versión v{version} está disponible, pero el nombre del programa o de los archivos de la versión ha cambiado. La actualización automática no está disponible. Abre GitHub Releases y actualiza manualmente."
        ),
        LanguageId::French => format!(
            "La version v{version} est disponible, mais le nom du programme ou des fichiers de publication a changé. La mise à jour automatique est indisponible. Ouvrez GitHub Releases et mettez à jour manuellement."
        ),
        LanguageId::German => format!(
            "Version v{version} ist verfügbar, aber der Programmname oder der Name der Release-Datei hat sich geändert. Die automatische Aktualisierung ist nicht möglich. Öffnen Sie GitHub Releases und aktualisieren Sie manuell."
        ),
        LanguageId::Japanese => format!(
            "v{version} を利用できますが、プログラム名またはリリースファイル名が変更されています。自動更新できません。GitHub Releases を開き、手動で更新してください。"
        ),
        LanguageId::Korean => format!(
            "v{version} 버전을 사용할 수 있지만 프로그램 이름 또는 릴리스 파일 이름이 변경되었습니다. 자동 업데이트를 사용할 수 없습니다. GitHub Releases를 열어 수동으로 업데이트하세요."
        ),
        LanguageId::SimplifiedChinese => format!(
            "检测到 v{version}，但程序名称或发布文件名称已发生变化，无法自动升级。请前往 GitHub Releases 手动更新。"
        ),
        LanguageId::TraditionalChinese => format!(
            "偵測到 v{version}，但程式名稱或發佈檔案名稱已變更，無法自動升級。請前往 GitHub Releases 手動更新。"
        ),
        LanguageId::Russian => format!(
            "Доступна версия v{version}, но название программы или файла релиза изменилось. Автоматическое обновление недоступно. Откройте GitHub Releases и обновитесь вручную."
        ),
        LanguageId::PortugueseBrazil => format!(
            "A versão v{version} está disponível, mas o nome do programa ou do arquivo da versão mudou. A atualização automática não está disponível. Abra o GitHub Releases e atualize manualmente."
        ),
    }
}

fn open_github_releases(_hwnd: HWND) {
    unsafe {
        let operation = native_interop::wide_str("open");
        let url = native_interop::wide_str(GITHUB_RELEASES_URL);
        // Do not use the Explorer-embedded child HWND as the ShellExecute
        // owner; Explorer may recreate that child while the popup is closing.
        let result = ShellExecuteW(
            HWND::default(),
            PCWSTR::from_raw(operation.as_ptr()),
            PCWSTR::from_raw(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if (result.0 as isize) <= 32 {
            diagnose::log(format!(
                "open GitHub Releases failed ShellExecute code={}",
                result.0 as isize
            ));
        }
    }
}

fn set_window_title(hwnd: HWND, strings: Strings) {
    unsafe {
        let title = native_interop::wide_str(strings.window_title);
        let _ = SetWindowTextW(hwnd, PCWSTR::from_raw(title.as_ptr()));
    }
}

fn apply_language_to_state(state: &mut AppState, language_override: Option<LanguageId>) {
    let language_override = language_override.map(LanguageId::ui_supported);
    state.language_override = language_override;
    state.language = localization::resolve_language(language_override);
    set_window_title(state.hwnd.to_hwnd(), state.language.strings());
    refresh_usage_texts(state);
}

fn update_language_change() -> bool {
    let mut state = lock_state();
    let Some(app_state) = state.as_mut() else {
        return false;
    };

    if app_state.language_override.is_some() {
        return false;
    }

    let new_language = localization::detect_system_language();
    if new_language == app_state.language {
        return false;
    }

    apply_language_to_state(app_state, None);
    true
}

const STARTUP_REGISTRY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_REGISTRY_KEY: &str = "CodexUsageWin";
const LEGACY_STARTUP_REGISTRY_KEY: &str = "CodexUsage";

/// Returns true only if the startup registry value points to this executable.
fn is_startup_enabled() -> bool {
    let Some(reg_value) = read_startup_value(STARTUP_REGISTRY_KEY) else {
        return false;
    };
    let Some(current_exe) = current_exe_path_string() else {
        return false;
    };
    reg_value.eq_ignore_ascii_case(&current_exe)
}

fn current_exe_path_string() -> Option<String> {
    unsafe {
        let mut exe_buf = [0u16; 260];
        let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
        (len > 0).then(|| String::from_utf16_lossy(&exe_buf[..len]))
    }
}

fn read_startup_value(key: &str) -> Option<String> {
    unsafe {
        let path = native_interop::wide_str(STARTUP_REGISTRY_PATH);
        let key_name = native_interop::wide_str(key);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        );
        if result.is_err() {
            return None;
        }

        // Query the size of the value
        let mut data_size: u32 = 0;
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            None,
            Some(&mut data_size),
        );
        if result.is_err() || data_size == 0 {
            let _ = RegCloseKey(hkey);
            return None;
        }

        // Read the value
        let mut buf = vec![0u8; data_size as usize];
        let result = RegQueryValueExW(
            hkey,
            PCWSTR::from_raw(key_name.as_ptr()),
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut data_size),
        );
        let _ = RegCloseKey(hkey);
        if result.is_err() {
            return None;
        }

        // Convert the registry value (UTF-16) to a string
        let wide_slice =
            std::slice::from_raw_parts(buf.as_ptr() as *const u16, data_size as usize / 2);
        Some(
            String::from_utf16_lossy(wide_slice)
                .trim_end_matches('\0')
                .to_string(),
        )
    }
}

fn delete_startup_value(key: &str) {
    unsafe {
        let path = native_interop::wide_str(STARTUP_REGISTRY_PATH);
        let key_name = native_interop::wide_str(key);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        )
        .is_ok()
        {
            let _ = RegDeleteValueW(hkey, PCWSTR::from_raw(key_name.as_ptr()));
            let _ = RegCloseKey(hkey);
        }
    }
}

fn migrate_legacy_startup_entry() {
    let legacy_exists = read_startup_value(LEGACY_STARTUP_REGISTRY_KEY).is_some();
    let current_exists = read_startup_value(STARTUP_REGISTRY_KEY).is_some();
    if !legacy_exists {
        return;
    }

    if should_write_migrated_startup(legacy_exists, current_exists) {
        set_startup_enabled(true);
    }

    if read_startup_value(STARTUP_REGISTRY_KEY).is_some() {
        delete_startup_value(LEGACY_STARTUP_REGISTRY_KEY);
        diagnose::log("migrated legacy startup registry entry to CodexUsageWin");
    }
}

fn should_write_migrated_startup(legacy_exists: bool, current_exists: bool) -> bool {
    legacy_exists && !current_exists
}

fn set_startup_enabled(enable: bool) {
    unsafe {
        let path = native_interop::wide_str(STARTUP_REGISTRY_PATH);

        let mut hkey = HKEY::default();
        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR::from_raw(path.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut hkey,
        );
        if result.is_err() {
            return;
        }

        let key_name = native_interop::wide_str(STARTUP_REGISTRY_KEY);

        if enable {
            let mut exe_buf = [0u16; 260];
            let len = GetModuleFileNameW(None, &mut exe_buf) as usize;
            if len > 0 {
                // Write the wide string including null terminator
                let byte_len = ((len + 1) * 2) as u32;
                let _ = RegSetValueExW(
                    hkey,
                    PCWSTR::from_raw(key_name.as_ptr()),
                    0,
                    REG_SZ,
                    Some(std::slice::from_raw_parts(
                        exe_buf.as_ptr() as *const u8,
                        byte_len as usize,
                    )),
                );
            }
        } else {
            let _ = RegDeleteValueW(hkey, PCWSTR::from_raw(key_name.as_ptr()));
            let legacy_key_name = native_interop::wide_str(LEGACY_STARTUP_REGISTRY_KEY);
            let _ = RegDeleteValueW(hkey, PCWSTR::from_raw(legacy_key_name.as_ptr()));
        }

        let _ = RegCloseKey(hkey);
    }
}

// Dimensions matching the C# version
const SEGMENT_W: i32 = 10;
const SEGMENT_H: i32 = 13;
const SEGMENT_GAP: i32 = 1;

const DRAG_HANDLE_HIT_W: i32 = 12;
const SMALL_TASKBAR_THRESHOLD: i32 = 34;
const SMALL_WIDGET_HEIGHT: i32 = 28;
const DRAG_HANDLE_HIT_H: i32 = 24;
const VERTICAL_WIDGET_MIN_WIDTH: i32 = 32;
const VERTICAL_WIDGET_MAX_WIDTH: i32 = 68;
// All vertical positions are measured from the start of a quota row.
const VERTICAL_WIDGET_HEIGHT: i32 = 168;
const VERTICAL_MINIMAL_WIDGET_HEIGHT: i32 = 82;
const VERTICAL_WIDGET_INSET: i32 = 2;
const VERTICAL_DRAG_HANDLE_H: i32 = 18;
const VERTICAL_FIRST_ROW_TOP: i32 = 20;
const VERTICAL_QUOTA_ROW_HEIGHT: i32 = 70;
const VERTICAL_QUOTA_ROW_GAP: i32 = 4;
// The embedded taskbar reserves an 18px draggable header. Desktop mode has
// a separate external tab, so content starts at a true panel inset instead.
// Desktop content has equal inset on all sides, distinct from embedded taskbar.
const DESKTOP_PANEL_CONTENT_PADDING: i32 = 10;
const DESKTOP_MINIMAL_ROW_HEIGHT: i32 = 24;
const DESKTOP_MINIMAL_ROW_GAP: i32 = 6;
const VERTICAL_LABEL_CENTER_Y: i32 = 9;
const VERTICAL_RING_CENTER_Y: i32 = 35;
const VERTICAL_RESET_CENTER_Y: i32 = 61;
const VERTICAL_RING_MAX_RADIUS: i32 = 13;
// A 36-degree open gap straddles 6 o'clock; progress sweeps clockwise.
const VERTICAL_RING_GAP_HALF_RADIANS: f32 = std::f32::consts::PI / 10.0;
const VERTICAL_RING_START_RADIANS: f32 = std::f32::consts::PI + VERTICAL_RING_GAP_HALF_RADIANS;
const VERTICAL_MINIMAL_FIRST_PERCENT_Y: i32 = 34;
const VERTICAL_MINIMAL_SECOND_PERCENT_Y: i32 = 63;

fn vertical_drag_handle_hit(x: i32, y: i32, width: i32, height: i32) -> bool {
    x >= 0 && x < width && y >= 0 && y < sc(VERTICAL_DRAG_HANDLE_H).min(height)
}

fn is_drag_handle_point(client_x: i32, client_y: i32) -> bool {
    {
        let state = lock_state();
        if let Some(s) = state.as_ref() {
            if s.floating_mode {
                if s.floating_edge.is_none() && s.floating_grip_alpha == 0 {
                    return false;
                }
                return floating_drag_handle_hit(
                    client_x, client_y,
                    floating_canvas_size_for_state(s).0,
                    floating_canvas_size_for_state(s).1,
                    s.floating_edge, sc(FLOAT_EDGE_PEEK),
                );
            }
        }
    }
    let vertical_mode = lock_state()
        .as_ref()
        .is_some_and(|s| s.vertical_taskbar_mode);
    if vertical_mode {
        // Only the dedicated top grip starts a drag. Quota content stays
        // available for hover tooltips without moving the component.
        let state = lock_state();
        let Some(state) = state.as_ref() else { return false };
        return vertical_drag_handle_hit(
            client_x, client_y,
            total_widget_width_for_state(state), widget_height_for_state(state),
        );
    }
    let hit_h = sc(DRAG_HANDLE_HIT_H);
    let widget_height = {
        let state = lock_state();
        state
            .as_ref()
            .map(widget_height_for_state)
            .unwrap_or(sc(AppearancePreset::Default.metrics().widget_height))
    };
    let hit_top = (widget_height - hit_h).max(0) / 2;
    client_x >= 0
        && client_x < sc(DRAG_HANDLE_HIT_W)
        && client_y >= hit_top
        && client_y < (hit_top + hit_h).min(widget_height)
}

fn cursor_is_on_drag_handle(hwnd: HWND) -> bool {
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() || !ScreenToClient(hwnd, &mut pt).as_bool() {
            return false;
        }
        is_drag_handle_point(pt.x, pt.y)
    }
}

fn is_small_taskbar_height_at_dpi(taskbar_height: i32, dpi: u32) -> bool {
    let threshold = (SMALL_TASKBAR_THRESHOLD as f64 * dpi as f64 / 96.0).round() as i32;
    taskbar_height <= threshold
}

fn is_small_taskbar_height(taskbar_height: i32) -> bool {
    is_small_taskbar_height_at_dpi(taskbar_height, CURRENT_DPI.load(Ordering::Relaxed))
}

fn effective_appearance_preset(state: &AppState) -> AppearancePreset {
    match state.appearance_preset.configured() {
        AppearancePreset::Default => state.effective_appearance_preset,
        AppearancePreset::Minimal => AppearancePreset::Minimal,
        AppearancePreset::Adaptive => unreachable!(),
    }
}

// The configured Default is always adaptive, without changing stored settings.
fn vertical_effective_preset(preset: AppearancePreset) -> AppearancePreset {
    preset.configured()
}

fn vertical_widget_height_for_preset(preset: AppearancePreset) -> i32 {
    sc(if preset == AppearancePreset::Minimal {
        VERTICAL_MINIMAL_WIDGET_HEIGHT
    } else {
        VERTICAL_WIDGET_HEIGHT
    })
}

fn desktop_vertical_content_height(
    preset: AppearancePreset, show_session: bool, show_weekly: bool,
) -> i32 {
    let rows = i32::from(show_session) + i32::from(show_weekly);
    if rows == 0 { return sc(DESKTOP_PANEL_CONTENT_PADDING * 2); }
    let (row_height, row_gap) = if preset == AppearancePreset::Minimal {
        (DESKTOP_MINIMAL_ROW_HEIGHT, DESKTOP_MINIMAL_ROW_GAP)
    } else {
        (VERTICAL_QUOTA_ROW_HEIGHT, VERTICAL_QUOTA_ROW_GAP)
    };
    sc(DESKTOP_PANEL_CONTENT_PADDING * 2 + row_height * rows
        + row_gap * (rows - 1))
}

fn widget_height_for_state(state: &AppState) -> i32 {
    if state.vertical_taskbar_mode {
        if state.floating_mode {
            return desktop_vertical_content_height(
                effective_appearance_preset(state),
                state.show_session_window, state.show_weekly_window,
            );
        }
        return vertical_widget_height_for_preset(effective_appearance_preset(state));
    }
    if state.small_taskbar_mode {
        sc(SMALL_WIDGET_HEIGHT)
    } else {
        sc(effective_appearance_preset(state).metrics().widget_height)
    }
}

fn current_appearance_preset() -> AppearancePreset {
    let state = lock_state();
    state
        .as_ref()
        .map(effective_appearance_preset)
        .unwrap_or_default()
}

fn widget_hidden_for_taskbar_space() -> bool {
    let state = lock_state();
    state
        .as_ref()
        .map(|s| s.hidden_for_taskbar_space)
        .unwrap_or(false)
}

fn current_theme_style() -> ThemeStyle {
    let state = lock_state();
    state
        .as_ref()
        .map(|s| s.styles.active(s.is_dark).clone())
        .unwrap_or_else(ThemeStyle::dark_default)
}

fn current_style_color(target: StyleColorTarget) -> Color {
    current_theme_style().color(target)
}

fn row_bar_segment_count(preset: AppearancePreset) -> i32 {
    match preset {
        AppearancePreset::Default | AppearancePreset::Adaptive => 8,
        AppearancePreset::Minimal => 6,
    }
}

fn usage_layout_widths(_language: LanguageId, preset: AppearancePreset) -> (i32, i32) {
    let metrics = preset.metrics();
    (metrics.label_width, metrics.reset_width)
}

fn usage_percent_for_display(_language: LanguageId, used_percentage: f64) -> f64 {
    poller::remaining_percentage(used_percentage)
}

fn total_widget_width_for_preset(language: LanguageId, preset: AppearancePreset) -> i32 {
    let metrics = preset.metrics();
    if preset == AppearancePreset::Minimal {
        return sc(DRAG_HANDLE_HIT_W)
            + sc(metrics.outer_padding)
            + sc(metrics.percent_width)
            + sc(metrics.outer_padding);
    }

    let bar_segments = row_bar_segment_count(preset);
    let (label_width, reset_width) = usage_layout_widths(language, preset);
    let progress_width = (sc(SEGMENT_W) + sc(SEGMENT_GAP)) * bar_segments - sc(SEGMENT_GAP);
    let usage_width = progress_width
        + sc(metrics.bar_percent_gap)
        + sc(metrics.percent_width)
        + if reset_width > 0 {
            sc(metrics.percent_reset_gap) + sc(reset_width)
        } else {
            0
        };

    sc(DRAG_HANDLE_HIT_W)
        + sc(metrics.outer_padding)
        + sc(label_width)
        + sc(metrics.label_bar_gap)
        + usage_width
        + sc(metrics.outer_padding)
}

fn total_widget_width_for(language: LanguageId) -> i32 {
    total_widget_width_for_preset(language, AppearancePreset::Default)
}

fn horizontal_content_left(metrics: &appearance::StyleMetrics, floating: bool) -> i32 {
    if floating {
        sc(DESKTOP_PANEL_CONTENT_PADDING)
    } else {
        sc(metrics.outer_padding) + sc(DRAG_HANDLE_HIT_W)
    }
}

/// The desktop tab is outside the body; no embedded drag-strip width is
/// needed. Compute the right edge from the widest *visible* usage row.
/// A missing reset label also removes its unused column from the panel.
fn desktop_horizontal_content_width(
    language: LanguageId, preset: AppearancePreset,
    show_session: bool, show_weekly: bool, session: &str, weekly: &str,
) -> i32 {
    let metrics = preset.metrics();
    let padding = sc(DESKTOP_PANEL_CONTENT_PADDING);
    if preset == AppearancePreset::Minimal {
        return padding * 2 + sc(metrics.percent_width);
    }
    let (label, reset) = usage_layout_widths(language, preset);
    let segment_count = row_bar_segment_count(preset);
    let bar = segment_count * (sc(SEGMENT_W) + sc(SEGMENT_GAP)) - sc(SEGMENT_GAP);
    let row_width = |value: &str| {
        sc(label) + sc(metrics.label_bar_gap) + bar
            + sc(metrics.bar_percent_gap) + sc(metrics.percent_width)
            + if value.split_once("  ").is_some() && reset > 0 {
                sc(metrics.percent_reset_gap) + sc(reset)
            } else { 0 }
    };
    let mut content = 0;
    if show_session { content = content.max(row_width(session)); }
    if show_weekly { content = content.max(row_width(weekly)); }
    padding * 2 + content
}

fn total_widget_width_for_state(state: &AppState) -> i32 {
    if state.vertical_taskbar_mode {
        state.vertical_widget_width
    } else if state.floating_mode {
        desktop_horizontal_content_width(
            state.language, effective_appearance_preset(state),
            state.show_session_window, state.show_weekly_window,
            &state.codex_session_text, &state.codex_weekly_text,
        )
    } else {
        total_widget_width_for_preset(state.language, effective_appearance_preset(state))
    }
}

fn total_widget_width() -> i32 {
    let state = lock_state();
    state
        .as_ref()
        .map(total_widget_width_for_state)
        .unwrap_or_else(|| total_widget_width_for(LanguageId::English))
}

fn quota_bar_color(displayed_percent: f64) -> Color {
    let remaining = displayed_percent.clamp(0.0, 100.0);
    if remaining > 50.0 {
        current_style_color(StyleColorTarget::ProgressHigh)
    } else if remaining > 20.0 {
        current_style_color(StyleColorTarget::ProgressMedium)
    } else {
        current_style_color(StyleColorTarget::ProgressLow)
    }
}
pub fn run() {
    // Enable Per-Monitor DPI Awareness V2 for crisp rendering at any scale factor
    unsafe {
        InitCommonControls();
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CURRENT_DPI.store(GetDpiForSystem(), Ordering::Relaxed);
    }
    diagnose::log("window::run started");

    // Single-instance guard: silently exit if another instance is running.
    // Exception: when relaunched after an explorer restart (ENV_RELAUNCH set),
    // wait for the previous instance to release the mutex, then take over.
    let is_relaunch = std::env::var(ENV_RELAUNCH).is_ok();
    let mutex_name = native_interop::wide_str("Global\\CodexUsageWin");
    let _mutex = unsafe {
        let handle = CreateMutexW(None, true, PCWSTR::from_raw(mutex_name.as_ptr()));
        match handle {
            Ok(h) => {
                if GetLastError() == ERROR_ALREADY_EXISTS {
                    if is_relaunch {
                        diagnose::log("relaunch: waiting for previous instance to exit");
                        let wait_result = WaitForSingleObject(h, 10_000);
                        if wait_result != WAIT_OBJECT_0 && wait_result != WAIT_ABANDONED {
                            diagnose::log(format!(
                                "startup aborted: previous instance did not exit cleanly ({wait_result:?})"
                            ));
                            return;
                        }
                    } else {
                        diagnose::log("startup aborted: another instance is already running");
                        return;
                    }
                }
                h
            }
            Err(error) => {
                diagnose::log_error(
                    "startup aborted: unable to create single-instance mutex",
                    error,
                );
                return;
            }
        }
    };

    migrate_legacy_startup_entry();

    let class_name = native_interop::wide_str("CodexUsageWin");

    unsafe {
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let (large_icon, small_icon) = load_embedded_app_icons();

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hIcon: large_icon,
            hIconSm: small_icon,
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };

        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            diagnose::log("RegisterClassExW returned 0");
        }

        let settings = load_settings();
        let language_override = settings
            .language
            .as_deref()
            .and_then(LanguageId::from_code)
            .map(LanguageId::ui_supported);
        let language = localization::resolve_language(language_override);

        // Create as layered popup (will be reparented into taskbar)
        let title = native_interop::wide_str(language.strings().window_title);
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            total_widget_width_for(language),
            sc(AppearancePreset::Default.metrics().widget_height),
            HWND::default(),
            HMENU::default(),
            hinstance,
            None,
        )
        .unwrap();

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

        diagnose::log(format!("main window created hwnd={:?}", hwnd));
        TASKBAR_PRIMARY_WINDOW.with(|cell| cell.set(hwnd.0 as isize));
        if !register_taskbar_replica_class() {
            diagnose::log("taskbar replica class registration failed; cross-taskbar drag unavailable");
        }

        let system_is_dark = theme::is_dark_mode();
        let is_dark = match settings.theme_mode {
            ThemeMode::System => system_is_dark,
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
        };
        let mut embedded = false;

        {
            let mut state = lock_state();
            *state = Some(AppState {
                hwnd: SendHwnd::from_hwnd(hwnd),
                taskbar_hwnd: None,
                tray_notify_hwnd: None,
                win_event_hook: None,
                is_dark,
                embedded: false,
                language_override,
                language,
                appearance_preset: settings.appearance_preset,
                display_position: settings.display_position,
                desktop_direction: settings.desktop_direction,
                effective_appearance_preset: match settings.appearance_preset {
                    AppearancePreset::Minimal => AppearancePreset::Minimal,
                    _ => AppearancePreset::Default,
                },
                hidden_for_taskbar_space: false,
                taskbar_space_warning_shown: false,
                floating_mode: false,
                floating_hwnd: None,
                parked_taskbar_foreground: None,
                floating_x: settings.floating_x.unwrap_or(i32::MIN),
                floating_y: settings.floating_y.unwrap_or(i32::MIN),
                floating_edge: settings.floating_edge.as_deref().and_then(FloatingEdge::parse),
                floating_collapsed: false,
                floating_reveal: 100,
                floating_grip_alpha: 0,
                floating_slide_from: 100,
                floating_slide_target: 100,
                floating_slide_started: Instant::now(),
                floating_fade_from: 0,
                floating_fade_target: 0,
                floating_fade_started: Instant::now(),
                floating_grip_hover: false,
                floating_first_collapse_guard: false,
                theme_mode: settings.theme_mode,
                styles: settings.styles.clone(),
                composition_blur_active: false,
                small_taskbar_mode: false,
                small_show_weekly: false,
                minimal_hover_target: None,
                drag_handle_hovered: false,
                codex_session_percent: 0.0,
                codex_session_text: "--".to_string(),
                codex_weekly_percent: 0.0,
                codex_weekly_text: "--".to_string(),
                show_session_window: settings.show_session_window,
                show_weekly_window: settings.show_weekly_window,
                alert_threshold_percent: settings.alert_threshold_percent,
                notified_quota_windows: settings.notified_quota_windows.into_iter().collect(),
                data: None,
                poll_interval_ms: settings.poll_interval_ms,
                retry_count: 0,
                force_notify_auth_error: false,
                auth_error_paused_polling: false,
                auth_watch_mode: poller::CredentialWatchMode::ActiveSource,
                auth_watch_snapshot: Vec::new(),
                last_poll_ok: false,
                available_update_version: None,
                taskbar_index: settings.taskbar_index,
                taskbar_monitor: settings.taskbar_monitor.clone(),
                taskbar_left_offset: settings.taskbar_left_offset.unwrap_or(0),
                taskbar_top_offset: settings.taskbar_top_offset.unwrap_or(0),
                vertical_taskbar_mode: false,
                vertical_widget_width: sc(VERTICAL_WIDGET_MIN_WIDTH),
                legacy_tray_offset: settings
                    .taskbar_left_offset
                    .is_none()
                    .then_some(settings.tray_offset),
                dragging: false,
                drag_anchor_logical_x: 0,
                drag_anchor_logical_y: 0,
                floating_drag_anchor_x: 0,
                floating_drag_anchor_y: 0,
                drag_reparenting: false,
        failed_drag_target: None,
            });
        }

        // Prefer the stable monitor identity. taskbar_index is retained only
        // as a compatibility fallback for settings written by older versions.
        if attach_to_preferred_taskbar(
            hwnd,
            settings.taskbar_monitor.as_deref(),
            settings.taskbar_index,
        ) {
            embedded = true;
        }

        // If not embedded, fall back to topmost popup with SetLayeredWindowAttributes
        if !embedded {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }

        // Register system tray icon(s)
        sync_tray_icons(hwnd);

        // UI Automation must never run synchronously on this UI thread after the
        // widget has been embedded into Explorer. The dedicated MTA worker keeps
        // a cache and posts a lightweight update message back to this window.
        spawn_taskbar_layout_worker(hwnd);

        // Position and show when a safe taskbar slot exists. If space is
        // temporarily exhausted, position_at_taskbar() keeps the widget hidden.
        position_at_taskbar();
        if !widget_hidden_for_taskbar_space() {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            diagnose::log("window shown");
        }

        // Initial render via UpdateLayeredWindow (for embedded) or InvalidateRect (fallback)
        render_layered();

        #[cfg(feature = "github-update")]
        {
            let update_success_notified =
                if let Some(version) = updater::successful_update_version_from_args() {
                    let strings = language.strings();
                    let message = format!("{} v{}", strings.update_success, version);
                    tray_icon::notify_info(
                        hwnd,
                        tray_icon::TrayIconKind::Codex,
                        strings.update_title,
                        &message,
                    );
                    true
                } else {
                    false
                };
            updater::start_startup_update_check(hwnd, update_success_notified);
        }

        // Poll timer: 15 minutes
        let initial_poll_ms = {
            let state = lock_state();
            state
                .as_ref()
                .map(|s| s.poll_interval_ms)
                .unwrap_or(POLL_15_MIN)
        };
        SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, initial_poll_ms, None);
        // Watch for explorer.exe restarts so we can re-embed and re-add the tray
        // icon (the shell discards tray registrations when it restarts). This
        // runs on a dedicated thread, NOT a window timer: once explorer destroys
        // the taskbar, our embedded child window stops receiving all messages
        // (WM_TIMER included), so a timer would never fire again.
        spawn_taskbar_watchdog();

        // Initial poll
        let send_hwnd = SendHwnd::from_hwnd(hwnd);
        std::thread::spawn(move || {
            diagnose::log("initial poll thread started");
            do_poll(send_hwnd);
        });

        // Initial theme check
        check_theme_change();

        // Message loop
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, HWND::default(), 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn blur_backdrop_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_blur_backdrop_class() {
    unsafe {
        let class_name = native_interop::wide_str("CodexUsageBlurBackdrop");
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(blur_backdrop_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
    }
}

fn preview_frame_due(last: &Mutex<Option<Instant>>, interval_ms: u64, force: bool) -> bool {
    let now = Instant::now();
    let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
    if force
        || last
            .map(|previous| now.duration_since(previous) >= Duration::from_millis(interval_ms))
            .unwrap_or(true)
    {
        *last = Some(now);
        true
    } else {
        false
    }
}

fn render_style_preview(force: bool) {
    if preview_frame_due(&LAST_STYLE_PREVIEW_RENDER, STYLE_PREVIEW_FRAME_MS, force) {
        render_layered();
    }
}

fn drag_frame_due(force: bool) -> bool {
    preview_frame_due(&LAST_DRAG_FRAME, DRAG_FRAME_MS, force)
}

/// Render the foreground widget through UpdateLayeredWindow. Frosted mode keeps
/// the foreground layered and uses a separate Windows Composition Gaussian backdrop.
fn render_layered() {
    render_layered_at(None);
}

/// Commit a new desktop grip direction and canvas size together with its
/// screen position. This avoids exposing stale pixels between SetWindowPos
/// and UpdateLayeredWindow when the user releases the mouse.
fn render_layered_at(desktop_origin: Option<POINT>) {
    refresh_dpi();
    if widget_hidden_for_taskbar_space() {
        return;
    }
    let (
        hwnd_val,
        is_dark,
        language,
        strings,
        style,
        codex_session_pct,
        codex_session_text,
        codex_weekly_pct,
        codex_weekly_text,
        show_session_window,
        show_weekly_window,
        last_poll_ok,
    ) = {
        let state = lock_state();
        match state.as_ref() {
            Some(s) => (
                s.hwnd,
                s.is_dark,
                s.language,
                s.language.strings(),
                s.styles.active(s.is_dark).clone(),
                s.codex_session_percent,
                s.codex_session_text.clone(),
                s.codex_weekly_percent,
                s.codex_weekly_text.clone(),
                s.show_session_window,
                s.show_weekly_window,
                s.last_poll_ok,
            ),
            None => return,
        }
    };

    let hwnd = hwnd_val.to_hwnd();
    let frosted_strength = style.panel_frosted_strength.min(FROSTED_STRENGTH_MAX);
    let floating = lock_state().as_ref().is_some_and(|s| s.floating_mode);
    // Embedded and desktop panels use the same Gaussian-blur theme path.
    // The desktop backdrop is clipped to the original body (never the tab).
    let blur_requested = style.panel_frosted_active();
    let blur_amount = blur_amount_for_strength(frosted_strength);
    let blur_tint = style.color(StyleColorTarget::PanelBackground);
    let mut composition_blur_active = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| s.composition_blur_active)
            .unwrap_or(false)
    };

    if blur_requested {
        if composition_blur_active
            && ensure_blur_backdrop(blur_amount, blur_tint).is_none()
        {
            composition_blur_active = false;
            let mut state = lock_state();
            if let Some(s) = state.as_mut() {
                s.composition_blur_active = false;
            }
        }
        if !composition_blur_active {
            composition_blur_active =
                activate_blur_backdrop(hwnd, blur_amount, blur_tint);
        }
    } else if composition_blur_active {
        restore_layered_taskbar_mode(hwnd);
        composition_blur_active = false;
    }

    let embedded = lock_state().as_ref().is_some_and(|s| s.embedded);
    if !embedded && !composition_blur_active && !floating {
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
            let _ = UpdateWindow(hwnd);
        }
        return;
    }

    let width = total_widget_width();
    let height = {
        let state = lock_state();
        state
            .as_ref()
            .map(widget_height_for_state)
            .unwrap_or(sc(AppearancePreset::Default.metrics().widget_height))
    };
    let bg_color = if is_dark {
        Color::from_hex("#1C1C1CFF")
    } else {
        Color::from_hex("#F3F3F3FF")
    };
    let mut surface_style = style.clone();
    let background = style.color(StyleColorTarget::PanelBackground);
    if composition_blur_active {
        // Composition owns the visible background, so the layered foreground only
        // needs a virtually invisible alpha to keep blank areas hit-testable.
        surface_style.panel_background = Color::rgba(
            background.r,
            background.g,
            background.b,
            MIN_INTERACTIVE_ALPHA,
        )
        .to_hex_rgba();
    } else if background.a == 0 {
        // A fully transparent normal panel must remain interactive as well.
        // Alpha=1 is visually indistinguishable from transparent but prevents
        // Windows from treating the panel interior as mouse-pass-through.
        surface_style.panel_background = Color::rgba(
            background.r,
            background.g,
            background.b,
            MIN_INTERACTIVE_ALPHA,
        )
        .to_hex_rgba();
    }

    let border = style.color(StyleColorTarget::PanelBorder);
    if border.a == 0 {
        surface_style.panel_border =
            Color::rgba(border.r, border.g, border.b, MIN_INTERACTIVE_ALPHA).to_hex_rgba();
    }

    unsafe {
        let screen_dc = GetDC(hwnd);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let mem_dc = CreateCompatibleDC(screen_dc);
        let dib =
            CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
        if dib.is_invalid() || bits.is_null() {
            let _ = DeleteDC(mem_dc);
            ReleaseDC(hwnd, screen_dc);
            return;
        }

        let old_bmp = SelectObject(mem_dc, dib);
        let pixel_count = (width * height) as usize;
        let pixel_data = std::slice::from_raw_parts_mut(bits as *mut u32, pixel_count);
        fill_bitmap(pixel_data, bg_color);

        blend_panel_bitmap(pixel_data, width, height, &surface_style);
        let panel_pixels = pixel_data.to_vec();

        paint_content(
            mem_dc,
            width,
            height,
            is_dark,
            &bg_color,
            language,
            strings,
            codex_session_pct,
            &codex_session_text,
            codex_weekly_pct,
            &codex_weekly_text,
            show_session_window,
            show_weekly_window,
            last_poll_ok,
            false,
        );

        finalize_layered_bitmap(
            pixel_data,
            &panel_pixels,
            width,
            height,
            &surface_style,
        );

        render_taskbar_text(
            pixel_data,
            width,
            height,
            &surface_style,
            &style,
            &bg_color,
            language,
            strings,
            codex_session_pct,
            codex_weekly_pct,
            &codex_session_text,
            &codex_weekly_text,
            show_session_window,
            show_weekly_window,
            last_poll_ok,
        );

        let drag_hovered = lock_state().as_ref().is_some_and(|s| s.drag_handle_hovered);
        let accent = drag_hover_outline_color(&style);
        if !floating && drag_hovered {
            draw_drag_hover_outline(
                pixel_data, width, height,
                RECT { left: 0, top: 0, right: width, bottom: height },
                style.panel_corner_radius, accent,
            );
        }

        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: 0,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: 1,
        };
        if floating {
            let (edge, hidden, hovered, grip_alpha) = {
                let state = lock_state();
                state.as_ref().map(|s| (
                    s.floating_edge,
                    s.floating_collapsed && s.floating_reveal == 0,
                    s.drag_handle_hovered,
                    if s.floating_edge.is_some() || s.dragging { 100 }
                    else { s.floating_grip_alpha },
                )).unwrap_or((None, false, false, 0))
            };
            // Composite the already rendered taskbar-size panel into a separate
            // transparent desktop canvas. Neither the panel border nor quota
            // graphics are ever stretched or shifted to make room for the tab.
            let (canvas_w, canvas_h, body) = floating_canvas_geometry(width, height, edge);
            let canvas_info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: canvas_w,
                    biHeight: -canvas_h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let canvas_dc = CreateCompatibleDC(screen_dc);
            let mut canvas_bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let canvas_dib = CreateDIBSection(
                canvas_dc, &canvas_info, DIB_RGB_COLORS, &mut canvas_bits, None, 0,
            ).unwrap_or_default();
            if !canvas_dib.is_invalid() && !canvas_bits.is_null() {
                let old_canvas_bmp = SelectObject(canvas_dc, canvas_dib);
                let canvas = std::slice::from_raw_parts_mut(
                    canvas_bits as *mut u32, (canvas_w * canvas_h) as usize,
                );
                canvas.fill(0);
                if !hidden {
                    for y in 0..height {
                        let dst = ((y + body.top) * canvas_w + body.left) as usize;
                        let src = (y * width) as usize;
                        canvas[dst..dst + width as usize]
                            .copy_from_slice(&pixel_data[src..src + width as usize]);
                    }
                }
                render_floating_handle_bitmap(
                    canvas, canvas_w, canvas_h, edge, hovered, &style, grip_alpha,
                );
                if hovered && !hidden {
                    // Separate visual rings: one around the actual desktop
                    // body and one around its independently positioned tab.
                    draw_drag_hover_outline(
                        canvas, canvas_w, canvas_h, body, style.panel_corner_radius, accent,
                    );
                    let tab = floating_grip_rect(
                        canvas_w, canvas_h, edge, sc(FLOAT_EDGE_PEEK),
                    );
                    draw_drag_hover_outline(
                        canvas, canvas_w, canvas_h, tab, style.panel_corner_radius, accent,
                    );
                }
                let sz = SIZE { cx: canvas_w, cy: canvas_h };
                let _ = UpdateLayeredWindow(
                    hwnd, screen_dc, desktop_origin.as_ref().map(|p| p as *const POINT), Some(&sz),
                    canvas_dc, Some(&pt_src),
                    COLORREF(0), Some(&blend), ULW_ALPHA,
                );
                SelectObject(canvas_dc, old_canvas_bmp);
            }
            if !canvas_dib.is_invalid() { let _ = DeleteObject(canvas_dib); }
            let _ = DeleteDC(canvas_dc);
        } else {
            let sz = SIZE { cx: width, cy: height };
            let _ = UpdateLayeredWindow(
                hwnd, screen_dc, None, Some(&sz), mem_dc, Some(&pt_src),
                COLORREF(0), Some(&blend), ULW_ALPHA,
            );
        }
        SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(dib);
        let _ = DeleteDC(mem_dc);
        ReleaseDC(hwnd, screen_dc);
    }

    if composition_blur_active {
        sync_blur_backdrop_geometry(hwnd);
    }
}

/// Keep the drag dots fixed-size. Hover feedback is now on the full panel
/// outline (and on the independent desktop tab), so it cannot be covered
/// by the pointer sitting directly above the small grip.
fn drag_handle_dot_style(color: Color, _hovered: bool, vertical: bool) -> (i32, i32, i32, Color) {
    let dot = sc(2).max(1);
    let compact_gap = sc(1).max(1);
    let long_gap = sc(2).max(1);
    let (gap_x, gap_y) = if vertical {
        (compact_gap, long_gap)
    } else {
        (long_gap, compact_gap)
    };
    (dot, gap_x, gap_y, color)
}

/// Derive a legible outline from the selected preset or customized palette.
/// ProgressHigh is the theme accent; fall back to the text color if a custom
/// theme makes that accent indistinguishable from its panel background.
fn drag_hover_outline_color(style: &ThemeStyle) -> Color {
    let background = style.color(StyleColorTarget::PanelBackground);
    let accent = style.color(StyleColorTarget::ProgressHigh);
    let separated = |candidate: Color| {
        (i32::from(candidate.r) - i32::from(background.r)).abs()
            + (i32::from(candidate.g) - i32::from(background.g)).abs()
            + (i32::from(candidate.b) - i32::from(background.b)).abs()
    };
    let mut chosen = if accent.a > 20 && separated(accent) >= 150 {
        accent
    } else {
        style.color(StyleColorTarget::Remaining)
    };
    if chosen.a <= 20 || separated(chosen) < 150 {
        chosen = if u32::from(background.r) * 299
            + u32::from(background.g) * 587
            + u32::from(background.b) * 114 > 128_000 {
            Color::rgba(18, 92, 132, 255)
        } else {
            Color::rgba(112, 214, 238, 255)
        };
    }
    Color::rgba(chosen.r, chosen.g, chosen.b, 255)
}

/// A narrow highlight stroke sits on a broader Gaussian-like falloff.
/// Free-floating desktop canvases reserve transparent pixels beyond the body
/// and tab; attached taskbar and docked edge geometry remain unchanged.
fn drag_hover_glow_coverage(distance_from_edge: f32) -> u8 {
    let distance = distance_from_edge.abs();
    // Soft halo falls off on either side of the visible panel perimeter.
    let diffuse = 120.0 * (-(distance / 4.6).powi(2) * 0.5).exp();
    // Retain the antialiased hairline while making the outer halo dominant.
    let hairline = 65.0 * (1.0 - distance / 0.7).max(0.0);
    (diffuse + hairline).round().clamp(0.0, 200.0) as u8
}

/// Rounded-rectangle signed distance. Negative is inside the actual panel,
/// positive is outside; returns zero at its edge.
fn drag_hover_signed_edge_distance(width: i32, height: i32, x: i32, y: i32, radius: i32) -> f32 {
    let inset = sc(1).max(1) as f32;
    let half_w = (width as f32 / 2.0 - inset).max(0.5);
    let half_h = (height as f32 / 2.0 - inset).max(0.5);
    let radius = (radius as f32).min(half_w.min(half_h)).max(0.0);
    let dx = (x as f32 + 0.5 - width as f32 / 2.0).abs() - (half_w - radius);
    let dy = (y as f32 + 0.5 - height as f32 / 2.0).abs() - (half_h - radius);
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius
}

/// Feather hover feedback around the actual body or detached grip perimeter.
/// The free-floating desktop canvas permits 12 logical pixels of outer glow,
/// while docked and taskbar modes keep their previous hit-test geometry.
fn draw_drag_hover_outline(
    pixels: &mut [u32], canvas_w: i32, canvas_h: i32,
    bounds: RECT, corner_radius: u8, color: Color,
) {
    if canvas_w <= 0 || canvas_h <= 0 || bounds.right <= bounds.left
        || bounds.bottom <= bounds.top { return; }
    let w = bounds.right - bounds.left;
    let h = bounds.bottom - bounds.top;
    let radius = sc(i32::from(corner_radius))
        .min((w.min(h) / 2).max(0));
    let extent = sc(12).max(2);
    // Restrict work to the edge envelope, avoiding per-pixel work over the
    // center of larger horizontal panels.
    for y in bounds.top.saturating_sub(extent).max(0)
        ..bounds.bottom.saturating_add(extent).min(canvas_h) {
        for x in bounds.left.saturating_sub(extent).max(0)
            ..bounds.right.saturating_add(extent).min(canvas_w) {
            let local_x = x - bounds.left;
            let local_y = y - bounds.top;
            let distance = drag_hover_signed_edge_distance(w, h, local_x, local_y, radius);
            if distance.abs() > extent as f32 { continue; }
            let coverage = drag_hover_glow_coverage(distance);
            if coverage == 0 { continue; }
            let index = (y * canvas_w + x) as usize;
            pixels[index] = composite_premultiplied_text(pixels[index], color, coverage);
        }
    }
}

/// Draw the dedicated desktop drag tab into the premultiplied layered surface.
/// When collapsed, every pixel outside the tab is fully transparent and cannot
/// obstruct other desktop windows or leave a visible panel edge.
fn render_floating_handle_bitmap(
    pixels: &mut [u32], width: i32, height: i32,
    edge: Option<FloatingEdge>, hovered: bool, style: &ThemeStyle, opacity: u8,
) {
    let grip = floating_grip_rect(width, height, edge, sc(FLOAT_EDGE_PEEK));
    // The tab is a miniature panel, not a separate style. Its fill, border,
    // corner radius, transparency and glass strength follow ThemeStyle.
    let center_x = (grip.left + grip.right) / 2;
    let center_y = (grip.top + grip.bottom) / 2;
    let vertical_grip = matches!(edge, Some(FloatingEdge::Left | FloatingEdge::Right));
    let (dot, gap_x, gap_y, dots) = drag_handle_dot_style(
        style.color(StyleColorTarget::DragHandle), hovered, vertical_grip,
    );
    // Long side of the six-dot grip follows the long side of the tab.
    let (cols, rows) = if vertical_grip { (2, 3) } else { (3, 2) };
    let dots_w = cols * dot + (cols - 1) * gap_x;
    let dots_h = rows * dot + (rows - 1) * gap_y;
    for y in grip.top.max(0)..grip.bottom.min(height) {
        for x in grip.left.max(0)..grip.right.min(width) {
            let local_x = x - grip.left;
            let local_y = y - grip.top;
            let grip_w = grip.right - grip.left;
            let grip_h = grip.bottom - grip.top;
            let Some(panel_color) = panel_color_at(
                style, grip_w, grip_h, local_x, local_y,
            ) else { continue };
            let dot_pixel = (0..rows).any(|r| (0..cols).any(|c| {
                let dx = center_x - dots_w / 2 + c * (dot + gap_x);
                let dy = center_y - dots_h / 2 + r * (dot + gap_y);
                x >= dx && x < dx + dot && y >= dy && y < dy + dot
            }));
            let mut color = if dot_pixel { dots } else { panel_color };
            color.a = ((u16::from(color.a) * u16::from(opacity.min(100)) + 50) / 100) as u8;
            pixels[(y * width + x) as usize] = premultiplied_pixel(color);
        }
    }
}

fn fill_bitmap(pixels: &mut [u32], color: Color) {
    let value = ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32;
    pixels.fill(value);
}

fn blend_pixel(pixel: u32, color: Color) -> u32 {
    if color.a == 0 {
        return pixel & 0x00FFFFFF;
    }
    if color.a == 255 {
        return ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32;
    }
    let bg_b = pixel & 0xFF;
    let bg_g = (pixel >> 8) & 0xFF;
    let bg_r = (pixel >> 16) & 0xFF;
    let a = color.a as u32;
    let inv = 255 - a;
    let r = (color.r as u32 * a + bg_r * inv + 127) / 255;
    let g = (color.g as u32 * a + bg_g * inv + 127) / 255;
    let b = (color.b as u32 * a + bg_b * inv + 127) / 255;
    (r << 16) | (g << 8) | b
}

fn point_in_rounded_box(
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    inset: i32,
    radius: i32,
) -> bool {
    if x < inset || x >= width - inset || y < inset || y >= height - inset {
        return false;
    }
    let radius = radius.max(1);
    let left = inset;
    let top = inset;
    let right = width - inset - 1;
    let bottom = height - inset - 1;
    let cx = if x < left + radius {
        left + radius
    } else if x > right - radius {
        right - radius
    } else {
        x
    };
    let cy = if y < top + radius {
        top + radius
    } else if y > bottom - radius {
        bottom - radius
    } else {
        y
    };
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}

fn blend_panel_bitmap(pixels: &mut [u32], width: i32, height: i32, style: &ThemeStyle) {
    let outer_inset = sc(1).max(1);
    let inner_inset = outer_inset + PANEL_BORDER_WIDTH_PX;
    let outer_radius = sc(i32::from(style.panel_corner_radius))
        .min(((width - outer_inset * 2).min(height - outer_inset * 2) / 2).max(0));
    let inner_radius = (outer_radius - PANEL_BORDER_WIDTH_PX).max(1);
    let border = style.color(StyleColorTarget::PanelBorder);
    let fill = style.color(StyleColorTarget::PanelBackground);
    for y in outer_inset..(height - outer_inset).max(outer_inset) {
        for x in outer_inset..(width - outer_inset).max(outer_inset) {
            let outer_inside = outer_radius == 0
                || point_in_rounded_box(width, height, x, y, outer_inset, outer_radius);
            if !outer_inside {
                continue;
            }
            let inner_inside = if outer_radius > 0 {
                point_in_rounded_box(width, height, x, y, inner_inset, inner_radius)
            } else {
                x >= inner_inset
                    && x < width - inner_inset
                    && y >= inner_inset
                    && y < height - inner_inset
            };
            let idx = (y * width + x) as usize;
            pixels[idx] = blend_pixel(pixels[idx], if inner_inside { fill } else { border });
        }
    }
}

fn panel_color_at(style: &ThemeStyle, width: i32, height: i32, x: i32, y: i32) -> Option<Color> {
    let outer_inset = sc(1).max(1);
    let inner_inset = outer_inset + PANEL_BORDER_WIDTH_PX;
    let outer_radius = sc(i32::from(style.panel_corner_radius))
        .min(((width - outer_inset * 2).min(height - outer_inset * 2) / 2).max(0));
    if outer_radius > 0 {
        if !point_in_rounded_box(width, height, x, y, outer_inset, outer_radius) {
            return None;
        }
        let inner_radius = (outer_radius - PANEL_BORDER_WIDTH_PX).max(1);
        if point_in_rounded_box(width, height, x, y, inner_inset, inner_radius) {
            Some(style.color(StyleColorTarget::PanelBackground))
        } else {
            Some(style.color(StyleColorTarget::PanelBorder))
        }
    } else if x < outer_inset
        || x >= width - outer_inset
        || y < outer_inset
        || y >= height - outer_inset
    {
        None
    } else if x >= inner_inset
        && x < width - inner_inset
        && y >= inner_inset
        && y < height - inner_inset
    {
        Some(style.color(StyleColorTarget::PanelBackground))
    } else {
        Some(style.color(StyleColorTarget::PanelBorder))
    }
}

fn premultiplied_pixel(color: Color) -> u32 {
    let alpha = color.a as u32;
    if alpha == 0 {
        return 0;
    }
    let r = (color.r as u32 * alpha + 127) / 255;
    let g = (color.g as u32 * alpha + 127) / 255;
    let b = (color.b as u32 * alpha + 127) / 255;
    (alpha << 24) | (r << 16) | (g << 8) | b
}

fn finalize_layered_bitmap(
    pixels: &mut [u32],
    panel_pixels: &[u32],
    width: i32,
    height: i32,
    style: &ThemeStyle,
) {
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            let Some(panel_color) = panel_color_at(style, width, height, x, y) else {
                pixels[idx] = 0;
                continue;
            };

            // Foreground graphics are kept opaque. Panel pixels retain the
            // configured RGBA alpha; the GDI text prepass is replaced afterward
            // by the final DirectWrite coverage render.
            if pixels[idx] != panel_pixels[idx] {
                pixels[idx] = (pixels[idx] & 0x00FFFFFF) | 0xFF000000;
            } else {
                pixels[idx] = premultiplied_pixel(panel_color);
            }
        }
    }
}

fn composite_premultiplied_text(dst: u32, color: Color, coverage: u8) -> u32 {
    let src_alpha = (coverage as u32 * color.a as u32 + 127) / 255;
    if src_alpha == 0 {
        return dst;
    }

    let inv = 255 - src_alpha;
    let dst_alpha = (dst >> 24) & 0xFF;
    let dst_r = (dst >> 16) & 0xFF;
    let dst_g = (dst >> 8) & 0xFF;
    let dst_b = dst & 0xFF;

    let out_alpha = src_alpha + (dst_alpha * inv + 127) / 255;
    let out_r = (color.r as u32 * src_alpha + dst_r * inv + 127) / 255;
    let out_g = (color.g as u32 * src_alpha + dst_g * inv + 127) / 255;
    let out_b = (color.b as u32 * src_alpha + dst_b * inv + 127) / 255;

    (out_alpha << 24) | (out_r << 16) | (out_g << 8) | out_b
}

fn restore_text_background_rect(
    pixels: &mut [u32],
    width: i32,
    height: i32,
    style: &ThemeStyle,
    rect: RECT,
) {
    let left = rect.left.clamp(0, width);
    let top = rect.top.clamp(0, height);
    let right = rect.right.clamp(left, width);
    let bottom = rect.bottom.clamp(top, height);

    for y in top..bottom {
        for x in left..right {
            let idx = (y * width + x) as usize;
            pixels[idx] = panel_color_at(style, width, height, x, y)
                .map(premultiplied_pixel)
                .unwrap_or(0);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HorizontalTextAlign { Start, InkStart, Center, End }

/// Align the visible ink, not the reserved width of a text column.
fn directwrite_mask_x_offset(
    mask: &[u8], width: i32, height: i32, align: HorizontalTextAlign,
) -> i32 {
    if width <= 0 || height <= 0 || align == HorizontalTextAlign::Start { return 0; }
    let mut first = width;
    let mut last = -1;
    for x in 0..width {
        if (0..height).any(|y| {
            mask.get((y * width + x) as usize).is_some_and(|&alpha| alpha != 0)
        }) {
            first = first.min(x);
            last = x;
        }
    }
    if last < 0 { return 0; }
    match align {
        HorizontalTextAlign::Start => 0,
        HorizontalTextAlign::InkStart => -first,
        HorizontalTextAlign::Center => (width - (last - first + 1)) / 2 - first,
        HorizontalTextAlign::End => width - 1 - last,
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_directwrite_text(
    pixels: &mut [u32],
    surface_width: i32,
    surface_height: i32,
    surface_style: &ThemeStyle,
    rect: RECT,
    text: &str,
    font_height: i32,
    font_weight: i32,
    color: Color,
    align: HorizontalTextAlign,
) {
    let mask_width = rect.right - rect.left;
    let mask_height = rect.bottom - rect.top;
    if text.is_empty() || mask_width <= 0 || mask_height <= 0 {
        return;
    }

    // Remove the provisional GDI glyphs in this text rectangle before
    // compositing the final DirectWrite grayscale coverage.
    restore_text_background_rect(pixels, surface_width, surface_height, surface_style, rect);

    let Some(mask) = native_interop::directwrite_text_mask(
        text,
        fonts::face(fonts::FontRole::Taskbar, None),
        font_height.abs() as f32,
        font_weight,
        mask_width,
        mask_height,
    ) else {
        return;
    };

    let x_offset = directwrite_mask_x_offset(&mask, mask_width, mask_height, align);
    for mask_y in 0..mask_height {
        let dst_y = rect.top + mask_y;
        if dst_y < 0 || dst_y >= surface_height {
            continue;
        }
        for mask_x in 0..mask_width {
            let dst_x = rect.left + mask_x + x_offset;
            if dst_x < 0 || dst_x >= surface_width {
                continue;
            }
            let coverage = mask[(mask_y * mask_width + mask_x) as usize];
            if coverage == 0 {
                continue;
            }
            let idx = (dst_y * surface_width + dst_x) as usize;
            pixels[idx] = composite_premultiplied_text(pixels[idx], color, coverage);
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn render_taskbar_text(
    pixels: &mut [u32],
    width: i32,
    height: i32,
    surface_style: &ThemeStyle,
    style: &ThemeStyle,
    bg: &Color,
    language: LanguageId,
    strings: Strings,
    codex_session_pct: f64,
    codex_weekly_pct: f64,
    codex_session_text: &str,
    codex_weekly_text: &str,
    show_session_window: bool,
    show_weekly_window: bool,
    last_poll_ok: bool,
) {
    if lock_state().as_ref().is_some_and(|s| s.vertical_taskbar_mode) {
        // Poller data stores used quota. Convert exactly once for the layered
        // vertical renderer, matching the horizontal and GDI fallback paths.
        let codex_session_pct = usage_percent_for_display(language, codex_session_pct);
        let codex_weekly_pct = usage_percent_for_display(language, codex_weekly_pct);
        let is_minimal = current_appearance_preset() == AppearancePreset::Minimal;
        let panel_base = style.color(StyleColorTarget::PanelBackground).blend_over(*bg);
        let label_color = style.color(StyleColorTarget::QuotaType).blend_over(panel_base);
        let value_color = style.color(if last_poll_ok {
            StyleColorTarget::Remaining
        } else {
            StyleColorTarget::Error
        }).blend_over(panel_base);
        let reset_color = style.color(StyleColorTarget::ResetTime).blend_over(panel_base);
        let track = style.color(StyleColorTarget::ProgressConsumed).blend_over(panel_base);
        if is_minimal {
            // Compact vertical view renders only two remaining percentages:
            // no rings, type captions or reset timestamps.
            let (session_center, weekly_center) = vertical_minimal_percent_centers(
                height, show_session_window, show_weekly_window,
            );
            let cx = width / 2;
            for (center, pct) in [
                (session_center, codex_session_pct),
                (weekly_center, codex_weekly_pct),
            ] {
                let Some(cy) = center else { continue };
                let value = vertical_quota_text(pct, last_poll_ok);
                let size = if value.len() >= 4 { sc(9) } else { sc(12) };
                render_vertical_centered_text(
                    pixels, width, height, cx, cy, &value, size,
                    FW_SEMIBOLD.0 as i32, value_color,
                );
            }
            return;
        }
        let (session_reset, weekly_reset) = {
            let state = lock_state();
            let codex = state.as_ref().and_then(|s| s.data.as_ref())
                .and_then(|data| data.codex.as_ref());
            (
                vertical_reset_label(codex.and_then(|c| c.session.resets_at), false),
                vertical_reset_label(codex.and_then(|c| c.weekly.resets_at), true),
            )
        };
        let (session_top, weekly_top) = vertical_row_tops(
            height, show_session_window, show_weekly_window,
        );
        let radius = (((width - sc(7)) as f32) * 0.5)
            .min(sc(VERTICAL_RING_MAX_RADIUS) as f32).max(1.0);
        for (top, label, pct, reset) in [
            (session_top, "5H", codex_session_pct, session_reset),
            (weekly_top, "7D", codex_weekly_pct, weekly_reset),
        ] {
            let Some(top) = top else { continue };
            let cx = width / 2;
            let cy = top + sc(VERTICAL_RING_CENTER_Y);
            render_vertical_ring(
                pixels, width, height, cx, cy, radius, pct,
                track, quota_bar_color(pct.clamp(0.0, 100.0)).blend_over(track),
                last_poll_ok,
            );
            render_vertical_centered_text(
                pixels, width, height, cx, top + sc(VERTICAL_LABEL_CENTER_Y),
                label, sc(10), FW_MEDIUM.0 as i32, label_color,
            );
            let percent = vertical_quota_text(pct, last_poll_ok);
            let percent_size = if percent.len() >= 4 { sc(8) } else { sc(11) };
            render_vertical_centered_text(
                pixels, width, height, cx, cy,
                &percent, percent_size, FW_SEMIBOLD.0 as i32, value_color,
            );
            render_vertical_centered_text(
                pixels, width, height, cx, top + sc(VERTICAL_RESET_CENTER_Y),
                &reset, sc(9), FW_NORMAL.0 as i32, reset_color,
            );
        }
        return;
    }

    let preset = current_appearance_preset();
    let metrics = preset.metrics();
    let (label_width, text_width) = usage_layout_widths(language, preset);
    let panel_base = style
        .color(StyleColorTarget::PanelBackground)
        .blend_over(*bg);
    let quota_type_color = style.color(StyleColorTarget::QuotaType).blend_over(panel_base);
    let primary_color = if last_poll_ok {
        style.color(StyleColorTarget::Remaining)
    } else {
        style.color(StyleColorTarget::Error)
    }
    .blend_over(panel_base);
    let reset_color = style.color(StyleColorTarget::ResetTime).blend_over(panel_base);

    let (small_taskbar_mode, small_show_weekly) = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| (s.small_taskbar_mode, s.small_show_weekly))
            .unwrap_or((false, false))
    };
    let effective_show_session = if small_taskbar_mode {
        !small_show_weekly
    } else {
        show_session_window
    };
    let effective_show_weekly = if small_taskbar_mode {
        small_show_weekly
    } else {
        show_weekly_window
    };

    let floating = lock_state().as_ref().is_some_and(|s| s.floating_mode);
    let content_x = horizontal_content_left(&metrics, floating);
    let visible_reset = floating && (
        (effective_show_session && codex_session_text.split_once("  ").is_some())
        || (effective_show_weekly && codex_weekly_text.split_once("  ").is_some())
    );
    let row2_y = height - sc(4) - sc(SEGMENT_H);
    let row1_y = row2_y - sc(metrics.row_gap) - sc(SEGMENT_H);
    let single_row_y = (height - sc(SEGMENT_H)) / 2;
    let row_height = sc(SEGMENT_H);

    let mut render_row = |y: i32, label: &str, value_text: &str| {
        if preset == AppearancePreset::Minimal {
            let primary = value_text
                .split_once("  ")
                .map(|(primary, _)| primary)
                .unwrap_or(value_text);
            draw_directwrite_text(
                pixels,
                width,
                height,
                surface_style,
                RECT {
                    left: content_x,
                    top: y,
                    right: content_x + sc(metrics.percent_width),
                    bottom: y + row_height,
                },
                primary,
                sc(metrics.value_font_height),
                FW_SEMIBOLD.0 as i32,
                primary_color,
                if floating { HorizontalTextAlign::Center } else { HorizontalTextAlign::Start },
            );
            return;
        }

        draw_directwrite_text(
            pixels,
            width,
            height,
            surface_style,
            RECT {
                left: content_x,
                top: y,
                right: content_x + sc(label_width),
                bottom: y + row_height,
            },
            label,
            sc(metrics.font_height),
            FW_MEDIUM.0 as i32,
            quota_type_color,
            if floating {
                HorizontalTextAlign::InkStart
            } else {
                HorizontalTextAlign::Start
            },
        );

        let segment_count = row_bar_segment_count(preset);
        let progress_width =
            segment_count * (sc(SEGMENT_W) + sc(SEGMENT_GAP)) - sc(SEGMENT_GAP);
        let text_x = content_x
            + sc(label_width)
            + sc(metrics.label_bar_gap)
            + progress_width
            + sc(metrics.bar_percent_gap);
        let (primary, secondary) = value_text
            .split_once("  ")
            .map(|(primary, secondary)| (primary, Some(secondary)))
            .unwrap_or((value_text, None));

        draw_directwrite_text(
            pixels,
            width,
            height,
            surface_style,
            RECT {
                left: text_x,
                top: y,
                right: text_x + sc(metrics.percent_width),
                bottom: y + row_height,
            },
            primary,
            sc(metrics.value_font_height),
            FW_SEMIBOLD.0 as i32,
            primary_color,
            if floating && !visible_reset {
                HorizontalTextAlign::End
            } else {
                HorizontalTextAlign::Start
            },
        );

        if let Some(secondary) = secondary {
            let secondary_x =
                text_x + sc(metrics.percent_width) + sc(metrics.percent_reset_gap);
            draw_directwrite_text(
                pixels,
                width,
                height,
                surface_style,
                RECT {
                    left: secondary_x,
                    top: y,
                    right: secondary_x + sc(text_width),
                    bottom: y + row_height,
                },
                secondary,
                sc(metrics.secondary_font_height),
                FW_NORMAL.0 as i32,
                reset_color,
                if floating { HorizontalTextAlign::End } else { HorizontalTextAlign::Start },
            );
        }
    };

    if effective_show_session {
        render_row(
            if effective_show_weekly { row1_y } else { single_row_y },
            strings.session_window,
            codex_session_text,
        );
    }
    if effective_show_weekly {
        render_row(
            if effective_show_session { row2_y } else { single_row_y },
            strings.weekly_window,
            codex_weekly_text,
        );
    }
}

fn vertical_row_tops(height: i32, show_session: bool, show_weekly: bool) -> (Option<i32>, Option<i32>) {
    vertical_row_tops_for_layout(
        height, show_session, show_weekly,
        lock_state().as_ref().is_some_and(|s| s.floating_mode),
    )
}

fn vertical_row_tops_for_layout(
    height: i32, show_session: bool, show_weekly: bool, floating: bool,
) -> (Option<i32>, Option<i32>) {
    let first = if floating {
        sc(DESKTOP_PANEL_CONTENT_PADDING)
    } else if show_session && show_weekly {
        sc(VERTICAL_FIRST_ROW_TOP)
    } else {
        ((height - sc(VERTICAL_QUOTA_ROW_HEIGHT)) / 2).max(sc(VERTICAL_DRAG_HANDLE_H))
    };
    let second = first + sc(VERTICAL_QUOTA_ROW_HEIGHT + VERTICAL_QUOTA_ROW_GAP);
    (
        show_session.then_some(first),
        show_weekly.then_some(if show_session { second } else { first }),
    )
}

fn vertical_minimal_percent_centers(
    height: i32, show_session: bool, show_weekly: bool,
) -> (Option<i32>, Option<i32>) {
    vertical_minimal_percent_centers_for_layout(
        height, show_session, show_weekly,
        lock_state().as_ref().is_some_and(|s| s.floating_mode),
    )
}

fn vertical_minimal_percent_centers_for_layout(
    height: i32, show_session: bool, show_weekly: bool, floating: bool,
) -> (Option<i32>, Option<i32>) {
    if floating {
        let center = sc(DESKTOP_PANEL_CONTENT_PADDING + DESKTOP_MINIMAL_ROW_HEIGHT / 2);
        let second = center + sc(DESKTOP_MINIMAL_ROW_HEIGHT + DESKTOP_MINIMAL_ROW_GAP);
        return (
            show_session.then_some(center),
            show_weekly.then_some(if show_session { second } else { center }),
        );
    }
    let one_center = (height + sc(VERTICAL_DRAG_HANDLE_H)) / 2;
    if show_session && show_weekly {
        (Some(sc(VERTICAL_MINIMAL_FIRST_PERCENT_Y)), Some(sc(VERTICAL_MINIMAL_SECOND_PERCENT_Y)))
    } else {
        (show_session.then_some(one_center), show_weekly.then_some(one_center))
    }
}

fn vertical_reset_label(reset_at: Option<SystemTime>, weekly: bool) -> String {
    let Some(local) = reset_at.and_then(native_interop::system_time_to_local) else {
        return "--".to_string();
    };
    if weekly {
        format!("{:02}/{:02}", local.wMonth, local.wDay)
    } else {
        format!("{:02}:{:02}", local.wHour, local.wMinute)
    }
}

/// Antialiased ring composited into the already-premultiplied layered bitmap.
/// Remaining quota follows a 324-degree clockwise arc, leaving a 36-degree
/// gap at 6 o'clock, including when remaining quota is 100%.
fn vertical_ring_fraction(angle: f32) -> Option<f32> {
    let sweep = std::f32::consts::TAU - 2.0 * VERTICAL_RING_GAP_HALF_RADIANS;
    let offset = (angle - VERTICAL_RING_START_RADIANS).rem_euclid(std::f32::consts::TAU);
    let fraction = offset / sweep;
    (0.0..=1.0).contains(&fraction).then_some(fraction)
}

#[allow(clippy::too_many_arguments)]
fn render_vertical_ring(
    pixels: &mut [u32],
    width: i32,
    height: i32,
    cx: i32,
    cy: i32,
    radius: f32,
    percent: f64,
    track: Color,
    accent: Color,
    has_data: bool,
) {
    let stroke = sc(3) as f32;
    let half_stroke = stroke * 0.5;
    let extent = (radius + half_stroke + 1.5).ceil() as i32;
    let completed = if has_data { (percent / 100.0).clamp(0.0, 1.0) as f32 } else { 0.0 };
    for y in (cy - extent).max(0)..(cy + extent + 1).min(height) {
        for x in (cx - extent).max(0)..(cx + extent + 1).min(width) {
            let dx = x as f32 + 0.5 - cx as f32;
            let dy = y as f32 + 0.5 - cy as f32;
            let distance = (dx * dx + dy * dy).sqrt();
            let coverage = (half_stroke + 0.5 - (distance - radius).abs()).clamp(0.0, 1.0);
            if coverage <= 0.0 { continue; }
            let mut angle = dx.atan2(-dy);
            if angle < 0.0 { angle += std::f32::consts::TAU; }
            let Some(fraction) = vertical_ring_fraction(angle) else { continue };
            let color = if completed > 0.0 && fraction <= completed {
                accent
            } else {
                track
            };
            // Antialias the open ends as well as the radial edges.
            let offset = (angle - VERTICAL_RING_START_RADIANS).rem_euclid(std::f32::consts::TAU);
            let sweep = std::f32::consts::TAU - 2.0 * VERTICAL_RING_GAP_HALF_RADIANS;
            let start_distance = offset * radius;
            let end_distance = (sweep - offset) * radius;
            let end_coverage = (start_distance.min(end_distance) + 0.5).clamp(0.0, 1.0);
            let opacity = (coverage * end_coverage * 255.0).round() as u8;
            if opacity == 0 { continue; }
            let index = (y * width + x) as usize;
            pixels[index] = composite_premultiplied_text(pixels[index], color, opacity);
        }
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn render_vertical_centered_text(
    pixels: &mut [u32],
    width: i32,
    height: i32,
    center_x: i32,
    center_y: i32,
    value: &str,
    font_size: i32,
    weight: i32,
    color: Color,
) {
    if value.is_empty() { return; }
    let mask_width = (width - sc(4)).max(1);
    let mask_height = sc(18).max(1);
    let Some(mask) = native_interop::directwrite_text_mask(
        value,
        fonts::face(fonts::FontRole::Taskbar, None),
        font_size as f32,
        weight,
        mask_width,
        mask_height,
    ) else { return };
    let mut min_x = mask_width;
    let mut min_y = mask_height;
    let mut max_x = -1;
    let mut max_y = -1;
    for y in 0..mask_height {
        for x in 0..mask_width {
            if mask[(y * mask_width + x) as usize] == 0 { continue; }
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    if max_x < 0 { return; }
    let origin_x = center_x - (min_x + max_x + 1) / 2;
    let origin_y = center_y - (min_y + max_y + 1) / 2;
    for y in min_y..=max_y {
        let target_y = origin_y + y;
        if target_y < 0 || target_y >= height { continue; }
        for x in min_x..=max_x {
            let target_x = origin_x + x;
            if target_x < 0 || target_x >= width { continue; }
            let coverage = mask[(y * mask_width + x) as usize];
            if coverage == 0 { continue; }
            let index = (target_y * width + target_x) as usize;
            pixels[index] = composite_premultiplied_text(pixels[index], color, coverage);
        }
    }
}

fn vertical_quota_text(percentage: f64, last_poll_ok: bool) -> String {
    if !last_poll_ok || !percentage.is_finite() {
        "--".to_string()
    } else {
        format!("{:.0}%", percentage.clamp(0.0, 100.0))
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_vertical_widget_content(
    hdc: HDC,
    width: i32,
    height: i32,
    session_pct: f64,
    weekly_pct: f64,
    show_session: bool,
    show_weekly: bool,
    last_poll_ok: bool,
    quota_type_color: Color,
    primary_color: Color,
    reset_color: Color,
    track: Color,
    drag_color: Color,
    hovered: bool,
    paint_text: bool,
    minimal: bool,
) {
    unsafe {
        if !lock_state().as_ref().is_some_and(|s| s.floating_mode) {
            let (dot, gap_x, gap_y, color) =
                drag_handle_dot_style(drag_color, hovered, false);
            let matrix_w = 3 * dot + 2 * gap_x;
            for row in 0..2 {
                for col in 0..3 {
                    let left = width / 2 - matrix_w / 2 + col * (dot + gap_x);
                    let top = sc(5) + row * (dot + gap_y);
                    let rect = RECT {
                        left, top, right: left + dot, bottom: top + dot,
                    };
                    draw_rounded_rect(hdc, &rect, &color, sc(1).max(1));
                }
            }
        }

        // The layered renderer composites rings and text with DirectWrite.
        // GDI is only needed for the compact fallback popup (when embedding
        // into Explorer is unavailable), not for the normal embedded path.
        if paint_text && minimal {
            let (session_center, weekly_center) = vertical_minimal_percent_centers(
                height, show_session, show_weekly,
            );
            let font_name = native_interop::wide_str(
                fonts::face(fonts::FontRole::Taskbar, None),
            );
            let _ = SetBkMode(hdc, TRANSPARENT);
            for (center, percent) in [
                (session_center, session_pct),
                (weekly_center, weekly_pct),
            ] {
                let Some(cy) = center else { continue };
                let label = vertical_quota_text(percent, last_poll_ok);
                let font_size = if label.len() >= 4 { sc(9) } else { sc(12) };
                let font = CreateFontW(
                    font_size, 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0,
                    DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
                    CLIP_DEFAULT_PRECIS.0 as u32, gdi_text_quality(),
                    (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                    PCWSTR::from_raw(font_name.as_ptr()),
                );
                let old_font = SelectObject(hdc, font);
                let _ = SetTextColor(hdc, COLORREF(primary_color.to_colorref()));
                let mut wide: Vec<u16> = label.encode_utf16().collect();
                let mut bounds = RECT {
                    left: sc(1), top: cy - sc(10), right: width - sc(1),
                    bottom: cy + sc(10),
                };
                let _ = DrawTextW(
                    hdc, &mut wide, &mut bounds,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
                let _ = SelectObject(hdc, old_font);
                let _ = DeleteObject(font);
            }
        } else if paint_text {
            let (session_top, weekly_top) = vertical_row_tops(height, show_session, show_weekly);
            let radius = ((width - sc(7)) / 2).min(sc(VERTICAL_RING_MAX_RADIUS)).max(1);
            let font_name = native_interop::wide_str(
                fonts::face(fonts::FontRole::Taskbar, None),
            );
            let _ = SetBkMode(hdc, TRANSPARENT);
            for (top, percent, label) in [
                (session_top, session_pct, "5H"),
                (weekly_top, weekly_pct, "7D"),
            ] {
                let Some(top) = top else { continue };
                let cx = width / 2;
                let cy = top + sc(VERTICAL_RING_CENTER_Y);
                let bounds = RECT {
                    left: cx - radius, top: cy - radius,
                    right: cx + radius + 1, bottom: cy + radius + 1,
                };
                let start = VERTICAL_RING_START_RADIANS;
                let sweep = std::f32::consts::TAU - 2.0 * VERTICAL_RING_GAP_HALF_RADIANS;
                let arc_point = |angle: f32| (
                    cx + (radius as f32 * angle.sin()).round() as i32,
                    cy - (radius as f32 * angle.cos()).round() as i32,
                );
                let (start_x, start_y) = arc_point(start);
                let (end_x, end_y) = arc_point(start + sweep);
                // GDI Arc runs counter-clockwise; reverse start/end coordinates
                // to render the same clockwise fill as the layered bitmap.
                let pen = CreatePen(PS_SOLID, sc(3), COLORREF(track.to_colorref()));
                let old_pen = SelectObject(hdc, pen);
                let _ = Arc(hdc, bounds.left, bounds.top, bounds.right, bounds.bottom,
                    end_x, end_y, start_x, start_y);
                let _ = SelectObject(hdc, old_pen);
                let _ = DeleteObject(pen);
                if last_poll_ok && percent > 0.0 {
                    let accent = quota_bar_color(percent).blend_over(track);
                    let pen = CreatePen(PS_SOLID, sc(3), COLORREF(accent.to_colorref()));
                    let old_pen = SelectObject(hdc, pen);
                    let (progress_x, progress_y) = arc_point(
                        start + sweep * (percent / 100.0).clamp(0.0, 1.0) as f32,
                    );
                    let _ = Arc(hdc, bounds.left, bounds.top, bounds.right, bounds.bottom,
                        progress_x, progress_y, start_x, start_y);
                    let _ = SelectObject(hdc, old_pen);
                    let _ = DeleteObject(pen);
                }

                let reset = {
                    let state = lock_state();
                    let codex = state.as_ref().and_then(|s| s.data.as_ref())
                        .and_then(|data| data.codex.as_ref());
                    vertical_reset_label(codex.and_then(|c| if label == "5H" {
                        c.session.resets_at
                    } else {
                        c.weekly.resets_at
                    }), label == "7D")
                };
                for (text, y, font_height, color) in [
                    (label.to_string(), top + sc(VERTICAL_LABEL_CENTER_Y), sc(10), quota_type_color),
                    (vertical_quota_text(percent, last_poll_ok), cy,
                        if percent >= 99.5 { sc(8) } else { sc(11) }, primary_color),
                    (reset, top + sc(VERTICAL_RESET_CENTER_Y), sc(9), reset_color),
                ] {
                    let font = CreateFontW(
                        font_height, 0, 0, 0, FW_SEMIBOLD.0 as i32, 0, 0, 0,
                        DEFAULT_CHARSET.0 as u32, OUT_TT_PRECIS.0 as u32,
                        CLIP_DEFAULT_PRECIS.0 as u32, gdi_text_quality(),
                        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                        PCWSTR::from_raw(font_name.as_ptr()),
                    );
                    let old_font = SelectObject(hdc, font);
                    let _ = SetTextColor(hdc, COLORREF(color.to_colorref()));
                    let mut chars: Vec<u16> = text.encode_utf16().collect();
                    let mut bounds = RECT {
                        left: sc(1), top: y - sc(9), right: width - sc(1),
                        bottom: y + sc(9),
                    };
                    let _ = DrawTextW(
                        hdc, &mut chars, &mut bounds,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                    );
                    let _ = SelectObject(hdc, old_font);
                    let _ = DeleteObject(font);
                }
            }
        }
    }
}

/// Paint widget foreground. For the normal-window fallback this also paints the
/// panel using the same style, while the layered taskbar path prepares the
/// backdrop and panel pixels before calling this function.
#[allow(clippy::too_many_arguments)]
fn paint_content(
    hdc: HDC,
    width: i32,
    height: i32,
    _is_dark: bool,
    bg: &Color,
    language: LanguageId,
    strings: Strings,
    codex_session_pct: f64,
    codex_session_text: &str,
    codex_weekly_pct: f64,
    codex_weekly_text: &str,
    show_session_window: bool,
    show_weekly_window: bool,
    last_poll_ok: bool,
    paint_background: bool,
) {
    unsafe {
        let codex_session_pct = usage_percent_for_display(language, codex_session_pct);
        let codex_weekly_pct = usage_percent_for_display(language, codex_weekly_pct);
        let preset = current_appearance_preset();
        let metrics = preset.metrics();
        let (label_width, text_width) = usage_layout_widths(language, preset);
        let style = current_theme_style();
        let panel_base = style
            .color(StyleColorTarget::PanelBackground)
            .blend_over(*bg);
        let quota_type_color = style.color(StyleColorTarget::QuotaType).blend_over(panel_base);
        let primary_color = if last_poll_ok {
            style.color(StyleColorTarget::Remaining)
        } else {
            style.color(StyleColorTarget::Error)
        }
        .blend_over(panel_base);
        let reset_color = style.color(StyleColorTarget::ResetTime).blend_over(panel_base);
        let track = style
            .color(StyleColorTarget::ProgressConsumed)
            .blend_over(panel_base);
        let drag_color = style.color(StyleColorTarget::DragHandle).blend_over(panel_base);

        let (small_taskbar_mode, small_show_weekly, drag_handle_hovered) = {
            let state = lock_state();
            state
                .as_ref()
                .map(|s| (s.small_taskbar_mode, s.small_show_weekly, s.drag_handle_hovered))
                .unwrap_or((false, false, false))
        };
        let effective_show_session = if small_taskbar_mode {
            !small_show_weekly
        } else {
            show_session_window
        };
        let effective_show_weekly = if small_taskbar_mode {
            small_show_weekly
        } else {
            show_weekly_window
        };

        if paint_background {
            let client_rect = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let bg_brush = CreateSolidBrush(COLORREF(bg.to_colorref()));
            FillRect(hdc, &client_rect, bg_brush);
            let _ = DeleteObject(bg_brush);

            let border = if drag_handle_hovered {
                drag_hover_outline_color(&style)
            } else {
                style.color(StyleColorTarget::PanelBorder).blend_over(*bg)
            };
            draw_panel(
                hdc,
                width,
                height,
                &border,
                &panel_base,
                style.panel_corner_radius,
            );
        }

        let vertical_mode = lock_state()
            .as_ref()
            .is_some_and(|s| s.vertical_taskbar_mode);
        if vertical_mode {
            paint_vertical_widget_content(
                hdc, width, height, codex_session_pct, codex_weekly_pct,
                show_session_window, show_weekly_window, last_poll_ok,
                quota_type_color, primary_color, reset_color, track, drag_color,
                drag_handle_hovered, paint_background,
                preset == AppearancePreset::Minimal,
            );
            return;
        }

        if !lock_state().as_ref().is_some_and(|s| s.floating_mode) {
            draw_drag_handle(hdc, height, &drag_color, drag_handle_hovered);
        }

        let floating = lock_state().as_ref().is_some_and(|s| s.floating_mode);
        let content_x = horizontal_content_left(&metrics, floating);
        let row2_y = height - sc(4) - sc(SEGMENT_H);
        let row1_y = row2_y - sc(metrics.row_gap) - sc(SEGMENT_H);
        let single_row_y = (height - sc(SEGMENT_H)) / 2;

        let _ = SetBkMode(hdc, TRANSPARENT);
        let font_name = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
        let font = CreateFontW(
            sc(metrics.font_height),
            0,
            0,
            0,
            FW_MEDIUM.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_TT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            gdi_text_quality(),
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            PCWSTR::from_raw(font_name.as_ptr()),
        );
        let old_font = SelectObject(hdc, font);

        if preset == AppearancePreset::Minimal {
            if effective_show_session {
                draw_minimal_usage_value(
                    hdc,
                    content_x,
                    if effective_show_weekly { row1_y } else { single_row_y },
                    codex_session_text,
                    &primary_color,
                );
            }
            if effective_show_weekly {
                draw_minimal_usage_value(
                    hdc,
                    content_x,
                    if effective_show_session { row2_y } else { single_row_y },
                    codex_weekly_text,
                    &primary_color,
                );
            }
        } else {
            if effective_show_session {
                draw_row(
                    hdc,
                    content_x,
                    if effective_show_weekly { row1_y } else { single_row_y },
                    &quota_type_color,
                    &primary_color,
                    &reset_color,
                    strings.session_window,
                    codex_session_pct,
                    codex_session_text,
                    &track,
                    label_width,
                    text_width,
                    panel_base,
                );
            }
            if effective_show_weekly {
                draw_row(
                    hdc,
                    content_x,
                    if effective_show_session { row2_y } else { single_row_y },
                    &quota_type_color,
                    &primary_color,
                    &reset_color,
                    strings.weekly_window,
                    codex_weekly_pct,
                    codex_weekly_text,
                    &track,
                    label_width,
                    text_width,
                    panel_base,
                );
            }
        }

        SelectObject(hdc, old_font);
        let _ = DeleteObject(font);
    }
}

fn poll_error_display_label(error: poller::PollError, language: LanguageId) -> &'static str {
    match error {
        poller::PollError::AuthRequired
        | poller::PollError::NoCredentials
        | poller::PollError::TokenExpired => "!",
        poller::PollError::NetworkUnavailable => {
            if language == LanguageId::SimplifiedChinese {
                "网络"
            } else {
                "NET"
            }
        }
        poller::PollError::RateLimited => {
            if language == LanguageId::SimplifiedChinese {
                "限流"
            } else {
                "429"
            }
        }
        poller::PollError::ServerError => {
            if language == LanguageId::SimplifiedChinese {
                "服务"
            } else {
                "5XX"
            }
        }
        poller::PollError::RequestFailed => {
            if language == LanguageId::SimplifiedChinese {
                "错误"
            } else {
                "ERR"
            }
        }
    }
}

fn do_poll(send_hwnd: SendHwnd) {
    let hwnd = send_hwnd.to_hwnd();
    match poller::poll() {
        Ok(data) => {
            let mut state = lock_state();
            let mut quota_alerts = Vec::new();
            if let Some(s) = state.as_mut() {
                if let Some(codex) = data.codex.as_ref() {
                    s.codex_session_percent = codex.session.percentage;
                    s.codex_weekly_percent = codex.weekly.percentage;
                }
                if !poller::app_is_past_reset(&data) {
                    unsafe {
                        let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_RESET_POLL);
                    }
                }
                quota_alerts = collect_low_quota_alerts(s, &data);
                s.data = Some(data);
                s.last_poll_ok = true;
                refresh_usage_texts(s);
                if s.retry_count > 0 {
                    s.retry_count = 0;
                    unsafe {
                        SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, s.poll_interval_ms, None);
                    }
                }
                s.force_notify_auth_error = false;
                s.auth_error_paused_polling = false;
                s.auth_watch_mode = poller::CredentialWatchMode::ActiveSource;
                s.auth_watch_snapshot.clear();
            }
            drop(state);
            notify_quota_alerts(hwnd, &quota_alerts);
            if !quota_alerts.is_empty() {
                save_state_settings();
            }
            unsafe {
                let _ = PostMessageW(hwnd, WM_APP_USAGE_UPDATED, WPARAM(0), LPARAM(0));
            }
        }
        Err(e) => {
            let auth_watch = match e {
                poller::PollError::AuthRequired | poller::PollError::TokenExpired => Some((
                    poller::CredentialWatchMode::ActiveSource,
                    poller::credential_watch_snapshot(poller::CredentialWatchMode::ActiveSource),
                )),
                poller::PollError::NoCredentials => Some((
                    poller::CredentialWatchMode::AllSources,
                    poller::credential_watch_snapshot(poller::CredentialWatchMode::AllSources),
                )),
                _ => None,
            };
            let notify_auth_error = {
                let mut state = lock_state();
                let mut notify = false;
                if let Some(s) = state.as_mut() {
                    s.last_poll_ok = false;
                    if let Some((mode, snapshot)) = auth_watch {
                        notify = s.retry_count == 0 || s.force_notify_auth_error;
                        s.force_notify_auth_error = false;
                        s.auth_error_paused_polling = true;
                        s.auth_watch_mode = mode;
                        s.auth_watch_snapshot = snapshot;
                        s.codex_session_text = "!".to_string();
                        s.codex_weekly_text = "!".to_string();
                        s.retry_count = s.retry_count.saturating_add(1);
                        unsafe {
                            let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL);
                            let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_RESET_POLL);
                            let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_COUNTDOWN);
                            SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, s.poll_interval_ms, None);
                        }
                    } else {
                        s.force_notify_auth_error = false;
                        s.auth_error_paused_polling = false;
                        s.auth_watch_mode = poller::CredentialWatchMode::ActiveSource;
                        s.auth_watch_snapshot.clear();
                        let label = poll_error_display_label(e, s.language).to_string();
                        s.codex_session_text = label.clone();
                        s.codex_weekly_text = label;
                        s.retry_count = s.retry_count.saturating_add(1);
                        let backoff = RETRY_BASE_MS.saturating_mul(
                            1u32.checked_shl(s.retry_count - 1).unwrap_or(u32::MAX),
                        );
                        unsafe {
                            let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_RESET_POLL);
                            SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, backoff.min(s.poll_interval_ms), None);
                        }
                    }
                }
                notify
            };
            if notify_auth_error {
                let state = lock_state();
                if let Some(s) = state.as_ref() {
                    tray_icon::notify_warning(
                        hwnd,
                        tray_icon::TrayIconKind::Codex,
                        s.language.strings().codex_token_expired_title,
                        s.language.strings().codex_token_expired_body,
                    );
                }
            }
            unsafe {
                let _ = PostMessageW(hwnd, WM_APP_USAGE_UPDATED, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn schedule_countdown_timer() {
    let state = lock_state();
    let s = match state.as_ref() {
        Some(s) => s,
        None => return,
    };

    let hwnd = taskbar_replica_service_hwnd(s.hwnd.to_hwnd());
    if !s.last_poll_ok {
        unsafe {
            let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_COUNTDOWN);
            let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_RESET_POLL);
        }
        return;
    }

    let data = match &s.data {
        Some(d) => d,
        None => return,
    };

    // If a reset time has passed, poll every 5s to pick up fresh data
    if poller::app_is_past_reset(data) {
        unsafe {
            SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_RESET_POLL, 5_000, None);
        }
    }

    let delays = [
        data.codex
            .as_ref()
            .and_then(|usage| poller::time_until_display_change(usage.session.resets_at)),
        data.codex
            .as_ref()
            .and_then(|usage| poller::time_until_display_change(usage.weekly.resets_at)),
    ];
    let min_delay = delays.into_iter().flatten().min();

    let ms = min_delay
        .unwrap_or(Duration::from_secs(60))
        .as_millis()
        .max(1000) as u32;

    unsafe {
        SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_COUNTDOWN, ms, None);
    }
}

fn check_theme_change() {
    let system_dark = theme::is_dark_mode();
    let changed = {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            if s.theme_mode != ThemeMode::System {
                false
            } else if s.is_dark != system_dark {
                s.is_dark = system_dark;
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if changed {
        close_style_editors();
        render_layered();
        style_window::sync(style_settings_snapshot());
    }
}

fn check_language_change() {
    if update_language_change() {
        render_layered();
    }
}

fn update_display() {
    let mut state = lock_state();
    let s = match state.as_mut() {
        Some(s) => s,
        None => return,
    };

    // Don't overwrite error text with stale cached data
    if !s.last_poll_ok {
        return;
    }

    refresh_usage_texts(s);
}

fn suppress_tray_reposition_for(duration: Duration) {
    let mut until = SUPPRESS_TRAY_REPOSITION_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *until = Some(Instant::now() + duration);
}

fn tray_reposition_is_suppressed() -> bool {
    let now = Instant::now();
    let mut until = SUPPRESS_TRAY_REPOSITION_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    match *until {
        Some(deadline) if now < deadline => true,
        Some(_) => {
            *until = None;
            false
        }
        None => false,
    }
}

// Desktop fallback keeps a separately owned popup alive instead of
// detaching/reparenting Explorer's active taskbar child window.
const FLOAT_EDGE_SNAP: i32 = 24;
const FLOAT_EDGE_PEEK: i32 = 16;
const FLOAT_GRIP_LENGTH: i32 = 36;
// Desktop free-floating mode reserves genuinely transparent space for a
// wider hover glow. Docked edge peeks preserve their V24 geometry unchanged.
const FLOAT_FREE_GLOW_MARGIN: i32 = 12;
const FLOAT_EXPAND_DURATION_MS: u64 = 220;
const FLOAT_RETRACT_DURATION_MS: u64 = 240;
const FLOAT_GRIP_FADE_IN_MS: u64 = 140;
const FLOAT_GRIP_FADE_OUT_MS: u64 = 170;
const TIMER_FLOAT_ANIMATION: usize = 0xF141;

/// This rectangle is used for the rendered tab, pointer hit testing and
/// collapsed alpha mask. Desktop-only, independent of the embedded taskbar grip.
fn floating_grip_rect(width: i32, height: i32, edge: Option<FloatingEdge>, peek: i32) -> RECT {
    let peek = peek.min(width.min(height)).max(1);
    let grip_width = sc(FLOAT_GRIP_LENGTH).min(width).max(1);
    let grip_height = sc(FLOAT_GRIP_LENGTH).min(height).max(1);
    match edge {
        Some(FloatingEdge::Left) => RECT {
            left: width - peek, right: width,
            top: (height - grip_height) / 2, bottom: (height + grip_height) / 2,
        },
        Some(FloatingEdge::Right) => RECT {
            left: 0, right: peek,
            top: (height - grip_height) / 2, bottom: (height + grip_height) / 2,
        },
        Some(FloatingEdge::Top) => RECT {
            left: (width - grip_width) / 2, right: (width + grip_width) / 2,
            top: height - peek, bottom: height,
        },
        Some(FloatingEdge::Bottom) => RECT {
            left: (width - grip_width) / 2, right: (width + grip_width) / 2,
            top: 0, bottom: peek,
        },
        None => RECT {
            left: (width - grip_width) / 2, right: (width + grip_width) / 2,
            top: sc(FLOAT_FREE_GLOW_MARGIN),
            bottom: sc(FLOAT_FREE_GLOW_MARGIN) + peek,
        },
    }
}

fn floating_drag_handle_hit(
    x: i32, y: i32, width: i32, height: i32, edge: Option<FloatingEdge>, peek: i32,
) -> bool {
    point_in_rect(floating_grip_rect(width, height, edge, peek), x, y)
}

// The desktop tab lives OUTSIDE the panel body. Hover must be checked against
// its final screen-space rectangle, especially after the canvas origin changes
// on drop from a docked edge to the free-floating top tab.
fn floating_drag_grip_contains_screen_point(
    origin_x: i32, origin_y: i32, canvas_w: i32, canvas_h: i32,
    edge: Option<FloatingEdge>, pointer: POINT,
) -> bool {
    floating_drag_handle_hit(
        pointer.x - origin_x, pointer.y - origin_y,
        canvas_w, canvas_h, edge, sc(FLOAT_EDGE_PEEK),
    )
}

// A stale WM_MOUSELEAVE can be queued when mouse capture ends or a layered
// popup is repositioned. Recheck where the pointer actually is, rather than
// clearing an otherwise valid hover until another WM_MOUSEMOVE arrives.
fn floating_pointer_still_over_free_grip(hwnd: HWND) -> bool {
    let (canvas_w, canvas_h) = {
        let state = lock_state();
        let Some(s) = state.as_ref().filter(|s|
            s.floating_mode && s.hwnd.to_hwnd() == hwnd && s.floating_edge.is_none()
        ) else { return false };
        floating_canvas_size_for_state(s)
    };
    let Some(rect) = native_interop::get_window_rect_safe(hwnd) else {
        return false;
    };
    let mut pointer = POINT::default();
    if unsafe { GetCursorPos(&mut pointer) }.is_err() { return false; }
    floating_drag_grip_contains_screen_point(
        rect.left, rect.top, canvas_w, canvas_h, None, pointer,
    )
}

/// Desktop tabs occupy an independent, transparent gutter. The panel dimensions
/// remain identical to the taskbar renderer, including progress and ring layout.
fn floating_canvas_geometry(
    body_width: i32, body_height: i32, edge: Option<FloatingEdge>,
) -> (i32, i32, RECT) {
    let tab = sc(FLOAT_EDGE_PEEK);
    match edge {
        Some(FloatingEdge::Left) => (
            body_width + tab, body_height,
            RECT { left: 0, top: 0, right: body_width, bottom: body_height },
        ),
        Some(FloatingEdge::Right) => (
            body_width + tab, body_height,
            RECT { left: tab, top: 0, right: body_width + tab, bottom: body_height },
        ),
        Some(FloatingEdge::Top) => (
            body_width, body_height + tab,
            RECT { left: 0, top: 0, right: body_width, bottom: body_height },
        ),
        Some(FloatingEdge::Bottom) => (
            body_width, body_height + tab,
            RECT { left: 0, top: tab, right: body_width, bottom: body_height + tab },
        ),
        None => {
            let glow = sc(FLOAT_FREE_GLOW_MARGIN);
            (
                body_width + glow * 2,
                body_height + tab + glow * 2,
                RECT {
                    left: glow, top: tab + glow,
                    right: body_width + glow, bottom: body_height + tab + glow,
                },
            )
        },
    }
}

fn floating_canvas_size_for_state(state: &AppState) -> (i32, i32) {
    let (width, height, _) = floating_canvas_geometry(
        total_widget_width_for_state(state), widget_height_for_state(state), state.floating_edge,
    );
    (width, height)
}

fn vertical_placement_for_gaps(
    preset: AppearancePreset, taskbar_rect: RECT, spans: &[VerticalSpan], desired: i32,
) -> Option<(AppearancePreset, i32)> {
    let normal = vertical_widget_height_for_preset(AppearancePreset::Default);
    let compact = vertical_widget_height_for_preset(AppearancePreset::Minimal);
    let choices = match preset.configured() {
        AppearancePreset::Minimal => [Some((AppearancePreset::Minimal, compact)), None],
        AppearancePreset::Default => [
            Some((AppearancePreset::Default, normal)),
            Some((AppearancePreset::Minimal, compact)),
        ],
        AppearancePreset::Adaptive => unreachable!(),
    };
    for choice in choices.into_iter().flatten() {
        if let Some(top) = nearest_free_top_offset(taskbar_rect, spans, choice.1, desired) {
            return Some((choice.0, top));
        }
    }
    None
}

/// Keep edge tabs in the monitor's *work area*, not under a vertical taskbar.
/// A left-side Explorer taskbar otherwise covers the docked grip even when
/// the desktop popup itself is in the topmost band.
fn floating_usable_bounds(monitor: RECT, work: RECT) -> RECT {
    if work.left >= monitor.left && work.top >= monitor.top
        && work.right <= monitor.right && work.bottom <= monitor.bottom
        && work.right > work.left && work.bottom > work.top {
        // The bottom grip must be attached to the physical screen boundary
        // (including over the taskbar), not to rcWork.bottom above the taskbar.
        RECT { bottom: monitor.bottom, ..work }
    } else {
        monitor
    }
}

fn floating_monitor_geometry(point: POINT) -> Option<(RECT, RECT)> {
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info).as_bool().then_some((
            info.rcMonitor, floating_usable_bounds(info.rcMonitor, info.rcWork),
        ))
    }
}

fn floating_monitor_bounds(point: POINT) -> Option<RECT> {
    floating_monitor_geometry(point).map(|(_, usable)| usable)
}

/// A display seam is not a hideable edge: its grip would disappear beneath
/// the neighboring monitor's desktop. Probe along the physical perimeter at
/// all three portions of the grip, not merely at the drop cursor position.
fn floating_edge_exposed(monitor: RECT, edge: FloatingEdge, pointer: POINT) -> bool {
    let half_grip = sc(FLOAT_GRIP_LENGTH) / 2;
    let offsets = [-half_grip + 1, 0, half_grip - 1];
    offsets.into_iter().all(|offset| {
        let probe = match edge {
            FloatingEdge::Left => POINT {
                x: monitor.left.saturating_sub(1),
                y: (pointer.y + offset).clamp(monitor.top, monitor.bottom - 1),
            },
            FloatingEdge::Right => POINT {
                x: monitor.right,
                y: (pointer.y + offset).clamp(monitor.top, monitor.bottom - 1),
            },
            FloatingEdge::Top => POINT {
                x: (pointer.x + offset).clamp(monitor.left, monitor.right - 1),
                y: monitor.top.saturating_sub(1),
            },
            FloatingEdge::Bottom => POINT {
                x: (pointer.x + offset).clamp(monitor.left, monitor.right - 1),
                y: monitor.bottom,
            },
        };
        unsafe { MonitorFromPoint(probe, MONITOR_DEFAULTTONULL) == HMONITOR::default() }
    })
}

fn floating_clamp_position(bounds: RECT, width: i32, height: i32, x: i32, y: i32) -> (i32, i32) {
    (
        x.clamp(bounds.left, (bounds.right - width).max(bounds.left)),
        y.clamp(bounds.top, (bounds.bottom - height).max(bounds.top)),
    )
}

fn floating_nearest_edge(bounds: RECT, rect: RECT, threshold: i32) -> Option<FloatingEdge> {
    let edges = [
        (FloatingEdge::Left, (rect.left - bounds.left).abs()),
        (FloatingEdge::Right, (bounds.right - rect.right).abs()),
        (FloatingEdge::Top, (rect.top - bounds.top).abs()),
        (FloatingEdge::Bottom, (bounds.bottom - rect.bottom).abs()),
    ];
    edges.into_iter().min_by_key(|(_, distance)| *distance)
        .and_then(|(edge, distance)| (distance <= threshold).then_some(edge))
}

/// Called only on mouse release, including a release while the original
/// snapped body is still partly outside the monitor.
fn floating_drop_edge(
    bounds: RECT, rect: RECT, threshold: i32, exposed: [bool; 4],
) -> Option<FloatingEdge> {
    let edges = [
        (FloatingEdge::Left, exposed[0],
            rect.left < bounds.left && rect.right > bounds.left,
            (rect.left - bounds.left).abs()),
        (FloatingEdge::Right, exposed[1],
            rect.right > bounds.right && rect.left < bounds.right,
            (bounds.right - rect.right).abs()),
        (FloatingEdge::Top, exposed[2],
            rect.top < bounds.top && rect.bottom > bounds.top,
            (rect.top - bounds.top).abs()),
        (FloatingEdge::Bottom, exposed[3],
            rect.bottom > bounds.bottom && rect.top < bounds.bottom,
            (bounds.bottom - rect.bottom).abs()),
    ];
    if let Some((edge, _, _, _)) = edges.iter().find(|(_, allowed, overlaps, _)|
        *allowed && *overlaps
    ) {
        return Some(*edge);
    }
    // An unexposed seam never qualifies, even if it is the geometrically
    // nearest edge. A different, real exterior edge may still qualify.
    if exposed.iter().all(|&allowed| allowed) {
        return floating_nearest_edge(bounds, rect, threshold);
    }
    edges.into_iter()
        .filter(|(_, allowed, _, distance)| *allowed && *distance <= threshold)
        .min_by_key(|(_, _, _, distance)| *distance)
        .map(|(edge, _, _, _)| edge)
}

// The stable expanded position is always in screen coordinates. Only the
// visible sliver of a docked popup is on-screen while collapsed.
#[allow(clippy::too_many_arguments)]
fn floating_edge_position(
    bounds: RECT, x: i32, y: i32, width: i32, height: i32,
    edge: Option<FloatingEdge>, collapsed: bool, peek: i32,
) -> (i32, i32) {
    let (x, y) = floating_clamp_position(bounds, width, height, x, y);
    match edge {
        Some(FloatingEdge::Left) => (
            if collapsed { bounds.left - width + peek } else { bounds.left }, y,
        ),
        Some(FloatingEdge::Right) => (
            if collapsed { bounds.right - peek } else { bounds.right - width }, y,
        ),
        Some(FloatingEdge::Top) => (
            x, if collapsed { bounds.top - height + peek } else { bounds.top },
        ),
        Some(FloatingEdge::Bottom) => (
            x, if collapsed { bounds.bottom - peek } else { bounds.bottom - height },
        ),
        None => (x, y),
    }
}

fn move_floating_widget(hwnd: HWND) {
    let (x, y, edge, reveal, width, height, dragging) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        if !s.floating_mode { return; }
        let (width, height) = floating_canvas_size_for_state(s);
        (s.floating_x, s.floating_y, s.floating_edge, s.floating_reveal,
         width, height, s.dragging)
    };
    let (left, top) = if dragging || edge.is_none() {
        // Keep mouse positioning stable during drag and across connected
        // displays. A free popup may straddle two screens at a seam.
        (x, y)
    } else {
        let center = POINT { x: x.saturating_add(width / 2), y: y.saturating_add(height / 2) };
        let Some(bounds) = floating_monitor_bounds(center) else { return };
        let (expanded_x, expanded_y) = floating_edge_position(
            bounds, x, y, width, height, edge, false, sc(FLOAT_EDGE_PEEK),
        );
        let (collapsed_x, collapsed_y) = floating_edge_position(
            bounds, x, y, width, height, edge, true, sc(FLOAT_EDGE_PEEK),
        );
        // floating_reveal already contains the time-based eased progress.
        // Applying a second smoothstep here would distort both curves.
        let progress = f64::from(reveal) / 100.0;
        let interpolate = |a: i32, b: i32| {
            a + (f64::from(b - a) * progress).round() as i32
        };
        (interpolate(collapsed_x, expanded_x), interpolate(collapsed_y, expanded_y))
    };
    let blur = lock_state().as_ref().is_some_and(|s| s.composition_blur_active);
    if blur {
        // The desktop pair is raised when blur is activated or when docking
        // changes; dragging must only move its HWNDs, never reorder them.
        move_frosted_pair(hwnd, left, top, width, height);
    } else {
        unsafe {
            let _ = SetWindowPos(
                hwnd, HWND_TOPMOST, left, top, width, height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }
}

fn set_floating_grip_hover(hovered: bool) {
    let hwnd = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        if !s.floating_mode || s.dragging || s.floating_edge.is_some()
            || s.floating_grip_hover == hovered { return; }
        s.floating_grip_hover = hovered;
        s.hwnd.to_hwnd()
    };
    unsafe {
        SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION, 16, None);
    }
}

fn floating_hover_can_reveal(collapsed: bool, guard: bool, dragging: bool) -> bool {
    collapsed && !guard && !dragging
}

fn set_floating_collapsed(collapsed: bool) {
    let hwnd = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        if !s.floating_mode || s.dragging || s.floating_edge.is_none()
            || (!collapsed && s.floating_first_collapse_guard)
            || s.floating_collapsed == collapsed { return; }
        s.floating_collapsed = collapsed;
        s.hwnd.to_hwnd()
    };
    if collapsed { hide_minimal_usage_tooltip(); }
    unsafe {
        let service = taskbar_replica_service_hwnd(hwnd);
        SetTimer(service, TIMER_FLOAT_ANIMATION, 16, None);
    }
    move_floating_widget(hwnd);
}

fn step_floating_animation(hwnd: HWND) {
    let (active, completed, moved, appearance_changed, guarded) = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        if !s.floating_mode {
            unsafe { let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION); }
            return;
        }
        let now = Instant::now();
        let destination = if s.floating_collapsed { 0 } else { 100 };
        let old_reveal = s.floating_reveal;
        let old_alpha = s.floating_grip_alpha;
        if s.floating_slide_target != destination {
            s.floating_slide_from = s.floating_reveal;
            s.floating_slide_target = destination;
            s.floating_slide_started = now;
        }
        let base_ms = if destination == 100 {
            FLOAT_EXPAND_DURATION_MS
        } else {
            FLOAT_RETRACT_DURATION_MS
        };
        let slide_duration = animation_partial_duration(
            base_ms, s.floating_slide_from, destination,
        );
        s.floating_reveal = eased_value_at(
            s.floating_slide_from, destination,
            now.duration_since(s.floating_slide_started), slide_duration,
        );
        let grip_target = if s.floating_edge.is_some() || s.dragging || s.floating_grip_hover {
            100
        } else { 0 };
        if s.floating_fade_target != grip_target {
            s.floating_fade_from = s.floating_grip_alpha;
            s.floating_fade_target = grip_target;
            s.floating_fade_started = now;
        }
        let fade_ms = if grip_target == 100 {
            FLOAT_GRIP_FADE_IN_MS
        } else {
            FLOAT_GRIP_FADE_OUT_MS
        };
        s.floating_grip_alpha = eased_value_at(
            s.floating_fade_from, grip_target,
            now.duration_since(s.floating_fade_started),
            animation_partial_duration(fade_ms, s.floating_fade_from, grip_target),
        );
        (s.hwnd.to_hwnd(),
         s.floating_reveal == destination && s.floating_grip_alpha == grip_target,
         s.floating_reveal != old_reveal,
         s.floating_reveal != old_reveal || s.floating_grip_alpha != old_alpha,
         s.floating_first_collapse_guard && s.floating_collapsed)
    };
    if moved { move_floating_widget(active); }
    if appearance_changed { render_layered(); }

    // The new dock's first contraction is non-interactive. Re-arm hover only
    // after it is fully hidden AND the pointer has left the now-visible tab.
    // Poll slowly when the pointer rests on the tab, without repainting.
    if completed && guarded {
        if floating_pointer_is_outside_grip(active) {
            if let Some(s) = lock_state().as_mut() {
                if s.hwnd.to_hwnd() == active { s.floating_first_collapse_guard = false; }
            }
        } else {
            unsafe {
                SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION, 80, None);
            }
            return;
        }
    }
    if completed {
        unsafe { let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION); }
    }
}

/// False while the pointer still overlaps the collapsed exposed edge grip.
fn floating_pointer_is_outside_grip(hwnd: HWND) -> bool {
    let (edge, width, height) = {
        let state = lock_state();
        let Some(s) = state.as_ref().filter(|s| s.floating_mode) else { return false };
        let (width, height) = floating_canvas_size_for_state(s);
        (s.floating_edge, width, height)
    };
    let Some(edge) = edge else { return true };
    let Some(rect) = native_interop::get_window_rect_safe(hwnd) else { return false };
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() { return false; }
    let grip = floating_grip_rect(width, height, Some(edge), sc(FLOAT_EDGE_PEEK));
    !point_in_rect(RECT {
        left: rect.left + grip.left,
        top: rect.top + grip.top,
        right: rect.left + grip.right,
        bottom: rect.top + grip.bottom,
    }, point.x, point.y)
}

fn animation_partial_duration(full_ms: u64, from: u8, to: u8) -> u64 {
    let distance = i32::from(from).abs_diff(i32::from(to));
    (full_ms * u64::from(distance) / 100).max(48)
}

/// Time-based cubic easing. Expansion settles quickly (ease-out); retraction
/// starts and stops gently (ease-in-out), without overshooting screen bounds.
fn eased_value_at(from: u8, to: u8, elapsed: Duration, duration_ms: u64) -> u8 {
    if from == to { return to; }
    let t = (elapsed.as_secs_f64() * 1000.0 / duration_ms.max(1) as f64).clamp(0.0, 1.0);
    let eased = if to > from {
        1.0 - (1.0 - t).powi(3)
    } else if t < 0.5 {
        4.0 * t.powi(3)
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    };
    (f64::from(from) + (f64::from(to) - f64::from(from)) * eased)
        .round().clamp(0.0, 100.0) as u8
}

fn update_floating_drag(hwnd: HWND, pointer: POINT) {
    let (anchor_x, anchor_y) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        if !s.floating_mode { return; }
        (s.floating_drag_anchor_x, s.floating_drag_anchor_y)
    };
    {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        // Deliberately do not touch floating_edge or the mouse grab anchor.
        // A docked side grip remains on that side throughout the drag.
        s.floating_x = pointer.x - anchor_x;
        s.floating_y = pointer.y - anchor_y;
        s.floating_collapsed = false;
        s.floating_reveal = 100;
    }
    // No full-window clamping: a collapsed edge tab begins with most of its
    // panel offscreen. Clamping would teleport the panel on the first frame.
    // Reconcile the desktop bounds and grip direction only on button release.
    move_floating_widget(hwnd);
}

fn finish_floating_drag(hwnd: HWND) {
    let (x, y, body_w, body_h, old_edge) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        if !s.floating_mode { return };
        (s.floating_x, s.floating_y,
         total_widget_width_for_state(s), widget_height_for_state(s), s.floating_edge)
    };
    let (old_w, old_h, old_body) = floating_canvas_geometry(body_w, body_h, old_edge);
    let mut pointer = POINT { x: x + old_w / 2, y: y + old_h / 2 };
    unsafe { let _ = GetCursorPos(&mut pointer); }
    let Some((monitor, bounds)) = floating_monitor_geometry(pointer) else {
        // Even if monitor enumeration fails, do not leave the drag state
        // latched and force the detached tab to flicker on the next move.
        if let Some(s) = lock_state().as_mut() { s.dragging = false; }
        return;
    };
    let rect = RECT { left: x, top: y, right: x + old_w, bottom: y + old_h };
    // Physical monitor seams must remain a continuous dragging surface. Only
    // actual outside edges receive collapsed tabs; a neighboring display
    // blocks docking even when the old popup rectangle straddles the seam.
    let exposed = [
        floating_edge_exposed(monitor, FloatingEdge::Left, pointer),
        floating_edge_exposed(monitor, FloatingEdge::Right, pointer),
        floating_edge_exposed(monitor, FloatingEdge::Top, pointer),
        floating_edge_exposed(monitor, FloatingEdge::Bottom, pointer),
    ];
    let edge = floating_drop_edge(bounds, rect, sc(FLOAT_EDGE_SNAP), exposed);
    let blocked_seam = edge.is_none()
        && floating_drop_edge(bounds, rect, sc(FLOAT_EDGE_SNAP), [true; 4]).is_some();
    let (new_w, new_h, new_body) = floating_canvas_geometry(body_w, body_h, edge);
    let desired_x = x + old_body.left - new_body.left;
    let desired_y = y + old_body.top - new_body.top;
    let (x, y) = if blocked_seam {
        // Do not snap, clamp, or warp the body to either monitor while it
        // overlaps their common boundary.
        (desired_x, desired_y)
    } else {
        floating_edge_position(
            bounds, desired_x, desired_y, new_w, new_h, edge, false, sc(FLOAT_EDGE_PEEK),
        )
    };
    // If the drop is free-floating, preserve its original panel screen
    // position even though the now-detached tab moves to the top edge.
    let hovered = edge.is_none() && floating_drag_grip_contains_screen_point(
        x, y, new_w, new_h, edge, pointer,
    );
    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.floating_x = x;
            s.floating_y = y;
            s.dragging = false;
            s.drag_handle_hovered = hovered;
            s.floating_edge = edge;
            s.floating_collapsed = edge.is_some();
            s.floating_first_collapse_guard = edge.is_some();
            s.floating_reveal = 100;
            s.floating_slide_from = 100;
            s.floating_slide_target = 100;
            s.floating_slide_started = Instant::now();
            s.floating_grip_hover = hovered;
            s.floating_grip_alpha = if edge.is_some() || hovered { 100 } else { 0 };
            s.floating_fade_from = s.floating_grip_alpha;
            s.floating_fade_target = s.floating_grip_alpha;
        }
    }
    // Publish the resized/reoriented tab and the final HWND coordinates in
    // one layered-window update. No intermediate frame uses the old bitmap.
    render_layered_at(Some(POINT { x, y }));
    // Explorer can reorder its taskbar while a desktop grip is being dragged.
    // Re-raise the composition pair once on release, never per drag frame.
    if lock_state().as_ref().is_some_and(|s| s.composition_blur_active) {
        sync_blur_backdrop_zorder(hwnd);
    }
    if edge.is_some() {
        unsafe {
            SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION, 16, None);
        }
    }
    save_state_settings();
}

fn enable_floating_widget(reason: &str, vertical: bool) {
    let vertical = {
        let state = lock_state();
        state.as_ref().is_some_and(|s|
            if s.display_position == DisplayPosition::Desktop {
                s.desktop_direction == DesktopDirection::Vertical
            } else { vertical }
        )
    };
    let is_already_floating = lock_state().as_ref().is_some_and(|s| s.floating_mode);
    if is_already_floating {
        let hwnd = {
            let mut state = lock_state();
            let Some(s) = state.as_mut() else { return };
            s.vertical_taskbar_mode = vertical;
            s.small_taskbar_mode = false;
            s.effective_appearance_preset = vertical_effective_preset(s.appearance_preset);
            s.vertical_widget_width = sc(VERTICAL_WIDGET_MIN_WIDTH).max(sc(48));
            s.floating_collapsed = false;
            s.floating_reveal = 100;
            s.floating_grip_hover = false;
            s.floating_first_collapse_guard = false;
            s.floating_grip_alpha = if s.floating_edge.is_some() { 100 } else { 0 };
            s.hwnd.to_hwnd()
        };
        // Diagnostics for the precise native phase if a driver or Explorer
        // still stalls a future desktop orientation change.
        if reason == "explicit desktop preference" {
            diagnose::log_lazy(|| format!(
                "desktop orientation: moving popup direction={}",
                if vertical { "Vertical" } else { "Horizontal" },
            ));
        }
        move_floating_widget(hwnd);
        if reason == "explicit desktop preference" {
            diagnose::log_lazy(|| "desktop orientation: popup moved; painting layered frame".to_string());
        }
        render_layered();
        if reason == "explicit desktop preference" {
            diagnose::log_lazy(|| "desktop orientation: layered frame painted".to_string());
        }
        return;
    }
    let (existing, old, x, y, language, width, height) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        let preset = if vertical { vertical_effective_preset(s.appearance_preset) }
            else { effective_appearance_preset(s) };
        (
            s.floating_hwnd.map(|h| h.to_hwnd()), s.hwnd.to_hwnd(),
            s.floating_x, s.floating_y, s.language,
            if vertical { sc(VERTICAL_WIDGET_MIN_WIDTH).max(sc(48)) }
            else {
                desktop_horizontal_content_width(
                    s.language, preset,
                    s.show_session_window, s.show_weekly_window,
                    &s.codex_session_text, &s.codex_weekly_text,
                )
            },
            if vertical {
                desktop_vertical_content_height(
                    preset, s.show_session_window, s.show_weekly_window,
                )
            } else { sc(preset.metrics().widget_height) },
        )
    };
    let (window_width, window_height, _) = floating_canvas_geometry(
        width, height, lock_state().as_ref().and_then(|s| s.floating_edge),
    );
    let hwnd = match existing {
        Some(hwnd) if unsafe { IsWindow(hwnd).as_bool() } => hwnd,
        _ => unsafe {
            let class = native_interop::wide_str("CodexUsageWin");
            let title = native_interop::wide_str(language.strings().window_title);
            let instance = match GetModuleHandleW(PCWSTR::null()) {
                Ok(module) => module,
                Err(_) => return,
            };
            match CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
                PCWSTR::from_raw(class.as_ptr()), PCWSTR::from_raw(title.as_ptr()),
                WS_POPUP, 0, 0, window_width, window_height, HWND::default(),
                HMENU::default(), instance, None,
            ) {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    diagnose::log(format!("floating popup create failed: {error:?}"));
                    return;
                }
            }
        },
    };
    // Per-pixel UpdateLayeredWindow is required for a grip-only collapsed
    // popup; SetLayeredWindowAttributes would disable that rendering path.
    let origin = native_interop::get_window_rect_safe(old);
    let default_point = origin.map(|r| POINT { x: r.left, y: r.top })
        .unwrap_or(POINT { x: 0, y: 0 });
    let bounds = floating_monitor_bounds(default_point).unwrap_or(RECT {
        left: 0, top: 0, right: 1920, bottom: 1080,
    });
    let start_x = if x == i32::MIN { bounds.right - window_width - sc(32) } else { x };
    let start_y = if y == i32::MIN {
        bounds.top + (bounds.bottom - bounds.top - window_height) / 2
    } else { y };
    let (x, y) = floating_clamp_position(
        bounds, window_width, window_height, start_x, start_y,
    );
    hide_minimal_usage_tooltip();
    {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        s.parked_taskbar_foreground = s.embedded.then_some(s.hwnd);
        // Embedded blur backdrop belongs to Explorer and cannot follow
        // the desktop popup; the fallback uses opaque GDI or popup blur.
        s.composition_blur_active = false;
        s.floating_hwnd = Some(SendHwnd::from_hwnd(hwnd));
        s.hwnd = SendHwnd::from_hwnd(hwnd);
        s.floating_mode = true;
        s.embedded = false;
        s.hidden_for_taskbar_space = false;
        s.floating_collapsed = false;
        s.floating_reveal = 100;
        s.floating_grip_hover = false;
        s.floating_first_collapse_guard = false;
        s.floating_grip_alpha = if s.floating_edge.is_some() { 100 } else { 0 };
        s.floating_x = x;
        s.floating_y = y;
        s.vertical_taskbar_mode = vertical;
        s.small_taskbar_mode = false;
        s.effective_appearance_preset = if vertical {
            vertical_effective_preset(s.appearance_preset)
        } else {
            effective_appearance_preset(s)
        };
        s.vertical_widget_width = width;
    }
    unsafe { let _ = ShowWindow(old, SW_HIDE); }
    if let Some(backdrop) = blur_backdrop_hwnd() {
        unsafe { let _ = ShowWindow(backdrop, SW_HIDE); }
    }
    move_floating_widget(hwnd);
    style_window::set_preview_orientation(vertical);
    render_layered();
    save_state_settings();
    diagnose::log(format!("taskbar has no safe slot: desktop popup enabled ({reason})"));
}

fn restore_floating_widget_to_taskbar(taskbar: HWND) -> bool {
    let (float, target) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return false };
        if !s.floating_mode { return true; }
        let Some(target) = s.parked_taskbar_foreground else { return false };
        (s.hwnd.to_hwnd(), target.to_hwnd())
    };
    if !unsafe { IsWindow(target).as_bool()
        && GetAncestor(target, GA_PARENT) == taskbar } {
        return false;
    }
    hide_minimal_usage_tooltip();
    // A desktop backdrop is top-level; it cannot be reused as an Explorer child.
    destroy_blur_backdrop();
    {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return false };
        s.composition_blur_active = false;
        s.hwnd = SendHwnd::from_hwnd(target);
        s.embedded = true;
        s.floating_mode = false;
        s.floating_collapsed = false;
        s.floating_reveal = 100;
        s.floating_grip_hover = false;
        s.floating_first_collapse_guard = false;
        s.floating_grip_alpha = 0;
        s.parked_taskbar_foreground = None;
        s.hidden_for_taskbar_space = false;
    }
    unsafe {
        let _ = KillTimer(taskbar_replica_service_hwnd(float), TIMER_FLOAT_ANIMATION);
        let _ = ShowWindow(float, SW_HIDE);
    }
    diagnose::log("safe taskbar slot recovered; desktop popup parked");
    true
}

fn hide_widget_for_taskbar_space(
    hwnd: HWND,
    _language: LanguageId,
    reason: &str,
    cache_pending: bool,
) {
    // Ignore a transient uninitialized Explorer layout cache only until the
    // next worker update. Actual no-space conditions never hide or alert.
    if !cache_pending {
        let vertical = lock_state().as_ref()
            .is_some_and(|s| s.taskbar_hwnd.and_then(native_interop::get_taskbar_rect)
                .is_some_and(|rect| taskbar_axis(rect) == TaskbarAxis::Vertical));
        enable_floating_widget(reason, vertical);
        return;
    }
    if lock_state().as_ref().is_some_and(|s| s.floating_mode) { return; }
    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() { s.hidden_for_taskbar_space = true; }
    }
    unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
    diagnose::log(format!("waiting for taskbar layout cache: {reason}"));
}

fn position_vertical_taskbar(
    hwnd: HWND,
    _embedded: bool,
    taskbar_hwnd: HWND,
    rect: RECT,
    blur_active: bool,
    language: LanguageId,
) {
    let Some(width) = vertical_widget_width(rect) else {
        hide_widget_for_taskbar_space(hwnd, language, "vertical taskbar is too narrow", false);
        return;
    };
    let (desired_top, configured) = {
        let state = lock_state();
        state.as_ref().map(|s| (s.taskbar_top_offset, s.appearance_preset))
            .unwrap_or((0, AppearancePreset::Default))
    };
    let Some(free) = taskbar_free_vertical_spans(taskbar_hwnd, rect) else {
        hide_widget_for_taskbar_space(hwnd, language, "vertical taskbar layout cache is not ready", true);
        return;
    };
    let Some((preset, top)) =
        vertical_placement_for_gaps(configured, rect, &free, desired_top)
    else {
        hide_widget_for_taskbar_space(hwnd, language, "no safe vertical space is available", false);
        return;
    };
    if !restore_floating_widget_to_taskbar(taskbar_hwnd) {
        diagnose::log("desktop popup retained: original Explorer child is no longer valid");
        return;
    }
    let (hwnd, embedded, was_hidden, changed) = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        let was_hidden = s.hidden_for_taskbar_space;
        let changed = s.effective_appearance_preset != preset || !s.vertical_taskbar_mode;
        s.hidden_for_taskbar_space = false;
        s.taskbar_space_warning_shown = false;
        s.vertical_taskbar_mode = true;
        s.effective_appearance_preset = preset;
        s.vertical_widget_width = width;
        s.small_taskbar_mode = false;
        if changed { refresh_usage_texts(s); }
        (s.hwnd.to_hwnd(), s.embedded, was_hidden, changed)
    };
    let height = vertical_widget_height_for_preset(preset);
    let x = rect.left + (rect.right - rect.left - width) / 2;
    if embedded {
        native_interop::move_window(hwnd, x - rect.left, top, width, height);
    } else if blur_active {
        move_frosted_pair(hwnd, x, rect.top + top, width, height);
    } else {
        native_interop::move_window(hwnd, x, rect.top + top, width, height);
    }
    unsafe { let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE); }
    if blur_active { sync_blur_backdrop_zorder(hwnd); }
    if changed { render_layered(); }
    diagnose::log_lazy(|| format!(
        "vertical taskbar slot top={} w={} h={} preset={preset:?} gaps={} restored={was_hidden}",
        top, width, height, free.len(),
    ));
}

fn position_at_taskbar() {
    refresh_dpi();
    // A desktop popup does not depend on Explorer's taskbar geometry.
    // NEVER call SHAppBarMessage(ABM_GETTASKBARPOS) from this branch:
    // it synchronously enters Explorer while our own UI message is active.
    // If Explorer is handling one of our embedded child windows concurrently,
    // the two UI threads can end up waiting for each other and both the
    // widget and the Windows taskbar become unresponsive.
    let desktop_selected = lock_state().as_ref()
        .is_some_and(|s| s.display_position == DisplayPosition::Desktop);
    if desktop_selected {
        // The user-selected orientation is resolved inside
        // enable_floating_widget, not from the current taskbar axis.
        enable_floating_widget("explicit desktop preference", false);
        return;
    }
    let (
        hwnd,
        embedded,
        desired_left_offset,
        legacy_tray_offset,
        taskbar_hwnd,
        taskbar_monitor,
        blur_active,
        configured_preset,
        language,
    ) = {
        let state = lock_state();
        let s = match state.as_ref() {
            Some(s) => s,
            None => return,
        };
        if s.dragging {
            return;
        }
        let taskbar_hwnd = match s.taskbar_hwnd {
            Some(h) => h,
            None => {
                diagnose::log("position_at_taskbar skipped: no taskbar handle");
                return;
            }
        };
        (
            s.hwnd.to_hwnd(),
            s.embedded,
            s.taskbar_left_offset,
            s.legacy_tray_offset,
            taskbar_hwnd,
            s.taskbar_monitor.clone(),
            s.composition_blur_active,
            s.appearance_preset,
            s.language,
        )
    };

    let taskbar_rect = match native_interop::get_taskbar_rect(taskbar_hwnd) {
        Some(r) => r,
        None => {
            diagnose::log("position_at_taskbar skipped: unable to query taskbar rect");
            return;
        }
    };
    if taskbar_axis(taskbar_rect) == TaskbarAxis::Vertical {
        position_vertical_taskbar(
            hwnd, embedded, taskbar_hwnd, taskbar_rect, blur_active, language,
        );
        style_window::set_preview_orientation(true);
        return;
    }
    // The horizontal placement path remains unchanged. Restore its native
    // dimensions when the widget returns from a vertical monitor.
    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.vertical_taskbar_mode = false;
        }
    }
    style_window::set_preview_orientation(false);
    let taskbar_height = taskbar_rect.bottom - taskbar_rect.top;
    let small_mode = is_small_taskbar_height(taskbar_height);
    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            if s.small_taskbar_mode != small_mode {
                s.small_taskbar_mode = small_mode;
                if small_mode {
                    s.small_show_weekly = false;
                }
            }
        }
    }

    let migration_preset = match configured_preset {
        AppearancePreset::Minimal => AppearancePreset::Minimal,
        _ => AppearancePreset::Default,
    };
    let migration_width = total_widget_width_for_preset(language, migration_preset);
    let desired_left_offset = if let Some(legacy_tray_offset) = legacy_tray_offset {
        let migrated = legacy_left_offset(
            taskbar_hwnd,
            taskbar_rect,
            migration_width,
            legacy_tray_offset,
        );
        {
            let mut state = lock_state();
            if let Some(s) = state.as_mut() {
                s.taskbar_left_offset = migrated;
                s.legacy_tray_offset = None;
            }
        }
        diagnose::log(format!(
            "migrated legacy tray_offset={} to taskbar_left_offset={}",
            legacy_tray_offset, migrated
        ));
        save_state_settings();
        migrated
    } else {
        desired_left_offset
    };

    let Some(free_spans) = taskbar_free_spans(taskbar_hwnd, taskbar_rect) else {
        hide_widget_for_taskbar_space(
            hwnd,
            language,
            "safe taskbar layout cache is not ready",
            true,
        );
        return;
    };
    let Some((effective_preset, widget_width, actual_left_offset)) =
        placement_for_free_spans(
            configured_preset,
            language,
            taskbar_rect,
            &free_spans,
            desired_left_offset,
        )
    else {
        hide_widget_for_taskbar_space(
            hwnd,
            language,
            "no empty span is wide enough",
            false,
        );
        return;
    };

    if !restore_floating_widget_to_taskbar(taskbar_hwnd) {
        diagnose::log("desktop popup retained: original Explorer taskbar child unavailable");
        return;
    }
    let (hwnd, embedded) = lock_state().as_ref()
        .map(|s| (s.hwnd.to_hwnd(), s.embedded)).unwrap_or((hwnd, embedded));
    let (was_hidden, effective_changed) = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else {
            return;
        };
        let was_hidden = s.hidden_for_taskbar_space;
        let effective_changed = s.effective_appearance_preset != effective_preset;
        s.hidden_for_taskbar_space = false;
        s.taskbar_space_warning_shown = false;
        s.effective_appearance_preset = effective_preset;
        if effective_changed {
            refresh_usage_texts(s);
        }
        (was_hidden, effective_changed)
    };

    let widget_height = {
        let state = lock_state();
        state
            .as_ref()
            .map(widget_height_for_state)
            .unwrap_or(sc(AppearancePreset::Default.metrics().widget_height))
    };
    let y = compute_anchor_y(taskbar_rect.top, taskbar_height, widget_height);
    if embedded {
        native_interop::move_window(
            hwnd,
            actual_left_offset,
            y - taskbar_rect.top,
            widget_width,
            widget_height,
        );
    } else {
        let x = taskbar_rect.left + actual_left_offset;
        if blur_active {
            move_frosted_pair(hwnd, x, y, widget_width, widget_height);
        } else {
            native_interop::move_window(hwnd, x, y, widget_width, widget_height);
        }
    }

    {
        let active = lock_state().as_ref().map(|s| s.hwnd.to_hwnd()).unwrap_or(hwnd);
        unsafe { let _ = ShowWindow(active, SW_SHOWNOACTIVATE); }
        if blur_active { sync_blur_backdrop_zorder(active); }
    }

    diagnose::log_lazy(|| format!(
        "positioned taskbar widget monitor={} configured={:?} effective={:?} desired_left={} actual_left={} free_spans={} w={} h={} restored={} layout_changed={}",
        taskbar_monitor.as_deref().unwrap_or("<unknown>"),
        configured_preset,
        effective_preset,
        desired_left_offset,
        actual_left_offset,
        free_spans.len(),
        widget_width,
        widget_height,
        was_hidden,
        effective_changed
    ));
}
fn compute_anchor_y(anchor_top: i32, anchor_height: i32, widget_height: i32) -> i32 {
    anchor_top + (anchor_height - widget_height).max(0) / 2
}

/// WinEvent callback for tray icon location changes
unsafe extern "system" fn on_tray_location_changed(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _thread: u32,
    _time: u32,
) {
    static LAST_REPOSITION: Mutex<Option<std::time::Instant>> = Mutex::new(None);

    let is_tray = {
        let state = lock_state();
        state
            .as_ref()
            .and_then(|s| s.tray_notify_hwnd)
            .map(|h| h == hwnd)
            .unwrap_or(false)
    };

    if is_tray {
        if tray_reposition_is_suppressed() {
            return;
        }

        let should_reposition = {
            let mut last = LAST_REPOSITION.lock().unwrap_or_else(|e| e.into_inner());
            let now = std::time::Instant::now();
            if last
                .map(|t| now.duration_since(t).as_millis() > 500)
                .unwrap_or(true)
            {
                *last = Some(now);
                true
            } else {
                false
            }
        };
        if should_reposition {
            position_at_taskbar();
            render_layered();
        }
    }
}

fn tooltip_blur_hwnd() -> Option<HWND> {
    TOOLTIP_BLUR_HWND.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|h| h.to_hwnd())
}

fn tooltip_blur_context() -> Option<usize> {
    *TOOLTIP_BLUR_CONTEXT.lock().unwrap_or_else(|e| e.into_inner())
}

fn destroy_tooltip_blur_backdrop() {
    if let Some(context) = TOOLTIP_BLUR_CONTEXT.lock().unwrap_or_else(|e| e.into_inner()).take() {
        native_interop::destroy_composition_blur(context);
    }
    *TOOLTIP_BLUR_PARAMS.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if let Some(hwnd) = TOOLTIP_BLUR_HWND.lock().unwrap_or_else(|e| e.into_inner()).take().map(|h| h.to_hwnd()) {
        unsafe { let _ = DestroyWindow(hwnd); }
    }
}

fn hide_tooltip_blur_backdrop() {
    if let Some(hwnd) = tooltip_blur_hwnd() {
        unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
    }
}

/// Keep both top-level tooltip layers owned by the currently active Explorer
/// taskbar. When Start raises that taskbar, an unowned topmost tooltip can
/// otherwise be covered by it. This must never run on the embedded widget.
fn sync_minimal_tooltip_owner(hwnd: HWND, taskbar_owner: Option<HWND>) {
    let Some(owner) = taskbar_owner else { return };
    unsafe {
        if IsWindow(owner).as_bool() && GetWindow(hwnd, GW_OWNER).ok() != Some(owner) {
            let _ = SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner.0 as isize);
        }
    }
}

fn ensure_tooltip_blur_backdrop(blur_amount: f32, tint: Color, width: i32, height: i32) -> Option<HWND> {
    let params = BlurBackdropParams { blur_bits: blur_amount.to_bits(), tint };
    let (clip_inset_px, clip_radius_px) = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| {
                let outer_radius =
                    sc(i32::from(s.styles.active(s.is_dark).tooltip_corner_radius));
                (1.0, (outer_radius - 1).max(0) as f32)
            })
            .unwrap_or((0.0, 0.0))
    };
    if let (Some(hwnd), Some(context)) = (tooltip_blur_hwnd(), tooltip_blur_context()) {
        let unchanged = *TOOLTIP_BLUR_PARAMS.lock().unwrap_or_else(|e| e.into_inner()) == Some(params);
        if unsafe { IsWindow(hwnd).as_bool() }
            && (unchanged || (native_interop::set_composition_blur_amount(context, blur_amount)
                && native_interop::set_composition_blur_tint(context, tint))) {
            let _ = native_interop::set_composition_blur_bounds(
                context,
                width,
                height,
                clip_inset_px,
                clip_radius_px,
            );
            *TOOLTIP_BLUR_PARAMS.lock().unwrap_or_else(|e| e.into_inner()) = Some(params);
            return Some(hwnd);
        }
        destroy_tooltip_blur_backdrop();
    }

    register_blur_backdrop_class();
    unsafe {
        let class_name = native_interop::wide_str("CodexUsageBlurBackdrop");
        let title = native_interop::wide_str("");
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_NOREDIRECTIONBITMAP,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_POPUP,
            0, 0, width.max(1), height.max(1),
            HWND::default(), HMENU::default(), GetModuleHandleW(PCWSTR::null()).ok()?, None,
        ).ok()?;
        let context = native_interop::create_composition_blur(hwnd, blur_amount, tint)?;
        let _ = native_interop::set_composition_blur_bounds(
                context,
                width,
                height,
                clip_inset_px,
                clip_radius_px,
            );
        let owner = { let state = lock_state(); state.as_ref().and_then(|s| s.taskbar_hwnd) };
        native_interop::set_popup_owner(hwnd, owner);
        *TOOLTIP_BLUR_HWND.lock().unwrap_or_else(|e| e.into_inner()) = Some(SendHwnd::from_hwnd(hwnd));
        *TOOLTIP_BLUR_CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(context);
        *TOOLTIP_BLUR_PARAMS.lock().unwrap_or_else(|e| e.into_inner()) = Some(params);
        Some(hwnd)
    }
}

// GDI's GetTextExtentPoint32W can measure slightly narrower than the
// DirectWrite glyph mask used for painting. Reserve asymmetric-safe
// horizontal inset plus extra width so trailing digits are never clipped.
const TOOLTIP_TEXT_INSET: i32 = 12;
const TOOLTIP_TEXT_MEASUREMENT_SLACK: i32 = 20;

fn tooltip_width_with_render_slack(measured_text_px: i32, logical: impl Fn(i32) -> i32) -> i32 {
    (measured_text_px.max(0)
        + logical(2 * TOOLTIP_TEXT_INSET + TOOLTIP_TEXT_MEASUREMENT_SLACK))
        .max(logical(48))
}

unsafe fn render_minimal_tooltip_layered(hwnd: HWND, width: i32, height: i32, frosted_active: bool) {
    let (mut background_color, border_color, text_color, tooltip_corner_radius) = {
        let state = lock_state();
        if let Some(s) = state.as_ref() {
            let style = s.styles.active(s.is_dark);
            (
                style.color(StyleColorTarget::TooltipBackground),
                style.color(StyleColorTarget::TooltipBorder),
                style.color(StyleColorTarget::ResetTime),
                style.tooltip_corner_radius,
            )
        } else {
            (
                Color::from_hex("#30343CFF"),
                Color::from_hex("#626A76FF"),
                Color::from_hex("#F4F6F8FF"),
                0,
            )
        }
    };
    if frosted_active {
        background_color.a = MIN_INTERACTIVE_ALPHA;
    }

    let text = MINIMAL_TOOLTIP_TEXT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let screen_dc = GetDC(hwnd);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let mem_dc = CreateCompatibleDC(screen_dc);
    let dib =
        CreateDIBSection(mem_dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0).unwrap_or_default();
    if dib.is_invalid() || bits.is_null() {
        let _ = DeleteDC(mem_dc);
        ReleaseDC(hwnd, screen_dc);
        return;
    }

    let old_bmp = SelectObject(mem_dc, dib);
    let pixel_data =
        std::slice::from_raw_parts_mut(bits as *mut u32, (width * height) as usize);
    let outer_radius = sc(i32::from(tooltip_corner_radius))
        .min((width.min(height) / 2).max(0));
    let inner_radius = (outer_radius - 1).max(1);
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            if outer_radius > 0
                && !point_in_rounded_box(width, height, x, y, 0, outer_radius)
            {
                pixel_data[idx] = 0;
                continue;
            }
            let inner = if outer_radius > 0 {
                point_in_rounded_box(width, height, x, y, 1, inner_radius)
            } else {
                x > 0 && y > 0 && x < width - 1 && y < height - 1
            };
            pixel_data[idx] = premultiplied_pixel(if inner {
                background_color
            } else {
                border_color
            });
        }
    }

    let text_margin = sc(TOOLTIP_TEXT_INSET);
    let text_rect = RECT {
        left: text_margin,
        top: 0,
        right: (width - text_margin).max(text_margin),
        bottom: height,
    };
    if let Some(mask) = native_interop::directwrite_text_mask(
        &text,
        fonts::face(fonts::FontRole::Ui, None),
        12.0 * CURRENT_DPI.load(Ordering::Relaxed) as f32 / 96.0,
        FW_NORMAL.0 as i32,
        text_rect.right - text_rect.left,
        text_rect.bottom - text_rect.top,
    ) {
        let mask_width = text_rect.right - text_rect.left;
        let mask_height = text_rect.bottom - text_rect.top;
        for mask_y in 0..mask_height {
            for mask_x in 0..mask_width {
                let coverage = mask[(mask_y * mask_width + mask_x) as usize];
                if coverage == 0 {
                    continue;
                }
                let x = text_rect.left + mask_x;
                let y = text_rect.top + mask_y;
                let idx = (y * width + x) as usize;
                pixel_data[idx] =
                    composite_premultiplied_text(pixel_data[idx], text_color, coverage);
            }
        }
    }

    let pt_src = POINT { x: 0, y: 0 };
    let size = SIZE { cx: width, cy: height };
    let blend = BLENDFUNCTION {
        BlendOp: 0,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: 1,
    };
    let _ = UpdateLayeredWindow(
        hwnd,
        screen_dc,
        None,
        Some(&size),
        mem_dc,
        Some(&pt_src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );

    let _ = SelectObject(mem_dc, old_bmp);
    let _ = DeleteObject(dib);
    let _ = DeleteDC(mem_dc);
    ReleaseDC(hwnd, screen_dc);
}

unsafe extern "system" fn minimal_tooltip_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCHITTEST => LRESULT(-1),
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_minimal_tooltip_class() {
    unsafe {
        let class_name = native_interop::wide_str(MINIMAL_TOOLTIP_CLASS);
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(minimal_tooltip_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
    }
}

fn minimal_tooltip_hwnd() -> Option<HWND> {
    {
        let mut state = MINIMAL_TOOLTIP_HWND
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(hwnd) = *state {
            if unsafe { IsWindow(hwnd.to_hwnd()).as_bool() } {
                return Some(hwnd.to_hwnd());
            }
            // Explorer may destroy an owned popup when its taskbar restarts.
            *state = None;
        }
    }

    register_minimal_tooltip_class();
    unsafe {
        let class_name = native_interop::wide_str(MINIMAL_TOOLTIP_CLASS);
        let title = native_interop::wide_str("");
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            sc(112),
            sc(28),
            HWND::default(),
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).ok()?,
            None,
        )
        .ok()?;
        let mut state = MINIMAL_TOOLTIP_HWND
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *state = Some(SendHwnd::from_hwnd(hwnd));
        Some(hwnd)
    }
}

fn minimal_hover_text(target: MinimalHoverTarget) -> Option<String> {
    let state = lock_state();
    let s = state.as_ref()?;
    let codex = s.data.as_ref()?.codex.as_ref()?;
    let strings = s.language.strings();
    let (label, section) = match target {
        MinimalHoverTarget::Session => (strings.session_window, &codex.session),
        MinimalHoverTarget::Weekly => (strings.weekly_window, &codex.weekly),
    };

    let reset = format_precise_reset_time(section.resets_at)
        .unwrap_or_else(|| "--".to_string());
    Some(appearance::quota_reset_tooltip_text(
        effective_appearance_preset(s), label, &reset,
    ))
}

fn hide_minimal_usage_tooltip() {
    let hwnd = {
        let state = MINIMAL_TOOLTIP_HWND
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|hwnd| hwnd.to_hwnd())
    };
    if let Some(hwnd) = hwnd {
        unsafe {
            let was_visible = IsWindowVisible(hwnd).as_bool();
            let _ = ShowWindow(hwnd, SW_HIDE);
            if was_visible {
                
            }
        }
    }
    hide_tooltip_blur_backdrop();
}

unsafe fn measure_minimal_tooltip_width(text: &str) -> i32 {
    let hdc = GetDC(HWND::default());
    if hdc.0.is_null() {
        // Failed measurement must not fall back to a tiny fixed-width box.
        return tooltip_width_with_render_slack(
            sc((text.chars().count().min(64) as i32) * 12),
            sc,
        );
    }
    let font_name = native_interop::wide_str(fonts::face(fonts::FontRole::Ui, None));
    let font = CreateFontW(
        -sc(12),
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
        ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
        PCWSTR::from_raw(font_name.as_ptr()),
    );
    let old_font = SelectObject(hdc, font);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
    let _ = ReleaseDC(HWND::default(), hdc);
    tooltip_width_with_render_slack(size.cx, sc)
}

fn show_minimal_usage_tooltip(target: MinimalHoverTarget) {
    let Some(text) = minimal_hover_text(target) else {
        hide_minimal_usage_tooltip();
        return;
    };
    let Some(tooltip) = minimal_tooltip_hwnd() else {
        return;
    };
    let taskbar_owner = lock_state().as_ref().and_then(|s| s.taskbar_hwnd);
    sync_minimal_tooltip_owner(tooltip, taskbar_owner);
    let measured_width = unsafe { measure_minimal_tooltip_width(&text) };

    {
        let mut tooltip_text = MINIMAL_TOOLTIP_TEXT
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *tooltip_text = text;
    }

    unsafe {
        let mut width = measured_width;
        let height = sc(28);
        let gap = sc(10);
        let margin = sc(4);
        let mut cursor = POINT::default();
        if GetCursorPos(&mut cursor).is_err() {
            return;
        }

        let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        let mut monitor_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let monitor_rect = if GetMonitorInfoW(monitor, &mut monitor_info).as_bool() {
            monitor_info.rcMonitor
        } else {
            RECT {
                left: cursor.x - width,
                top: cursor.y - height,
                right: cursor.x + width,
                bottom: cursor.y + height,
            }
        };
        let max_tooltip_width =
            (monitor_rect.right - monitor_rect.left - margin * 2).max(sc(48));
        width = width.min(max_tooltip_width);

        let mut x = cursor.x + gap;
        let mut y = cursor.y - height - gap;

        if x + width + margin > monitor_rect.right {
            x = cursor.x - width - gap;
        }
        if y < monitor_rect.top + margin {
            y = cursor.y + gap;
        }

        let max_x = (monitor_rect.right - width - margin).max(monitor_rect.left + margin);
        let max_y = (monitor_rect.bottom - height - margin).max(monitor_rect.top + margin);
        x = x.clamp(monitor_rect.left + margin, max_x);
        y = y.clamp(monitor_rect.top + margin, max_y);

        let (tooltip_frosted_strength, tooltip_background, blur_allowed) = {
            let state = lock_state();
            state.as_ref().map(|s| {
                let style = s.styles.active(s.is_dark);
                (
                    style.tooltip_frosted_strength(),
                    style.color(StyleColorTarget::TooltipBackground),
                    style.tooltip_frosted_active(),
                )
            }).unwrap_or((0, Color::from_hex("#30343CFF"), false))
        };
        let frosted_active = if blur_allowed {
            if let Some(backdrop) = ensure_tooltip_blur_backdrop(
                blur_amount_for_strength(tooltip_frosted_strength),
                tooltip_background,
                width,
                height,
            ) {
                sync_minimal_tooltip_owner(backdrop, taskbar_owner);
                let _ = SetWindowPos(backdrop, HWND_TOPMOST, x, y, width, height,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW);
                true
            } else {
                false
            }
        } else {
            hide_tooltip_blur_backdrop();
            false
        };
        let _ = SetWindowPos(
            tooltip,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        ).is_ok();
        render_minimal_tooltip_layered(tooltip, width, height, frosted_active);
        
    }
}

fn point_in_rect(rect: RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

fn minimal_percent_hit(
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Option<(MinimalHoverTarget, RECT)> {
    let (preset, small_taskbar_mode, small_show_weekly, show_session, show_weekly) = {
        let state = lock_state();
        let s = state.as_ref()?;
        (
            effective_appearance_preset(s),
            s.small_taskbar_mode,
            s.small_show_weekly,
            s.show_session_window,
            s.show_weekly_window,
        )
    };
    if preset != AppearancePreset::Minimal {
        return None;
    }

    let effective_show_session = if small_taskbar_mode {
        !small_show_weekly
    } else {
        show_session
    };
    let effective_show_weekly = if small_taskbar_mode {
        small_show_weekly
    } else {
        show_weekly
    };

    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let height = client.bottom - client.top;
    let metrics = preset.metrics();
    let floating = lock_state().as_ref().is_some_and(|s| s.floating_mode);
    let content_x = horizontal_content_left(&metrics, floating);
    let row2_y = height - sc(4) - sc(SEGMENT_H);
    let row1_y = row2_y - sc(metrics.row_gap) - sc(SEGMENT_H);
    let single_row_y = (height - sc(SEGMENT_H)) / 2;
    let width = sc(metrics.percent_width);

    let make_rect = |top: i32| RECT {
        left: content_x - sc(2),
        top: top - sc(2),
        right: content_x + width + sc(2),
        bottom: top + sc(SEGMENT_H) + sc(2),
    };

    if effective_show_session {
        let rect = make_rect(if effective_show_weekly { row1_y } else { single_row_y });
        if point_in_rect(rect, x, y) {
            return Some((MinimalHoverTarget::Session, rect));
        }
    }
    if effective_show_weekly {
        let rect = make_rect(if effective_show_session { row2_y } else { single_row_y });
        if point_in_rect(rect, x, y) {
            return Some((MinimalHoverTarget::Weekly, rect));
        }
    }
    None
}

fn default_reset_hit(
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Option<(MinimalHoverTarget, RECT)> {
    let (preset, small_taskbar_mode, small_show_weekly, show_session, show_weekly) = {
        let state = lock_state();
        let s = state.as_ref()?;
        (
            effective_appearance_preset(s),
            s.small_taskbar_mode,
            s.small_show_weekly,
            s.show_session_window,
            s.show_weekly_window,
        )
    };
    if preset != AppearancePreset::Default {
        return None;
    }

    let effective_show_session = if small_taskbar_mode {
        !small_show_weekly
    } else {
        show_session
    };
    let effective_show_weekly = if small_taskbar_mode {
        small_show_weekly
    } else {
        show_weekly
    };

    let mut client = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut client);
    }
    let height = client.bottom - client.top;
    let metrics = preset.metrics();
    let (label_width, reset_width) = usage_layout_widths(LanguageId::English, preset);
    let segment_count = row_bar_segment_count(preset);
    let progress_width =
        segment_count * (sc(SEGMENT_W) + sc(SEGMENT_GAP)) - sc(SEGMENT_GAP);
    let floating = lock_state().as_ref().is_some_and(|s| s.floating_mode);
    let content_x = horizontal_content_left(&metrics, floating);
    let bar_x = content_x + sc(label_width) + sc(metrics.label_bar_gap);
    let text_x = bar_x + progress_width + sc(metrics.bar_percent_gap);
    let reset_x = text_x + sc(metrics.percent_width) + sc(metrics.percent_reset_gap);
    let row2_y = height - sc(4) - sc(SEGMENT_H);
    let row1_y = row2_y - sc(metrics.row_gap) - sc(SEGMENT_H);
    let single_row_y = (height - sc(SEGMENT_H)) / 2;

    let make_rect = |top: i32| RECT {
        left: reset_x - sc(2),
        top: top - sc(2),
        right: reset_x + sc(reset_width) + sc(2),
        bottom: top + sc(SEGMENT_H) + sc(2),
    };

    if effective_show_session {
        let rect = make_rect(if effective_show_weekly { row1_y } else { single_row_y });
        if point_in_rect(rect, x, y) {
            return Some((MinimalHoverTarget::Session, rect));
        }
    }
    if effective_show_weekly {
        let rect = make_rect(if effective_show_session { row2_y } else { single_row_y });
        if point_in_rect(rect, x, y) {
            return Some((MinimalHoverTarget::Weekly, rect));
        }
    }
    None
}

fn vertical_usage_hover_hit(
    width: i32, height: i32, minimal: bool,
    show_session: bool, show_weekly: bool, x: i32, y: i32,
) -> Option<(MinimalHoverTarget, RECT)> {
    if minimal {
        let (session, weekly) = vertical_minimal_percent_centers(
            height, show_session, show_weekly,
        );
        for (target, center) in [
            (MinimalHoverTarget::Session, session),
            (MinimalHoverTarget::Weekly, weekly),
        ] {
            let Some(cy) = center else { continue };
            let rect = RECT {
                left: sc(2), top: cy - sc(12),
                right: width - sc(2), bottom: cy + sc(12),
            };
            if point_in_rect(rect, x, y) {
                return Some((target, rect));
            }
        }
    } else {
        // Only the short, visible reset label is hoverable. The type, ring,
        // percentage and empty space do not trigger the default tooltip.
        let (session, weekly) = vertical_row_tops(height, show_session, show_weekly);
        let reset_width = sc(36).min(width - sc(4)).max(1);
        for (target, top) in [
            (MinimalHoverTarget::Session, session),
            (MinimalHoverTarget::Weekly, weekly),
        ] {
            let Some(top) = top else { continue };
            let cy = top + sc(VERTICAL_RESET_CENTER_Y);
            let rect = RECT {
                left: (width - reset_width) / 2,
                top: cy - sc(9),
                right: (width + reset_width) / 2,
                bottom: cy + sc(9),
            };
            if point_in_rect(rect, x, y) {
                return Some((target, rect));
            }
        }
    }
    None
}

fn desktop_horizontal_usage_hover_hit(
    width: i32, height: i32, preset: AppearancePreset,
    show_session: bool, show_weekly: bool, x: i32, y: i32,
) -> Option<(MinimalHoverTarget, RECT)> {
    let metrics = preset.metrics();
    let left = horizontal_content_left(&metrics, true);
    let row2_y = height - sc(4) - sc(SEGMENT_H);
    let row1_y = row2_y - sc(metrics.row_gap) - sc(SEGMENT_H);
    let single_y = (height - sc(SEGMENT_H)) / 2;
    let (hit_left, hit_right) = if preset == AppearancePreset::Minimal {
        (left - sc(2), left + sc(metrics.percent_width) + sc(2))
    } else {
        let (label_width, reset_width) = usage_layout_widths(LanguageId::English, preset);
        let bar = row_bar_segment_count(preset)
            * (sc(SEGMENT_W) + sc(SEGMENT_GAP)) - sc(SEGMENT_GAP);
        let x = left + sc(label_width) + sc(metrics.label_bar_gap)
            + bar + sc(metrics.bar_percent_gap)
            + sc(metrics.percent_width) + sc(metrics.percent_reset_gap);
        (x - sc(2), x + sc(reset_width) + sc(2))
    };
    for (target, visible, row_y) in [
        (MinimalHoverTarget::Session, show_session,
         if show_weekly { row1_y } else { single_y }),
        (MinimalHoverTarget::Weekly, show_weekly,
         if show_session { row2_y } else { single_y }),
    ] {
        if !visible { continue; }
        let rect = RECT {
            left: hit_left, top: row_y - sc(2),
            right: hit_right.min(width), bottom: row_y + sc(SEGMENT_H) + sc(2),
        };
        if point_in_rect(rect, x, y) { return Some((target, rect)); }
    }
    None
}

fn usage_hover_hit(
    hwnd: HWND,
    x: i32,
    y: i32,
) -> Option<(MinimalHoverTarget, RECT)> {
    let desktop = {
        let state = lock_state();
        state.as_ref().filter(|s| s.floating_mode).map(|s| (
            total_widget_width_for_state(s),
            widget_height_for_state(s),
            s.floating_edge,
            s.vertical_taskbar_mode,
            effective_appearance_preset(s),
            s.show_session_window, s.show_weekly_window,
        ))
    };
    if let Some((width, height, edge, vertical, preset, show_session, show_weekly)) = desktop {
        let (_, _, body) = floating_canvas_geometry(width, height, edge);
        if !point_in_rect(body, x, y) { return None; }
        let local_x = x - body.left;
        let local_y = y - body.top;
        let hit = if vertical {
            vertical_usage_hover_hit(
                width, height, preset == AppearancePreset::Minimal,
                show_session, show_weekly, local_x, local_y,
            )
        } else {
            desktop_horizontal_usage_hover_hit(
                width, height, preset, show_session, show_weekly,
                local_x, local_y,
            )
        };
        return hit.map(|(target, rect)| (target, RECT {
            left: rect.left + body.left,
            top: rect.top + body.top,
            right: rect.right + body.left,
            bottom: rect.bottom + body.top,
        }));
    }
    let (vertical, minimal, show_session, show_weekly) = {
        let state = lock_state();
        let s = state.as_ref()?;
        (
            s.vertical_taskbar_mode,
            effective_appearance_preset(s) == AppearancePreset::Minimal,
            s.show_session_window,
            s.show_weekly_window,
        )
    };
    if vertical {
        let mut client = RECT::default();
        unsafe { let _ = GetClientRect(hwnd, &mut client); }
        return vertical_usage_hover_hit(
            client.right - client.left, client.bottom - client.top,
            minimal, show_session, show_weekly, x, y,
        );
    }
    minimal_percent_hit(hwnd, x, y).or_else(|| default_reset_hit(hwnd, x, y))
}

fn update_minimal_hover(hwnd: HWND, x: i32, y: i32) {
    let hit = usage_hover_hit(hwnd, x, y);
    let target = hit.map(|(target, _)| target);
    {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else {
            return;
        };
        s.minimal_hover_target = target;
    }

    if let Some((target, _)) = hit {
        show_minimal_usage_tooltip(target);
    } else {
        hide_minimal_usage_tooltip();
    }
}

/// Main window procedure
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Main HWND still owns poll timers, tray callbacks and background updates,
    // but mouse messages from a parked main HWND must never affect the drag
    // state of an active replica.
    if lock_state().as_ref().is_some_and(|s| s.hwnd.to_hwnd() != hwnd)
        && matches!(msg, WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP
            | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_CANCELMODE | WM_CAPTURECHANGED)
    {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    match msg {
        WM_NCHITTEST if lock_state().as_ref().is_some_and(|s|
            s.floating_mode && s.floating_edge.is_none()
        ) => {
            // V24.2: outer glow pixels are visual-only, not clickable glass.
            // The body and detached grip keep their original hit targets.
            let (body, grip) = {
                let state = lock_state();
                let s = state.as_ref().expect("free desktop mode checked above");
                let (w, h, body) = floating_canvas_geometry(
                    total_widget_width_for_state(s), widget_height_for_state(s), None,
                );
                (body, floating_grip_rect(w, h, None, sc(FLOAT_EDGE_PEEK)))
            };
            let screen_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let screen_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let Some(rect) = native_interop::get_window_rect_safe(hwnd) else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let client_x = screen_x - rect.left;
            let client_y = screen_y - rect.top;
            if point_in_rect(body, client_x, client_y)
                || point_in_rect(grip, client_x, client_y) {
                LRESULT(1) // HTCLIENT
            } else {
                LRESULT(-1) // HTTRANSPARENT: ignore the halo-only perimeter
            }
        }
        WM_NCHITTEST if lock_state().as_ref().is_some_and(|s| s.vertical_taskbar_mode) => {
            // Quota content remains a client area for tooltips, while only the
            // top grip starts a drag on WM_LBUTTONDOWN.
            LRESULT(1) // HTCLIENT
        }
        WM_PAINT => {
            let (embedded, frosted_active) = {
                let state = lock_state();
                state
                    .as_ref()
                    .map(|s| (s.embedded, s.composition_blur_active))
                    .unwrap_or((false, false))
            };
            if embedded || frosted_active || lock_state().as_ref().is_some_and(|s| s.floating_mode) {
                // Both taskbar mode and frosted foreground use UpdateLayeredWindow.
                let mut ps = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut ps);
                let _ = EndPaint(hwnd, &ps);
            } else {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                paint(hdc, hwnd, false);
                let _ = EndPaint(hwnd, &ps);
            }
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_DISPLAYCHANGE | WM_DPICHANGED_MSG | WM_SETTINGCHANGE => {
            if msg == WM_DPICHANGED_MSG {
                let new_dpi = (wparam.0 & 0xFFFF) as u32;
                CURRENT_DPI.store(new_dpi, Ordering::Relaxed);
            }
            if msg == WM_SETTINGCHANGE {
                check_theme_change();
                check_language_change();
            }
            refresh_dpi();
            position_at_taskbar();
            render_layered();
            LRESULT(0)
        }
        WM_TIMER => {
            let timer_id = wparam.0;
            match timer_id {
                TIMER_FLOAT_ANIMATION => {
                    step_floating_animation(hwnd);
                }
                TIMER_POLL => {
                    let auth_watch = {
                        let state = lock_state();
                        state.as_ref().map(|s| {
                            (
                                s.auth_error_paused_polling,
                                s.auth_watch_mode,
                                s.auth_watch_snapshot.clone(),
                            )
                        })
                    };
                    match auth_watch {
                        Some((true, watch_mode, previous_snapshot)) => {
                            let current_snapshot = poller::credential_watch_snapshot(watch_mode);
                            if current_snapshot != previous_snapshot {
                                let mut state = lock_state();
                                if let Some(s) = state.as_mut() {
                                    if s.auth_error_paused_polling
                                        && s.auth_watch_mode == watch_mode
                                    {
                                        s.auth_watch_snapshot = current_snapshot;
                                    }
                                }
                                drop(state);
                                let sh = SendHwnd::from_hwnd(hwnd);
                                std::thread::spawn(move || {
                                    do_poll(sh);
                                });
                            }
                        }
                        Some((false, _, _)) => {
                            let sh = SendHwnd::from_hwnd(hwnd);
                            std::thread::spawn(move || {
                                do_poll(sh);
                            });
                        }
                        None => {}
                    }
                }
                TIMER_COUNTDOWN => {
                    update_display();
                    render_layered();
                    schedule_countdown_timer();
                }
                TIMER_RESET_POLL => {
                    let should_poll = {
                        let state = lock_state();
                        state
                            .as_ref()
                            .map(|s| !s.auth_error_paused_polling)
                            .unwrap_or(false)
                    };
                    if should_poll {
                        let sh = SendHwnd::from_hwnd(hwnd);
                        std::thread::spawn(move || {
                            do_poll(sh);
                        });
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        _ if msg == WM_APP_TASKBAR_LAYOUT_UPDATED => {
            position_at_taskbar();
            render_layered();
            LRESULT(0)
        }
        WM_APP_USAGE_UPDATED => {
            check_theme_change();
            check_language_change();
            render_layered();
            schedule_countdown_timer();
            suppress_tray_reposition_for(Duration::from_millis(
                TRAY_ICON_UPDATE_REPOSITION_SUPPRESS_MS,
            ));
            sync_tray_icons(hwnd);
            LRESULT(0)
        }
        WM_SETCURSOR => {
            let is_dragging = {
                let state = lock_state();
                state.as_ref().map(|s| s.dragging).unwrap_or(false)
            };
            if is_dragging {
                let cursor = LoadCursorW(HINSTANCE::default(), IDC_SIZEALL).unwrap_or_default();
                SetCursor(cursor);
                return LRESULT(1);
            }
            if cursor_is_on_drag_handle(hwnd) {
                let cursor = LoadCursorW(HINSTANCE::default(), IDC_SIZEALL).unwrap_or_default();
                SetCursor(cursor);
                return LRESULT(1);
            }
            let small_taskbar_mode = {
                let state = lock_state();
                state
                    .as_ref()
                    .map(|s| s.small_taskbar_mode)
                    .unwrap_or(false)
            };
            if small_taskbar_mode {
                let cursor = LoadCursorW(HINSTANCE::default(), IDC_HAND).unwrap_or_default();
                SetCursor(cursor);
                return LRESULT(1);
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_LBUTTONDOWN => {

            let client_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let client_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            if !is_drag_handle_point(client_x, client_y) {
                return LRESULT(0);
            }
            let window_dpi = GetDpiForWindow(hwnd);
            let dpi = if window_dpi > 0 {
                window_dpi
            } else {
                CURRENT_DPI.load(Ordering::Relaxed)
            }
            .max(1);
            let anchor_logical_x = (client_x as f64 * 96.0 / dpi as f64).round() as i32;
            let anchor_logical_y = (client_y as f64 * 96.0 / dpi as f64).round() as i32;

            let reveal_hidden_panel = {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.dragging = true;
                    s.drag_handle_hovered = true;
                    s.drag_anchor_logical_x = anchor_logical_x;
                    s.drag_anchor_logical_y = anchor_logical_y;
                    s.floating_drag_anchor_x = client_x;
                    s.floating_drag_anchor_y = client_y;
                    s.failed_drag_target = None;
                    if s.floating_mode {
                        s.floating_first_collapse_guard = false;
                        s.floating_collapsed = false;
                        s.floating_reveal = 100;
                        s.floating_slide_from = 100;
                        s.floating_slide_target = 100;
                        s.floating_grip_hover = true;
                        s.floating_grip_alpha = 100;
                        s.floating_fade_from = 100;
                        s.floating_fade_target = 100;
                        true
                    } else { false }
                } else { false }
            };
            // Restore panel pixels and a fully visible tab at the current HWND
            // location before dragging, without changing the docking geometry.
            if reveal_hidden_panel { render_layered(); }
            {
                let mut last = LAST_DRAG_FRAME.lock().unwrap_or_else(|e| e.into_inner());
                *last = None;
            }
            let previous_capture = SetCapture(hwnd);
            diagnose::log_lazy(|| format!(
                "drag start hwnd={:?} vertical={} client=({}, {}) previous_capture={:?}",
                hwnd,
                lock_state().as_ref().is_some_and(|s| s.vertical_taskbar_mode),
                client_x, client_y, previous_capture,
            ));
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if lock_state().as_ref().is_some_and(|s|
                s.floating_mode && floating_hover_can_reveal(
                    s.floating_collapsed, s.floating_first_collapse_guard, s.dragging,
                )
            ) {
                set_floating_collapsed(false);
            }
            let is_dragging = {
                let state = lock_state();
                state.as_ref().map(|s| s.dragging).unwrap_or(false)
            };
            if is_dragging {
                hide_minimal_usage_tooltip();
            } else {
                set_floating_grip_hover(true);
                let client_x = (lparam.0 & 0xFFFF) as i16 as i32;
                let client_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let drag_hovered = is_drag_handle_point(client_x, client_y);
                let drag_hover_changed = {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        let changed = s.drag_handle_hovered != drag_hovered;
                        s.drag_handle_hovered = drag_hovered;
                        changed
                    } else {
                        false
                    }
                };
                update_minimal_hover(hwnd, client_x, client_y);
                if drag_hover_changed {
                    render_layered();
                }
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
            }
            if is_dragging {
                if !drag_frame_due(false) {
                    return LRESULT(0);
                }
                let mut pt = POINT::default();
                if GetCursorPos(&mut pt).is_err() {
                    return LRESULT(0);
                }

                if lock_state().as_ref().is_some_and(|s| s.floating_mode) {
                    update_floating_drag(hwnd, pt);
                    return LRESULT(0);
                }
                let (current_taskbar_hwnd, blur_active) = {
                    let state = lock_state();
                    state
                        .as_ref()
                        .map(|s| (s.taskbar_hwnd, s.composition_blur_active))
                        .unwrap_or((None, false))
                };

                // Switching taskbars still requires a precise taskbar hit.
                // If the cursor leaves all taskbars, keep moving the widget
                // horizontally on its validated current parent instead.
                let Some((hovered_taskbar_index, hovered_taskbar)) = taskbar_for_drag_point(pt) else {
                    return LRESULT(0);
                };

                let mut switched_taskbar = false;
                if current_taskbar_hwnd != Some(hovered_taskbar.hwnd) {
                    if lock_state().as_ref().and_then(|s| s.failed_drag_target) == Some(hovered_taskbar.hwnd) {
                        return LRESULT(0);
                    }
                        let previous_dpi = CURRENT_DPI.load(Ordering::Relaxed);
                        let target_dpi = GetDpiForWindow(hovered_taskbar.hwnd);
                        if target_dpi > 0 {
                            CURRENT_DPI.store(target_dpi, Ordering::Relaxed);
                        }

                        {
                            let mut state = lock_state();
                            if let Some(s) = state.as_mut() {
                                s.drag_reparenting = true;
                            }
                        }
                        
                        let _ = ReleaseCapture();
                        

                        let switched = attach_to_taskbar_window(
                            hwnd, hovered_taskbar_index, &hovered_taskbar
                        );
                        if switched {
                            {
                                let mut state = lock_state();
                                if let Some(s) = state.as_mut() {
                                    s.dragging = true;
                                    s.drag_reparenting = false;
                                    s.failed_drag_target = None;
                                }
                            }
                            // After a replica switch the active HWND changes.
                            // Re-capturing the parked/hidden old HWND would leave
                            // the new visible widget unresponsive to dragging.
                            let active_hwnd = lock_state()
                                .as_ref()
                                .map(|s| s.hwnd.to_hwnd())
                                .unwrap_or(hwnd);
                            SetCapture(active_hwnd);
                            
                            switched_taskbar = true;
                        } else {
                            CURRENT_DPI.store(previous_dpi, Ordering::Relaxed);
                            let mut state = lock_state();
                            if let Some(s) = state.as_mut() {
                                s.drag_reparenting = false;
                                s.failed_drag_target = Some(hovered_taskbar.hwnd);
                            }
                            SetCapture(hwnd);
                            
                            // The widget is still a child of the old taskbar.
                            // Its client coordinates must not follow the pointer
                            // into another monitor or it will be clipped away.
                            return LRESULT(0);
                        }
                }

                let drag_context = {
                    let state = lock_state();
                    state.as_ref().and_then(|s| {
                        s.taskbar_hwnd.map(|taskbar_hwnd| {
                            (taskbar_hwnd, s.drag_anchor_logical_x, s.drag_anchor_logical_y, s.embedded)
                        })
                    })
                };

                if let Some((taskbar_hwnd, anchor_logical_x, anchor_logical_y, embedded)) = drag_context {
                    if let Some(taskbar_rect) = native_interop::get_taskbar_rect(taskbar_hwnd) {
                        let dpi = GetDpiForWindow(taskbar_hwnd);
                        if dpi > 0 {
                            CURRENT_DPI.store(dpi, Ordering::Relaxed);
                        }
                        let effective_dpi = CURRENT_DPI.load(Ordering::Relaxed).max(1);
                        let anchor_px = drag_anchor_px_for_dpi(anchor_logical_x, effective_dpi);
                        let anchor_y_px = drag_anchor_px_for_dpi(anchor_logical_y, effective_dpi);
                        let taskbar_height = taskbar_rect.bottom - taskbar_rect.top;
                        let vertical = taskbar_axis(taskbar_rect) == TaskbarAxis::Vertical;
                        let small_mode = !vertical && is_small_taskbar_height(taskbar_height);

                        let (widget_width, widget_height, small_mode_changed) = {
                            let mut state = lock_state();
                            let s = match state.as_mut() {
                                Some(s) => s,
                                None => return LRESULT(0),
                            };
                            let changed = s.small_taskbar_mode != small_mode
                                || s.vertical_taskbar_mode != vertical;
                            if changed {
                                s.vertical_taskbar_mode = vertical;
                                s.small_taskbar_mode = small_mode;
                                if vertical {
                                    if let Some(width) = vertical_widget_width(taskbar_rect) {
                                        s.vertical_widget_width = width;
                                    }
                                }
                                if small_mode {
                                    s.small_show_weekly = false;
                                }
                            }
                            (
                                total_widget_width_for_state(s),
                                widget_height_for_state(s),
                                changed,
                            )
                        };
                        let (window_x, window_y) = if vertical {
                            let left = (taskbar_rect.right - taskbar_rect.left - widget_width) / 2;
                            let top = drag_top_for_pointer(
                                taskbar_rect, pt, anchor_y_px, widget_height,
                            );
                            if embedded {
                                (left, top)
                            } else {
                                (taskbar_rect.left + left, taskbar_rect.top + top)
                            }
                        } else {
                            let drag_left = drag_left_for_pointer(
                                taskbar_rect, pt, anchor_px, widget_width,
                            );
                            let anchor_y =
                                compute_anchor_y(taskbar_rect.top, taskbar_height, widget_height);
                            if embedded {
                                (drag_left, anchor_y - taskbar_rect.top)
                            } else {
                                (taskbar_rect.left + drag_left, anchor_y)
                            }
                        };

                        // Use the current visible HWND, not the old source HWND
                        // whose WM_MOUSEMOVE initiated this cross-monitor switch.
                        let drag_hwnd = lock_state()
                            .as_ref()
                            .map(|s| s.hwnd.to_hwnd())
                            .unwrap_or(hwnd);
                        // Embedded mode uses taskbar-client coordinates; Composition blur popup
                        // mode uses absolute screen coordinates.
                        if blur_active {
                            move_frosted_pair(
                                drag_hwnd,
                                window_x,
                                window_y,
                                widget_width,
                                widget_height,
                            );
                        } else {
                            move_window_without_repaint(
                                drag_hwnd,
                                window_x,
                                window_y,
                                widget_width,
                                widget_height,
                            );
                        }

                        if vertical {
                            diagnose::log_lazy(|| format!(
                                "vertical drag move hwnd={:?} pointer=({}, {}) new=({}, {}) size=({}, {})",
                                drag_hwnd, pt.x, pt.y, window_x, window_y, widget_width, widget_height,
                            ));
                        }
                        if switched_taskbar || small_mode_changed {
                            render_layered();
                        }
                    }
                }
            }
            LRESULT(0)
        }
        _ if msg == WM_MOUSELEAVE_MSG => {
            if lock_state().as_ref().is_some_and(|s| s.floating_mode && s.dragging) {
                return LRESULT(0);
            }
            if floating_pointer_still_over_free_grip(hwnd) {
                // Capture change or popup movement can dispatch a stale leave
                // despite the cursor still being over the detached tab.
                // Re-arm tracking so the *next real exit* still hides the glow.
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                return LRESULT(0);
            }
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.minimal_hover_target = None;
                    s.drag_handle_hovered = false;
                }
            }
            hide_minimal_usage_tooltip();
            set_floating_grip_hover(false);
            set_floating_collapsed(true);
            render_layered();
            LRESULT(0)
        }
        WM_CANCELMODE => {
            
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.dragging = false;
                    s.drag_reparenting = false;
                }
            }
            let _ = ReleaseCapture();
            LRESULT(0)
        }
        WM_CAPTURECHANGED => {
            
            let mut state = lock_state();
            if let Some(s) = state.as_mut() {
                if !s.drag_reparenting {
                    s.dragging = false;
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if lock_state().as_ref().is_some_and(|s| s.floating_mode) {
                let was_dragging = lock_state().as_ref().is_some_and(|s| s.dragging);
                let _ = drag_frame_due(true);
                if was_dragging {
                    // Keep the grip fully visible until its final rectangle
                    // and hover state are committed in one layered frame.
                    finish_floating_drag(hwnd);
                } else if let Some(s) = lock_state().as_mut() {
                    s.dragging = false;
                }
                let _ = ReleaseCapture();
                if floating_pointer_still_over_free_grip(hwnd) {
                    // Capture ended but no mouse motion is required to
                    // maintain hover or re-arm leave tracking.
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                }
                return LRESULT(0);
            }

            let mut pt = POINT::default();
            let cursor_valid = GetCursorPos(&mut pt).is_ok();
            let drag_result = {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.drag_reparenting = false;
                    let was_dragging = s.dragging;
                    s.dragging = false;
                    let failed_target = s.failed_drag_target.take();
                    if was_dragging {
                        Some((s.taskbar_hwnd, s.drag_anchor_logical_x, s.drag_anchor_logical_y, failed_target))
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            let _ = ReleaseCapture();
            let _ = drag_frame_due(true);

            if drag_result.is_none() {
                let client_x = (lparam.0 & 0xFFFF) as i16 as i32;
                let client_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                if !is_drag_handle_point(client_x, client_y) {
                    let toggled = {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            if s.small_taskbar_mode {
                                s.small_show_weekly = !s.small_show_weekly;
                                true
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    };
                    if toggled {
                        render_layered();
                    }
                }
            }

            if let Some((current_taskbar_hwnd, anchor_logical_x, anchor_logical_y, failed_target)) = drag_result {
                // Releasing over the desktop commits the last horizontal
                // drag position on the current taskbar instead of reverting.
                // An actual hit on another taskbar still takes precedence.
                let release_taskbar = if cursor_valid {
                    taskbar_for_drag_point(pt)
                } else {
                    None
                };
                if let Some((target_index, target_taskbar)) = release_taskbar {
                    if current_taskbar_hwnd != Some(target_taskbar.hwnd) && failed_target != Some(target_taskbar.hwnd) {
                        let switched = attach_to_taskbar_window(hwnd, target_index, &target_taskbar);
                        if !switched {
                            // A failed cross-taskbar drop must not save the
                            // target monitor's screen X as an old-taskbar offset.
                            // Restore the last known safe position and visibility.
                            diagnose::log("drag release: taskbar switch failed; restoring original taskbar placement");
                            position_at_taskbar();
                            render_layered();
                            return LRESULT(0);
                        }
                    }

                    refresh_dpi();
                    let final_taskbar = {
                        let state = lock_state();
                        state.as_ref().and_then(|s| s.taskbar_hwnd)
                    };
                    if let Some(taskbar_hwnd) = final_taskbar {
                        if let Some(taskbar_rect) = native_interop::get_taskbar_rect(taskbar_hwnd) {
                            let dpi = GetDpiForWindow(taskbar_hwnd);
                            if dpi > 0 {
                                CURRENT_DPI.store(dpi, Ordering::Relaxed);
                            }
                            let effective_dpi = CURRENT_DPI.load(Ordering::Relaxed).max(1);
                            if taskbar_axis(taskbar_rect) == TaskbarAxis::Vertical {
                                let anchor_y_px = drag_anchor_px_for_dpi(anchor_logical_y, effective_dpi);
                                let height = {
                                    let state = lock_state();
                                    state.as_ref()
                                        .map(|s| vertical_widget_height_for_preset(effective_appearance_preset(s)))
                                        .unwrap_or(sc(VERTICAL_WIDGET_HEIGHT))
                                };
                                let desired_top = drag_top_from_cursor(
                                    taskbar_rect, pt, anchor_y_px,
                                );
                                let clamped_top = clamp_left_offset(
                                    desired_top,
                                    (taskbar_rect.bottom - taskbar_rect.top - height).max(0),
                                );
                                let safe_top = taskbar_free_vertical_spans(taskbar_hwnd, taskbar_rect)
                                    .and_then(|spans| nearest_free_top_offset(
                                        taskbar_rect, &spans, height, clamped_top,
                                    ));
                                if let Some(top) = safe_top {
                                    diagnose::log_lazy(|| format!(
                                        "vertical drag released y={} committed_top={}", pt.y, top,
                                    ));
                                    let mut state = lock_state();
                                    if let Some(s) = state.as_mut() {
                                        s.taskbar_top_offset = top;
                                        s.legacy_tray_offset = None;
                                    }
                                } else {
                                    diagnose::log("drag release found no safe vertical gap");
                                }
                                position_at_taskbar();
                                render_layered();
                                save_state_settings();
                                return LRESULT(0);
                            }
                            let anchor_px =
                                drag_anchor_px_for_dpi(anchor_logical_x, effective_dpi);
                            let final_drag_left =
                                drag_left_from_cursor(taskbar_rect, pt, anchor_px);
                            let (configured_preset, language, current_width) = {
                                let state = lock_state();
                                state
                                    .as_ref()
                                    .map(|s| {
                                        (
                                            s.appearance_preset,
                                            s.language,
                                            total_widget_width_for_state(s),
                                        )
                                    })
                                    .unwrap_or((
                                        AppearancePreset::Default,
                                        LanguageId::English,
                                        total_widget_width_for(LanguageId::English),
                                    ))
                            };
                            let clamped_drag_left = left_offset_for_drag_left(
                                taskbar_hwnd,
                                taskbar_rect,
                                current_width,
                                final_drag_left,
                            );
                            let safe_left_offset = taskbar_free_spans(taskbar_hwnd, taskbar_rect)
                                .and_then(|free_spans| {
                                    placement_for_free_spans(
                                        configured_preset,
                                        language,
                                        taskbar_rect,
                                        &free_spans,
                                        clamped_drag_left,
                                    )
                                })
                                .map(|(_, _, left)| left);

                            if let Some(new_left_offset) = safe_left_offset {
                                {
                                    let mut state = lock_state();
                                    if let Some(s) = state.as_mut() {
                                        s.taskbar_left_offset = new_left_offset;
                                        s.legacy_tray_offset = None;
                                    }
                                }
                                let monitor = {
                                    let state = lock_state();
                                    state
                                        .as_ref()
                                        .and_then(|s| s.taskbar_monitor.clone())
                                        .unwrap_or_else(|| "<unknown>".to_string())
                                };
                                diagnose::log(format!(
                                    "drag saved safe taskbar_left_offset={} monitor={}",
                                    new_left_offset, monitor
                                ));
                            } else {
                                diagnose::log(
                                    "drag release found no safe taskbar span; keeping desired offset and applying hidden state",
                                );
                            }
                            position_at_taskbar();
                            render_layered();
                        }
                    }
                    save_state_settings();
                } else {
                    // Without a valid taskbar parent or cursor position, do
                    // not persist an invented placement; restore the last one.
                    position_at_taskbar();
                    render_layered();
                    diagnose::log(
                        "drag released outside taskbars; restored last valid taskbar position",
                    );
                }
            }
            LRESULT(0)
        }
        updater::WM_APP_STARTUP_UPDATE_RESULT => {
            if let Some(result) = updater::take_startup_update_result() {
                match result {
                    updater::StartupUpdateCheckResult::Available { version } => {
                        let strings = {
                            let mut state = lock_state();
                            match state.as_mut() {
                                Some(s) => {
                                    s.available_update_version = Some(version.clone());
                                    s.language.strings()
                                }
                                None => LanguageId::English.strings(),
                            }
                        };
                        tray_icon::clear_notification(hwnd);
                        let message = format!("{} v{}", strings.update_available, version);
                        tray_icon::notify_info(
                            hwnd,
                            tray_icon::TrayIconKind::Codex,
                            strings.update_title,
                            &message,
                        );
                    }
                    updater::StartupUpdateCheckResult::Current => {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.available_update_version = None;
                        }
                    }
                    updater::StartupUpdateCheckResult::Failed => {
                        diagnose::log("startup update check ended without a user notification");
                    }
                }
            }
            LRESULT(0)
        }
        updater::WM_APP_UPDATE_PROGRESS => {
            while let Some(progress) = updater::take_progress() {
                let strings = {
                    let state = lock_state();
                    state
                        .as_ref()
                        .map(|s| s.language.strings())
                        .unwrap_or_else(|| LanguageId::English.strings())
                };
                let message = match progress {
                    updater::UpdateProgress::Checking => strings.update_checking.to_string(),
                    updater::UpdateProgress::Updating { version } => {
                        tray_icon::clear_notification(hwnd);
                        format!("{} v{}…", strings.update_downloading, version)
                    }
                };
                tray_icon::notify_info(
                    hwnd,
                    tray_icon::TrayIconKind::Codex,
                    strings.update_title,
                    &message,
                );
            }
            LRESULT(0)
        }
        updater::WM_APP_UPDATE_RESULT => {
            if let Some(result) = updater::take_ui_result() {
                match result {
                    updater::UpdateUiResult::Current { version } => {
                        let strings = {
                            let state = lock_state();
                            state
                                .as_ref()
                                .map(|s| s.language.strings())
                                .unwrap_or_else(|| LanguageId::English.strings())
                        };
                        tray_icon::clear_notification(hwnd);
                        let message = format!("{} v{}", strings.update_current, version);
                        tray_icon::notify_info(
                            hwnd,
                            tray_icon::TrayIconKind::Codex,
                            strings.update_title,
                            &message,
                        );
                    }
                    updater::UpdateUiResult::ManualUpdateRequired { version } => {
                        let (strings, language) = {
                            let state = lock_state();
                            state
                                .as_ref()
                                .map(|s| (s.language.strings(), s.language))
                                .unwrap_or_else(|| {
                                    (LanguageId::English.strings(), LanguageId::English)
                                })
                        };
                        tray_icon::clear_notification(hwnd);
                        let message = manual_update_required_message(language, &version);
                        tray_icon::notify_info(
                            hwnd,
                            tray_icon::TrayIconKind::Codex,
                            strings.update_title,
                            &message,
                        );
                    }
                    updater::UpdateUiResult::Failed { error, detail } => {
                        let strings = {
                            let state = lock_state();
                            state
                                .as_ref()
                                .map(|s| s.language.strings())
                                .unwrap_or_else(|| LanguageId::English.strings())
                        };
                        let message = match error {
                            updater::UpdateError::CheckFailed
                            | updater::UpdateError::InvalidRelease => strings.update_check_failed,
                            updater::UpdateError::DownloadFailed
                            | updater::UpdateError::AssetMissing => strings.update_download_failed,
                            updater::UpdateError::InvalidChecksum
                            | updater::UpdateError::ChecksumMismatch => {
                                strings.update_checksum_failed
                            }
                            updater::UpdateError::TargetNotWritable => {
                                strings.update_target_not_writable
                            }
                            updater::UpdateError::HelperLaunchFailed => {
                                strings.update_helper_failed
                            }
                        };
                        let message = if detail.is_empty() {
                            message.to_string()
                        } else {
                            format!("{message}: {detail}")
                        };
                        tray_icon::clear_notification(hwnd);
                        tray_icon::notify_warning(
                            hwnd,
                            tray_icon::TrayIconKind::Codex,
                            strings.update_title,
                            &message,
                        );
                    }
                    updater::UpdateUiResult::ReadyToRestart => {
                        tray_icon::clear_notification(hwnd);
                        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            let mut anchor = POINT {
                x: (lparam.0 & 0xFFFF) as i16 as i32,
                y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = ClientToScreen(hwnd, &mut anchor);
            show_context_menu(hwnd, anchor, false);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 as u16;
            match id {
                1 => {
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.codex_session_text = "...".to_string();
                            s.codex_weekly_text = "...".to_string();
                            s.force_notify_auth_error = true;
                        }
                    }
                    render_layered();
                    let sh = SendHwnd::from_hwnd(taskbar_replica_service_hwnd(hwnd));
                    std::thread::spawn(move || {
                        do_poll(sh);
                    });
                }
                2 => {
                    destroy_blur_backdrop();
                    destroy_parked_blur_backdrops();
            destroy_tooltip_blur_backdrop();
                    let hook = {
                        let state = lock_state();
                        state.as_ref().and_then(|s| s.win_event_hook)
                    };
                    if let Some(h) = hook {
                        native_interop::unhook_win_event(h);
                    }
                    PostQuitMessage(0);
                }
                IDM_CHECK_UPDATE => {
                    #[cfg(feature = "github-update")]
                    {
                        diagnose::log("update command requested");
                        let _ = updater::start_update(taskbar_replica_service_hwnd(hwnd));
                    }
                }
                IDM_OPEN_RELEASES => {
                    open_github_releases(hwnd);
                }
                IDM_RESET_POSITION => {
                    let target = {
                        let state = lock_state();
                        state.as_ref().and_then(|s| {
                            let taskbar_hwnd = s.taskbar_hwnd?;
                            let taskbar_rect = native_interop::get_taskbar_rect(taskbar_hwnd)?;
                            if taskbar_axis(taskbar_rect) == TaskbarAxis::Vertical {
                                let spans = taskbar_free_vertical_spans(taskbar_hwnd, taskbar_rect)?;
                                let desired_bottom = taskbar_rect.bottom - taskbar_rect.top;
                                let top = nearest_free_top_offset(
                                    taskbar_rect, &spans,
                                    vertical_widget_height_for_preset(effective_appearance_preset(s)),
                                    desired_bottom,
                                )?;
                                return Some((TaskbarAxis::Vertical, top));
                            }
                            let free_spans = taskbar_free_spans(taskbar_hwnd, taskbar_rect)?;
                            let desired_right = taskbar_rect.right - taskbar_rect.left;
                            placement_for_free_spans(
                                s.appearance_preset,
                                s.language,
                                taskbar_rect,
                                &free_spans,
                                desired_right,
                            )
                            .map(|(_, _, left)| (TaskbarAxis::Horizontal, left))
                        })
                    };
                    if let Some((axis, target)) = target {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            match axis {
                                TaskbarAxis::Horizontal => s.taskbar_left_offset = target,
                                TaskbarAxis::Vertical => s.taskbar_top_offset = target,
                            }
                            s.legacy_tray_offset = None;
                        }
                    }
                    save_state_settings();
                    position_at_taskbar();
                    render_layered();
                }
                IDM_START_WITH_WINDOWS => {
                    set_startup_enabled(!is_startup_enabled());
                }
                IDM_FREQ_1MIN | IDM_FREQ_5MIN | IDM_FREQ_15MIN | IDM_FREQ_1HOUR => {
                    let new_interval = match id {
                        IDM_FREQ_1MIN => POLL_1_MIN,
                        IDM_FREQ_5MIN => POLL_5_MIN,
                        IDM_FREQ_15MIN => POLL_15_MIN,
                        IDM_FREQ_1HOUR => POLL_1_HOUR,
                        _ => POLL_15_MIN,
                    };
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.poll_interval_ms = new_interval;
                        }
                    }
                    save_state_settings();
                    // Reset the poll timer with the new interval
                    SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, new_interval, None);
                }
                IDM_SHOW_SESSION_WINDOW | IDM_SHOW_WEEKLY_WINDOW => {
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            match id {
                                IDM_SHOW_SESSION_WINDOW
                                    if s.show_weekly_window || !s.show_session_window =>
                                {
                                    s.show_session_window = !s.show_session_window;
                                }
                                IDM_SHOW_WEEKLY_WINDOW
                                    if s.show_session_window || !s.show_weekly_window =>
                                {
                                    s.show_weekly_window = !s.show_weekly_window;
                                }
                                _ => {}
                            }
                        }
                    }
                    save_state_settings();
                    render_layered();
                    sync_tray_icons(hwnd);
                }
                IDM_ALERT_OFF | IDM_ALERT_10 | IDM_ALERT_20 | IDM_ALERT_30 => {
                    let threshold = match id {
                        IDM_ALERT_10 => 10,
                        IDM_ALERT_20 => 20,
                        IDM_ALERT_30 => 30,
                        _ => 0,
                    };
                    let alerts = {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.alert_threshold_percent = threshold;
                            if threshold == 0 {
                                s.notified_quota_windows.clear();
                                Vec::new()
                            } else if let Some(data) = s.data.clone() {
                                collect_low_quota_alerts(s, &data)
                            } else {
                                Vec::new()
                            }
                        } else {
                            Vec::new()
                        }
                    };
                    notify_quota_alerts(hwnd, &alerts);
                    save_state_settings();
                }
                IDM_THEME_SYSTEM | IDM_THEME_DARK | IDM_THEME_LIGHT => {
                    close_style_editors();
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.theme_mode = match id {
                                IDM_THEME_DARK => ThemeMode::Dark,
                                IDM_THEME_LIGHT => ThemeMode::Light,
                                _ => ThemeMode::System,
                            };
                            s.is_dark = match s.theme_mode {
                                ThemeMode::System => theme::is_dark_mode(),
                                ThemeMode::Dark => true,
                                ThemeMode::Light => false,
                            };
                        }
                    }
                    save_state_settings();
                    render_layered();
                    style_window::sync(style_settings_snapshot());
                }
                IDM_LAYOUT_DEFAULT | IDM_LAYOUT_MINIMAL | IDM_LAYOUT_ADAPTIVE => {
                    // The retired Adaptive command is an alias for Default.
                    let preset = if id == IDM_LAYOUT_MINIMAL {
                        AppearancePreset::Minimal
                    } else {
                        AppearancePreset::Default
                    };
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.appearance_preset = preset;
                            refresh_usage_texts(s);
                        }
                    }
                    hide_minimal_usage_tooltip();
                    save_state_settings();
                    position_at_taskbar();
                    render_layered();
                    sync_tray_icons(hwnd);
                    style_window::sync(style_settings_snapshot());
                }
                IDM_STYLE_SETTINGS => {
                    // Popup-menu root commands are dispatched synchronously
                    // before the root is destroyed. Defer focus until its
                    // z-order and activation changes have finished.
                    let _ = PostMessageW(
                        taskbar_replica_service_hwnd(hwnd),
                        WM_APP_FOCUS_EXISTING_SETTINGS,
                        WPARAM(0),
                        LPARAM(0),
                    );
                }
                IDM_STYLE_PANEL_BACKGROUND
                | IDM_STYLE_PANEL_BORDER
                | IDM_STYLE_QUOTA_TYPE
                | IDM_STYLE_REMAINING
                | IDM_STYLE_RESET_TIME
                | IDM_STYLE_ERROR
                | IDM_STYLE_PROGRESS_HIGH
                | IDM_STYLE_PROGRESS_MEDIUM
                | IDM_STYLE_PROGRESS_LOW
                | IDM_STYLE_PROGRESS_CONSUMED
                | IDM_STYLE_DRAG_HANDLE => {
                    let target = match id {
                        IDM_STYLE_PANEL_BACKGROUND => StyleColorTarget::PanelBackground,
                        IDM_STYLE_PANEL_BORDER => StyleColorTarget::PanelBorder,
                        IDM_STYLE_QUOTA_TYPE => StyleColorTarget::QuotaType,
                        IDM_STYLE_REMAINING => StyleColorTarget::Remaining,
                        IDM_STYLE_RESET_TIME => StyleColorTarget::ResetTime,
                        IDM_STYLE_ERROR => StyleColorTarget::Error,
                        IDM_STYLE_PROGRESS_HIGH => StyleColorTarget::ProgressHigh,
                        IDM_STYLE_PROGRESS_MEDIUM => StyleColorTarget::ProgressMedium,
                        IDM_STYLE_PROGRESS_LOW => StyleColorTarget::ProgressLow,
                        IDM_STYLE_PROGRESS_CONSUMED => StyleColorTarget::ProgressConsumed,
                        _ => StyleColorTarget::DragHandle,
                    };
                    open_color_editor(hwnd, target);
                }
                IDM_STYLE_PANEL_BLUR => {
                    open_blur_editor(hwnd);
                }
                IDM_STYLE_RESET_CURRENT => {
                    close_style_editors();
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.styles.reset_active(s.is_dark);
                        }
                    }
                    save_state_settings();
                    render_layered();
                }
                IDM_LANG_SYSTEM
                | IDM_LANG_ENGLISH
                | IDM_LANG_DUTCH
                | IDM_LANG_SPANISH
                | IDM_LANG_FRENCH
                | IDM_LANG_GERMAN
                | IDM_LANG_JAPANESE
                | IDM_LANG_KOREAN
                | IDM_LANG_SIMPLIFIED_CHINESE
                | IDM_LANG_TRADITIONAL_CHINESE
                | IDM_LANG_RUSSIAN
                | IDM_LANG_PORTUGUESE_BRAZIL => {
                    let language_override = match id {
                        IDM_LANG_SYSTEM => None,
                        IDM_LANG_ENGLISH => Some(LanguageId::English),
                        IDM_LANG_DUTCH => Some(LanguageId::Dutch),
                        IDM_LANG_SPANISH => Some(LanguageId::Spanish),
                        IDM_LANG_FRENCH => Some(LanguageId::French),
                        IDM_LANG_GERMAN => Some(LanguageId::German),
                        IDM_LANG_JAPANESE => Some(LanguageId::Japanese),
                        IDM_LANG_KOREAN => Some(LanguageId::Korean),
                        IDM_LANG_SIMPLIFIED_CHINESE => Some(LanguageId::SimplifiedChinese),
                        IDM_LANG_TRADITIONAL_CHINESE => Some(LanguageId::TraditionalChinese),
                        IDM_LANG_RUSSIAN => Some(LanguageId::Russian),
                        IDM_LANG_PORTUGUESE_BRAZIL => Some(LanguageId::PortugueseBrazil),
                        _ => None,
                    };
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            apply_language_to_state(s, language_override);
                        }
                    }
                    save_state_settings();
                    render_layered();
                }
                _ => {}
            }
            LRESULT(0)
        }
        _ if msg == WM_APP_FOCUS_EXISTING_SETTINGS => {
            close_style_editors();
            style_window::open_or_focus(
                taskbar_replica_service_hwnd(hwnd),
                style_settings_snapshot(),
            );
            LRESULT(0)
        }
        _ if msg == WM_APP_TRAY => {
            match tray_icon::handle_message(lparam) {
                tray_icon::TrayAction::ShowContextMenu => {
                    let mut anchor = POINT::default();
                    let _ = GetCursorPos(&mut anchor);
                    show_context_menu(hwnd, anchor, true);
                }
                tray_icon::TrayAction::None => {}
            }
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_COLOR_PREVIEW => {
            if let Some(target) = style_window::decode_color_target(wparam.0) {
                let color = style_window::unpack_color(lparam.0);
                {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.styles.active_mut(s.is_dark).set_color(target, color);
                    }
                }
                render_layered();
            }
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_TOOLTIP_BLUR_PREVIEW => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.styles.active_mut(s.is_dark).set_tooltip_frosted_strength(wparam.0 as u8);
                }
            }
            save_state_settings();
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_CORNER_PREVIEW => {
            let section_code = wparam.0 & 0xFF;
            let radius = ((wparam.0 >> 8) & 0xFF) as u8;
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    let style = s.styles.active_mut(s.is_dark);
                    match section_code {
                        0 => style.panel_corner_radius = radius,
                        1 => style.tooltip_corner_radius = radius,
                        2 => style.progress_corner_radius = radius,
                        _ => return LRESULT(0),
                    }
                }
            }
            hide_minimal_usage_tooltip();
            render_layered();
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_BLUR_PREVIEW => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.styles.active_mut(s.is_dark).panel_frosted_strength =
                        (wparam.0 as u8).min(FROSTED_STRENGTH_MAX);
                }
            }
            render_layered();
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_SAVE => {
            save_state_settings();
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_THEME_CHANGE => {
            close_style_editors();
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.theme_mode = match wparam.0 {
                        1 => ThemeMode::Dark,
                        2 => ThemeMode::Light,
                        _ => ThemeMode::System,
                    };
                    s.is_dark = match s.theme_mode {
                        ThemeMode::System => theme::is_dark_mode(),
                        ThemeMode::Dark => true,
                        ThemeMode::Light => false,
                    };
                }
            }
            save_state_settings();
            render_layered();
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_LAYOUT_CHANGE => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.appearance_preset = match wparam.0 {
                        1 => AppearancePreset::Minimal,
                        2 => AppearancePreset::Default,
                        _ => AppearancePreset::Default,
                    };
                    s.styles.clamp_corner_radii(s.appearance_preset);
                    refresh_usage_texts(s);
                }
            }
            hide_minimal_usage_tooltip();
            save_state_settings();
            position_at_taskbar();
            render_layered();
            sync_tray_icons(hwnd);
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_POSITION_CHANGE => {
            let position = if wparam.0 == 1 {
                DisplayPosition::Desktop
            } else {
                DisplayPosition::Taskbar
            };
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.display_position = position;
                }
            }
            hide_minimal_usage_tooltip();
            save_state_settings();
            position_at_taskbar();
            render_layered();
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_DESKTOP_DIRECTION_CHANGE => {
            let direction = if wparam.0 == 1 {
                DesktopDirection::Vertical
            } else {
                DesktopDirection::Horizontal
            };
            let changed = {
                let mut state = lock_state();
                match state.as_mut() {
                    Some(s) if s.display_position == DisplayPosition::Desktop
                        && s.desktop_direction != direction => {
                        s.desktop_direction = direction;
                        true
                    }
                    _ => false,
                }
            };
            if !changed {
                return LRESULT(0);
            }
            let start = Instant::now();
            diagnose::log_lazy(|| format!(
                "desktop orientation switch start direction={direction:?}",
            ));
            save_state_settings();
            diagnose::log_lazy(|| "desktop orientation switch: applying window geometry".to_string());
            // In desktop mode position_at_taskbar() calls
            // enable_floating_widget(), which already moves and repaints the
            // popup. A second render_layered() here repeated the expensive
            // WinRT/DirectComposition updates after every toggle.
            position_at_taskbar();
            diagnose::log_lazy(|| format!(
                "desktop orientation switch: geometry applied direction={direction:?} elapsed_ms={}",
                start.elapsed().as_millis(),
            ));
            style_window::sync(style_settings_snapshot());
            diagnose::log_lazy(|| format!(
                "desktop orientation switch complete direction={direction:?} elapsed_ms={}",
                start.elapsed().as_millis(),
            ));
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_PRESET_CHANGE => {
            let Some(preset) = ThemePreset::from_index(wparam.0) else {
                return LRESULT(0);
            };
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.styles.active_mut(s.is_dark).apply_preset(s.is_dark, preset);
                }
            }
            hide_minimal_usage_tooltip();
            save_state_settings();
            render_layered();
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        _ if msg == style_window::WM_SETTINGS_REFRESH_CHANGE => {
            let interval = wparam.0 as u32;
            if matches!(interval, POLL_1_MIN | POLL_5_MIN | POLL_15_MIN | POLL_1_HOUR) {
                {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.poll_interval_ms = interval;
                    }
                }
                save_state_settings();
                unsafe {
                    SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, interval, None);
                }
                style_window::sync(style_settings_snapshot());
            }
            LRESULT(0)
        }
        _ if msg == style_window::WM_SETTINGS_USAGE_CHANGE => {
            let mask = wparam.0;
            let show_session = mask & 1 != 0;
            let show_weekly = mask & 2 != 0;
            if show_session || show_weekly {
                {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.show_session_window = show_session;
                        s.show_weekly_window = show_weekly;
                    }
                }
                save_state_settings();
                position_at_taskbar();
                render_layered();
                sync_tray_icons(hwnd);
                style_window::sync(style_settings_snapshot());
            }
            LRESULT(0)
        }
        _ if msg == style_window::WM_SETTINGS_ALERT_CHANGE => {
            let threshold = wparam.0 as u8;
            if matches!(threshold, 0 | 10 | 20 | 30) {
                let alerts = {
                    let mut state = lock_state();
                    if let Some(s) = state.as_mut() {
                        s.alert_threshold_percent = threshold;
                        if threshold == 0 {
                            s.notified_quota_windows.clear();
                            Vec::new()
                        } else if let Some(data) = s.data.clone() {
                            collect_low_quota_alerts(s, &data)
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    }
                };
                notify_quota_alerts(hwnd, &alerts);
                save_state_settings();
                style_window::sync(style_settings_snapshot());
            }
            LRESULT(0)
        }
        _ if msg == style_window::WM_SETTINGS_STARTUP_CHANGE => {
            set_startup_enabled(wparam.0 != 0);
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        _ if msg == style_window::WM_SETTINGS_LANGUAGE_CHANGE => {
            let Some(language) = LanguageId::SELECTABLE.get(wparam.0).copied() else {
                return LRESULT(0);
            };
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    apply_language_to_state(s, Some(language));
                }
            }
            save_state_settings();
            render_layered();
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        _ if msg == style_window::WM_SETTINGS_JSON_APPLY => {
            if let Some(settings) = style_window::take_pending_editable_settings() {
                if let Err(error) = apply_editable_settings(hwnd, settings) {
                    diagnose::log(format!("JSON settings apply rejected: {error}"));
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            hide_minimal_usage_tooltip();

            destroy_blur_backdrop();
            destroy_parked_blur_backdrops();
            let hook = {
                let state = lock_state();
                state.as_ref().and_then(|s| s.win_event_hook)
            };
            if let Some(h) = hook {
                native_interop::unhook_win_event(h);
            }
            tray_icon::remove_all(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}


fn style_color_target_label(target: StyleColorTarget, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match target {
        StyleColorTarget::PanelBackground => if zh { "背景颜色" } else { "Background color" },
        StyleColorTarget::PanelBorder => if zh { "边框颜色" } else { "Border color" },
        StyleColorTarget::TooltipBackground => if zh { "浮框背景颜色" } else { "Tooltip background" },
        StyleColorTarget::TooltipBorder => if zh { "浮框边框颜色" } else { "Tooltip border" },
        StyleColorTarget::QuotaType => if zh { "额度类型" } else { "Quota type" },
        StyleColorTarget::Remaining => if zh { "剩余额度" } else { "Remaining quota" },
        StyleColorTarget::ResetTime => if zh { "重置时间" } else { "Reset time" },
        StyleColorTarget::Error => if zh { "异常状态" } else { "Error state" },
        StyleColorTarget::ProgressHigh => if zh { "充足额度颜色" } else { "High quota color" },
        StyleColorTarget::ProgressMedium => if zh { "中等额度颜色" } else { "Medium quota color" },
        StyleColorTarget::ProgressLow => if zh { "低额度颜色" } else { "Low quota color" },
        StyleColorTarget::ProgressConsumed => if zh { "已消耗部分颜色" } else { "Consumed color" },
        StyleColorTarget::DragHandle => if zh { "拖拽点颜色" } else { "Drag handle color" },
    }
}

fn close_style_editors() {
    let color_hwnd = {
        let state = COLOR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.hwnd.to_hwnd())
    };
    let blur_hwnd = {
        let state = BLUR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.hwnd.to_hwnd())
    };
    if color_hwnd.is_some() || blur_hwnd.is_some() {
        save_state_settings();
    }
    unsafe {
        if let Some(hwnd) = color_hwnd {
            let _ = DestroyWindow(hwnd);
        }
        if let Some(hwnd) = blur_hwnd {
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn refresh_widget_after_style_editor_close() {
    let hwnd = {
        let state = lock_state();
        state.as_ref().map(|s| s.hwnd.to_hwnd())
    };
    render_layered();
    if widget_hidden_for_taskbar_space() {
        return;
    }
    if let Some(hwnd) = hwnd {
        let (embedded, frosted_active) = {
            let state = lock_state();
            state
                .as_ref()
                .map(|s| (s.embedded, s.composition_blur_active))
                .unwrap_or((true, false))
        };
        if frosted_active {
            sync_blur_backdrop_zorder(hwnd);
        } else if !embedded {
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    }
}

fn apply_style_color(theme_is_dark: bool, target: StyleColorTarget, color: Color) {
    let mut state = lock_state();
    if let Some(s) = state.as_mut() {
        s.styles.active_mut(theme_is_dark).set_color(target, color);
    }
}

fn apply_frosted_strength(theme_is_dark: bool, strength: u8) {
    let mut state = lock_state();
    if let Some(s) = state.as_mut() {
        s.styles.active_mut(theme_is_dark).panel_frosted_strength =
            strength.min(FROSTED_STRENGTH_MAX);
    }
}

unsafe fn create_editor_static(
    parent: HWND,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Option<HWND> {
    let class = native_interop::wide_str("STATIC");
    let text = native_interop::wide_str(text);
    CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(text.as_ptr()),
        WS_CHILD | WS_VISIBLE,
        x,
        y,
        width,
        height,
        parent,
        HMENU::default(),
        GetModuleHandleW(PCWSTR::null()).ok()?,
        None,
    )
    .ok()
}

unsafe fn create_editor_trackbar(
    parent: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    max_value: i32,
    value: i32,
) -> Option<HWND> {
    let class = native_interop::wide_str("msctls_trackbar32");
    let title = native_interop::wide_str("");
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(title.as_ptr()),
        WS_CHILD | WS_VISIBLE,
        x,
        y,
        width,
        height,
        parent,
        HMENU::default(),
        GetModuleHandleW(PCWSTR::null()).ok()?,
        None,
    )
    .ok()?;
    let range = ((max_value as u32) << 16) as isize;
    let _ = SendMessageW(hwnd, TBM_SETRANGE_MSG, WPARAM(1), LPARAM(range));
    let _ = SendMessageW(
        hwnd,
        TBM_SETPOS_MSG,
        WPARAM(1),
        LPARAM(value.clamp(0, max_value) as isize),
    );
    Some(hwnd)
}

fn update_color_editor_labels(editor: ColorEditorState, color: Color) {
    let values = [color.r, color.g, color.b, color.a];
    unsafe {
        for (label, value) in editor.value_labels.iter().zip(values) {
            let text = native_interop::wide_str(&value.to_string());
            let _ = SetWindowTextW(label.to_hwnd(), PCWSTR::from_raw(text.as_ptr()));
        }
        let hex = native_interop::wide_str(&color.to_hex_rgba());
        let _ = SetWindowTextW(editor.hex_label.to_hwnd(), PCWSTR::from_raw(hex.as_ptr()));
    }
}

unsafe fn color_editor_value(editor: &ColorEditorState) -> Color {
    let values = editor.sliders.map(|slider| {
        SendMessageW(slider.to_hwnd(), TBM_GETPOS_MSG, WPARAM(0), LPARAM(0)).0 as u8
    });
    Color::rgba(values[0], values[1], values[2], values[3])
}

unsafe extern "system" fn color_editor_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HSCROLL => {
            let editor = {
                let state = COLOR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().copied().filter(|s| s.hwnd.to_hwnd() == hwnd)
            };
            if let Some(editor) = editor {
                let color = color_editor_value(&editor);
                update_color_editor_labels(editor, color);
                apply_style_color(editor.theme_is_dark, editor.target, color);
                let code = (wparam.0 & 0xFFFF) as u16;
                let final_frame = code == TB_ENDTRACK_CODE;
                render_style_preview(final_frame);
                if final_frame {
                    save_state_settings();
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            save_state_settings();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            {
                let mut state = COLOR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
                if state.as_ref().map(|s| s.hwnd.to_hwnd()) == Some(hwnd) {
                    *state = None;
                }
            }
            refresh_widget_after_style_editor_close();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_color_editor_class() {
    unsafe {
        let class_name = native_interop::wide_str("CodexUsageColorEditor");
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(color_editor_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            hbrBackground: CreateSolidBrush(COLORREF(native_interop::colorref(240, 240, 240))),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
    }
}

fn open_color_editor(owner: HWND, target: StyleColorTarget) {
    close_style_editors();
    register_color_editor_class();
    let (theme_is_dark, language, color) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else {
            return;
        };
        (s.is_dark, s.language, s.styles.active(s.is_dark).color(target))
    };

    unsafe {
        let class_name = native_interop::wide_str("CodexUsageColorEditor");
        let title = format!(
            "{} - {}",
            if language == LanguageId::SimplifiedChinese { "样式" } else { "Style" },
            style_color_target_label(target, language)
        );
        let title = native_interop::wide_str(&title);
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            pt.x + 12,
            pt.y + 12,
            350,
            245,
            owner,
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) else {
            return;
        };

        let channel_names = ["R", "G", "B", "A"];
        let channel_values = [color.r, color.g, color.b, color.a];
        let mut sliders = [SendHwnd(0); 4];
        let mut labels = [SendHwnd(0); 4];
        for index in 0..4 {
            let y = 18 + index as i32 * 38;
            let Some(_) = create_editor_static(hwnd, channel_names[index], 12, y + 5, 20, 22)
            else {
                let _ = DestroyWindow(hwnd);
                return;
            };
            let Some(slider) = create_editor_trackbar(hwnd, 34, y, 230, 30, 255, channel_values[index] as i32)
            else {
                let _ = DestroyWindow(hwnd);
                return;
            };
            let Some(value_label) =
                create_editor_static(hwnd, &channel_values[index].to_string(), 274, y + 5, 50, 22)
            else {
                let _ = DestroyWindow(hwnd);
                return;
            };
            sliders[index] = SendHwnd::from_hwnd(slider);
            labels[index] = SendHwnd::from_hwnd(value_label);
        }

        let Some(hex_label) =
            create_editor_static(hwnd, &color.to_hex_rgba(), 34, 172, 150, 24)
        else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let hint = if language == LanguageId::SimplifiedChinese {
            "拖动时实时预览，操作结束自动保存"
        } else {
            "Live preview while dragging; changes save automatically"
        };
        let _ = create_editor_static(hwnd, hint, 34, 196, 290, 22);

        let editor = ColorEditorState {
            hwnd: SendHwnd::from_hwnd(hwnd),
            theme_is_dark,
            target,
            sliders,
            value_labels: labels,
            hex_label: SendHwnd::from_hwnd(hex_label),
        };
        {
            let mut state = COLOR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
            *state = Some(editor);
        }
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn update_blur_editor_label(editor: BlurEditorState, strength: u8, language: LanguageId) {
    let text = if strength == 0 {
        if language == LanguageId::SimplifiedChinese {
            "0%（关闭）".to_string()
        } else {
            "0% (Off)".to_string()
        }
    } else {
        format!("{}%", strength.min(FROSTED_STRENGTH_MAX))
    };
    unsafe {
        let text = native_interop::wide_str(&text);
        let _ = SetWindowTextW(editor.value_label.to_hwnd(), PCWSTR::from_raw(text.as_ptr()));
    }
}

unsafe extern "system" fn blur_editor_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_HSCROLL => {
            let editor = {
                let state = BLUR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().copied().filter(|s| s.hwnd.to_hwnd() == hwnd)
            };
            if let Some(editor) = editor {
                let strength =
                    SendMessageW(editor.slider.to_hwnd(), TBM_GETPOS_MSG, WPARAM(0), LPARAM(0)).0
                        as u8;
                let language = {
                    let state = lock_state();
                    state.as_ref().map(|s| s.language).unwrap_or(LanguageId::English)
                };
                update_blur_editor_label(editor, strength, language);
                apply_frosted_strength(editor.theme_is_dark, strength);
                let code = (wparam.0 & 0xFFFF) as u16;
                let final_frame = code == TB_ENDTRACK_CODE;
                render_style_preview(final_frame);
                if final_frame {
                    save_state_settings();
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            save_state_settings();
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            {
                let mut state = BLUR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
                if state.as_ref().map(|s| s.hwnd.to_hwnd()) == Some(hwnd) {
                    *state = None;
                }
            }
            refresh_widget_after_style_editor_close();
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn register_blur_editor_class() {
    unsafe {
        let class_name = native_interop::wide_str("CodexUsageBlurEditor");
        let hinstance = GetModuleHandleW(PCWSTR::null()).unwrap();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(blur_editor_wnd_proc),
            hInstance: HINSTANCE(hinstance.0),
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default(),
            hbrBackground: CreateSolidBrush(COLORREF(native_interop::colorref(240, 240, 240))),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
    }
}

fn open_blur_editor(owner: HWND) {
    close_style_editors();
    register_blur_editor_class();
    let (theme_is_dark, language, strength) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else {
            return;
        };
        (
            s.is_dark,
            s.language,
            s.styles.active(s.is_dark).panel_frosted_strength,
        )
    };

    unsafe {
        let class_name = native_interop::wide_str("CodexUsageBlurEditor");
        let title = native_interop::wide_str(if language == LanguageId::SimplifiedChinese {
            "样式 - 磨砂强度"
        } else {
            "Style - Frosted intensity"
        });
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            pt.x + 12,
            pt.y + 12,
            360,
            160,
            owner,
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) else {
            return;
        };

        let Some(slider) = create_editor_trackbar(
            hwnd,
            18,
            20,
            260,
            32,
            i32::from(FROSTED_STRENGTH_MAX),
            i32::from(strength.min(FROSTED_STRENGTH_MAX)),
        )
        else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let Some(value_label) = create_editor_static(hwnd, "", 286, 25, 64, 22) else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let hint = if language == LanguageId::SimplifiedChinese {
            "0%=关闭；1–100%=磨砂强度；拖动实时预览并自动保存"
        } else {
            "0%=Off; 1–100%=frosted intensity; live preview and auto-save"
        };
        let _ = create_editor_static(hwnd, hint, 18, 65, 330, 22);

        let editor = BlurEditorState {
            hwnd: SendHwnd::from_hwnd(hwnd),
            theme_is_dark,
            slider: SendHwnd::from_hwnd(slider),
            value_label: SendHwnd::from_hwnd(value_label),
        };
        {
            let mut state = BLUR_EDITOR_STATE.lock().unwrap_or_else(|e| e.into_inner());
            *state = Some(editor);
        }
        update_blur_editor_label(editor, strength, language);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetForegroundWindow(hwnd);
    }
}

fn editable_settings_from_state(state: &AppState, startup_enabled: bool) -> EditableSettings {
    EditableSettings {
        schema_version: EDITABLE_SETTINGS_SCHEMA_VERSION,
        general: EditableGeneral {
            refresh_interval: match state.poll_interval_ms {
                POLL_1_MIN => "1m",
                POLL_5_MIN => "5m",
                POLL_1_HOUR => "1h",
                _ => "15m",
            }
            .to_string(),
            show_usage: EditableUsage {
                session_5h: state.show_session_window,
                weekly: state.show_weekly_window,
            },
            quota_alert_percent: state.alert_threshold_percent,
            start_with_windows: startup_enabled,
            language: state
                .language_override
                .map(|language| language.code().to_string())
                .unwrap_or_else(|| "system".to_string()),
        },
        appearance: EditableAppearance {
            theme: match state.theme_mode {
                ThemeMode::System => "system",
                ThemeMode::Dark => "dark",
                ThemeMode::Light => "light",
            }
            .to_string(),
            layout: match state.appearance_preset.configured() {
                AppearancePreset::Default => "default",
                AppearancePreset::Minimal => "minimal",
                AppearancePreset::Adaptive => unreachable!(),
            }
            .to_string(),
            position: if state.display_position == DisplayPosition::Desktop {
                "desktop"
            } else { "taskbar" }.to_string(),
            desktop_direction: if state.desktop_direction == DesktopDirection::Vertical {
                "vertical"
            } else { "horizontal" }.to_string(),
            dark: EditableThemeStyle::from_theme_style(&state.styles.dark),
            light: EditableThemeStyle::from_theme_style(&state.styles.light),
        },
    }
}

fn default_editable_settings() -> EditableSettings {
    EditableSettings {
        schema_version: EDITABLE_SETTINGS_SCHEMA_VERSION,
        general: EditableGeneral {
            refresh_interval: "15m".to_string(),
            show_usage: EditableUsage {
                session_5h: true,
                weekly: true,
            },
            quota_alert_percent: 0,
            start_with_windows: false,
            language: "system".to_string(),
        },
        appearance: EditableAppearance {
            theme: "system".to_string(),
            layout: "default".to_string(),
            position: "taskbar".to_string(),
            desktop_direction: "horizontal".to_string(),
            dark: EditableThemeStyle::from_theme_style(&ThemeStyle::dark_default()),
            light: EditableThemeStyle::from_theme_style(&ThemeStyle::light_default()),
        },
    }
}

fn style_settings_snapshot() -> style_window::StyleWindowSnapshot {
    let startup_enabled = is_startup_enabled();
    let state = lock_state();
    if let Some(s) = state.as_ref() {
        style_window::StyleWindowSnapshot {
            language: s.language,
            theme_mode: s.theme_mode,
            is_dark: s.is_dark,
            appearance_preset: s.appearance_preset.configured(),
            display_position: s.display_position,
            desktop_direction: s.desktop_direction,
            vertical_taskbar_mode: s.vertical_taskbar_mode,
            active_style: s.styles.active(s.is_dark).clone(),
            editable_settings: editable_settings_from_state(s, startup_enabled),
        }
    } else {
        style_window::StyleWindowSnapshot {
            language: LanguageId::English,
            theme_mode: ThemeMode::System,
            is_dark: true,
            appearance_preset: AppearancePreset::Default,
            display_position: DisplayPosition::Taskbar,
            desktop_direction: DesktopDirection::Horizontal,
            vertical_taskbar_mode: false,
            active_style: ThemeStyle::dark_default(),
            editable_settings: default_editable_settings(),
        }
    }
}

fn apply_editable_settings(hwnd: HWND, settings: EditableSettings) -> Result<(), String> {
    let settings = settings.validate_and_normalize()?;
    let poll_interval_ms = match settings.general.refresh_interval.as_str() {
        "1m" => POLL_1_MIN,
        "5m" => POLL_5_MIN,
        "1h" => POLL_1_HOUR,
        _ => POLL_15_MIN,
    };
    let language_override = if settings.general.language == "system" {
        None
    } else {
        LanguageId::SELECTABLE
            .iter()
            .copied()
            .find(|language| language.code() == settings.general.language)
    };
    if settings.general.language != "system" && language_override.is_none() {
        return Err(format!(
            "unsupported language code: {}",
            settings.general.language
        ));
    }
    let theme_mode = match settings.appearance.theme.as_str() {
        "dark" => ThemeMode::Dark,
        "light" => ThemeMode::Light,
        _ => ThemeMode::System,
    };
    let appearance_preset = match settings.appearance.layout.as_str() {
        "minimal" => AppearancePreset::Minimal,
        "adaptive" => AppearancePreset::Default,
        _ => AppearancePreset::Default,
    };
    let startup_enabled = settings.general.start_with_windows;

    let alerts = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else {
            return Err("application state is unavailable".to_string());
        };
        s.poll_interval_ms = poll_interval_ms;
        s.show_session_window = settings.general.show_usage.session_5h;
        s.show_weekly_window = settings.general.show_usage.weekly;
        s.alert_threshold_percent = settings.general.quota_alert_percent;
        if s.alert_threshold_percent == 0 {
            s.notified_quota_windows.clear();
        }

        s.theme_mode = theme_mode;
        s.is_dark = match theme_mode {
            ThemeMode::System => theme::is_dark_mode(),
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
        };
        s.appearance_preset = appearance_preset;
        s.display_position = if settings.appearance.position == "desktop" {
            DisplayPosition::Desktop
        } else { DisplayPosition::Taskbar };
        s.desktop_direction = if settings.appearance.desktop_direction == "vertical" {
            DesktopDirection::Vertical
        } else { DesktopDirection::Horizontal };
        s.styles.dark = settings.appearance.dark.to_theme_style();
        s.styles.light = settings.appearance.light.to_theme_style();
        s.styles.clamp_corner_radii(s.appearance_preset);
        apply_language_to_state(s, language_override);
        refresh_usage_texts(s);

        if s.alert_threshold_percent > 0 {
            if let Some(data) = s.data.clone() {
                collect_low_quota_alerts(s, &data)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    };

    set_startup_enabled(startup_enabled);
    save_state_settings();
    unsafe {
        SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, poll_interval_ms, None);
    }
    hide_minimal_usage_tooltip();
    position_at_taskbar();
    render_layered();
    sync_tray_icons(hwnd);
    notify_quota_alerts(hwnd, &alerts);
    style_window::sync(style_settings_snapshot());
    Ok(())
}


fn show_context_menu(hwnd: HWND, anchor: POINT, preserve_foreground: bool) {
    let (strings, language, available_update_version) = {
        let state = lock_state();
        match state.as_ref() {
            Some(s) => (
                s.language.strings(),
                s.language,
                s.available_update_version.clone(),
            ),
            None => (
                LanguageId::English.strings(),
                LanguageId::English,
                None,
            ),
        }
    };

    let settings_text = match language {
        LanguageId::SimplifiedChinese => "设置...",
        LanguageId::TraditionalChinese => "設定...",
        _ => "Settings...",
    };
    let check_text = match language {
        LanguageId::SimplifiedChinese => "检查更新",
        LanguageId::TraditionalChinese => "檢查更新",
        LanguageId::Japanese => "更新を確認",
        LanguageId::Korean => "업데이트 확인",
        _ => "Check for updates",
    };
    let github_text = match language {
        LanguageId::SimplifiedChinese | LanguageId::TraditionalChinese => "前往 GitHub",
        _ => "Open GitHub",
    };
    let version_label = if cfg!(feature = "github-update") {
        match available_update_version.as_deref() {
            Some(latest) => format!("v{} --> v{}", env!("CARGO_PKG_VERSION"), latest),
            None => format!("v{}", env!("CARGO_PKG_VERSION")),
        }
    } else {
        format!("v{} (Microsoft Store)", env!("CARGO_PKG_VERSION"))
    };

    let check_item = if cfg!(feature = "github-update") {
        PopupItem::command(check_text, IDM_CHECK_UPDATE)
    } else {
        PopupItem::disabled_command(check_text, IDM_CHECK_UPDATE)
    };

    let version_items = vec![
        check_item,
        PopupItem::command(github_text, IDM_OPEN_RELEASES),
    ];
    let items = vec![
        PopupItem::command(strings.refresh, 1),
        PopupItem::command(strings.reset_position, IDM_RESET_POSITION),
        PopupItem::separator(),
        PopupItem::command(settings_text, IDM_STYLE_SETTINGS),
        PopupItem::separator(),
        PopupItem::submenu(version_label, version_items),
        PopupItem::separator(),
        PopupItem::command(strings.exit, 2),
    ];

    popup_menu::show(
        hwnd,
        items,
        theme::is_dark_mode(),
        fonts::face(fonts::FontRole::Ui, Some(language)),
        anchor,
        preserve_foreground,
    );
}

/// Paint for non-embedded fallback (normal WM_PAINT path)
fn paint(hdc: HDC, hwnd: HWND, composition_blur_active: bool) {
    let (
        is_dark,
        language,
        strings,
        codex_session_pct,
        codex_session_text,
        codex_weekly_pct,
        codex_weekly_text,
        show_session_window,
        show_weekly_window,
        last_poll_ok,
    ) = {
        let state = lock_state();
        match state.as_ref() {
            Some(s) => (
                s.is_dark,
                s.language,
                s.language.strings(),
                s.codex_session_percent,
                s.codex_session_text.clone(),
                s.codex_weekly_percent,
                s.codex_weekly_text.clone(),
                s.show_session_window,
                s.show_weekly_window,
                s.last_poll_ok,
            ),
            None => return,
        }
    };

    let bg_color = if is_dark {
        Color::from_hex("#1C1C1CFF")
    } else {
        Color::from_hex("#F3F3F3FF")
    };

    unsafe {
        let mut client_rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut client_rect);
        let width = client_rect.right - client_rect.left;
        let height = client_rect.bottom - client_rect.top;
        if width <= 0 || height <= 0 {
            return;
        }

        if composition_blur_active {
            // The Composition backdrop owns the blurred background. Draw only foreground content.
            paint_content(
                hdc,
                width,
                height,
                is_dark,
                &bg_color,
                language,
                strings,
                codex_session_pct,
                &codex_session_text,
                codex_weekly_pct,
                &codex_weekly_text,
                show_session_window,
                show_weekly_window,
                last_poll_ok,
                false,
            );
            return;
        }

        let mem_dc = CreateCompatibleDC(hdc);
        let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
        let old_bmp = SelectObject(mem_dc, mem_bmp);
        paint_content(
            mem_dc,
            width,
            height,
            is_dark,
            &bg_color,
            language,
            strings,
            codex_session_pct,
            &codex_session_text,
            codex_weekly_pct,
            &codex_weekly_text,
            show_session_window,
            show_weekly_window,
            last_poll_ok,
            true,
        );
        let _ = BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);
        SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(mem_bmp);
        let _ = DeleteDC(mem_dc);
    }
}

fn draw_minimal_usage_value(
    hdc: HDC,
    x: i32,
    y: i32,
    text: &str,
    primary_color: &Color,
) {
    let row_height = sc(SEGMENT_H);
    draw_usage_value_text(
        hdc,
        x,
        y,
        row_height,
        text,
        primary_color,
        primary_color,
        0,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_row(
    hdc: HDC,
    x: i32,
    y: i32,
    quota_type_color: &Color,
    primary_color: &Color,
    reset_color: &Color,
    label: &str,
    percent: f64,
    value_text: &str,
    track: &Color,
    label_width: i32,
    text_width: i32,
    _panel_base: Color,
) {
    let seg_h = sc(SEGMENT_H);
    let preset = current_appearance_preset();
    let segment_count = row_bar_segment_count(preset);
    let metrics = preset.metrics();

    unsafe {
        let _ = SetTextColor(hdc, COLORREF(quota_type_color.to_colorref()));
        let mut label_wide: Vec<u16> = label.encode_utf16().collect();
        let mut label_rect = RECT {
            left: x,
            top: y,
            right: x + sc(label_width),
            bottom: y + seg_h,
        };
        let _ = DrawTextW(
            hdc,
            &mut label_wide,
            &mut label_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        let bar_x = x + sc(label_width) + sc(metrics.label_bar_gap);
        let bar_color = quota_bar_color(percent).blend_over(*track);
        draw_usage_bar(
            hdc,
            bar_x,
            y,
            segment_count,
            percent,
            value_text,
            &bar_color,
            track,
            primary_color,
            reset_color,
            text_width,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_usage_bar(
    hdc: HDC,
    bar_x: i32,
    y: i32,
    segment_count: i32,
    percent: f64,
    text: &str,
    accent: &Color,
    track: &Color,
    primary_color: &Color,
    reset_color: &Color,
    text_width: i32,
) {
    let seg_w = sc(SEGMENT_W);
    let seg_h = sc(SEGMENT_H);
    let seg_gap = sc(SEGMENT_GAP);
    let metrics = current_appearance_preset().metrics();
    let progress_width = segment_count * (seg_w + seg_gap) - seg_gap;
    let bar_h = sc(metrics.bar_height).min(seg_h);
    let bar_y = y + (seg_h - bar_h) / 2;

    unsafe {
        let percent_clamped = percent.clamp(0.0, 100.0);
        let bar_rect = RECT {
            left: bar_x,
            top: bar_y,
            right: bar_x + progress_width,
            bottom: bar_y + bar_h,
        };
        let requested_radius =
            sc(i32::from(current_theme_style().progress_corner_radius));
        let radius = requested_radius.min(bar_h / 2).max(0);
        if radius > 0 {
            draw_rounded_rect(hdc, &bar_rect, track, radius);
        } else {
            let track_brush = CreateSolidBrush(COLORREF(track.to_colorref()));
            FillRect(hdc, &bar_rect, track_brush);
            let _ = DeleteObject(track_brush);
        }

        let fill_width = (progress_width as f64 * percent_clamped / 100.0).round() as i32;
        if fill_width > 0 {
            let fill_rect = RECT {
                left: bar_x,
                top: bar_y,
                right: bar_x + fill_width,
                bottom: bar_y + bar_h,
            };
            if radius > 0 {
                draw_rounded_rect(
                    hdc,
                    &fill_rect,
                    accent,
                    radius.min((fill_width / 2).max(1)),
                );
            } else {
                let fill_brush = CreateSolidBrush(COLORREF(accent.to_colorref()));
                FillRect(hdc, &fill_rect, fill_brush);
                let _ = DeleteObject(fill_brush);
            }
        }

        let text_x = bar_x + progress_width + sc(metrics.bar_percent_gap);
        draw_usage_value_text(
            hdc,
            text_x,
            y,
            seg_h,
            text,
            primary_color,
            reset_color,
            text_width,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_usage_value_text(
    hdc: HDC,
    text_x: i32,
    y: i32,
    row_height: i32,
    text: &str,
    primary_color: &Color,
    secondary_color: &Color,
    total_text_width: i32,
) {
    let preset = current_appearance_preset();
    let metrics = preset.metrics();
    let (primary, secondary) = text
        .split_once("  ")
        .map(|(primary, secondary)| (primary, Some(secondary)))
        .unwrap_or((text, None));

    unsafe {
        let font_name = native_interop::wide_str(fonts::face(fonts::FontRole::Taskbar, None));
        let primary_font = CreateFontW(
            sc(metrics.value_font_height),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_TT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            gdi_text_quality(),
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            PCWSTR::from_raw(font_name.as_ptr()),
        );
        let old_font = SelectObject(hdc, primary_font);
        let _ = SetTextColor(hdc, COLORREF(primary_color.to_colorref()));
        let mut primary_wide: Vec<u16> = primary.encode_utf16().collect();
        let mut primary_rect = RECT {
            left: text_x,
            top: y,
            right: text_x + sc(metrics.percent_width),
            bottom: y + row_height,
        };
        let _ = DrawTextW(
            hdc,
            &mut primary_wide,
            &mut primary_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        if let Some(secondary) = secondary {
            let secondary_font = CreateFontW(
                sc(metrics.secondary_font_height),
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
                gdi_text_quality(),
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                PCWSTR::from_raw(font_name.as_ptr()),
            );
            SelectObject(hdc, secondary_font);
            let _ = SetTextColor(hdc, COLORREF(secondary_color.to_colorref()));
            let mut secondary_wide: Vec<u16> = secondary.encode_utf16().collect();
            let secondary_x = text_x + sc(metrics.percent_width) + sc(metrics.percent_reset_gap);
            let mut secondary_rect = RECT {
                left: secondary_x,
                top: y,
                right: secondary_x + sc(total_text_width),
                bottom: y + row_height,
            };
            let _ = DrawTextW(
                hdc,
                &mut secondary_wide,
                &mut secondary_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
            SelectObject(hdc, primary_font);
            let _ = DeleteObject(secondary_font);
        }

        SelectObject(hdc, old_font);
        let _ = DeleteObject(primary_font);
    }
}

fn draw_panel(
    hdc: HDC,
    width: i32,
    height: i32,
    border: &Color,
    fill: &Color,
    corner_radius: u8,
) {
    let outer_inset = sc(1).max(1);
    let outer = RECT {
        left: outer_inset,
        top: outer_inset,
        right: width - outer_inset,
        bottom: height - outer_inset,
    };
    let inner_inset = outer_inset + PANEL_BORDER_WIDTH_PX;
    let inner = RECT {
        left: inner_inset,
        top: inner_inset,
        right: width - inner_inset,
        bottom: height - inner_inset,
    };
    unsafe {
        let radius = sc(i32::from(corner_radius))
            .min(((outer.right - outer.left).min(outer.bottom - outer.top) / 2).max(0));
        if radius > 0 {
            draw_rounded_rect(hdc, &outer, border, radius);
            draw_rounded_rect(
                hdc,
                &inner,
                fill,
                (radius - PANEL_BORDER_WIDTH_PX).max(1),
            );
        } else {
            let border_brush = CreateSolidBrush(COLORREF(border.to_colorref()));
            FillRect(hdc, &outer, border_brush);
            let _ = DeleteObject(border_brush);

            let fill_brush = CreateSolidBrush(COLORREF(fill.to_colorref()));
            FillRect(hdc, &inner, fill_brush);
            let _ = DeleteObject(fill_brush);
        }
    }
}

fn draw_drag_handle(hdc: HDC, height: i32, color: &Color, hovered: bool) {
    let (dot, gap_x, gap_y, hover_color) =
        drag_handle_dot_style(*color, hovered, true);
    let matrix_w = dot * 2 + gap_x;
    let matrix_h = dot * 3 + gap_y * 2;
    // Nudge the visible grip away from the rounded left edge without
    // changing the established drag hit target or content layout.
    let hit_center_x = sc(DRAG_HANDLE_HIT_W) / 2;
    let origin_x = hit_center_x + sc(2) - matrix_w / 2;
    let origin_y = (height - matrix_h) / 2;

    for row in 0..3 {
        for col in 0..2 {
            let left = origin_x + col * (dot + gap_x);
            let top = origin_y + row * (dot + gap_y);
            let rect = RECT {
                left,
                top,
                right: left + dot,
                bottom: top + dot,
            };
            draw_rounded_rect(hdc, &rect, &hover_color, sc(1).max(1));
        }
    }
}
fn draw_rounded_rect(hdc: HDC, rect: &RECT, color: &Color, radius: i32) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(color.to_colorref()));
        let rgn = CreateRoundRectRgn(
            rect.left,
            rect.top,
            rect.right + 1,
            rect.bottom + 1,
            radius * 2,
            radius * 2,
        );
        let _ = FillRgn(hdc, rgn, brush);
        let _ = DeleteObject(rgn);
        let _ = DeleteObject(brush);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplied_text_composition_preserves_partial_coverage() {
        let panel = premultiplied_pixel(Color::rgba(240, 240, 240, 128));
        let mixed = composite_premultiplied_text(
            panel,
            Color::rgba(32, 32, 32, 255),
            128,
        );
        let alpha = (mixed >> 24) & 0xFF;
        assert!(alpha > 128 && alpha < 255);
    }

    #[test]
    fn frosted_strength_maps_linearly_to_gaussian_radius() {
        assert_eq!(blur_amount_for_strength(0), 0.0);
        assert!((blur_amount_for_strength(1) - 0.2).abs() < f32::EPSILON);
        assert!((blur_amount_for_strength(10) - 2.0).abs() < f32::EPSILON);
        assert!((blur_amount_for_strength(50) - 10.0).abs() < f32::EPSILON);
        assert!((blur_amount_for_strength(100) - 20.0).abs() < f32::EPSILON);
        assert!((blur_amount_for_strength(255) - 20.0).abs() < f32::EPSILON);
    }

    #[test]
    fn point_in_taskbar_rect_uses_half_open_bounds() {
        let rect = RECT {
            left: 100,
            top: 200,
            right: 500,
            bottom: 260,
        };
        assert!(point_is_in_taskbar_rect(POINT { x: 100, y: 200 }, rect));
        assert!(point_is_in_taskbar_rect(POINT { x: 499, y: 259 }, rect));
        assert!(!point_is_in_taskbar_rect(POINT { x: 500, y: 259 }, rect));
        assert!(!point_is_in_taskbar_rect(POINT { x: 499, y: 260 }, rect));
    }

    #[test]
    fn transient_taskbar_enumeration_gap_keeps_valid_cached_handle_active() {
        let cache = vec![TaskbarLayoutSnapshot {
            taskbar_hwnd_raw: 11,
            controls: vec![RECT {
                left: 100,
                top: 0,
                right: 200,
                bottom: 48,
            }],
        }];
        let mut active = BTreeSet::new();

        preserve_valid_cached_taskbars(&mut active, &cache, |raw| raw == 11);

        assert!(active.contains(&11));
    }

    #[test]
    fn invalid_cached_handle_is_not_kept_active() {
        let cache = vec![TaskbarLayoutSnapshot {
            taskbar_hwnd_raw: 11,
            controls: vec![RECT {
                left: 100,
                top: 0,
                right: 200,
                bottom: 48,
            }],
        }];
        let mut active = BTreeSet::new();

        preserve_valid_cached_taskbars(&mut active, &cache, |_| false);

        assert!(!active.contains(&11));
    }

    #[test]
    fn transient_taskbar_query_failure_keeps_last_known_good_cache() {
        let original = RECT {
            left: 100,
            top: 0,
            right: 200,
            bottom: 48,
        };
        let mut cache = vec![TaskbarLayoutSnapshot {
            taskbar_hwnd_raw: 11,
            controls: vec![original],
        }];
        let active = BTreeSet::from([11isize]);

        let changed = merge_taskbar_layout_cache(&mut cache, &active, Vec::new());

        assert!(!changed);
        assert_eq!(cache.len(), 1);
        assert!(same_control_rects(&cache[0].controls, &[original]));
    }

    #[test]
    fn successful_taskbar_query_replaces_last_known_good_cache() {
        let original = RECT {
            left: 100,
            top: 0,
            right: 200,
            bottom: 48,
        };
        let updated = RECT {
            left: 120,
            top: 0,
            right: 220,
            bottom: 48,
        };
        let mut cache = vec![TaskbarLayoutSnapshot {
            taskbar_hwnd_raw: 11,
            controls: vec![original],
        }];
        let active = BTreeSet::from([11isize]);

        let changed = merge_taskbar_layout_cache(
            &mut cache,
            &active,
            vec![TaskbarLayoutSnapshot {
                taskbar_hwnd_raw: 11,
                controls: vec![updated],
            }],
        );

        assert!(changed);
        assert!(same_control_rects(&cache[0].controls, &[updated]));
    }

    #[test]
    fn disappeared_taskbar_is_removed_from_layout_cache() {
        let mut cache = vec![TaskbarLayoutSnapshot {
            taskbar_hwnd_raw: 11,
            controls: vec![RECT {
                left: 100,
                top: 0,
                right: 200,
                bottom: 48,
            }],
        }];
        let active = BTreeSet::new();

        let changed = merge_taskbar_layout_cache(&mut cache, &active, Vec::new());

        assert!(changed);
        assert!(cache.is_empty());
    }

    #[test]
    fn left_and_right_taskbars_select_vertical_axis() {
        let left = RECT { left: 0, top: 0, right: 48, bottom: 1080 };
        let right = RECT { left: 1920, top: -100, right: 1980, bottom: 980 };
        let horizontal = RECT { left: -1920, top: 1040, right: 0, bottom: 1080 };
        assert_eq!(taskbar_axis(left), TaskbarAxis::Vertical);
        assert_eq!(taskbar_axis(right), TaskbarAxis::Vertical);
        assert_eq!(taskbar_axis(horizontal), TaskbarAxis::Horizontal);
        assert!(vertical_widget_width(left).is_some());
        assert!(vertical_widget_width(right).is_some());
        assert_eq!(vertical_widget_width(RECT { right: 28, ..left }), None);
    }

    #[test]
    fn vertical_taskbar_free_gaps_exclude_pinned_icons_and_tray() {
        let taskbar = RECT { left: 1880, top: -100, right: 1928, bottom: 980 };
        let controls = [
            RECT { left: 1880, top: -100, right: 1928, bottom: 20 },
            RECT { left: 1880, top: 120, right: 1928, bottom: 340 },
            RECT { left: 1880, top: 860, right: 1928, bottom: 980 },
            // A window outside the taskbar's horizontal footprint is not an obstacle.
            RECT { left: 0, top: 400, right: 60, bottom: 500 },
        ];
        let gaps = free_vertical_spans_from_occupied(taskbar, 860, &controls, 4);
        assert_eq!(gaps, vec![
            VerticalSpan { top: 24, bottom: 116 },
            VerticalSpan { top: 344, bottom: 856 },
        ]);
        assert_eq!(nearest_free_top_offset(taskbar, &gaps, 104, -100), Some(444));
        assert_eq!(nearest_free_top_offset(taskbar, &gaps, 104, 900), Some(852));
    }

    #[test]
    fn vertical_ring_has_correct_clockwise_progress_and_clear_center() {
        let width = 48;
        let height = 128;
        let mut pixels = vec![0u32; (width * height) as usize];
        let track = Color::rgba(70, 70, 70, 255);
        let accent = Color::rgba(0, 230, 0, 255);
        render_vertical_ring(
            &mut pixels, width, height, 24, 40, 15.0, 25.0,
            track, accent, true,
        );
        let pixel = |x: i32, y: i32| pixels[(y * width + x) as usize];
        // The bottom opening stays transparent; 25% begins at the lower left
        // and advances clockwise to the left rather than representing usage.
        assert_eq!(pixel(24, 55), 0);
        assert!((pixel(9, 40) & 0x0000FF00) > (pixel(24, 25) & 0x0000FF00));
        assert_eq!(pixel(24, 40), 0);
        assert_ne!(pixel(24, 25), 0);
        assert!(vertical_ring_fraction(0.0).is_some());
        assert_eq!(vertical_ring_fraction(std::f32::consts::PI), None);
        let mut complete_pixels = vec![0u32; (width * height) as usize];
        render_vertical_ring(
            &mut complete_pixels, width, height, 24, 40, 15.0, 100.0,
            track, accent, true,
        );
        assert_eq!(complete_pixels[(55 * width + 24) as usize], 0);
        assert_ne!(complete_pixels[(25 * width + 24) as usize], 0);
        assert_eq!(vertical_quota_text(2.0, true), "2%");
        assert_eq!(vertical_quota_text(100.0, true), "100%");
    }

    #[test]
    fn vertical_layout_keeps_both_reset_lines_inside_compact_height() {
        let height = sc(VERTICAL_WIDGET_HEIGHT);
        let (session, weekly) = vertical_row_tops(height, true, true);
        let first = session.unwrap();
        let second = weekly.unwrap();
        // The handle, text, ring, reset time and next row must be disjoint.
        assert!(first >= sc(VERTICAL_DRAG_HANDLE_H));
        assert!(second - first >= sc(VERTICAL_QUOTA_ROW_HEIGHT + VERTICAL_QUOTA_ROW_GAP));
        let outer_radius = sc(VERTICAL_RING_MAX_RADIUS) + sc(2);
        assert!(sc(VERTICAL_LABEL_CENTER_Y + 9) < sc(VERTICAL_RING_CENTER_Y) - outer_radius);
        assert!(sc(VERTICAL_RING_CENTER_Y) + outer_radius < sc(VERTICAL_RESET_CENTER_Y - 9));
        assert!(second + sc(VERTICAL_RESET_CENTER_Y + 9) <= height - sc(2));
        assert!(vertical_drag_handle_hit(16, sc(8), sc(32), height));
        assert!(!vertical_drag_handle_hit(16, first, sc(32), height));
        assert!(!vertical_drag_handle_hit(16, second, sc(32), height));
        assert!(!vertical_drag_handle_hit(16, sc(18), sc(32), height));
        assert_eq!(vertical_reset_label(None, false), "--");
        assert_eq!(vertical_reset_label(None, true), "--");
    }

    #[test]
    fn desktop_vertical_panel_height_tracks_visible_rows_and_minimal_preset() {
        for preset in [AppearancePreset::Default, AppearancePreset::Minimal] {
            let one = desktop_vertical_content_height(preset, true, false);
            let weekly_only = desktop_vertical_content_height(preset, false, true);
            let both = desktop_vertical_content_height(preset, true, true);
            assert_eq!(one, weekly_only);
            assert!(both > one);
            assert!(both < vertical_widget_height_for_preset(preset));
            let inset = sc(DESKTOP_PANEL_CONTENT_PADDING);
            if preset == AppearancePreset::Minimal {
                for (height, session, weekly) in [
                    (one, true, false), (one, false, true), (both, true, true),
                ] {
                    let (a, b) = vertical_minimal_percent_centers_for_layout(
                        height, session, weekly, true,
                    );
                    for center in [a, b].into_iter().flatten() {
                        assert!(center - sc(12) >= inset - sc(1));
                        assert!(center + sc(12) <= height - inset + sc(1));
                    }
                }
            } else {
                for (height, session, weekly) in [
                    (one, true, false), (one, false, true), (both, true, true),
                ] {
                    let (a, b) = vertical_row_tops_for_layout(
                        height, session, weekly, true,
                    );
                    let first_visible = a.or(b).unwrap();
                    assert_eq!(first_visible, inset,
                        "first visible row must begin at the top panel inset");
                    if let (Some(first), Some(second)) = (a, b) {
                        assert_eq!(second - first,
                            sc(VERTICAL_QUOTA_ROW_HEIGHT + VERTICAL_QUOTA_ROW_GAP));
                    }
                }
                let (_, weekly) = vertical_row_tops_for_layout(both, true, true, true);
                let last_reset_bottom =
                    weekly.unwrap() + sc(VERTICAL_RESET_CENTER_Y + 9);
                assert!(last_reset_bottom <= both - inset + sc(1));
            }
        }
        assert_eq!(
            vertical_widget_height_for_preset(AppearancePreset::Default),
            sc(VERTICAL_WIDGET_HEIGHT),
            "embedded taskbar's legacy height must stay unchanged",
        );
    }

    #[test]
    fn desktop_horizontal_width_tracks_active_usage_text_without_embedded_drag_strip() {
        let lang = LanguageId::English;
        let normal = AppearancePreset::Default;
        let both_resets = desktop_horizontal_content_width(
            lang, normal, true, true, "70%  15:05", "88%  10/25",
        );
        let one_reset = desktop_horizontal_content_width(
            lang, normal, true, false, "70%  15:05", "88%",
        );
        assert_eq!(both_resets, one_reset, "rows share one aligned width");
        assert_eq!(
            both_resets + sc(DRAG_HANDLE_HIT_W)
                - 2 * (sc(DESKTOP_PANEL_CONTENT_PADDING)
                    - sc(normal.metrics().outer_padding)),
            total_widget_width_for_preset(lang, normal),
            "desktop replaces the taskbar drag strip with equal larger content insets",
        );
        let no_reset = desktop_horizontal_content_width(
            lang, normal, true, true, "70%", "88%",
        );
        assert!(no_reset < both_resets, "absent reset labels free their whole column");
        let minimal = desktop_horizontal_content_width(
            lang, AppearancePreset::Minimal, true, true, "70%", "88%",
        );
        assert_eq!(
            minimal + sc(DRAG_HANDLE_HIT_W)
                - 2 * (sc(DESKTOP_PANEL_CONTENT_PADDING)
                    - sc(AppearancePreset::Minimal.metrics().outer_padding)),
            total_widget_width_for_preset(lang, AppearancePreset::Minimal),
        );
        assert_eq!(horizontal_content_left(&normal.metrics(), true),
            sc(DESKTOP_PANEL_CONTENT_PADDING));
        assert_eq!(horizontal_content_left(&normal.metrics(), false),
            sc(normal.metrics().outer_padding + DRAG_HANDLE_HIT_W));
    }

    #[test]
    fn desktop_visual_ink_alignment_avoids_reserved_reset_column_whitespace() {
        let width = 34;
        let height = 12;
        let mut mask = vec![0u8; (width * height) as usize];
        // Mimic a 5-character time mask with 4 pixels of leading bearing
        // and trailing blank columns from the fixed-width text rectangle.
        for y in 2..10 {
            for x in 4..27 {
                mask[(y * width + x) as usize] = 255;
            }
        }
        assert_eq!(directwrite_mask_x_offset(
            &mask, width, height, HorizontalTextAlign::Start,
        ), 0, "embedded taskbar keeps its original layout");
        let shift_start = directwrite_mask_x_offset(
            &mask, width, height, HorizontalTextAlign::InkStart,
        );
        assert_eq!(4 + shift_start, 0,
            "desktop first visible glyph must align to the left content inset");
        let shift_end = directwrite_mask_x_offset(
            &mask, width, height, HorizontalTextAlign::End,
        );
        assert_eq!(26 + shift_end, width - 1,
            "reset text must reach the content inset, not stop at its reserved width");
        let shift_center = directwrite_mask_x_offset(
            &mask, width, height, HorizontalTextAlign::Center,
        );
        assert!(((4 + shift_center) - (width - 1 - (26 + shift_center))).abs() <= 1,
            "the minimal single value must be optically centered");
        assert_eq!(directwrite_mask_x_offset(
            &vec![0; (width * height) as usize],
            width, height, HorizontalTextAlign::End,
        ), 0);
    }

    #[test]
    fn desktop_horizontal_and_vertical_content_margins_are_symmetric() {
        let preset = AppearancePreset::Default;
        let metrics = preset.metrics();
        let left = horizontal_content_left(&metrics, true);
        let width = desktop_horizontal_content_width(
            LanguageId::English, preset, true, true, "74%  15:05", "88%  10/25",
        );
        let label = sc(metrics.label_width);
        let bar = row_bar_segment_count(preset)
            * (sc(SEGMENT_W) + sc(SEGMENT_GAP)) - sc(SEGMENT_GAP);
        let rightmost_text_edge = left + label + sc(metrics.label_bar_gap)
            + bar + sc(metrics.bar_percent_gap)
            + sc(metrics.percent_width) + sc(metrics.percent_reset_gap)
            + sc(metrics.reset_width);
        assert_eq!(left, sc(DESKTOP_PANEL_CONTENT_PADDING));
        assert_eq!(width - rightmost_text_edge, left,
            "left and right gutters must match around text ink");
        for p in [AppearancePreset::Default, AppearancePreset::Minimal] {
            for (show_session, show_weekly) in [(true, true), (true, false), (false, true)] {
                let h = desktop_vertical_content_height(p, show_session, show_weekly);
                let row_height = if p == AppearancePreset::Minimal {
                    sc(DESKTOP_MINIMAL_ROW_HEIGHT)
                } else {
                    sc(VERTICAL_QUOTA_ROW_HEIGHT)
                };
                let (first, last) = if p == AppearancePreset::Minimal {
                    let (a, b) = vertical_minimal_percent_centers_for_layout(
                        h, show_session, show_weekly, true,
                    );
                    let a = a.or(b).unwrap() - row_height / 2;
                    let b = b.or(Some(a + row_height / 2)).unwrap() + row_height / 2;
                    (a, b)
                } else {
                    let (a, b) = vertical_row_tops_for_layout(
                        h, show_session, show_weekly, true,
                    );
                    let first = a.or(b).unwrap();
                    let last = b.or(a).unwrap() + row_height;
                    (first, last)
                };
                assert!((first - (h - last)).abs() <= 1,
                    "vertical visible row envelope must have equal top/bottom padding");
            }
        }
        assert_eq!(horizontal_content_left(&metrics, false),
            sc(metrics.outer_padding + DRAG_HANDLE_HIT_W),
            "taskbar mode must preserve its original leading drag strip");
    }

    #[test]
    fn desktop_responsive_tooltip_uses_content_relative_coordinates() {
        let width = desktop_horizontal_content_width(
            LanguageId::English, AppearancePreset::Minimal,
            true, false, "70%", "88%",
        );
        let height = sc(AppearancePreset::Minimal.metrics().widget_height);
        let hit = desktop_horizontal_usage_hover_hit(
            width, height, AppearancePreset::Minimal, true, false,
            sc(10), height / 2,
        );
        assert_eq!(hit.map(|(target, _)| target), Some(MinimalHoverTarget::Session));
        assert_eq!(desktop_horizontal_usage_hover_hit(
            width, height, AppearancePreset::Minimal, true, false,
            width + 1, height / 2,
        ), None);
    }

    #[test]
    fn v24_3_drag_release_preserves_hover_on_detached_tab_without_mouse_motion() {
        let edges = [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
            Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)];
        for (body_w, body_h) in [(240, 42), (48, 168)] {
            for edge in edges {
                let (width, height, body) =
                    floating_canvas_geometry(body_w, body_h, edge);
                let tab = floating_grip_rect(width, height, edge, sc(FLOAT_EDGE_PEEK));
                let origin = POINT { x: -500, y: 320 };
                let pointer = POINT {
                    x: origin.x + (tab.left + tab.right) / 2,
                    y: origin.y + (tab.top + tab.bottom) / 2,
                };
                assert!(floating_drag_grip_contains_screen_point(
                    origin.x, origin.y, width, height, edge, pointer,
                ));
                // The grip is independent of the body. The previous
                // body-based check cleared hover despite a stationary cursor.
                assert!(!point_in_rect(body,
                    pointer.x - origin.x, pointer.y - origin.y));
                let outside = POINT { x: origin.x + body.left + 2,
                    y: origin.y + body.top + body_h / 2 };
                assert!(!floating_drag_grip_contains_screen_point(
                    origin.x, origin.y, width, height, edge, outside,
                ));
                assert!(!floating_drag_grip_contains_screen_point(
                    origin.x, origin.y, width, height, edge,
                    POINT { x: origin.x - 3, y: origin.y - 3 },
                ));
            }
        }
    }

    #[test]
    fn v24_3_release_grip_hover_and_glow_use_the_same_frame() {
        let style = ThemeStyle::preset(true, ThemePreset::Ocean);
        let (width, height, body) = floating_canvas_geometry(240, 42, None);
        let tab = floating_grip_rect(width, height, None, sc(FLOAT_EDGE_PEEK));
        let pointer = POINT { x: 420 + (tab.left + tab.right) / 2,
            y: 180 + (tab.top + tab.bottom) / 2 };
        let hovered = floating_drag_grip_contains_screen_point(
            420, 180, width, height, None, pointer,
        );
        assert!(hovered);
        let alpha = if hovered { 100 } else { 0 };
        assert_eq!(alpha, 100, "tab must not fade on mouse release");
        let mut canvas = vec![0u32; (width * height) as usize];
        render_floating_handle_bitmap(
            &mut canvas, width, height, None, hovered, &style, alpha,
        );
        let grip_pixel = ((tab.top + tab.bottom) / 2 * width
            + (tab.left + tab.right) / 2) as usize;
        assert_ne!(canvas[grip_pixel], 0, "tab remains visible");
        let accent = drag_hover_outline_color(&style);
        draw_drag_hover_outline(
            &mut canvas, width, height, body, style.panel_corner_radius, accent,
        );
        let ring_pixel = ((body.top + 1) * width + (body.left + body.right) / 2)
            as usize;
        assert_ne!(canvas[ring_pixel], 0, "body hover glow stays visible");
    }

    #[test]
    fn v24_2_free_desktop_canvas_has_real_outer_halo_space() {
        for (width, height) in [(240, 42), (48, 168)] {
            let halo = sc(FLOAT_FREE_GLOW_MARGIN);
            let (cw, ch, body) = floating_canvas_geometry(width, height, None);
            let tab = floating_grip_rect(cw, ch, None, sc(FLOAT_EDGE_PEEK));
            assert_eq!(cw, width + halo * 2);
            assert_eq!(ch, height + sc(FLOAT_EDGE_PEEK) + halo * 2);
            assert_eq!(body.left, halo);
            assert_eq!(body.top, halo + sc(FLOAT_EDGE_PEEK));
            assert_eq!(cw - body.right, halo);
            assert_eq!(ch - body.bottom, halo);
            assert_eq!(tab.top, halo);
            assert_eq!(tab.bottom, body.top);
            assert!(floating_drag_handle_hit(
                (tab.left + tab.right) / 2, (tab.top + tab.bottom) / 2,
                cw, ch, None, sc(FLOAT_EDGE_PEEK),
            ));
            assert!(!floating_drag_handle_hit(
                0, tab.top, cw, ch, None, sc(FLOAT_EDGE_PEEK),
            ));
            let style = ThemeStyle::preset(true, ThemePreset::Forest);
            let mut bitmap = vec![0u32; (cw * ch) as usize];
            draw_drag_hover_outline(&mut bitmap, cw, ch, body,
                style.panel_corner_radius, drag_hover_outline_color(&style));
            // A genuine halo pixel exists left of the panel, and fades
            // completely before the outermost transparent canvas boundary.
            let y = (body.top + body.bottom) / 2;
            let alpha_at = |x: i32| bitmap[(y * cw + x) as usize] >> 24;
            assert!(alpha_at(body.left - 2) > alpha_at(body.left - 6));
            assert!(alpha_at(body.left - 6) > alpha_at(body.left - 10));
            assert_eq!(alpha_at(0), 0);
        }
        // Docked tab geometry remains bit-for-bit the V24 layout.
        let (w, h, dock) = floating_canvas_geometry(
            240, 42, Some(FloatingEdge::Bottom),
        );
        assert_eq!((w, h, dock.left, dock.top), (240, 42 + sc(FLOAT_EDGE_PEEK), 0,
            sc(FLOAT_EDGE_PEEK)));
    }

    #[test]
    fn desktop_repeated_direction_switch_preserves_panel_and_external_grip() {
        // Reproduces the user's rapid horizontal/vertical toggles at the pure
        // geometry layer. Every switch must preserve the panel and its
        // external drag area without accumulating size or offset errors.
        let horizontal = (240, 42);
        let vertical = (48, 184);
        let edges = [
            None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
            Some(FloatingEdge::Top), Some(FloatingEdge::Bottom),
        ];
        for i in 0..1_000 {
            let (body_w, body_h) = if i % 2 == 0 { horizontal } else { vertical };
            for edge in edges {
                let (canvas_w, canvas_h, body) =
                    floating_canvas_geometry(body_w, body_h, edge);
                let grip = floating_grip_rect(
                    canvas_w, canvas_h, edge, sc(FLOAT_EDGE_PEEK),
                );
                assert_eq!(body.right - body.left, body_w);
                assert_eq!(body.bottom - body.top, body_h);
                assert!(grip.left >= 0 && grip.top >= 0
                    && grip.right <= canvas_w && grip.bottom <= canvas_h);
                assert!(grip.right <= body.left || grip.left >= body.right
                    || grip.bottom <= body.top || grip.top >= body.bottom);
            }
        }
    }

    #[test]
    fn desktop_grip_stays_inside_only_visible_edge_peek() {
        let width = 240;
        let height = 168;
        let peek = sc(FLOAT_EDGE_PEEK);
        for edge in [Some(FloatingEdge::Left), Some(FloatingEdge::Right),
                     Some(FloatingEdge::Top), Some(FloatingEdge::Bottom), None] {
            let r = floating_grip_rect(width, height, edge, peek);
            assert!(r.left >= 0 && r.top >= 0 && r.right <= width && r.bottom <= height);
            assert!(floating_drag_handle_hit((r.left + r.right) / 2,
                (r.top + r.bottom) / 2, width, height, edge, peek));
            assert!(!floating_drag_handle_hit(-1, -1, width, height, edge, peek));
            match edge {
                Some(FloatingEdge::Left) => assert_eq!(r.left, width - peek),
                Some(FloatingEdge::Right) => assert_eq!(r.right, peek),
                Some(FloatingEdge::Top) => assert_eq!(r.bottom, height),
                Some(FloatingEdge::Bottom) => assert_eq!(r.top, 0),
                None => assert_eq!(r.top, sc(FLOAT_FREE_GLOW_MARGIN)),
            }
        }
    }

    #[test]
    fn desktop_drag_grip_geometry_stays_fixed_until_release() {
        let body_w = 240;
        let body_h = 168;
        for edge in [FloatingEdge::Left, FloatingEdge::Right,
                     FloatingEdge::Top, FloatingEdge::Bottom] {
            let (w, h, body) = floating_canvas_geometry(body_w, body_h, Some(edge));
            let original_tab = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
            let anchor_x = (original_tab.left + original_tab.right) / 2;
            let anchor_y = (original_tab.top + original_tab.bottom) / 2;
            for pointer in [POINT { x: 300, y: 400 }, POINT { x: 700, y: 250 }] {
                let window_x = pointer.x - anchor_x;
                let window_y = pointer.y - anchor_y;
                let tab = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
                assert_eq!((window_x + tab.left + (tab.right - tab.left) / 2,
                            window_y + tab.top + (tab.bottom - tab.top) / 2),
                            (pointer.x, pointer.y));
                assert_eq!((body.right - body.left, body.bottom - body.top),
                           (body_w, body_h));
            }
        }
    }

    #[test]
    fn desktop_grip_fade_is_reversible_and_never_changes_body_pixels() {
        assert_eq!(eased_value_at(0, 100, Duration::ZERO, 140), 0);
        assert_eq!(eased_value_at(0, 100, Duration::from_millis(70), 140), 88);
        assert_eq!(eased_value_at(0, 100, Duration::from_millis(140), 140), 100);
        assert_eq!(eased_value_at(100, 0, Duration::ZERO, 170), 100);
        assert_eq!(eased_value_at(100, 0, Duration::from_millis(85), 170), 50);
        assert_eq!(eased_value_at(100, 0, Duration::from_millis(170), 170), 0);
        let width = 240;
        let height = 168;
        let style = ThemeStyle::light_default();
        let grip = floating_grip_rect(width, height, None, sc(FLOAT_EDGE_PEEK));
        let mut body = vec![0xFF445566u32; (width * height) as usize];
        for y in grip.top..grip.bottom {
            for x in grip.left..grip.right {
                body[(y * width + x) as usize] = 0; // detached tab gutter
            }
        }
        let mut hidden = body.clone();
        let mut visible = body.clone();
        render_floating_handle_bitmap(
            &mut hidden, width, height, None, false, &style, 0,
        );
        render_floating_handle_bitmap(
            &mut visible, width, height, None, false, &style, 100,
        );
        for y in 0..height {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                if !point_in_rect(grip, x, y) {
                    assert_eq!(hidden[idx], body[idx]);
                    assert_eq!(visible[idx], body[idx]);
                } else {
                    assert_eq!((hidden[idx] >> 24) & 0xFF, 0,
                        "fully transparent free grip must not intercept clicks");
                }
            }
        }
    }

    #[test]
    fn v24_drag_grip_dots_remain_stable_on_hover() {
        let base = Color::rgba(40, 70, 110, 160);
        for vertical in [false, true] {
            assert_eq!(
                drag_handle_dot_style(base, false, vertical),
                drag_handle_dot_style(base, true, vertical),
                "hover must no longer grow the grip under the mouse",
            );
        }
    }

    #[test]
    fn v24_hover_color_follows_theme_and_stays_readable() {
        for dark in [false, true] {
            for preset in ThemePreset::ALL {
                let style = ThemeStyle::preset(dark, preset);
                let color = drag_hover_outline_color(&style);
                assert_eq!(color.a, 255);
                let background = style.color(StyleColorTarget::PanelBackground);
                let difference = (i32::from(color.r) - i32::from(background.r)).abs()
                    + (i32::from(color.g) - i32::from(background.g)).abs()
                    + (i32::from(color.b) - i32::from(background.b)).abs();
                assert!(difference >= 150, "theme hover outline must remain visible");
                assert_eq!(color, Color::rgba(
                    style.color(StyleColorTarget::ProgressHigh).r,
                    style.color(StyleColorTarget::ProgressHigh).g,
                    style.color(StyleColorTarget::ProgressHigh).b,
                    255,
                ));
            }
        }
        let mut custom = ThemeStyle::dark_default();
        custom.set_color(StyleColorTarget::ProgressHigh,
            custom.color(StyleColorTarget::PanelBackground));
        assert_ne!(drag_hover_outline_color(&custom),
            custom.color(StyleColorTarget::ProgressHigh));
    }

    #[test]
    fn v24_1_glow_is_feathered_not_a_thick_solid_outline() {
        assert!(drag_hover_glow_coverage(0.0) > drag_hover_glow_coverage(1.0));
        assert!(drag_hover_glow_coverage(1.0) > drag_hover_glow_coverage(2.0));
        assert!(drag_hover_glow_coverage(2.0) > drag_hover_glow_coverage(3.0));
        assert!(drag_hover_glow_coverage(3.0) > drag_hover_glow_coverage(4.0));
        assert_eq!(drag_hover_glow_coverage(0.0), drag_hover_glow_coverage(-0.0));
        assert_eq!(drag_hover_glow_coverage(2.0), drag_hover_glow_coverage(-2.0));
        let style = ThemeStyle::preset(true, ThemePreset::Forest);
        let mut pixels = vec![0u32; 190 * 62];
        let body = RECT { left: 7, top: 7, right: 183, bottom: 55 };
        draw_drag_hover_outline(&mut pixels, 190, 62, body,
            style.panel_corner_radius, drag_hover_outline_color(&style));
        let sample_alpha = |y: usize| -> u32 { pixels[y * 190 + 95] >> 24 };
        // There is real translucent glow outside a panel when canvas space
        // exists, but it never paints the center or expands the body geometry.
        assert!(sample_alpha(6) > 0, "halo should extend beyond outer body edge");
        assert!(sample_alpha(6) < sample_alpha(8));
        assert!(sample_alpha(8) > sample_alpha(10));
        assert!(sample_alpha(10) > sample_alpha(12));
        assert_eq!(sample_alpha(20), 0);
        assert_eq!(pixels.len(), 190 * 62);
    }

    #[test]
    fn v24_hover_outlines_only_panel_and_detached_tab_without_resizing() {
        let style = ThemeStyle::preset(true, ThemePreset::Forest);
        let accent = drag_hover_outline_color(&style);
        for (w, h) in [(180, 40), (50, 168)] {
            let bounds = RECT { left: 0, top: 0, right: w, bottom: h };
            let mut pixels = vec![0xFF14211Du32; (w * h) as usize];
            let idle = pixels.clone();
            draw_drag_hover_outline(&mut pixels, w, h, bounds,
                style.panel_corner_radius, accent);
            assert_ne!(pixels, idle);
            assert_eq!(pixels[(h / 2 * w + w / 2) as usize],
                idle[(h / 2 * w + w / 2) as usize],
                "center content must never be covered by the hover ring");
            assert_ne!(pixels[(w + w / 2) as usize],
                idle[(w + w / 2) as usize], "top border must highlight");
        }
        for edge in [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
            Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)] {
            let (cw, ch, body) = floating_canvas_geometry(180, 40, edge);
            let tab = floating_grip_rect(cw, ch, edge, sc(FLOAT_EDGE_PEEK));
            let mut canvas = vec![0u32; (cw * ch) as usize];
            draw_drag_hover_outline(&mut canvas, cw, ch, body,
                style.panel_corner_radius, accent);
            let after_body = canvas.clone();
            draw_drag_hover_outline(&mut canvas, cw, ch, tab,
                style.panel_corner_radius, accent);
            assert_ne!(canvas, after_body, "desktop tab must highlight independently");
            assert_eq!(canvas.len(), (cw * ch) as usize);
            let body_center = ((body.top + body.bottom) / 2 * cw
                + (body.left + body.right) / 2) as usize;
            assert_eq!(canvas[body_center], 0,
                "independent hover borders must not fill the actual content");
        }
    }

    #[test]
    fn desktop_mini_grip_matches_panel_palette_corner_radius_and_alpha() {
        let mut style = ThemeStyle::dark_default();
        style.set_color(StyleColorTarget::PanelBackground, Color::rgba(14, 32, 25, 68));
        style.set_color(StyleColorTarget::PanelBorder, Color::rgba(40, 110, 87, 127));
        style.panel_corner_radius = 2;
        let (w, h, _) = floating_canvas_geometry(
            sc(FLOAT_GRIP_LENGTH), sc(FLOAT_EDGE_PEEK), None,
        );
        let grip = floating_grip_rect(w, h, None, sc(FLOAT_EDGE_PEEK));
        assert_eq!(grip.right - grip.left, sc(36));
        assert_eq!(grip.bottom - grip.top, sc(16));
        let mut pixels = vec![0u32; (w * h) as usize];
        render_floating_handle_bitmap(&mut pixels, w, h, None, false, &style, 100);
        let center = ((grip.top + grip.bottom) / 2 * w + grip.left + sc(6)) as usize;
        assert_eq!(pixels[center], premultiplied_pixel(
            style.color(StyleColorTarget::PanelBackground),
        ));
        // Panel corner clipping and the new outer halo gutter stay transparent.
        assert_eq!(pixels[0], 0);
        let mut transparent = vec![0u32; (w * h) as usize];
        render_floating_handle_bitmap(&mut transparent, w, h, None, false, &style, 0);
        assert!(transparent.iter().all(|&pixel| pixel == 0));
    }

    #[test]
    fn desktop_edge_slide_easing_is_time_based_and_directional() {
        assert_eq!(eased_value_at(0, 100, Duration::ZERO, 220), 0);
        assert_eq!(eased_value_at(0, 100, Duration::from_millis(110), 220), 88);
        assert_eq!(eased_value_at(100, 0, Duration::from_millis(120), 240), 50);
        assert_eq!(eased_value_at(0, 100, Duration::from_millis(220), 220), 100);
        assert_eq!(eased_value_at(100, 0, Duration::from_millis(240), 240), 0);
        assert_eq!(animation_partial_duration(240, 50, 0), 120);
        assert_eq!(animation_partial_duration(240, 100, 0), 240);
    }

    #[test]
    fn desktop_first_snap_retraction_ignores_hover_until_rearmed() {
        assert!(!floating_hover_can_reveal(true, true, false));
        assert!(!floating_hover_can_reveal(true, false, true));
        assert!(!floating_hover_can_reveal(false, false, false));
        assert!(floating_hover_can_reveal(true, false, false));
        // The first hidden tab remains visible, but the cursor crossing its
        // path during collapse cannot reverse the animation.
        assert_eq!(eased_value_at(
            100, 0, Duration::from_millis(FLOAT_RETRACT_DURATION_MS / 2),
            FLOAT_RETRACT_DURATION_MS,
        ), 50);
        assert_eq!(eased_value_at(
            100, 0, Duration::from_millis(FLOAT_RETRACT_DURATION_MS),
            FLOAT_RETRACT_DURATION_MS,
        ), 0);
    }

    #[test]
    fn desktop_bottom_docks_at_monitor_edge_not_above_taskbar() {
        let monitor = RECT { left: -1920, top: -100, right: 0, bottom: 980 };
        let work = RECT { left: -1920, top: -100, right: 0, bottom: 930 };
        let bounds = floating_usable_bounds(monitor, work);
        assert_eq!(bounds.bottom, monitor.bottom);
        let (cw, ch, _) = floating_canvas_geometry(240, 168, Some(FloatingEdge::Bottom));
        let grip = floating_grip_rect(
            cw, ch, Some(FloatingEdge::Bottom), sc(FLOAT_EDGE_PEEK),
        );
        let (_, y) = floating_edge_position(
            bounds, -600, 600, cw, ch, Some(FloatingEdge::Bottom),
            true, sc(FLOAT_EDGE_PEEK),
        );
        assert_eq!(y + grip.bottom, monitor.bottom);
        assert_ne!(y + grip.bottom, work.bottom);
    }

    #[test]
    fn desktop_connected_monitors_dont_snap_to_shared_seam() {
        let left_monitor = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
        let right_monitor = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
        let crossing_from_left = RECT {
            left: -180, top: 460, right: 18, bottom: 550,
        };
        let crossing_from_right = RECT {
            left: -10, top: 460, right: 210, bottom: 550,
        };
        assert_eq!(
            floating_drop_edge(left_monitor, crossing_from_left, 24, [true, false, true, true]),
            None,
        );
        assert_eq!(
            floating_drop_edge(right_monitor, crossing_from_right, 24, [false, true, true, true]),
            None,
        );
        // At an outer boundary of the same layouts, genuine dockability stays.
        assert_eq!(
            floating_drop_edge(left_monitor,
                RECT { left: -1930, top: 460, right: -1730, bottom: 550 },
                24, [true, false, true, true]),
            Some(FloatingEdge::Left),
        );
        // If two displays meet only over part of a side, a free segment can
        // still dock when its full grip isn't covered by the next screen.
        assert!(right_monitor.bottom > left_monitor.bottom);
    }

    #[test]
    fn desktop_left_taskbar_dock_keeps_visible_tab_outside_explorer() {
        let monitor = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
        let work = RECT { left: -1872, top: 0, right: 0, bottom: 1040 };
        let bounds = floating_usable_bounds(monitor, work);
        assert_eq!(bounds.left, -1872);
        let (width, height, _) = floating_canvas_geometry(
            240, 168, Some(FloatingEdge::Left),
        );
        let (x, y) = floating_edge_position(
            bounds, -1860, 140, width, height,
            Some(FloatingEdge::Left), true, sc(FLOAT_EDGE_PEEK),
        );
        let grip = floating_grip_rect(
            width, height, Some(FloatingEdge::Left), sc(FLOAT_EDGE_PEEK),
        );
        assert_eq!(x + grip.left, work.left);
        assert!(x + grip.right <= work.right);
        assert!(y + grip.top >= work.top && y + grip.bottom <= work.bottom);

        let invalid = RECT { left: -3000, top: 0, right: 10, bottom: 1080 };
        assert_eq!(floating_usable_bounds(monitor, invalid).left, monitor.left);
    }

    #[test]
    fn desktop_release_tab_switch_preserves_body_screen_origin() {
        let body_w = 240;
        let body_h = 150;
        let (old_w, old_h, old_body) =
            floating_canvas_geometry(body_w, body_h, Some(FloatingEdge::Left));
        let _ = (old_w, old_h);
        let (new_w, new_h, new_body) =
            floating_canvas_geometry(body_w, body_h, None);
        let _ = (new_w, new_h);
        let old_x = 500;
        let old_y = 250;
        let final_x = old_x + old_body.left - new_body.left;
        let final_y = old_y + old_body.top - new_body.top;
        assert_eq!(final_x + new_body.left, old_x + old_body.left);
        assert_eq!(final_y + new_body.top, old_y + old_body.top);
    }

    #[test]
    fn desktop_drag_drop_recognizes_partially_offscreen_old_canvas() {
        let bounds = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        let still_peeking = RECT { left: -200, top: 300, right: 20, bottom: 400 };
        assert_eq!(floating_drop_edge(bounds, still_peeking, 24, [true; 4]), Some(FloatingEdge::Left));
        let free = RECT { left: 640, top: 300, right: 860, bottom: 400 };
        assert_eq!(floating_drop_edge(bounds, free, 24, [true; 4]), None);
    }

    #[test]
    fn desktop_tab_is_outside_taskbar_sized_panel() {
        let body_w = 240;
        let body_h = 168;
        for edge in [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
                     Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)] {
            let (width, height, body) = floating_canvas_geometry(body_w, body_h, edge);
            let tab = floating_grip_rect(width, height, edge, sc(FLOAT_EDGE_PEEK));
            assert_eq!(body.right - body.left, body_w);
            assert_eq!(body.bottom - body.top, body_h);
            assert!(tab.right <= body.left || tab.left >= body.right
                || tab.bottom <= body.top || tab.top >= body.bottom);
            assert!(tab.left >= 0 && tab.top >= 0
                && tab.right <= width && tab.bottom <= height);
        }
    }

    #[test]
    fn desktop_collapsed_bitmap_only_keeps_grip_pixels_visible() {
        let w = 240;
        let h = 168;
        let style = ThemeStyle::dark_default();
        for edge in [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
                     Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)] {
            let mut pixels = vec![0u32; (w * h) as usize];
            render_floating_handle_bitmap(&mut pixels, w, h, edge, false, &style, 100);
            let r = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
            for y in 0..h {
                for x in 0..w {
                    let pixel = pixels[(y * w + x) as usize];
                    if !point_in_rect(r, x, y) { assert_eq!(pixel, 0); }
                }
            }
            assert_ne!(pixels[(((r.top + r.bottom) / 2) * w +
                (r.left + r.right) / 2) as usize], 0);
        }
    }

    #[test]
    fn desktop_horizontal_and_vertical_direction_round_trip() {
        let settings = SettingsFile {
            display_position: DisplayPosition::Desktop,
            desktop_direction: DesktopDirection::Vertical,
            ..Default::default()
        };
        let encoded = serde_json::to_string(&settings).unwrap();
        let parsed: SettingsFile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(parsed.desktop_direction, DesktopDirection::Vertical);
        assert_eq!(parsed.display_position, DisplayPosition::Desktop);
        let legacy: SettingsFile = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.desktop_direction, DesktopDirection::Horizontal);
        assert_eq!(legacy.display_position, DisplayPosition::Taskbar);
    }

    #[test]
    fn vertical_adaptive_uses_compact_before_desktop_fallback() {
        let bar = RECT { left: 1800, top: 0, right: 1850, bottom: 800 };
        let ample = [VerticalSpan { top: 20, bottom: 280 }];
        assert_eq!(vertical_placement_for_gaps(AppearancePreset::Adaptive, bar, &ample, 24),
            Some((AppearancePreset::Default, 24)));
        let short = [VerticalSpan { top: 20, bottom: 130 }];
        assert_eq!(vertical_placement_for_gaps(AppearancePreset::Adaptive, bar, &short, 24),
            Some((AppearancePreset::Minimal, 24)));
        assert_eq!(vertical_placement_for_gaps(AppearancePreset::Default, bar, &short, 24),
            Some((AppearancePreset::Minimal, 24)));
        let exhausted = [VerticalSpan { top: 20, bottom: 65 }];
        assert_eq!(vertical_placement_for_gaps(AppearancePreset::Adaptive, bar, &exhausted, 24), None);
        assert_eq!(vertical_placement_for_gaps(AppearancePreset::Minimal, bar, &short, 24),
            Some((AppearancePreset::Minimal, 24)));
    }

    #[test]
    fn floating_widget_snaps_to_edges_and_preserves_a_visible_peek() {
        let bounds = RECT { left: -1920, top: -80, right: 0, bottom: 1000 };
        let (x, y) = floating_clamp_position(bounds, 48, 168, -3000, 1200);
        assert_eq!((x, y), (-1920, 832));
        assert_eq!(floating_nearest_edge(bounds,
            RECT { left: -1910, top: 200, right: -1862, bottom: 368 }, 24),
            Some(FloatingEdge::Left));
        assert_eq!(floating_nearest_edge(bounds,
            RECT { left: -1100, top: 200, right: -1052, bottom: 368 }, 24), None);
        assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
            Some(FloatingEdge::Left), true, 15), (-1953, 100));
        assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
            Some(FloatingEdge::Left), false, 15), (-1920, 100));
        assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
            Some(FloatingEdge::Right), true, 15), (-15, 100));
        assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
            Some(FloatingEdge::Top), true, 15), (-1500, -233));
        assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
            Some(FloatingEdge::Bottom), true, 15), (-1500, 985));
        assert_eq!(FloatingEdge::parse("right"), Some(FloatingEdge::Right));
        assert_eq!(FloatingEdge::parse("unknown"), None);
    }

    #[test]
    fn vertical_minimal_layout_shows_two_separate_remaining_percentages() {
        let height = vertical_widget_height_for_preset(AppearancePreset::Minimal);
        assert_eq!(height, sc(VERTICAL_MINIMAL_WIDGET_HEIGHT));
        assert_eq!(vertical_widget_height_for_preset(AppearancePreset::Default), sc(VERTICAL_WIDGET_HEIGHT));
        let (session, weekly) = vertical_minimal_percent_centers(height, true, true);
        let session = session.unwrap();
        let weekly = weekly.unwrap();
        assert!(session - sc(12) >= sc(VERTICAL_DRAG_HANDLE_H));
        assert!(weekly - session >= sc(24));
        assert!(weekly + sc(12) < height);
        assert_eq!(vertical_minimal_percent_centers(height, true, false).1, None);
        assert_eq!(vertical_minimal_percent_centers(height, false, true).0, None);
        // Stored percentages are USED; display is always REMAINING.
        assert_eq!(usage_percent_for_display(LanguageId::English, 88.0), 12.0);
        assert_eq!(usage_percent_for_display(LanguageId::English, 63.0), 37.0);
        assert_eq!(vertical_quota_text(12.0, true), "12%");
        assert_eq!(vertical_quota_text(37.0, true), "37%");
    }

    #[test]
    fn vertical_default_tooltip_hits_only_reset_timestamp() {
        let width = sc(48);
        let height = sc(VERTICAL_WIDGET_HEIGHT);
        let (session, weekly) = vertical_row_tops(height, true, true);
        let first = session.unwrap();
        let second = weekly.unwrap();
        // Label, ring, center percentage and empty space do not trigger it.
        for y in [first + sc(VERTICAL_LABEL_CENTER_Y),
                  first + sc(VERTICAL_RING_CENTER_Y), second + sc(VERTICAL_RING_CENTER_Y)] {
            assert!(vertical_usage_hover_hit(width, height, false, true, true, width / 2, y).is_none());
        }
        assert_eq!(vertical_usage_hover_hit(
            width, height, false, true, true,
            width / 2, first + sc(VERTICAL_RESET_CENTER_Y),
        ).unwrap().0, MinimalHoverTarget::Session);
        assert_eq!(vertical_usage_hover_hit(
            width, height, false, true, true,
            width / 2, second + sc(VERTICAL_RESET_CENTER_Y),
        ).unwrap().0, MinimalHoverTarget::Weekly);
        assert!(vertical_usage_hover_hit(width, height, false, true, true, sc(1),
            first + sc(VERTICAL_RESET_CENTER_Y)).is_none());
    }

    #[test]
    fn vertical_minimal_tooltip_hits_percentages_not_drag_grip() {
        let width = sc(48);
        let height = sc(VERTICAL_MINIMAL_WIDGET_HEIGHT);
        let (session, weekly) = vertical_minimal_percent_centers(height, true, true);
        assert!(vertical_usage_hover_hit(width, height, true, true, true, width / 2, sc(8)).is_none());
        assert_eq!(vertical_usage_hover_hit(width, height, true, true, true,
            width / 2, session.unwrap()).unwrap().0, MinimalHoverTarget::Session);
        assert_eq!(vertical_usage_hover_hit(width, height, true, true, true,
            width / 2, weekly.unwrap()).unwrap().0, MinimalHoverTarget::Weekly);
        assert!(vertical_usage_hover_hit(width, height, true, true, true,
            sc(1), session.unwrap()).is_none());
    }

    #[test]
    fn vertical_drag_tracks_y_outside_taskbar_without_leaving_parent() {
        let taskbar = RECT { left: 1900, top: -200, right: 1950, bottom: 800 };
        let anchor = 12;
        let height = 104;
        // Keep ordinary tracking when cursor is inside the vertical taskbar.
        assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1920, y: 240 }, anchor, height), 428);
        // Outside left/right keeps using Y and clamps to the current taskbar.
        assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1800, y: 240 }, anchor, height), 428);
        assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1990, y: -500 }, anchor, height), 0);
        assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1990, y: 1000 }, anchor, height), 896);
    }

    #[test]
    fn taskbar_top_offset_remains_compatible_with_legacy_settings() {
        let old = r#"{"taskbar_index":0,"taskbar_left_offset":123}"#;
        let parsed: SettingsFile = serde_json::from_str(old).unwrap();
        assert_eq!(parsed.taskbar_left_offset, Some(123));
        assert_eq!(parsed.taskbar_top_offset, None);
        let mut new = parsed;
        new.taskbar_top_offset = Some(415);
        let serialized = serde_json::to_string(&new).unwrap();
        let decoded: SettingsFile = serde_json::from_str(&serialized).unwrap();
        assert_eq!(decoded.taskbar_top_offset, Some(415));
        assert_eq!(decoded.taskbar_left_offset, Some(123));
    }

    #[test]
    fn free_taskbar_spans_exclude_occupied_controls_and_tray() {
        let taskbar = RECT {
            left: 0,
            top: 100,
            right: 1000,
            bottom: 148,
        };
        let occupied = [
            RECT {
                left: 400,
                top: 104,
                right: 520,
                bottom: 144,
            },
            RECT {
                left: 560,
                top: 104,
                right: 680,
                bottom: 144,
            },
        ];
        let free = free_spans_from_occupied(taskbar, 900, &occupied, 4);
        assert_eq!(
            free,
            vec![
                HorizontalSpan { left: 0, right: 396 },
                HorizontalSpan { left: 524, right: 556 },
                HorizontalSpan { left: 684, right: 900 },
            ]
        );
    }

    #[test]
    fn free_space_snaps_to_nearest_safe_span() {
        let taskbar = RECT {
            left: 100,
            top: 0,
            right: 1100,
            bottom: 48,
        };
        let free = [
            HorizontalSpan { left: 100, right: 360 },
            HorizontalSpan { left: 700, right: 1000 },
        ];
        assert_eq!(nearest_free_left_offset(taskbar, &free, 180, 650), Some(650));
        assert_eq!(nearest_free_left_offset(taskbar, &free, 180, 900), Some(720));
        assert_eq!(nearest_free_left_offset(taskbar, &free, 180, 40), Some(40));
    }

    #[test]
    fn adaptive_layout_prefers_default_then_falls_back_to_minimal() {
        let taskbar = RECT {
            left: 0,
            top: 0,
            right: 1200,
            bottom: 48,
        };
        let default_width =
            total_widget_width_for_preset(LanguageId::English, AppearancePreset::Default);
        let minimal_width =
            total_widget_width_for_preset(LanguageId::English, AppearancePreset::Minimal);

        let wide = [HorizontalSpan {
            left: 100,
            right: 100 + default_width + 20,
        }];
        let placement = placement_for_free_spans(
            AppearancePreset::Adaptive,
            LanguageId::English,
            taskbar,
            &wide,
            100,
        )
        .unwrap();
        assert_eq!(placement.0, AppearancePreset::Default);

        let narrow = [HorizontalSpan {
            left: 100,
            right: 100 + minimal_width,
        }];
        let placement = placement_for_free_spans(
            AppearancePreset::Adaptive,
            LanguageId::English,
            taskbar,
            &narrow,
            100,
        )
        .unwrap();
        assert_eq!(placement.0, AppearancePreset::Minimal);
    }

    #[test]
    fn default_layout_prioritizes_full_display_before_compact_elsewhere() {
        let taskbar = RECT {
            left: 0,
            top: 0,
            right: 1600,
            bottom: 48,
        };
        let default_width =
            total_widget_width_for_preset(LanguageId::English, AppearancePreset::Default);
        let minimal_width =
            total_widget_width_for_preset(LanguageId::English, AppearancePreset::Minimal);
        let free = [
            HorizontalSpan {
                left: 100,
                right: 100 + default_width + 20,
            },
            HorizontalSpan {
                left: 900,
                right: 900 + minimal_width,
            },
        ];
        let placement = placement_for_free_spans(
            AppearancePreset::Adaptive,
            LanguageId::English,
            taskbar,
            &free,
            900,
        )
        .unwrap();
        assert_eq!(placement.0, AppearancePreset::Default);
        assert_eq!(placement.2, 120);
    }

    #[test]
    fn adaptive_layout_reports_no_placement_when_even_minimal_does_not_fit() {
        let taskbar = RECT {
            left: 0,
            top: 0,
            right: 1200,
            bottom: 48,
        };
        let minimal_width =
            total_widget_width_for_preset(LanguageId::English, AppearancePreset::Minimal);
        let too_small = [HorizontalSpan {
            left: 100,
            right: 100 + minimal_width - 1,
        }];
        assert!(placement_for_free_spans(
            AppearancePreset::Adaptive,
            LanguageId::English,
            taskbar,
            &too_small,
            100,
        )
        .is_none());
    }

    #[test]
    fn centers_widget_vertically() {
        assert_eq!(compute_anchor_y(100, 48, 42), 103);
        assert_eq!(compute_anchor_y(100, 32, 28), 102);
        assert_eq!(compute_anchor_y(100, 24, 28), 100);
    }

    #[test]
    fn small_taskbar_threshold_is_dpi_aware() {
        assert!(is_small_taskbar_height_at_dpi(32, 96));
        assert!(is_small_taskbar_height_at_dpi(34, 96));
        assert!(!is_small_taskbar_height_at_dpi(35, 96));
        assert!(is_small_taskbar_height_at_dpi(51, 144));
        assert!(!is_small_taskbar_height_at_dpi(52, 144));
    }

    #[test]
    fn tooltip_width_keeps_extra_directwrite_trailing_glyph_clearance() {
        assert_eq!(tooltip_width_with_render_slack(100, |v| v), 144);
        assert_eq!(tooltip_width_with_render_slack(100, |v| v * 2), 188);
        assert_eq!(tooltip_width_with_render_slack(0, |v| v), 48);
        assert_eq!(tooltip_width_with_render_slack(0, |v| v * 2), 96);
        let allocated = tooltip_width_with_render_slack(124, |v| v);
        let drawable = allocated - 2 * TOOLTIP_TEXT_INSET;
        assert!(drawable >= 124 + TOOLTIP_TEXT_MEASUREMENT_SLACK);
    }

    #[test]
    fn service_tooltip_combines_visible_quota_rows() {
        assert_eq!(
            service_tooltip(
                "Codex",
                "剩余13% 19:04重置",
                "剩余86% 07/18重置",
                true,
                true,
            ),
            "Codex: 5H 剩余13% 19:04重置 | 7D 剩余86% 07/18重置"
        );
    }

    fn test_settings_json(language: &str) -> String {
        format!(
            r#"{{
  "tray_offset": 321,
  "taskbar_index": 1,
  "poll_interval_ms": 60000,
  "language": "{language}"
}}"#
        )
    }

    #[test]
    fn legacy_settings_default_to_default_appearance() {
        let settings: SettingsFile = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.appearance_preset, AppearancePreset::Default);
    }

    #[test]
    fn display_position_and_legacy_adaptive_config_are_compatible() {
        let settings: SettingsFile = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.display_position, DisplayPosition::Taskbar);
        let desktop = SettingsFile {
            display_position: DisplayPosition::Desktop,
            appearance_preset: AppearancePreset::Default,
            ..Default::default()
        };
        let parsed: SettingsFile = serde_json::from_str(
            &serde_json::to_string(&desktop).unwrap()
        ).unwrap();
        assert_eq!(parsed.display_position, DisplayPosition::Desktop);
        assert_eq!(parsed.appearance_preset, AppearancePreset::Default);
        let legacy = normalize_settings(SettingsFile {
            appearance_preset: AppearancePreset::Adaptive, ..Default::default()
        });
        assert_eq!(legacy.appearance_preset, AppearancePreset::Default);
    }

    #[test]
    fn default_adapts_on_vertical_taskbar_without_mutating_selection() {
        let bar = RECT { left: 1800, top: 0, right: 1850, bottom: 800 };
        let ample = [VerticalSpan { top: 20, bottom: 280 }];
        let narrow = [VerticalSpan { top: 20, bottom: 130 }];
        let exhausted = [VerticalSpan { top: 20, bottom: 65 }];
        let configured = AppearancePreset::Default;
        assert_eq!(vertical_placement_for_gaps(configured, bar, &ample, 24).unwrap().0,
            AppearancePreset::Default);
        assert_eq!(vertical_placement_for_gaps(configured, bar, &narrow, 24).unwrap().0,
            AppearancePreset::Minimal);
        assert_eq!(vertical_placement_for_gaps(configured, bar, &exhausted, 24), None);
        assert_eq!(configured, AppearancePreset::Default);
    }

    #[test]
    fn stable_taskbar_position_round_trips() {
        let settings = SettingsFile {
            taskbar_index: 1,
            taskbar_left_offset: Some(0),
            taskbar_monitor: Some(r"\\.\DISPLAY2".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: SettingsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.taskbar_left_offset, Some(0));
        assert_eq!(parsed.taskbar_monitor.as_deref(), Some(r"\\.\DISPLAY2"));
    }

    #[test]
    fn drag_outside_taskbar_follows_pointer_x_with_fixed_taskbar_bounds() {
        let taskbar = RECT {
            left: 500, top: 1000, right: 1500, bottom: 1048,
        };
        let width = 200;
        let anchor = 30;
        // Y is above the taskbar, but horizontal movement still follows.
        assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 700, y: 900 }, anchor, width), 170);
        assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 900, y: 900 }, anchor, width), 370);
        // The full widget stays inside the taskbar when X exceeds its edges.
        assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 100, y: 900 }, anchor, width), 0);
        assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 1800, y: 900 }, anchor, width), 800);
        // Existing in-taskbar cursor positioning is unchanged.
        assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 900, y: 1020 }, anchor, width), 370);
    }

    #[test]
    fn drag_outside_taskbar_handles_negative_screen_coordinates() {
        let taskbar = RECT {
            left: -1600, top: 500, right: -600, bottom: 548,
        };
        assert_eq!(drag_left_for_pointer(
            taskbar, POINT { x: -1000, y: 300 }, 40, 200,
        ), 560);
        assert_eq!(drag_left_for_pointer(
            taskbar, POINT { x: -2000, y: 300 }, 40, 200,
        ), 0);
    }

    #[test]
    fn transient_taskbar_clamp_does_not_change_desired_left_offset() {
        let desired = 0;
        assert_eq!(clamp_left_offset(desired, 800), 0);
        assert_eq!(clamp_left_offset(desired, 120), 0);

        let desired = 700;
        assert_eq!(clamp_left_offset(desired, 650), 650);
        assert_eq!(clamp_left_offset(desired, 800), 700);
    }

    #[test]
    fn legacy_tray_offset_migrates_to_equivalent_left_offset() {
        assert_eq!(legacy_left_offset_from_max(1000, 0), 1000);
        assert_eq!(legacy_left_offset_from_max(1000, 321), 679);
        assert_eq!(legacy_left_offset_from_max(1000, 1200), 0);
    }

    #[test]
    fn monitor_identity_wins_over_legacy_taskbar_index() {
        let taskbars = vec![
            native_interop::TaskbarWindow {
                hwnd: HWND::default(),
                rect: RECT::default(),
                monitor_device: Some(r"\\.\DISPLAY1".to_string()),
            },
            native_interop::TaskbarWindow {
                hwnd: HWND::default(),
                rect: RECT::default(),
                monitor_device: Some(r"\\.\DISPLAY2".to_string()),
            },
        ];
        let (index, taskbar) =
            select_preferred_taskbar(&taskbars, Some(r"\\.\DISPLAY2"), 0).unwrap();
        assert_eq!(index, 1);
        assert_eq!(taskbar.monitor_device.as_deref(), Some(r"\\.\DISPLAY2"));
    }

    #[test]
    fn explicit_minimal_appearance_round_trips() {
        let settings = SettingsFile {
            appearance_preset: AppearancePreset::Minimal,
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let parsed: SettingsFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.appearance_preset, AppearancePreset::Minimal);
    }

    #[test]
    fn loads_legacy_settings_when_new_path_is_missing() {
        let base = std::env::temp_dir().join(format!(
            "codex-usage-settings-test-{}-{}",
            std::process::id(),
            now_unix_secs()
        ));
        let current = base.join("CodexUsage").join("settings.json");
        let legacy = base.join("ClaudeCodeUsageMonitor").join("settings.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, test_settings_json("zh-CN")).unwrap();
        let (settings, migrated) = load_settings_from_paths(&current, &legacy).unwrap();
        assert!(migrated);
        assert_eq!(settings.tray_offset, 321);
        assert_eq!(settings.taskbar_left_offset, None);
        assert_eq!(settings.taskbar_monitor, None);
        assert_eq!(settings.poll_interval_ms, 60_000);
        assert_eq!(settings.language.as_deref(), Some("zh-CN"));
        assert!(settings.show_session_window);
        assert!(settings.show_weekly_window);
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn new_settings_take_precedence_over_legacy_settings() {
        let base = std::env::temp_dir().join(format!(
            "codex-usage-settings-precedence-test-{}-{}",
            std::process::id(),
            now_unix_secs()
        ));
        let current = base.join("CodexUsage").join("settings.json");
        let legacy = base.join("ClaudeCodeUsageMonitor").join("settings.json");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&current, test_settings_json("en")).unwrap();
        std::fs::write(&legacy, test_settings_json("zh-CN")).unwrap();
        let (settings, migrated) = load_settings_from_paths(&current, &legacy).unwrap();
        assert!(!migrated);
        assert_eq!(settings.language.as_deref(), Some("en"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn startup_migration_only_writes_when_legacy_exists_without_current_entry() {
        assert!(should_write_migrated_startup(true, false));
        assert!(!should_write_migrated_startup(false, false));
        assert!(!should_write_migrated_startup(true, true));
        assert!(!should_write_migrated_startup(false, true));
    }

    #[test]
    fn displays_distinct_transient_error_categories() {
        assert_eq!(
            poll_error_display_label(
                poller::PollError::NetworkUnavailable,
                LanguageId::SimplifiedChinese
            ),
            "网络"
        );
        assert_eq!(
            poll_error_display_label(
                poller::PollError::RateLimited,
                LanguageId::SimplifiedChinese
            ),
            "限流"
        );
        assert_eq!(
            poll_error_display_label(poller::PollError::ServerError, LanguageId::English),
            "5XX"
        );
        assert_eq!(
            poll_error_display_label(poller::PollError::RequestFailed, LanguageId::English),
            "ERR"
        );
    }

    #[test]
    fn normalizes_usage_display_and_alert_settings() {
        let settings = normalize_settings(SettingsFile {
            show_session_window: false,
            show_weekly_window: false,
            alert_threshold_percent: 17,
            notified_quota_windows: vec!["codex:weekly:1".into(), "codex:weekly:1".into()],
            ..SettingsFile::default()
        });
        assert!(settings.show_session_window);
        assert!(!settings.show_weekly_window);
        assert_eq!(settings.alert_threshold_percent, 0);
        assert_eq!(settings.notified_quota_windows.len(), 1);
    }

    #[test]
    fn formats_precise_local_reset_time() {
        let local = SYSTEMTIME {
            wYear: 2026,
            wMonth: 7,
            wDay: 17,
            wHour: 18,
            wMinute: 30,
            ..Default::default()
        };
        assert_eq!(format_local_system_time(local), "2026-07-17 18:30");
        assert_eq!(format_precise_reset_time(None), None);
    }

    #[test]
    fn low_quota_alert_is_deduplicated_until_reset_window_changes() {
        let mut alerts = Vec::new();
        let mut notified = BTreeSet::new();
        let first_reset = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let first = crate::models::UsageSection {
            percentage: 85.0,
            resets_at: Some(first_reset),
        };
        append_quota_alert(
            &mut alerts,
            &mut notified,
            20,
            LanguageId::SimplifiedChinese,
            tray_icon::TrayIconKind::Codex,
            "codex",
            "Codex",
            "session",
            "5小时",
            &first,
        );
        append_quota_alert(
            &mut alerts,
            &mut notified,
            20,
            LanguageId::SimplifiedChinese,
            tray_icon::TrayIconKind::Codex,
            "codex",
            "Codex",
            "session",
            "5小时",
            &first,
        );
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].message.contains("剩余 15%"));

        let next = crate::models::UsageSection {
            percentage: 90.0,
            resets_at: Some(first_reset + Duration::from_secs(18_000)),
        };
        append_quota_alert(
            &mut alerts,
            &mut notified,
            20,
            LanguageId::SimplifiedChinese,
            tray_icon::TrayIconKind::Codex,
            "codex",
            "Codex",
            "session",
            "5小时",
            &next,
        );
        assert_eq!(alerts.len(), 2);
        assert_eq!(notified.len(), 1);
    }
}
