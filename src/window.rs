use std::cell::Cell;
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
use windows::Win32::UI::Shell::ShellExecuteW;
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
    default_desktop_auto_collapse, EditableAppearance, EditableGeneral, EditableSettings, EditableThemeStyle, EditableUsage,
    EDITABLE_SETTINGS_SCHEMA_VERSION,
};
use crate::style::{
    StyleColorTarget, StyleSettings, ThemeMode, ThemePreset, ThemeStyle, FROSTED_STRENGTH_MAX,
};
use crate::style_window;
use crate::theme;
use crate::tray_icon;
use crate::updater;
use crate::window_common::{load_embedded_app_icons, set_window_caption_icons, SendHwnd};

mod taskbar_replica;
mod blur_backdrop;
mod layered_bitmap;
mod tooltip_measure;
mod floating_geometry;
use self::floating_geometry::*;
use self::tooltip_measure::{cached_minimal_tooltip_width, TOOLTIP_TEXT_INSET};
use self::taskbar_replica::*;
use self::blur_backdrop::*;
use self::layered_bitmap::{DibSurface, WindowDc};

// Set only while restoring the desktop popup to a different monitor.
// SetWindowPos may synchronously emit WM_DPICHANGED, which must not re-enter
// position_at_taskbar() before the target-monitor layout has been committed.
thread_local! {
    static FLOATING_INITIAL_PLACEMENT: Cell<bool> = const { Cell::new(false) };
}

struct FloatingInitialPlacementGuard;
impl FloatingInitialPlacementGuard {
    fn begin() -> Self {
        FLOATING_INITIAL_PLACEMENT.with(|flag| flag.set(true));
        Self
    }
}
impl Drop for FloatingInitialPlacementGuard {
    fn drop(&mut self) {
        FLOATING_INITIAL_PLACEMENT.with(|flag| flag.set(false));
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
    desktop_auto_collapse: bool,
    hidden_for_taskbar_space: bool,
    taskbar_space_warning_shown: bool,
    floating_mode: bool,
    floating_hwnd: Option<SendHwnd>,
    parked_taskbar_foreground: Option<SendHwnd>,
    floating_x: i32,
    floating_y: i32,
    floating_edge: Option<FloatingEdge>,
    // The undocked tab is below the body when there is not enough room above.
    floating_free_grip_below: bool,
    // Whether an edge must be clipped within this monitor: either Explorer
    // reserves it for a taskbar, or another display shares the physical edge.
    // Both dock normally with in-place sliding, never bleeding to the next screen.
    floating_reserved_dock: bool,
    // Preserve the exact clicked tab location while expanding a collapsed
    // taskbar-edge popup into a captured drag.
    floating_reserved_drag_reveal: u8,
    // During an edge-tab drag, unlock the previously screen-clipped side's
    // external glow once the panel is clear of the physical monitor boundary.
    // The tab stays on the grabbed side until mouse-up.
    floating_drag_glow_detached: bool,
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
const IDM_RESET_POSITION: u16 = 30;
const IDM_CHECK_UPDATE: u16 = 60;
const IDM_OPEN_RELEASES: u16 = 61;
const IDM_LAYOUT_DEFAULT: u16 = 91;
const IDM_LAYOUT_MINIMAL: u16 = 92;
const IDM_THEME_SYSTEM: u16 = 93;
const IDM_THEME_DARK: u16 = 94;
const IDM_THEME_LIGHT: u16 = 95;
const IDM_STYLE_SETTINGS: u16 = 96;

const IDM_QUICK_POSITION_TASKBAR: u16 = 113;
const IDM_QUICK_POSITION_DESKTOP: u16 = 114;
const IDM_QUICK_DIRECTION_HORIZONTAL: u16 = 115;
const IDM_QUICK_DIRECTION_VERTICAL: u16 = 116;
const IDM_QUICK_COLLAPSE_ON: u16 = 117;
const IDM_QUICK_COLLAPSE_OFF: u16 = 118;

const PANEL_BORDER_WIDTH_PX: i32 = 1;
/// Layered windows treat fully transparent pixels as mouse-pass-through.
/// Keep an imperceptible alpha on visually transparent panel pixels so the
/// component remains draggable/right-clickable even when its background is 0 alpha.
const MIN_INTERACTIVE_ALPHA: u8 = 1;
const FROSTED_MAX_BLUR_PX: f32 = 20.0;
const DRAG_FRAME_MS: u64 = 8;
const TASKBAR_LAYOUT_POLL_MS: u32 = 1_000;
const TASKBAR_CONTROL_MARGIN_LOGICAL: i32 = 4;

const GITHUB_RELEASES_URL: &str =
    "https://github.com/walle-2017/codex-usage-win/releases";
const WM_DPICHANGED_MSG: u32 = 0x02E0;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const MINIMAL_TOOLTIP_CLASS: &str = "CodexUsageMinimalTooltip";
const WM_APP_FOCUS_EXISTING_SETTINGS: u32 = native_interop::WM_APP + 140;
const WM_APP_DESKTOP_HOVER_EXIT: u32 = native_interop::WM_APP + 141;
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
// Collected by the existing background taskbar worker. Geometry-only reads on
// the UI thread never synchronously enter Explorer or enumerate every window.
static TASKBAR_EDGE_RECTS: Mutex<Vec<RECT>> = Mutex::new(Vec::new());

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
    // While the desktop HWND is being dragged, its rectangular bounds may
    // straddle two monitors and GetDpiForWindow can switch before the pointer
    // does. The drag handler owns DPI selection/rebasing from GetCursorPos.
    // Never replace that value midway through a layered-frame render.
    if lock_state().as_ref().is_some_and(|s| s.floating_mode && s.dragging) {
        return;
    }
    let source = {
        let state = lock_state();
        state.as_ref().map(|s| {
            // A fully retracted popup may lie mostly beyond its own display
            // or across a multi-monitor seam. GetDpiForWindow(hwnd) then
            // describes the wrong screen. The *saved expanded position*
            // identifies the monitor containing the visible drag tab.
            let desktop_anchor = if s.floating_mode {
                let (width, height) = floating_canvas_size_for_state(s);
                floating_restore_anchor(
                    s.floating_x, s.floating_y, width, height,
                ).map(|center| floating_docked_monitor_anchor(
                    s.floating_edge, s.floating_x, s.floating_y, center,
                ))
            } else { None };
            let hwnd = if s.floating_mode { s.hwnd.to_hwnd() }
                else { s.taskbar_hwnd.unwrap_or_else(|| s.hwnd.to_hwnd()) };
            (hwnd, desktop_anchor)
        })
    };
    if let Some((hwnd, desktop_anchor)) = source {
        let dpi = desktop_anchor.and_then(floating_monitor_dpi)
            .unwrap_or_else(|| unsafe { GetDpiForWindow(hwnd) });
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

unsafe impl Send for AppState {}

static STATE: Mutex<Option<AppState>> = Mutex::new(None);

/// Lock STATE safely, recovering from poisoned mutex
fn lock_state() -> MutexGuard<'static, Option<AppState>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The visible widget HWND, not the stable polling/tray service owner.
/// Settings use it to center their window on the correct monitor.
pub(crate) fn settings_placement_anchor() -> Option<HWND> {
    let state = lock_state();
    state.as_ref().map(|s| s.hwnd.to_hwnd())
}

const SETTINGS_DIR: &str = "CodexUsage";

fn appdata_path(directory: &str) -> PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(appdata).join(directory).join("settings.json")
}

fn settings_path() -> PathBuf {
    appdata_path(SETTINGS_DIR)
}


#[derive(Debug, Serialize, Deserialize)]
struct SettingsFile {
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
    #[serde(default)]
    floating_free_grip_below: bool,
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
    #[serde(default = "default_desktop_auto_collapse")]
    desktop_auto_collapse: bool,
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
            taskbar_index: 0,
            taskbar_left_offset: None,
            taskbar_top_offset: None,
            floating_x: None,
            floating_y: None,
            floating_edge: None,
            floating_free_grip_below: false,
            taskbar_monitor: None,
            poll_interval_ms: default_poll_interval(),
            language: None,
            appearance_preset: AppearancePreset::Default,
            display_position: DisplayPosition::Taskbar,
            desktop_direction: DesktopDirection::Horizontal,
            desktop_auto_collapse: default_desktop_auto_collapse(),
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


// Current settings schema is generated from the active SettingsFile defaults.
// Optional fields omitted by skip_serializing_if are explicitly included so
// they remain eligible for retaining an existing saved user value.
fn settings_schema() -> serde_json::Value {
    let mut schema = serde_json::to_value(SettingsFile::default())
        .expect("settings defaults must serialize");
    let object = schema.as_object_mut().expect("settings schema must be an object");
    for name in [
        "taskbar_left_offset", "taskbar_top_offset", "floating_x", "floating_y",
        "floating_edge", "taskbar_monitor", "language",
    ] {
        object.insert(name.to_string(), serde_json::Value::Null);
    }
    object.insert("notified_quota_windows".to_string(), serde_json::json!([]));
    schema
}

/// Walk only schema-known names. Invalid values (wrong type, retired enum
/// variants, overflow) revert independently, not the entire user settings.
fn preserve_matching_settings(
    output: &mut serde_json::Value,
    schema: &serde_json::Value,
    incoming: &serde_json::Value,
    path: &mut Vec<String>,
) {
    let (Some(fields), Some(user_fields)) = (schema.as_object(), incoming.as_object()) else {
        return;
    };
    for (name, default) in fields {
        let Some(value) = user_fields.get(name) else { continue };
        path.push(name.clone());
        if default.is_object() && value.is_object() {
            preserve_matching_settings(output, default, value, path);
        } else {
            let pointer = format!("/{}", path.join("/"));
            let old = output.pointer(&pointer).cloned().expect("known setting");
            *output.pointer_mut(&pointer).expect("known setting") = value.clone();
            if serde_json::from_value::<SettingsFile>(output.clone()).is_err() {
                *output.pointer_mut(&pointer).expect("known setting") = old;
            }
        }
        path.pop();
    }
}

/// Missing fields come from defaults; extra fields are dropped, recursively.
/// Accepted fields must be representable by the current typed configuration.
fn reconcile_settings_json(text: &str) -> (SettingsFile, bool) {
    let Ok(input) = serde_json::from_str::<serde_json::Value>(text) else {
        return (SettingsFile::default(), true);
    };
    let schema = settings_schema();
    let mut output = schema.clone();
    preserve_matching_settings(&mut output, &schema, &input, &mut Vec::new());
    let decoded: SettingsFile = serde_json::from_value(output)
        .unwrap_or_else(|_| SettingsFile::default());
    let normalized = normalize_settings(decoded);
    let encoded = serde_json::to_value(&normalized)
        .expect("normalized settings must serialize");
    (normalized, encoded != input)
}

fn backup_invalid_settings(path: &std::path::Path) {
    // A malformed JSON document cannot be partially recovered as JSON.
    // Preserve it once before replacing it with defaults.
    let backup = path.with_extension("json.invalid.bak");
    if !backup.exists() {
        let _ = std::fs::copy(path, backup);
    }
}

fn load_settings() -> SettingsFile {
    let current_path = settings_path();
    let (settings, needs_rewrite) = load_settings_from_path(&current_path)
        .unwrap_or_else(|| (SettingsFile::default(), false));
    if needs_rewrite {
        save_settings(&settings);
        diagnose::log(format!(
            "reconciled settings file with current schema at {}",
            current_path.display()
        ));
    }
    settings
}

fn load_settings_from_path(path: &std::path::Path) -> Option<(SettingsFile, bool)> {
    let text = std::fs::read_to_string(path).ok()?;
    if serde_json::from_str::<serde_json::Value>(&text).is_err() {
        backup_invalid_settings(path);
    }
    Some(reconcile_settings_json(&text))
}

fn normalize_settings(mut settings: SettingsFile) -> SettingsFile {
    if !settings.show_session_window && !settings.show_weekly_window {
        settings.show_session_window = true;
    }
    if !matches!(settings.alert_threshold_percent, 0 | 10 | 20 | 30) {
        settings.alert_threshold_percent = 0;
    }
    if !matches!(
        settings.poll_interval_ms,
        POLL_1_MIN | POLL_5_MIN | POLL_15_MIN | POLL_1_HOUR
    ) {
        settings.poll_interval_ms = default_poll_interval();
    }
    if settings.floating_edge.as_deref()
            .is_some_and(|side| FloatingEdge::parse(side).is_none())
    {
        settings.floating_edge = None;
    }
    // The current settings UI supports only these two explicit language codes.
    // All retired codes fall back to system language on next startup.
    if settings.language.as_deref().is_some_and(|code| !matches!(code, "zh-CN" | "en")) {
        settings.language = None;
    }
    settings.notified_quota_windows.sort();
    settings.notified_quota_windows.dedup();
    settings.styles.normalize();
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
            taskbar_index: s.taskbar_index,
            taskbar_left_offset: Some(s.taskbar_left_offset),
            taskbar_top_offset: Some(s.taskbar_top_offset),
            floating_x: Some(s.floating_x),
            floating_y: Some(s.floating_y),
            floating_edge: s.floating_edge.map(|e| e.name().to_string()),
            floating_free_grip_below: s.floating_free_grip_below,
            taskbar_monitor: s.taskbar_monitor.clone(),
            poll_interval_ms: s.poll_interval_ms,
            language: s
                .language_override
                .map(|language| language.code().to_string()),
            appearance_preset: s.appearance_preset,
            display_position: s.display_position,
            desktop_direction: s.desktop_direction,
            desktop_auto_collapse: s.desktop_auto_collapse,
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


fn clamp_left_offset(left_offset: i32, max_left: i32) -> i32 {
    left_offset.clamp(0, max_left.max(0))
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
        let mut last_taskbar_geometry: BTreeMap<isize, (i32, i32, i32, i32)> = BTreeMap::new();

        loop {
            let taskbars = native_interop::find_taskbars();
            // Moving the Windows taskbar between screen edges can change its
            // rectangle without changing its UIA controls. Refresh desktop
            // docking on geometry changes too (without a new polling timer).
            let geometry: BTreeMap<isize, (i32, i32, i32, i32)> = taskbars.iter()
                .map(|bar| (bar.hwnd.0 as isize, (
                    bar.rect.left, bar.rect.top, bar.rect.right, bar.rect.bottom,
                )))
                .collect();
            let geometry_changed = geometry != last_taskbar_geometry;
            last_taskbar_geometry = geometry;
            if !taskbars.is_empty() {
                *TASKBAR_EDGE_RECTS.lock().unwrap_or_else(|e| e.into_inner()) =
                    taskbars.iter().map(|bar| bar.rect).collect();
            }
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

            if changed || geometry_changed {
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
    let choice = configured_preset;
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

// The desktop popup does not use the taskbar's adaptive space fallback.
// Rebuild the formatted usage strings *after* changing the effective preset:
// Minimal omits the reset suffix, so a stale Minimal string cannot recover
// its date/time merely by repainting a Default-width widget.
fn sync_floating_preset_and_usage(s: &mut AppState, preset: AppearancePreset) {
    if s.effective_appearance_preset != preset {
        s.effective_appearance_preset = preset;
        refresh_usage_texts(s);
    }
}

fn manual_update_required_message(language: LanguageId, version: &str) -> String {
    match language {
        LanguageId::English => format!("Version v{version} is available, but the release asset is missing. Open GitHub Releases and update manually."),
        LanguageId::SimplifiedChinese => format!("检测到 v{version}，但对应的发布文件不存在，无法自动升级。请前往 GitHub Releases 手动更新。"),
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

/// SetWindowTextW is synchronous and can re-enter window procedures, including
/// an Explorer-hosted taskbar child. Do not invoke it with the AppState mutex
/// held: re-entrant messages can need that same lock.
fn refresh_window_title() {
    let title = {
        let state = lock_state();
        state.as_ref().map(|s| (s.hwnd.to_hwnd(), s.language.strings()))
    };
    if let Some((hwnd, strings)) = title {
        set_window_title(hwnd, strings);
    }
}

/// Pure state update under the caller's lock. Perform synchronous Win32
/// title updates separately, after the lock guard has been dropped.
fn apply_language_to_state(state: &mut AppState, language_override: Option<LanguageId>) {
    let language_override = language_override.map(LanguageId::ui_supported);
    state.language_override = language_override;
    state.language = localization::resolve_language(language_override);
    refresh_usage_texts(state);
}

fn update_language_change() -> bool {
    let changed = {
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
    };
    if changed {
        refresh_window_title();
    }
    changed
}

const STARTUP_REGISTRY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_REGISTRY_KEY: &str = "CodexUsageWin";

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

/// A hit test that Windows has actually delivered for the expanded,
/// docked grip must not be discarded only because its fade-in animation has
/// not advanced beyond alpha zero yet. Free-floating tabs retain their
/// original alpha-zero behavior, and no invisible pixels are added here.
fn floating_grip_hit_allowed(
    edge: Option<FloatingEdge>, auto_collapse: bool, collapsed: bool, alpha: u8,
) -> bool {
    alpha > 0 || (edge.is_some() && !auto_collapse && !collapsed)
}

fn is_drag_handle_point(client_x: i32, client_y: i32) -> bool {
    {
        let state = lock_state();
        if let Some(s) = state.as_ref() {
            if s.floating_mode {
                if floating_grip_uses_hover_fade(s) && !floating_grip_hit_allowed(
                    s.floating_edge,
                    floating_auto_collapse_allowed(s.display_position, s.desktop_auto_collapse),
                    s.floating_collapsed,
                    s.floating_grip_alpha,
                ) {
                    return false;
                }
                return point_in_rect(
                    floating_state_grip_rect(s), client_x, client_y,
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
    match state.appearance_preset {
        AppearancePreset::Default => state.effective_appearance_preset,
        AppearancePreset::Minimal => AppearancePreset::Minimal,
    }
}

// The configured Default is always adaptive, without changing stored settings.
fn vertical_effective_preset(preset: AppearancePreset) -> AppearancePreset {
    preset
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
        AppearancePreset::Default => 8,
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

        set_window_caption_icons(hwnd, large_icon, small_icon);

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
                desktop_auto_collapse: settings.desktop_auto_collapse,
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
                floating_free_grip_below: settings.floating_free_grip_below,
                floating_reserved_dock: false,
                floating_reserved_drag_reveal: 100,
                floating_drag_glow_detached: false,
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
                dragging: false,
                drag_anchor_logical_x: 0,
                drag_anchor_logical_y: 0,
                floating_drag_anchor_x: 0,
                floating_drag_anchor_y: 0,
                drag_reparenting: false,
        failed_drag_target: None,
            });
        }

        // Prefer a persisted monitor identity; fall back to the saved index if unavailable.
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
        // position_at_taskbar() can switch to a separate desktop HWND and
        // explicitly hide this taskbar child. Do not immediately show the
        // old child again after the desktop popup is already visible.
        let desktop_active = lock_state().as_ref().is_some_and(|s| s.floating_mode);
        if !desktop_active && !widget_hidden_for_taskbar_space() {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            diagnose::log("taskbar window shown");
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

fn drag_frame_due(force: bool) -> bool {
    preview_frame_due(&LAST_DRAG_FRAME, DRAG_FRAME_MS, force)
}

/// Keep the Composition backdrop prepared while frost intensity is enabled,
/// even if the panel is temporarily fully opaque. Alpha=255 to alpha=254
/// must not create a new HWND in the same frame that the foreground becomes
/// transparent; the background composition visual is committed asynchronously.
/// The normal "frosted visibly active" style predicate remains unchanged.
fn panel_blur_backdrop_needed(style: &ThemeStyle) -> bool {
    style.panel_frosted_strength > 0
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
    // Prepare Gaussian blur as soon as its strength is nonzero. At alpha=255
    // the foreground remains opaque, hiding this prebuilt visual. This avoids
    // an empty frame when alpha drops to 254 and the background is first needed.
    // The desktop backdrop is clipped to the original body (never the tab).
    let blur_requested = panel_blur_backdrop_needed(&style);
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
    if composition_blur_active && background.a < u8::MAX {
        // Only let Composition own the *visible* fill once translucency is
        // requested. At alpha=255 the original opaque foreground must cover
        // the prewarmed blur, preventing a blink during alpha 255 -> 254.
        // Alpha=1 preserves hit testing of the layered panel interior.
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
        let Some(window_dc) = WindowDc::acquire(hwnd) else { return };
        let screen_dc = window_dc.handle();
        let Some(mut panel_surface) = DibSurface::create(screen_dc, width, height) else { return };
        let mem_dc = panel_surface.handle();
        let pixel_data = panel_surface.pixels();
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

        // Taskbar space cannot host an exterior halo reliably. Paint the
        // hover glow exclusively inside the existing rounded panel fill,
        // before the text pass, leaving the border and all exterior pixels
        // untouched. Desktop mode keeps its original exterior-only painter.
        let drag_hovered = lock_state().as_ref().is_some_and(|s| s.drag_handle_hovered);
        if !floating && drag_hovered {
            draw_taskbar_hover_inner_glow(
                pixel_data, width, height, &style, drag_hover_outline_color(&style),
            );
        }

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

        let pt_src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: 0,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: 1,
        };
        if floating {
            let (edge, free_below, hidden, hovered, grip_alpha, reserved, reveal, dragging, drag_reveal) = {
                let state = lock_state();
                state.as_ref().map(|s| (
                    s.floating_edge,
                    floating_free_grip_below_for_state(s),
                    s.floating_collapsed && s.floating_reveal == 0,
                    s.drag_handle_hovered,
                    floating_grip_visual_alpha(
                        floating_grip_uses_hover_fade(s),
                        s.dragging,
                        s.floating_grip_alpha,
                    ),
                    s.floating_reserved_dock,
                    s.floating_reveal,
                    s.dragging,
                    s.floating_reserved_drag_reveal,
                )).unwrap_or((None, false, false, false, 0, false, 100, false, 100))
            };
            // Composite the already rendered taskbar-size panel into a separate
            // transparent desktop canvas. Neither the panel border nor quota
            // graphics are ever stretched or shifted to make room for the tab.
            let (canvas_w, canvas_h, body) = {
                let state = lock_state();
                state.as_ref().map(floating_canvas_geometry_for_state)
                    .unwrap_or_else(|| floating_canvas_geometry(width, height, edge))
            };
            let grip = floating_layout_grip_rect(
                canvas_w, canvas_h, edge, free_below,
            );
            if let Some(mut canvas_surface) = DibSurface::create(screen_dc, canvas_w, canvas_h) {
                let canvas_dc = canvas_surface.handle();
                let canvas = canvas_surface.pixels();
                let accent = drag_hover_outline_color(&style);
                canvas.fill(0);
                if hovered && !hidden {
                    // Paint the external halo FIRST. The body and tab occupy
                    // exactly the mask used for the glow, so no colored pixels
                    // can enter the semi-transparent glass or rounded corners.
                    draw_drag_hover_outer_glow(
                        canvas, canvas_w, canvas_h, body, grip, &style, accent,
                    );
                }
                if !hidden {
                    for y in 0..height {
                        let dst = ((y + body.top) * canvas_w + body.left) as usize;
                        let src = (y * width) as usize;
                        for x in 0..width as usize {
                            let index = dst + x;
                            // Direct copies erased the underpainting even at
                            // completely transparent round-corner pixels,
                            // recreating a dark seam between body and halo.
                            canvas[index] = composite_premultiplied_pixel(
                                canvas[index], pixel_data[src + x],
                            );
                        }
                    }
                }
                render_floating_handle_bitmap_at(
                    canvas, canvas_w, canvas_h, grip, hovered, &style, grip_alpha,
                );
                if hovered && !hidden {
                    // Preserve V24.6's alpha-1 transition probe: light is
                    // visual-only and must never enlarge drag hit targets.
                    paint_floating_hover_hit_probe(
                        canvas, canvas_w, canvas_h, body, grip,
                        if edge.is_none() && free_below { Some(FloatingEdge::Top) } else { edge },
                    );
                }
                if reserved {
                    if let Some(edge) = edge {
                        // Same slide as an exposed monitor edge, alpha-clipped
                        // before reaching Explorer or an adjacent display.
                        floating_slide_layered_bitmap(
                            canvas, canvas_w, canvas_h, edge,
                            if dragging { drag_reveal } else { reveal },
                        );
                    }
                }
                let sz = SIZE { cx: canvas_w, cy: canvas_h };
                let _ = UpdateLayeredWindow(
                    hwnd, screen_dc, desktop_origin.as_ref().map(|p| p as *const POINT), Some(&sz),
                    canvas_dc, Some(&pt_src),
                    COLORREF(0), Some(&blend), ULW_ALPHA,
                );
            }
        } else {
            let sz = SIZE { cx: width, cy: height };
            let _ = UpdateLayeredWindow(
                hwnd, screen_dc, None, Some(&sz), mem_dc, Some(&pt_src),
                COLORREF(0), Some(&blend), ULW_ALPHA,
            );
        }
        // Both DIB surfaces are deselected/deleted before the window DC is
        // returned, including failure and early-exit paths.
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
/// Desktop canvases reserve transparent pixels beyond the visible panel
/// and detached grip; physical monitor edges necessarily clip external light.
fn drag_hover_glow_coverage(distance_from_edge: f32) -> u8 {
    let distance = distance_from_edge.abs();
    // Soft halo falls off on either side of the visible panel perimeter.
    let diffuse = 120.0 * (-(distance / 4.6).powi(2) * 0.5).exp();
    // Retain the antialiased hairline while making the outer halo dominant.
    let hairline = 65.0 * (1.0 - distance / 0.7).max(0.0);
    (diffuse + hairline).round().clamp(0.0, 200.0) as u8
}

/// Feather hover feedback around the actual body or detached grip perimeter.
/// Desktop canvases provide up to 12 logical pixels of outer glow. Visual-only
/// transparent margins never extend the component's interactive hit targets.
/// Place the desktop hover glow BEHIND the foreground, never on top of
/// partially transparent panel/drag-tab pixels. The occupancy mask uses
/// exactly the same corner geometry as panel_color_at(), so the nearest
/// exterior glow pixel is adjacent to the actual border even at rounded
/// corners. The two silhouettes are unioned before glowing: panel light
/// cannot bleed into the neighboring tab (or vice versa).
fn draw_drag_hover_outer_glow(
    pixels: &mut [u32], canvas_w: i32, canvas_h: i32,
    body: RECT, tab: RECT, style: &ThemeStyle, color: Color,
) {
    if canvas_w <= 0 || canvas_h <= 0 { return; }
    let area = (canvas_w * canvas_h) as usize;
    let mut occupied = vec![false; area];
    let mut border_pixels = vec![false; area];
    for rect in [body, tab] {
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        if width <= 0 || height <= 0 { continue; }
        for y in rect.top.max(0)..rect.bottom.min(canvas_h) {
            for x in rect.left.max(0)..rect.right.min(canvas_w) {
                if panel_outer_shape_contains(
                    width, height, x - rect.left, y - rect.top,
                    style.panel_corner_radius,
                ) {
                    let index = (y * canvas_w + x) as usize;
                    occupied[index] = true;
                    border_pixels[index] |= !panel_inner_shape_contains(
                        width, height, x - rect.left, y - rect.top,
                        style.panel_corner_radius,
                    );
                }
            }
        }
    }
    let extent = sc(12).max(2);
    let extent_squared = extent * extent;
    let mut glow = vec![0_u8; area];
    for y in 0..canvas_h {
        for x in 0..canvas_w {
            let index = (y * canvas_w + x) as usize;
            if !occupied[index] { continue; }
            // Only a real silhouette boundary casts exterior light. A shared
            // panel/tab contact cannot create an internal glowing seam.
            let boundary = x == 0 || x == canvas_w - 1
                || y == 0 || y == canvas_h - 1
                || !occupied[(y * canvas_w + (x - 1)) as usize]
                || !occupied[(y * canvas_w + (x + 1)) as usize]
                || !occupied[((y - 1) * canvas_w + x) as usize]
                || !occupied[((y + 1) * canvas_w + x) as usize];
            if !boundary { continue; }
            // V24.19: extend the same halo 0.75px below the *real* painted
            // border. The later alpha-over compositing hides this under
            // opaque pixels and lets it feather through only the outermost
            // translucent rim. Never underpaint the actual content interior.
            if border_pixels[index] {
                glow[index] = glow[index].max(drag_hover_glow_coverage(0.75));
            }
            for dy in -extent..=extent {
                let ny = y + dy;
                if ny < 0 || ny >= canvas_h { continue; }
                for dx in -extent..=extent {
                    let nx = x + dx;
                    if nx < 0 || nx >= canvas_w || dx * dx + dy * dy > extent_squared {
                        continue;
                    }
                    let other = (ny * canvas_w + nx) as usize;
                    if occupied[other] { continue; }
                    // The mask is pixel-exact, so the first outside pixel
                    // touches the rounded contour. Subtract one pixel of
                    // center-to-center distance to avoid a dark gap.
                    let distance = (((dx * dx + dy * dy) as f32).sqrt() - 1.0).max(0.0);
                    glow[other] = glow[other].max(drag_hover_glow_coverage(distance));
                }
            }
        }
    }
    for (index, coverage) in glow.into_iter().enumerate() {
        if coverage != 0 {
            pixels[index] = composite_premultiplied_text(pixels[index], color, coverage);
        }
    }
}


/// Taskbar hover is a one-sided inner glow: its hard clip is the same actual
/// rounded *fill* silhouette as panel_color_at(). The border itself, its outer
/// antialias region, and every pixel outside the panel remain bit-for-bit
/// unchanged. Desktop exterior-glow rendering deliberately does not use this.
fn draw_taskbar_hover_inner_glow(
    pixels: &mut [u32], width: i32, height: i32, style: &ThemeStyle, color: Color,
) {
    if width <= 0 || height <= 0 || pixels.len() < (width as usize * height as usize) {
        return;
    }
    let outer_inset = sc(1).max(1);
    let inner_inset = outer_inset + PANEL_BORDER_WIDTH_PX;
    let outer_radius = sc(i32::from(style.panel_corner_radius))
        .min(((width - outer_inset * 2).min(height - outer_inset * 2) / 2).max(0));
    let inner_radius = if outer_radius > 0 {
        (outer_radius - PANEL_BORDER_WIDTH_PX).max(1)
    } else {
        0
    } as f32;
    let half_w = (width as f32 * 0.5 - inner_inset as f32).max(0.5);
    let half_h = (height as f32 * 0.5 - inner_inset as f32).max(0.5);
    let radius = inner_radius.min(half_w.min(half_h));
    let reach = sc(9).max(3) as f32;
    for y in inner_inset..(height - inner_inset).max(inner_inset) {
        for x in inner_inset..(width - inner_inset).max(inner_inset) {
            if !panel_inner_shape_contains(
                width, height, x, y, style.panel_corner_radius,
            ) {
                continue;
            }
            // Rounded-box signed distance from the INNER edge of the painted
            // border. Apply a finite taper so there is no tint in the center.
            let dx = (x as f32 + 0.5 - width as f32 * 0.5).abs() - (half_w - radius);
            let dy = (y as f32 + 0.5 - height as f32 * 0.5).abs() - (half_h - radius);
            let signed_distance = dx.max(0.0).hypot(dy.max(0.0))
                + dx.max(dy).min(0.0) - radius;
            let inward = (-signed_distance).max(0.0);
            if inward >= reach {
                continue;
            }
            let fade = (1.0 - inward / reach).powi(2);
            let coverage = (f32::from(drag_hover_glow_coverage(inward)) * fade)
                .round().clamp(0.0, 255.0) as u8;
            if coverage != 0 {
                let index = (y * width + x) as usize;
                pixels[index] = composite_premultiplied_text(
                    pixels[index], color, coverage,
                );
            }
        }
    }
}

/// Fallback GDI taskbar mode must match the embedded layered mode. The GDI
/// panel is opaque and already painted; blend only inner-fill pixels into the
/// memory DC, before dots, progress bars and text are drawn.
fn draw_taskbar_hover_inner_glow_gdi(
    hdc: HDC, width: i32, height: i32, style: &ThemeStyle, base: Color,
) {
    if width <= 0 || height <= 0 {
        return;
    }
    let base_pixel = premultiplied_pixel(Color::rgba(base.r, base.g, base.b, 255));
    let mut pixels = vec![base_pixel; (width * height) as usize];
    draw_taskbar_hover_inner_glow(
        &mut pixels, width, height, style, drag_hover_outline_color(style),
    );
    unsafe {
        for y in 0..height {
            for x in 0..width {
                let pixel = pixels[(y * width + x) as usize];
                if pixel == base_pixel {
                    continue;
                }
                let color = Color::rgba(
                    ((pixel >> 16) & 255) as u8,
                    ((pixel >> 8) & 255) as u8,
                    (pixel & 255) as u8,
                    255,
                );
                let _ = SetPixelV(hdc, x, y, COLORREF(color.to_colorref()));
            }
        }
    }
}

fn render_floating_handle_bitmap_at(
    pixels: &mut [u32], width: i32, height: i32,
    grip: RECT, hovered: bool, style: &ThemeStyle, opacity: u8,
) {
    // The tab is a miniature panel, not a separate style. Its fill, border,
    // corner radius, transparency and glass strength follow ThemeStyle.
    let center_x = (grip.left + grip.right) / 2;
    let center_y = (grip.top + grip.bottom) / 2;
    let vertical_grip = grip.bottom - grip.top > grip.right - grip.left;
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
            let index = (y * width + x) as usize;
            pixels[index] = composite_premultiplied_pixel(
                pixels[index], premultiplied_pixel(color),
            );
            if opacity > 0 && pixels[index] >> 24 == 0 {
                // Layered windows discard alpha=0 pixels BEFORE WM_NCHITTEST.
                // With a fully transparent panel background, only the grip's
                // border and six dots otherwise receive mouse clicks.
                // Restore the existing V24.6 alpha-1 hit-test convention for
                // the real rounded grip interior (never its glow gutter or
                // transparent rounded corners). The opacity is visually
                // imperceptible, and this pixel remains valid premultiplied BGRA.
                pixels[index] = 0x01000000;
            }
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

/// Single source of truth for the *actual painted pixels* of a rounded
/// panel or drag tab. Reused by the panel renderer and desktop glow mask.
fn panel_outer_shape_contains(
    width: i32, height: i32, x: i32, y: i32, corner_radius: u8,
) -> bool {
    let inset = sc(1).max(1);
    if x < inset || x >= width - inset || y < inset || y >= height - inset {
        return false;
    }
    let radius = sc(i32::from(corner_radius))
        .min(((width - inset * 2).min(height - inset * 2) / 2).max(0));
    radius == 0 || point_in_rounded_box(width, height, x, y, inset, radius)
}

/// The inner fill contour must be identical in rendering and halo
/// underpainting. No light is ever placed underneath this content area.
fn panel_inner_shape_contains(
    width: i32, height: i32, x: i32, y: i32, corner_radius: u8,
) -> bool {
    let outer_inset = sc(1).max(1);
    let inner_inset = outer_inset + PANEL_BORDER_WIDTH_PX;
    let outer_radius = sc(i32::from(corner_radius))
        .min(((width - outer_inset * 2).min(height - outer_inset * 2) / 2).max(0));
    if outer_radius > 0 {
        point_in_rounded_box(
            width, height, x, y, inner_inset,
            (outer_radius - PANEL_BORDER_WIDTH_PX).max(1),
        )
    } else {
        x >= inner_inset && x < width - inner_inset
            && y >= inner_inset && y < height - inner_inset
    }
}

fn blend_panel_bitmap(pixels: &mut [u32], width: i32, height: i32, style: &ThemeStyle) {
    let outer_inset = sc(1).max(1);
    let border = style.color(StyleColorTarget::PanelBorder);
    let fill = style.color(StyleColorTarget::PanelBackground);
    for y in outer_inset..(height - outer_inset).max(outer_inset) {
        for x in outer_inset..(width - outer_inset).max(outer_inset) {
            let outer_inside = panel_outer_shape_contains(
                width, height, x, y, style.panel_corner_radius,
            );
            if !outer_inside {
                continue;
            }
            let inner_inside = panel_inner_shape_contains(
                width, height, x, y, style.panel_corner_radius,
            );
            let idx = (y * width + x) as usize;
            pixels[idx] = blend_pixel(pixels[idx], if inner_inside { fill } else { border });
        }
    }
}

fn panel_color_at(style: &ThemeStyle, width: i32, height: i32, x: i32, y: i32) -> Option<Color> {
    if !panel_outer_shape_contains(width, height, x, y, style.panel_corner_radius) {
        return None;
    }
    if panel_inner_shape_contains(width, height, x, y, style.panel_corner_radius) {
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

/// Premultiplied BGRA source-over for layered-window pixels. Unlike a
/// direct assignment, a transparent rounded corner cannot erase the
/// previously rendered background glow. Solid content remains unchanged.
fn composite_premultiplied_pixel(dst: u32, src: u32) -> u32 {
    let alpha = (src >> 24) & 0xFF;
    if alpha == 0 { return dst; }
    if alpha == 255 { return src; }
    let inv = 255 - alpha;
    let da = (dst >> 24) & 0xFF;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;
    let out_a = alpha + (da * inv + 127) / 255;
    let out_r = (sr + (dr * inv + 127) / 255).min(255);
    let out_g = (sg + (dg * inv + 127) / 255).min(255);
    let out_b = (sb + (db * inv + 127) / 255).min(255);
    (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b
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

            // Keep the original border color. Hover light must only occupy
            // the actual fill *inside* that border, never recolor the outline.
            let border = style.color(StyleColorTarget::PanelBorder).blend_over(*bg);
            draw_panel(
                hdc,
                width,
                height,
                &border,
                &panel_base,
                style.panel_corner_radius,
            );
            if drag_handle_hovered
                && !lock_state().as_ref().is_some_and(|s| s.floating_mode)
            {
                draw_taskbar_hover_inner_glow_gdi(
                    hdc, width, height, &style, panel_base,
                );
            }
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

// Desktop fallback keeps an independent popup alive instead of
// detaching/reparenting Explorer's active taskbar child window.
const FLOAT_EDGE_SNAP: i32 = 24;
const FLOAT_EDGE_PEEK: i32 = 16;
const FLOAT_GRIP_LENGTH: i32 = 36;
// Preserve the component's content size. Transparent canvas gutters allow
// external glow on free desktops and on the inward-facing sides of docked
// panels. Reserved taskbar edges use an in-place clip instead of sliding.
const FLOAT_FREE_GLOW_MARGIN: i32 = 12;
const FLOAT_EXPAND_DURATION_MS: u64 = 220;
const FLOAT_RETRACT_DURATION_MS: u64 = 240;
const FLOAT_GRIP_FADE_IN_MS: u64 = 140;
const FLOAT_GRIP_FADE_OUT_MS: u64 = 170;
const TIMER_FLOAT_ANIMATION: usize = 0xF141;
// Armed only after a docked popup receives a potentially spurious mouse-leave.
// A 140ms recheck prevents edge-pixel expand/retract oscillation without the
// permanent 50ms cursor polling removed in V24.5.
const TIMER_FLOAT_HOVER_RECHECK: usize = 0xF142;
const FLOAT_HOVER_RECHECK_MS: u32 = 140;
const FLOAT_HOVER_EDGE_SLOP: i32 = 3;

/// This rectangle is used for the rendered tab, pointer hit testing and
/// collapsed alpha mask. Desktop-only, independent of the embedded taskbar grip.
fn floating_grip_rect(width: i32, height: i32, edge: Option<FloatingEdge>, peek: i32) -> RECT {
    let peek = peek.min(width.min(height)).max(1);
    let grip_width = sc(FLOAT_GRIP_LENGTH).min(width).max(1);
    let grip_height = sc(FLOAT_GRIP_LENGTH).min(height).max(1);
    match edge {
        Some(FloatingEdge::Left) => RECT {
            left: width - sc(FLOAT_FREE_GLOW_MARGIN) - peek,
            right: width - sc(FLOAT_FREE_GLOW_MARGIN),
            top: (height - grip_height) / 2, bottom: (height + grip_height) / 2,
        },
        Some(FloatingEdge::Right) => RECT {
            left: sc(FLOAT_FREE_GLOW_MARGIN),
            right: sc(FLOAT_FREE_GLOW_MARGIN) + peek,
            top: (height - grip_height) / 2, bottom: (height + grip_height) / 2,
        },
        Some(FloatingEdge::Top) => RECT {
            left: (width - grip_width) / 2, right: (width + grip_width) / 2,
            top: height - sc(FLOAT_FREE_GLOW_MARGIN) - peek,
            bottom: height - sc(FLOAT_FREE_GLOW_MARGIN),
        },
        Some(FloatingEdge::Bottom) => RECT {
            left: (width - grip_width) / 2, right: (width + grip_width) / 2,
            top: sc(FLOAT_FREE_GLOW_MARGIN),
            bottom: sc(FLOAT_FREE_GLOW_MARGIN) + peek,
        },
        None => RECT {
            left: (width - grip_width) / 2, right: (width + grip_width) / 2,
            top: sc(FLOAT_FREE_GLOW_MARGIN),
            bottom: sc(FLOAT_FREE_GLOW_MARGIN) + peek,
        },
    }
}

/// During hover, fully transparent layered pixels bypass HWND hit-testing.
/// A near-transparent (alpha=1) probe strip across the panel/tab gap allows
/// WM_NCHITTEST to observe lateral cursor movement away from the real tab.
/// Returning HTTRANSPARENT from that strip still forbids dragging there.
fn floating_hover_probe_rect(body: RECT, grip: RECT, edge: Option<FloatingEdge>) -> RECT {
    match edge {
        Some(FloatingEdge::Left) => RECT {
            left: body.right, right: grip.right,
            top: body.top, bottom: body.bottom,
        },
        Some(FloatingEdge::Right) => RECT {
            left: grip.left, right: body.left,
            top: body.top, bottom: body.bottom,
        },
        Some(FloatingEdge::Top) => RECT {
            left: body.left, right: body.right,
            top: body.bottom, bottom: grip.bottom,
        },
        Some(FloatingEdge::Bottom) | None => RECT {
            left: body.left, right: body.right,
            top: grip.top, bottom: body.top,
        },
    }
}

fn paint_floating_hover_hit_probe(
    pixels: &mut [u32], width: i32, height: i32,
    body: RECT, grip: RECT, edge: Option<FloatingEdge>,
) {
    if width <= 0 || height <= 0 { return; }
    let probe = floating_hover_probe_rect(body, grip, edge);
    for y in probe.top.max(0)..probe.bottom.min(height) {
        for x in probe.left.max(0)..probe.right.min(width) {
            if point_in_rect(body, x, y) || point_in_rect(grip, x, y) { continue; }
            let index = (y * width + x) as usize;
            if pixels[index] >> 24 == 0 {
                // Premultiplied black at 1/255 opacity is not visually
                // distinguishable from the surrounding desktop.
                pixels[index] = 0x01000000;
            }
        }
    }
}

/// One source of truth for where desktop users can start a drag or keep its
/// glow active, independent of the much larger layered HWND and its halo.
fn floating_drag_tab_hit_code(body: RECT, grip: RECT, x: i32, y: i32) -> i32 {
    if point_in_rect(body, x, y) || point_in_rect(grip, x, y) { 1 }
    else { -1 } // HTCLIENT or HTTRANSPARENT
}

// A stale WM_MOUSELEAVE can be queued when mouse capture ends or a layered
// popup is repositioned. Recheck where the pointer actually is, rather than
// clearing an otherwise valid hover until another WM_MOUSEMOVE arrives.
fn floating_pointer_still_over_free_grip(hwnd: HWND) -> bool {
    let grip = {
        let state = lock_state();
        let Some(s) = state.as_ref().filter(|s|
            s.floating_mode && s.hwnd.to_hwnd() == hwnd && s.floating_edge.is_none()
        ) else { return false };
        floating_state_grip_rect(s)
    };
    let Some(rect) = native_interop::get_window_rect_safe(hwnd) else {
        return false;
    };
    let mut pointer = POINT::default();
    if unsafe { GetCursorPos(&mut pointer) }.is_err() { return false; }
    point_in_rect(grip, pointer.x - rect.left, pointer.y - rect.top)
}

fn floating_free_grip_below_for_state(s: &AppState) -> bool {
    s.display_position == DisplayPosition::Desktop
        && s.floating_edge.is_none() && s.floating_free_grip_below
}

fn floating_state_grip_rect(s: &AppState) -> RECT {
    let (w, h, _) = floating_canvas_geometry_for_state(s);
    let grip = floating_layout_grip_rect(
        w, h, s.floating_edge, floating_free_grip_below_for_state(s),
    );
    if s.floating_reserved_dock {
        if let Some(edge) = s.floating_edge {
            let reveal = if s.dragging { s.floating_reserved_drag_reveal }
                else { s.floating_reveal };
            return floating_slid_canvas_rect(grip, w, h, edge, reveal);
        }
    }
    grip
}

fn floating_canvas_geometry_for_state(s: &AppState) -> (i32, i32, RECT) {
    let body_w = total_widget_width_for_state(s);
    let body_h = widget_height_for_state(s);
    if s.floating_edge.is_none() {
        floating_free_canvas_geometry(body_w, body_h, floating_free_grip_below_for_state(s))
    } else {
        floating_drag_canvas_geometry(
            body_w, body_h, s.floating_edge,
            s.dragging && s.floating_drag_glow_detached,
        )
    }
}

/// Check the body against the current docking boundary. At a taskbar edge
/// this is the desktop side of the taskbar, not the physical screen edge.
fn floating_drag_has_cleared_edge(
    window_x: i32, window_y: i32, body: RECT,
    bounds: RECT, edge: Option<FloatingEdge>,
) -> bool {
    let threshold = sc(2);
    match edge {
        Some(FloatingEdge::Left) => window_x + body.left >= bounds.left + threshold,
        Some(FloatingEdge::Right) => window_x + body.right <= bounds.right - threshold,
        Some(FloatingEdge::Top) => window_y + body.top >= bounds.top + threshold,
        Some(FloatingEdge::Bottom) => window_y + body.bottom <= bounds.bottom - threshold,
        None => false,
    }
}

/// Physical screen edges only need to restore the outer gutter when the
/// body moves back onto the desktop. A taskbar edge is INSIDE the monitor,
/// so the mouse can also pull the body across the reserved boundary toward
/// Explorer. That direction must restore the formerly clipped halo too.
///
/// Test the body's formerly clipped outer edge, not the cursor: a tab grabbed
/// away from the body can move the HWND by a different distance.
fn floating_drag_has_crossed_taskbar_edge(
    window_x: i32, window_y: i32, body: RECT,
    bounds: RECT, edge: Option<FloatingEdge>,
) -> bool {
    let threshold = sc(2);
    match edge {
        Some(FloatingEdge::Left) => window_x + body.left <= bounds.left - threshold,
        Some(FloatingEdge::Right) => window_x + body.right >= bounds.right + threshold,
        Some(FloatingEdge::Top) => window_y + body.top <= bounds.top - threshold,
        Some(FloatingEdge::Bottom) => window_y + body.bottom >= bounds.bottom + threshold,
        None => false,
    }
}

fn floating_canvas_size_for_state(state: &AppState) -> (i32, i32) {
    let (width, height, _) = floating_canvas_geometry_for_state(state);
    (width, height)
}

fn vertical_placement_for_gaps(
    preset: AppearancePreset, taskbar_rect: RECT, spans: &[VerticalSpan], desired: i32,
) -> Option<(AppearancePreset, i32)> {
    let normal = vertical_widget_height_for_preset(AppearancePreset::Default);
    let compact = vertical_widget_height_for_preset(AppearancePreset::Minimal);
    let choices = match preset {
        AppearancePreset::Minimal => [Some((AppearancePreset::Minimal, compact)), None],
        AppearancePreset::Default => [
            Some((AppearancePreset::Default, normal)),
            Some((AppearancePreset::Minimal, compact)),
        ],
    };
    for choice in choices.into_iter().flatten() {
        if let Some(top) = nearest_free_top_offset(taskbar_rect, spans, choice.1, desired) {
            return Some((choice.0, top));
        }
    }
    None
}

/// rcWork is useful for finding reserved edges but can differ from the
/// actual Explorer taskbar rectangle by a few pixels. For a taskbar attached
/// to the physical monitor edge, place the desktop popup at the taskbar's
/// *real* inner face; never leave a visible gap or enter the taskbar.
fn floating_usable_bounds(monitor: RECT, work: RECT) -> RECT {
    if work.left >= monitor.left && work.top >= monitor.top
        && work.right <= monitor.right && work.bottom <= monitor.bottom
        && work.right > work.left && work.bottom > work.top {
        work
    } else {
        monitor
    }
}

fn floating_bounds_at_taskbar(monitor: RECT, work: RECT, taskbars: &[RECT]) -> RECT {
    let mut bounds = floating_usable_bounds(monitor, work);
    let mw = monitor.right - monitor.left;
    let mh = monitor.bottom - monitor.top;
    for bar in taskbars {
        let overlap_x = (bar.right.min(monitor.right) - bar.left.max(monitor.left)).max(0);
        let overlap_y = (bar.bottom.min(monitor.bottom) - bar.top.max(monitor.top)).max(0);
        // Ignore taskbars on other monitors, tiny toolbars and interior windows.
        if overlap_x >= mw / 2 && bar.bottom >= monitor.bottom - 8
            && bar.top > monitor.top && bar.top < monitor.bottom {
            bounds.bottom = bar.top;
        } else if overlap_x >= mw / 2 && bar.top <= monitor.top + 8
            && bar.bottom > monitor.top && bar.bottom < monitor.bottom {
            bounds.top = bar.bottom;
        } else if overlap_y >= mh / 2 && bar.left <= monitor.left + 8
            && bar.right > monitor.left && bar.right < monitor.right {
            bounds.left = bar.right;
        } else if overlap_y >= mh / 2 && bar.right >= monitor.right - 8
            && bar.left > monitor.left && bar.left < monitor.right {
            bounds.right = bar.left;
        }
    }
    if bounds.right > bounds.left && bounds.bottom > bounds.top {
        bounds
    } else {
        floating_usable_bounds(monitor, work)
    }
}

/// An inset boundary is reserved by Windows. Collapse at a reserved edge
/// clips the panel inside the desktop, rather than sliding under Explorer.
fn floating_edge_is_reserved(monitor: RECT, bounds: RECT, edge: FloatingEdge) -> bool {
    match edge {
        FloatingEdge::Left => bounds.left > monitor.left,
        FloatingEdge::Right => bounds.right < monitor.right,
        FloatingEdge::Top => bounds.top > monitor.top,
        FloatingEdge::Bottom => bounds.bottom < monitor.bottom,
    }
}

fn floating_monitor_geometry(point: POINT) -> Option<(RECT, RECT)> {
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let taskbars = TASKBAR_EDGE_RECTS.lock().unwrap_or_else(|e| e.into_inner());
        Some((info.rcMonitor, floating_bounds_at_taskbar(
            info.rcMonitor, info.rcWork, &taskbars,
        )))
    }
}

fn floating_monitor_bounds(point: POINT) -> Option<RECT> {
    floating_monitor_geometry(point).map(|(_, usable)| usable)
}

// Persisted desktop coordinates are virtual-screen coordinates, independent
// from taskbar_monitor / taskbar_index. Use a point inside the last desktop
// canvas to resolve its monitor; a missing coordinate is not a valid anchor.
fn floating_restore_anchor(x: i32, y: i32, width: i32, height: i32) -> Option<POINT> {
    (x != i32::MIN && y != i32::MIN).then_some(POINT {
        x: x.saturating_add(width / 2),
        y: y.saturating_add(height / 2),
    })
}

// For an edge-docked popup, the saved coordinate is the expanded HWND
// origin INSIDE its work area. Its canvas center can end up on the adjacent
// monitor when the two screens have different DPI scales. Probe just inside
// that original screen, rather than the hidden popup's current HWND bounds.
fn floating_docked_monitor_anchor(
    edge: Option<FloatingEdge>, x: i32, y: i32, center: POINT,
) -> POINT {
    if edge.is_some() {
        POINT { x: x.saturating_add(1), y: y.saturating_add(1) }
    } else {
        center
    }
}

fn floating_monitor_dpi(point: POINT) -> Option<u32> {
    unsafe {
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONULL);
        if monitor == HMONITOR::default() { return None; }
        let mut x_dpi = 0;
        let mut y_dpi = 0;
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x_dpi, &mut y_dpi)
            .ok()?;
        (x_dpi > 0).then_some(x_dpi)
    }
}

// A disconnected monitor must not make the popup disappear off-screen.
// Let callers probe a saved point with MONITOR_DEFAULTTONULL, rather than
// MONITOR_DEFAULTTONEAREST (which silently chooses the taskbar screen).
fn floating_restore_bounds(
    saved_anchor: Option<POINT>,
    taskbar_bounds: RECT,
    bounds_at_saved_anchor: impl FnOnce(POINT) -> Option<RECT>,
) -> RECT {
    saved_anchor.and_then(bounds_at_saved_anchor).unwrap_or(taskbar_bounds)
}

/// Detect whether the selected monitor's edge is physically exposed.
/// At a shared display seam we still DOCK; the neighboring monitor only
/// determines whether to alpha-clip the slide inside the selected monitor.
/// Sample all three portions of the grip to handle partially shared edges.
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

/// A taskbar or an adjacent display makes the edge a visual clip
/// boundary, not a no-dock boundary. Only the mouse's chosen monitor may
/// show the retracted tab; the other screen receives no overflow pixels.
fn floating_edge_needs_clip(
    monitor: RECT, bounds: RECT, edge: FloatingEdge, has_neighbor: bool,
) -> bool {
    floating_edge_is_reserved(monitor, bounds, edge) || has_neighbor
}

fn floating_edge_requires_clip(
    monitor: RECT, bounds: RECT, edge: FloatingEdge, pointer: POINT,
) -> bool {
    floating_edge_needs_clip(
        monitor, bounds, edge, !floating_edge_exposed(monitor, edge, pointer),
    )
}

fn floating_clamp_position(bounds: RECT, width: i32, height: i32, x: i32, y: i32) -> (i32, i32) {
    (
        x.clamp(bounds.left, (bounds.right - width).max(bounds.left)),
        y.clamp(bounds.top, (bounds.bottom - height).max(bounds.top)),
    )
}

// A single geometric definition of the *painted* widget and drag tab.
// Never include the transparent glow gutter in visibility or edge-contact
// calculations. Reuse this rectangle in both positioning and docking so
// changing the free tab side cannot create mismatched edge decisions.
fn floating_visible_content_rect(
    width: i32, height: i32, body: RECT,
    edge: Option<FloatingEdge>, free_below: bool,
) -> RECT {
    let grip = floating_layout_grip_rect(width, height, edge, free_below);
    RECT {
        left: body.left.min(grip.left),
        top: body.top.min(grip.top),
        right: body.right.max(grip.right),
        bottom: body.bottom.max(grip.bottom),
    }
}

// The free desktop canvas has transparent glow gutters on every side.
// Clamping the whole HWND makes the visible panel stop one glow margin away
// from each screen edge. Only the real panel and drag tab must stay within
// the selected monitor's usable desktop; transparent pixels may be offscreen.
// The currently dragged canvas retains its previous dock orientation until
// mouse release. Its actual tab can therefore be BELOW or BESIDE the panel,
// whereas the future free canvas would put the tab ABOVE it. Visibility and
// contact decisions must use that current orientation, not the future one.
#[allow(clippy::too_many_arguments)]
fn floating_clamp_drag_visible_position(
    bounds: RECT, width: i32, height: i32, body: RECT,
    edge: Option<FloatingEdge>, free_below: bool, x: i32, y: i32,
) -> (i32, i32) {
    let visible = floating_visible_content_rect(width, height, body, edge, free_below);
    let min_x = bounds.left.saturating_sub(visible.left);
    let max_x = bounds.right.saturating_sub(visible.right).max(min_x);
    let min_y = bounds.top.saturating_sub(visible.top);
    let max_y = bounds.bottom.saturating_sub(visible.bottom).max(min_y);
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

// Explicit Desktop docking requires contact between the body/drag tab that
// were ACTUALLY displayed at mouse release and the work-area boundary.
// Never substitute the future free-floating canvas before this check.
#[allow(clippy::too_many_arguments)]
fn floating_desktop_contact_edge(
    bounds: RECT, width: i32, height: i32, body: RECT,
    edge: Option<FloatingEdge>, free_below: bool, origin: POINT, pointer: POINT,
) -> Option<FloatingEdge> {
    let visible = floating_visible_content_rect(width, height, body, edge, free_below);
    [
        (FloatingEdge::Left, origin.x + visible.left == bounds.left,
            pointer.x.abs_diff(bounds.left)),
        (FloatingEdge::Right, origin.x + visible.right == bounds.right,
            pointer.x.abs_diff(bounds.right)),
        (FloatingEdge::Top, origin.y + visible.top == bounds.top,
            pointer.y.abs_diff(bounds.top)),
        (FloatingEdge::Bottom, origin.y + visible.bottom == bounds.bottom,
            pointer.y.abs_diff(bounds.bottom)),
    ].into_iter()
        .filter(|(_, touching, _)| *touching)
        .min_by_key(|(_, _, distance)| *distance)
        .map(|(edge, _, _)| edge)
}

// The visible free tab must have priority over choosing a new tab side:
// when the TOP tab already touches the top edge, flipping it BELOW the body
// would erase a legitimate docking contact. Same rule for a BELOW-body tab
// reaching the taskbar. Only actual visible contact counts; no snap radius.
fn floating_existing_free_tab_contact(
    bounds: RECT, width: i32, height: i32, body: RECT,
    free_below: bool, origin: POINT, pointer: POINT,
) -> Option<FloatingEdge> {
    let (x, y) = floating_clamp_drag_visible_position(
        bounds, width, height, body, None, free_below, origin.x, origin.y,
    );
    floating_desktop_contact_edge(
        bounds, width, height, body, None, free_below, POINT { x, y }, pointer,
    ).filter(|edge| matches!(
        (free_below, edge),
        (false, FloatingEdge::Top) | (true, FloatingEdge::Bottom)
    ))
}

/// The desktop reset target is the horizontal center of the selected
/// monitor's usable desktop, snapped against its top work-area boundary.
/// Keep virtual-desktop coordinates signed for screens left/above primary.
fn floating_reset_top_origin(bounds: RECT, width: i32, height: i32) -> (i32, i32) {
    let centered_x = (i64::from(bounds.left) +
        (i64::from(bounds.right) - i64::from(bounds.left) - i64::from(width)) / 2)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
    floating_edge_position(
        bounds, centered_x, bounds.top, width, height,
        Some(FloatingEdge::Top), false, sc(FLOAT_EDGE_PEEK),
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

/// Select the edge by the RELEASE CURSOR's perpendicular distance.
/// At screen corners two eligible boundaries can meet. The old logic picked
/// Left before Top/Bottom regardless of the actual cursor position, while a
/// reserved taskbar band could suppress the adjoining physical-screen edge.
/// Use the same distance rule for all four corners and every taskbar location.
///
/// The layered HWND rectangle remains an eligibility signal so detached-grip
/// drags retain their previous snapping behavior. The cursor additionally
/// qualifies an edge when close to its line or inside a reserved taskbar band,
/// even if the entire HWND has passed outside the work area.
fn floating_drop_edge_at_pointer(
    monitor: RECT, bounds: RECT, rect: RECT, pointer: POINT,
    threshold: i32, exposed: [bool; 4],
) -> Option<FloatingEdge> {
    let within_monitor = pointer.x >= monitor.left && pointer.x < monitor.right
        && pointer.y >= monitor.top && pointer.y < monitor.bottom;
    if !within_monitor {
        // A drag released beyond every monitor retains the legacy nearest
        // monitor rectangle fallback. Inside any monitor the actual cursor
        // distance and chosen display's four sides always take precedence.
        return floating_drop_edge(bounds, rect, threshold, exposed);
    }
    let distances = [
        pointer.x.abs_diff(bounds.left),
        pointer.x.abs_diff(bounds.right),
        pointer.y.abs_diff(bounds.top),
        pointer.y.abs_diff(bounds.bottom),
    ];
    let snap_distance = threshold.max(0) as u32;
    let eligible = [
        (
            FloatingEdge::Left, exposed[0],
            (rect.left < bounds.left && rect.right > bounds.left)
                || rect.left.abs_diff(bounds.left) <= snap_distance,
            within_monitor && (distances[0] <= snap_distance
                || (bounds.left > monitor.left && pointer.x < bounds.left)),
            distances[0],
        ),
        (
            FloatingEdge::Right, exposed[1],
            (rect.right > bounds.right && rect.left < bounds.right)
                || rect.right.abs_diff(bounds.right) <= snap_distance,
            within_monitor && (distances[1] <= snap_distance
                || (bounds.right < monitor.right && pointer.x >= bounds.right)),
            distances[1],
        ),
        (
            FloatingEdge::Top, exposed[2],
            (rect.top < bounds.top && rect.bottom > bounds.top)
                || rect.top.abs_diff(bounds.top) <= snap_distance,
            within_monitor && (distances[2] <= snap_distance
                || (bounds.top > monitor.top && pointer.y < bounds.top)),
            distances[2],
        ),
        (
            FloatingEdge::Bottom, exposed[3],
            (rect.bottom > bounds.bottom && rect.top < bounds.bottom)
                || rect.bottom.abs_diff(bounds.bottom) <= snap_distance,
            within_monitor && (distances[3] <= snap_distance
                || (bounds.bottom < monitor.bottom && pointer.y >= bounds.bottom)),
            distances[3],
        ),
    ];
    eligible.into_iter()
        .filter(|(_, allowed, hwnd_near, mouse_near, _)|
            *allowed && (*hwnd_near || *mouse_near))
        .min_by_key(|(_, _, _, _, distance)| *distance)
        .map(|(edge, _, _, _, _)| edge)
}

// The stable expanded position is always in screen coordinates. Only the
// visible sliver of a docked popup is on-screen while collapsed.
#[allow(clippy::too_many_arguments)]
fn floating_edge_position(
    bounds: RECT, x: i32, y: i32, width: i32, height: i32,
    edge: Option<FloatingEdge>, collapsed: bool, peek: i32,
) -> (i32, i32) {
    let (x, y) = floating_clamp_position(bounds, width, height, x, y);
    let grip = floating_grip_rect(width, height, edge, peek);
    match edge {
        Some(FloatingEdge::Left) => (
            if collapsed { bounds.left - grip.left } else { bounds.left }, y,
        ),
        Some(FloatingEdge::Right) => (
            if collapsed { bounds.right - grip.right } else { bounds.right - width }, y,
        ),
        Some(FloatingEdge::Top) => (
            x, if collapsed { bounds.top - grip.top } else { bounds.top },
        ),
        Some(FloatingEdge::Bottom) => (
            x, if collapsed { bounds.bottom - grip.bottom } else { bounds.bottom - height },
        ),
        None => (x, y),
    }
}

fn refresh_floating_reserved_dock(hwnd: HWND) {
    let (x, y, w, h, edge) = {
        let state = lock_state();
        let Some(s) = state.as_ref().filter(|s| s.floating_mode && !s.dragging) else { return };
        let (w, h) = floating_canvas_size_for_state(s);
        (s.floating_x, s.floating_y, w, h, s.floating_edge)
    };
    let point = POINT {
        x: x.saturating_add(w / 2),
        y: y.saturating_add(h / 2),
    };
    let reserved = edge.is_some_and(|edge| floating_monitor_geometry(point)
        .is_some_and(|(monitor, bounds)| {
            floating_edge_requires_clip(monitor, bounds, edge, point)
        }));
    let mut state = lock_state();
    if let Some(s) = state.as_mut() {
        if !s.floating_mode || s.dragging || s.hwnd.to_hwnd() != hwnd { return; }
        s.floating_reserved_dock = reserved;
    }
}

fn move_floating_widget(hwnd: HWND) {
    move_floating_widget_with_visibility(hwnd, true);
}

// When restoring a cached desktop HWND, position it while still hidden.
// Its previous layered bitmap may be expanded; only show it after the new
// collapsed/expanded frame has been committed with UpdateLayeredWindow.
fn move_floating_widget_with_visibility(hwnd: HWND, show: bool) {
    let (x, y, edge, reveal, width, height, dragging) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        if !s.floating_mode { return; }
        let (width, height) = floating_canvas_size_for_state(s);
        (s.floating_x, s.floating_y, s.floating_edge, s.floating_reveal,
         width, height, s.dragging)
    };
    let reserved = lock_state().as_ref().is_some_and(|s| s.floating_reserved_dock);
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
        if reserved {
            // For Explorer or adjacent-monitor edges, slide bitmap pixels
            // inside the chosen monitor instead of moving HWND across a seam.
            (expanded_x, expanded_y)
        } else {
            // Both paths use the exact same signed slide distance and easing.
            let (dx, dy) = edge.map(|edge|
                floating_slide_offset(width, height, edge, reveal)
            ).unwrap_or((0, 0));
            (expanded_x + dx, expanded_y + dy)
        }
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
                if show { SWP_NOACTIVATE | SWP_SHOWWINDOW } else { SWP_NOACTIVATE },
            );
        }
    }
    if show && blur {
        unsafe { let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE); }
    }
}

// A docked panel whose auto-collapse is disabled stays fully expanded, but
// its separate drag tab behaves visually just like a free desktop tab.
fn floating_grip_uses_hover_fade(s: &AppState) -> bool {
    s.floating_edge.is_none() || !floating_auto_collapse_allowed(
        s.display_position, s.desktop_auto_collapse,
    )
}

fn floating_grip_should_show(
    edge: Option<FloatingEdge>, auto_collapse: bool, hovered: bool, dragging: bool,
) -> bool {
    dragging || (edge.is_some() && auto_collapse) || hovered
}

/// Both the foreground drag tab and its independent Composition blur
/// background must use exactly the same visual opacity. In particular an
/// expanded edge-docked desktop with auto-collapse disabled fades *both*
/// layers out, rather than leaving a permanently visible frosted plate.
fn floating_grip_visual_alpha(hover_fade: bool, dragging: bool, animated_alpha: u8) -> u8 {
    if dragging || !hover_fade { 100 } else { animated_alpha.min(100) }
}

/// Layered desktop pixels may stop delivering WM_MOUSELEAVE after docking.
/// While a non-collapsing docked tab is hovered, poll the actual cursor so
/// a lost leave event cannot keep the detached grip visible indefinitely.
fn docked_grip_needs_hover_watch(
    edge: Option<FloatingEdge>, auto_collapse: bool, collapsed: bool, dragging: bool,
) -> bool {
    edge.is_some() && !auto_collapse && !collapsed && !dragging
}

fn set_floating_grip_hover(hovered: bool) {
    let hwnd = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        if !s.floating_mode || s.dragging || !floating_grip_uses_hover_fade(s)
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

// In fallback desktop mode (taskbar has no free space), preserve the
// existing auto-hide behavior. This option only affects explicitly selected
// Desktop mode, not Explorer's taskbar and not the docking geometry.
fn floating_auto_collapse_allowed(position: DisplayPosition, enabled: bool) -> bool {
    position != DisplayPosition::Desktop || enabled
}

// A docked desktop popup must enter with its *final* visibility; initializing
// at 100% reveal before sync_desktop_auto_collapse() momentarily exposes the
// entire panel and then starts a needless contraction animation.
fn initial_floating_visibility(
    edge: Option<FloatingEdge>, position: DisplayPosition, auto_collapse: bool,
) -> (bool, u8) {
    let collapsed = edge.is_some() && floating_auto_collapse_allowed(position, auto_collapse);
    (collapsed, if collapsed { 0 } else { 100 })
}

// The top tab has a different transparent canvas size from free and
// left/right-docked tabs. Reset must calculate the *destination* canvas
// BEFORE applying the hidden tab's slide distance; using the source canvas
// pushes the free tab partly offscreen and leaves side tabs below the edge.
fn floating_reset_top_canvas_size(body_width: i32, body_height: i32) -> (i32, i32) {
    let (width, height, _) = floating_canvas_geometry(
        body_width, body_height, Some(FloatingEdge::Top),
    );
    (width, height)
}

/// Reset a desktop popup without changing the stored taskbar offsets.
/// The saved expanded desktop origin identifies its screen even if the
/// current collapsed HWND is partially outside the monitor or across a seam.
fn reset_floating_widget_position() -> bool {
    let snapshot = {
        let state = lock_state();
        state.as_ref().and_then(|s| {
            if !s.floating_mode || s.dragging { return None; }
            let (width, height) = floating_canvas_size_for_state(s);
            let anchor = floating_restore_anchor(s.floating_x, s.floating_y, width, height)
                .map(|center| floating_docked_monitor_anchor(
                    s.floating_edge, s.floating_x, s.floating_y, center,
                ))
                .or_else(|| native_interop::get_window_rect_safe(s.hwnd.to_hwnd())
                    .map(|r| POINT {
                        x: r.left.saturating_add(width / 2),
                        y: r.top.saturating_add(height / 2),
                    }))?;
            let body_w = total_widget_width_for_state(s);
            let body_h = widget_height_for_state(s);
            let (reset_width, reset_height) =
                floating_reset_top_canvas_size(body_w, body_h);
            Some((s.hwnd.to_hwnd(), anchor, reset_width, reset_height,
                s.display_position, s.desktop_auto_collapse))
        })
    };
    let Some((hwnd, anchor, width, height, position, auto_collapse)) = snapshot
    else { return false; };
    let Some((monitor, bounds)) = floating_monitor_geometry(anchor)
    else { return false; };
    // An explicit reset deliberately places the popup at the top boundary.
    let (x, y) = floating_reset_top_origin(bounds, width, height);
    let edge = Some(FloatingEdge::Top);
    let (collapsed, reveal) = initial_floating_visibility(edge, position, auto_collapse);
    let pointer = POINT {
        x: bounds.left.saturating_add((bounds.right - bounds.left) / 2),
        y: bounds.top.saturating_add(1).min(bounds.bottom.saturating_sub(1)),
    };
    let reserved = edge.is_some_and(|edge|
        floating_edge_requires_clip(monitor, bounds, edge, pointer));
    let (dx, dy) = if reserved {
        (0, 0)
    } else {
        edge.map(|edge| floating_slide_offset(width, height, edge, reveal))
            .unwrap_or((0, 0))
    };
    let origin = POINT { x: x.saturating_add(dx), y: y.saturating_add(dy) };

    hide_minimal_usage_tooltip();
    floating_cancel_hover_recheck(hwnd);
    unsafe {
        let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION);
    }
    {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return false; };
        if !s.floating_mode || s.hwnd.to_hwnd() != hwnd || s.dragging { return false; }
        s.floating_x = x;
        s.floating_y = y;
        s.floating_edge = edge;
        s.floating_free_grip_below = false;
        s.floating_reserved_dock = reserved;
        s.floating_drag_glow_detached = false;
        s.floating_reserved_drag_reveal = reveal;
        s.floating_collapsed = collapsed;
        s.floating_reveal = reveal;
        s.floating_slide_from = reveal;
        s.floating_slide_target = reveal;
        s.floating_slide_started = Instant::now();
        s.floating_first_collapse_guard = false;
        s.drag_handle_hovered = false;
        s.floating_grip_hover = false;
        s.floating_grip_alpha = if floating_grip_should_show(
            edge, floating_auto_collapse_allowed(position, auto_collapse),
            false, false,
        ) { 100 } else { 0 };
        s.floating_fade_from = s.floating_grip_alpha;
        s.floating_fade_target = s.floating_grip_alpha;
        s.floating_fade_started = Instant::now();
    }

    // Publish new dock orientation, size, alpha mask and screen location in
    // one UpdateLayeredWindow. SetWindowPos first could briefly show the old
    // expanded frame or the previous tab at the wrong coordinates.
    let _placement_guard = FloatingInitialPlacementGuard::begin();
    render_layered_at(Some(origin));
    if lock_state().as_ref().is_some_and(|s| s.composition_blur_active) {
        sync_blur_backdrop_zorder(hwnd);
    }
    save_state_settings();
    diagnose::log_lazy(|| format!(
        "desktop position reset to top center at ({x},{y}) monitor=({}, {}, {}, {}) collapsed={collapsed} clipped={reserved}",
        monitor.left, monitor.top, monitor.right, monitor.bottom,
    ));
    true
}

fn sync_desktop_auto_collapse() {
    let (hwnd, collapse) = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        if !s.floating_mode || s.floating_edge.is_none() { return; }
        let collapse = floating_auto_collapse_allowed(
            s.display_position, s.desktop_auto_collapse,
        );
        if !collapse {
            // An expanded, edge-docked widget must fade out its tab when
            // auto-collapse is switched off, even if the body never moves.
            // Otherwise set_floating_collapsed(false) can be a no-op and
            // leave the previous fully opaque drag tab on screen.
            s.floating_grip_hover = false;
            if !s.dragging {
                s.drag_handle_hovered = false;
            }
        }
        s.floating_first_collapse_guard = false;
        (s.hwnd.to_hwnd(), collapse)
    };
    floating_cancel_hover_recheck(hwnd);
    set_floating_collapsed(collapse);
    if !collapse {
        // The collapse state may already be false: explicitly drive the
        // opacity transition independently of the slide animation.
        unsafe {
            let _ = SetTimer(
                taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION, 16, None,
            );
        }
    }
}

// A nonmodal custom context menu is a separate HWND. Moving from the
// expanded panel into that menu must not count as a hover exit.
fn floating_context_menu_blocks_collapse(hwnd: HWND) -> bool {
    popup_menu::is_open_for(hwnd) && lock_state().as_ref().is_some_and(|s|
        s.floating_mode && s.hwnd.to_hwnd() == hwnd
            && s.floating_edge.is_some() && !s.dragging
    )
}

fn set_floating_collapsed(collapsed: bool) {
    let hwnd = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        if collapsed && popup_menu::is_open_for(s.hwnd.to_hwnd()) {
            return;
        }
        if !s.floating_mode || s.dragging || s.floating_edge.is_none()
            || (collapsed && !floating_auto_collapse_allowed(
                s.display_position, s.desktop_auto_collapse,
            ))
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
        let grip_target = if floating_grip_should_show(
            s.floating_edge,
            floating_auto_collapse_allowed(s.display_position, s.desktop_auto_collapse),
            s.floating_grip_hover, s.dragging,
        ) { 100 } else { 0 };
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
         s.floating_reveal != old_reveal && !s.floating_reserved_dock,
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

/// Like free desktop hover retention, keep an expanded, edge-docked panel's
/// hidden tab visible while the pointer is anywhere on its real panel or tab.
/// This is only used when automatic edge collapse was explicitly disabled.
/// The exterior halo gutter is never a hover target.
fn floating_pointer_over_expanded_docked_panel(hwnd: HWND) -> bool {
    let (body, grip) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return false };
        if !s.floating_mode || s.hwnd.to_hwnd() != hwnd
            || s.floating_edge.is_none() || s.floating_collapsed
            || !floating_grip_uses_hover_fade(s)
        {
            return false;
        }
        let (w, h, body) = floating_canvas_geometry_for_state(s);
        let body = if s.floating_reserved_dock {
            floating_slid_canvas_rect(
                body, w, h, s.floating_edge.unwrap(),
                if s.dragging { s.floating_reserved_drag_reveal } else { s.floating_reveal },
            )
        } else {
            body
        };
        (body, floating_state_grip_rect(s))
    };
    let Some(rect) = native_interop::get_window_rect_safe(hwnd) else { return false };
    let mut pointer = POINT::default();
    if unsafe { GetCursorPos(&mut pointer) }.is_err() { return false; }
    floating_drag_tab_hit_code(
        body, grip, pointer.x - rect.left, pointer.y - rect.top,
    ) == 1
}

/// The fully expanded panel and edge tab may be separated by an alpha-zero
/// gutter. That gutter is NOT a click target, but its traversal must not hide
/// the tab before the pointer reaches the actual draggable pixels.
fn floating_transition_corridor_contains(
    body: RECT, grip: RECT, edge: FloatingEdge, x: i32, y: i32,
) -> bool {
    !point_in_rect(body, x, y)
        && !point_in_rect(grip, x, y)
        && point_in_rect(floating_hover_probe_rect(body, grip, Some(edge)), x, y)
}

/// Geometry rectangles abut, but their actual rounded silhouettes each
/// have an entirely transparent one-pixel outer inset. The two adjacent
/// alpha-zero pixel rows/columns form a native mouse-leave seam that is NOT
/// covered by the rectangle-only hover transition probe above.
///
/// This is a *hover recheck* region, never an extra hit-test surface: no
/// pixels are painted and no drag targets are widened.
fn floating_transparent_panel_grip_seam_contains(
    body: RECT, grip: RECT, edge: FloatingEdge, x: i32, y: i32,
) -> bool {
    let inset = sc(1).max(1);
    match edge {
        FloatingEdge::Left => {
            y >= body.top.max(grip.top) && y < body.bottom.min(grip.bottom)
                && x >= body.right - inset && x < grip.left + inset
        }
        FloatingEdge::Right => {
            y >= body.top.max(grip.top) && y < body.bottom.min(grip.bottom)
                && x >= grip.right - inset && x < body.left + inset
        }
        FloatingEdge::Top => {
            x >= body.left.max(grip.left) && x < body.right.min(grip.right)
                && y >= body.bottom - inset && y < grip.top + inset
        }
        FloatingEdge::Bottom => {
            x >= body.left.max(grip.left) && x < body.right.min(grip.right)
                && y >= grip.bottom - inset && y < body.top + inset
        }
    }
}

fn floating_pointer_in_expanded_docked_transition(hwnd: HWND) -> bool {
    let (body, grip, edge) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return false };
        if !s.floating_mode || s.hwnd.to_hwnd() != hwnd
            || s.floating_collapsed || !floating_grip_uses_hover_fade(s)
        {
            return false;
        }
        let Some(edge) = s.floating_edge else { return false };
        let (w, h, body) = floating_canvas_geometry_for_state(s);
        let body = if s.floating_reserved_dock {
            floating_slid_canvas_rect(
                body, w, h, edge,
                if s.dragging { s.floating_reserved_drag_reveal } else { s.floating_reveal },
            )
        } else { body };
        (body, floating_state_grip_rect(s), edge)
    };
    let Some(rect) = native_interop::get_window_rect_safe(hwnd) else { return false };
    let mut pointer = POINT::default();
    if unsafe { GetCursorPos(&mut pointer) }.is_err() { return false; }
    let x = pointer.x - rect.left;
    let y = pointer.y - rect.top;
    floating_transition_corridor_contains(body, grip, edge, x, y)
        || floating_transparent_panel_grip_seam_contains(body, grip, edge, x, y)
}

fn floating_pointer_over_actual_grip(hwnd: HWND) -> bool {
    let grip = {
        let state = lock_state();
        let Some(s) = state.as_ref().filter(|s|
            s.floating_mode && s.hwnd.to_hwnd() == hwnd
        ) else { return false };
        floating_state_grip_rect(s)
    };
    let Some(rect) = native_interop::get_window_rect_safe(hwnd) else { return false };
    let mut pointer = POINT::default();
    if unsafe { GetCursorPos(&mut pointer) }.is_err() { return false; }
    point_in_rect(grip, pointer.x - rect.left, pointer.y - rect.top)
}

/// False while the pointer still overlaps the collapsed exposed edge grip.
/// The cursor's hover *retention* area must not change with animation.
/// In particular a transparent pixel at the physical monitor edge can stop
/// receiving WM_MOUSEMOVE after expansion despite still being inside the
/// visually expanded panel. Include both the expanded panel and the collapsed
/// tab, allowing only a small inward tolerance and never crossing the chosen
/// monitor's work-area boundary.
fn floating_docked_hover_rect(
    expanded_body: RECT, collapsed_grip: RECT, bounds: RECT, slack: i32,
) -> RECT {
    let slack = slack.max(0);
    RECT {
        left: expanded_body.left.min(collapsed_grip.left)
            .saturating_sub(slack).max(bounds.left),
        right: expanded_body.right.max(collapsed_grip.right)
            .saturating_add(slack).min(bounds.right),
        top: expanded_body.top.min(collapsed_grip.top)
            .saturating_sub(slack).max(bounds.top),
        bottom: expanded_body.bottom.max(collapsed_grip.bottom)
            .saturating_add(slack).min(bounds.bottom),
    }
}

fn floating_docked_hover_rect_for_geometry(
    bounds: RECT, x: i32, y: i32, width: i32, height: i32,
    body: RECT, edge: FloatingEdge,
) -> RECT {
    let (x, y) = floating_edge_position(
        bounds, x, y, width, height, Some(edge), false, sc(FLOAT_EDGE_PEEK),
    );
    let grip = floating_grip_rect(width, height, Some(edge), sc(FLOAT_EDGE_PEEK));
    let (dx, dy) = floating_slide_offset(width, height, edge, 0);
    let expanded_body = RECT {
        left: x + body.left, right: x + body.right,
        top: y + body.top, bottom: y + body.bottom,
    };
    let collapsed_grip = RECT {
        left: x + grip.left + dx, right: x + grip.right + dx,
        top: y + grip.top + dy, bottom: y + grip.bottom + dy,
    };
    floating_docked_hover_rect(
        expanded_body, collapsed_grip, bounds, sc(FLOAT_HOVER_EDGE_SLOP),
    )
}

fn floating_docked_hover_rect_for_state(s: &AppState) -> Option<RECT> {
    let edge = s.floating_edge?;
    let (width, height, body) = floating_canvas_geometry_for_state(s);
    let center = POINT {
        x: s.floating_x.saturating_add(width / 2),
        y: s.floating_y.saturating_add(height / 2),
    };
    let (_, bounds) = floating_monitor_geometry(center)?;
    Some(floating_docked_hover_rect_for_geometry(
        bounds, s.floating_x, s.floating_y, width, height, body, edge,
    ))
}

/// Unlike HWND alpha-hit testing, the docked hover envelope is stable for the
/// entire slide. In the rare case WM_MOUSELEAVE fires inside this envelope,
/// keep checking *only while the leave is unresolved*, so a later exit from
/// transparent pixels can still collapse the widget.
fn floating_pointer_in_docked_hover_zone(hwnd: HWND) -> bool {
    let hover = {
        let state = lock_state();
        state.as_ref().filter(|s|
            s.floating_mode && s.hwnd.to_hwnd() == hwnd && !s.dragging
        ).and_then(floating_docked_hover_rect_for_state)
    };
    let Some(hover) = hover else { return false; };
    let mut cursor = POINT::default();
    if unsafe { GetCursorPos(&mut cursor) }.is_err() { return false; }
    point_in_rect(hover, cursor.x, cursor.y)
}

fn floating_arm_hover_recheck(hwnd: HWND) {
    unsafe {
        let _ = SetTimer(taskbar_replica_service_hwnd(hwnd),
            TIMER_FLOAT_HOVER_RECHECK, FLOAT_HOVER_RECHECK_MS, None);
    }
}

fn floating_cancel_hover_recheck(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_HOVER_RECHECK);
    }
}

fn floating_pointer_is_outside_grip(hwnd: HWND) -> bool {
    let grip = {
        let state = lock_state();
        let Some(s) = state.as_ref().filter(|s| s.floating_mode) else { return false };
        if s.floating_edge.is_none() { return true; }
        floating_state_grip_rect(s)
    };
    let Some(rect) = native_interop::get_window_rect_safe(hwnd) else { return false };
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_err() { return false; }
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

// Preserve the cursor's position *within the same part of the drag tab*
// across per-monitor DPI transitions. The anchor is in physical canvas pixels;
// after a scale change keeping it unchanged shifts the grip away from the
// cursor. Negative signed offsets (e.g. an initially clipped edge tab) are
// supported as well.
fn floating_drag_anchor_for_dpi(anchor: i32, from_dpi: u32, to_dpi: u32) -> i32 {
    let numerator = i64::from(anchor) * i64::from(to_dpi.max(1));
    let denominator = i64::from(from_dpi.max(1));
    let scaled = if numerator >= 0 {
        (numerator + denominator / 2) / denominator
    } else {
        -((-numerator + denominator / 2) / denominator)
    };
    scaled.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn update_floating_drag(hwnd: HWND, pointer: POINT) {
    // The *pointer* chooses the next screen, not the layered HWND center:
    // when a wide component straddles a seam, those can disagree for many
    // frames. Never send a dragging popup through position_at_taskbar().
    let previous_dpi = CURRENT_DPI.load(Ordering::Relaxed).max(1);
    let target_dpi = floating_monitor_dpi(pointer).unwrap_or(previous_dpi);
    let dpi_changed = target_dpi != previous_dpi;
    if dpi_changed {
        CURRENT_DPI.store(target_dpi, Ordering::Relaxed);
    }
    let (anchor_x, anchor_y, edge, free_below, body_w, body_h, unlocked, reserved, drag_reveal) = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        if !s.floating_mode || !s.dragging || s.hwnd.to_hwnd() != hwnd { return; }
        if dpi_changed {
            s.floating_drag_anchor_x = floating_drag_anchor_for_dpi(
                s.floating_drag_anchor_x, previous_dpi, target_dpi,
            );
            s.floating_drag_anchor_y = floating_drag_anchor_for_dpi(
                s.floating_drag_anchor_y, previous_dpi, target_dpi,
            );
            if s.vertical_taskbar_mode {
                s.vertical_widget_width = sc(VERTICAL_WIDGET_MIN_WIDTH).max(sc(48));
            }
        }
        (
            s.floating_drag_anchor_x, s.floating_drag_anchor_y, s.floating_edge,
            floating_free_grip_below_for_state(s),
            total_widget_width_for_state(s), widget_height_for_state(s),
            s.floating_drag_glow_detached, s.floating_reserved_dock,
            s.floating_reserved_drag_reveal,
        )
    };
    let (old_w, old_h, old_body) = if edge.is_none() {
        floating_free_canvas_geometry(body_w, body_h, free_below)
    } else {
        floating_drag_canvas_geometry(body_w, body_h, edge, unlocked)
    };
    // A clipped grip (taskbar or monitor seam) has a virtual pixel offset
    // inside a stationary HWND. On the first drag frame, transfer that to the HWND origin
    // and rebase the mouse anchor atomically, without moving the actual grip
    // on the screen. After this the normal drag path is identical everywhere.
    let (slide_x, slide_y) = if reserved {
        edge.map(|edge| floating_slide_offset(old_w, old_h, edge, drag_reveal))
            .unwrap_or((0, 0))
    } else { (0, 0) };
    let mut new_anchor_x = anchor_x - slide_x;
    let mut new_anchor_y = anchor_y - slide_y;
    let window_x = pointer.x - new_anchor_x;
    let window_y = pointer.y - new_anchor_y;
    let should_unlock = !unlocked && floating_monitor_geometry(pointer)
        .is_some_and(|(_, bounds)| {
            floating_drag_has_cleared_edge(
                window_x, window_y, old_body, bounds, edge,
            ) || (reserved && floating_drag_has_crossed_taskbar_edge(
                window_x, window_y, old_body, bounds, edge,
            ))
        });
    if should_unlock {
        let (_, _, expanded_body) =
            floating_drag_canvas_geometry(body_w, body_h, edge, true);
        // Expanding the canvas must not move the actual panel or the tab
        // under the pointer. Rebase the original mouse-to-HWND offset by
        // precisely the shift of the real panel content.
        new_anchor_x += expanded_body.left - old_body.left;
        new_anchor_y += expanded_body.top - old_body.top;
    }
    let new_x = pointer.x - new_anchor_x;
    let new_y = pointer.y - new_anchor_y;
    {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else { return };
        s.floating_x = new_x;
        s.floating_y = new_y;
        s.floating_drag_anchor_x = new_anchor_x;
        s.floating_drag_anchor_y = new_anchor_y;
        if should_unlock { s.floating_drag_glow_detached = true; }
        s.floating_reserved_drag_reveal = 100;
        s.floating_collapsed = false;
        s.floating_reveal = 100;
    }
    // The first reserved-edge drag frame rebases the bitmap's virtual slide
    // to real coordinates. Commit the new position with the new bitmap so
    // the drag tab doesn't jump, just like V24.7 halo restoration.
    if dpi_changed || should_unlock || slide_x != 0 || slide_y != 0 {
        // DPI changes alter the canvas and grip sizes. Repaint and position
        // atomically, without exposing an old-size bitmap for one frame.
        // A disconnected old monitor cannot influence the new cursor anchor.
        render_layered_at(Some(POINT { x: new_x, y: new_y }));
        diagnose::log_lazy(|| format!(
            "desktop drag: rebase edge={edge:?} taskbar={reserved} prior=({old_w},{old_h}) dpi={previous_dpi}->{target_dpi} pointer=({}, {}) anchor=({}, {})",
            pointer.x, pointer.y, new_anchor_x, new_anchor_y,
        ));
    } else {
        move_floating_widget(hwnd);
    }
}

fn finish_floating_drag(hwnd: HWND) {
    let (x, y, body_w, body_h, old_edge, old_free_below, explicit_desktop) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        if !s.floating_mode { return };
        (s.floating_x, s.floating_y,
         total_widget_width_for_state(s), widget_height_for_state(s),
         s.floating_edge, floating_free_grip_below_for_state(s),
         s.display_position == DisplayPosition::Desktop)
    };
    let (old_w, old_h, old_body) = {
        let state = lock_state();
        state.as_ref().map(floating_canvas_geometry_for_state)
            .unwrap_or_else(|| floating_canvas_geometry(body_w, body_h, old_edge))
    };
    let mut pointer = POINT { x: x + old_w / 2, y: y + old_h / 2 };
    unsafe { let _ = GetCursorPos(&mut pointer); }
    let Some((monitor, bounds)) = floating_monitor_geometry(pointer) else {
        // Even if monitor enumeration fails, do not leave the drag state
        // latched and force the detached tab to flicker on the next move.
        if let Some(s) = lock_state().as_mut() { s.dragging = false; }
        return;
    };
    let rect = RECT { left: x, top: y, right: x + old_w, bottom: y + old_h };
    // Decide where the undocked tab fits using the BODY location, before
    // clamping anything. Reversing the tab must not push the body downward.
    let body_top = y.saturating_add(old_body.top);
    let prefer_below = explicit_desktop
        && floating_choose_free_grip_below(bounds, body_top, body_h);
    // For a free widget, resolve contact on the ACTUAL dragged tab before
    // choosing the prospective side. Otherwise touching the top with an
    // above-body tab would flip the tab down and silently lose the contact.
    // A previously below-body tab retains its bottom-taskbar priority.
    let edge = if explicit_desktop {
        if old_edge.is_none() {
            let existing_contact = floating_existing_free_tab_contact(
                bounds, old_w, old_h, old_body, old_free_below,
                POINT { x, y }, pointer,
            );
            existing_contact.or_else(|| {
                let (w, h, body) =
                    floating_free_canvas_geometry(body_w, body_h, prefer_below);
                let desired_x = x + old_body.left - body.left;
                let desired_y = y + old_body.top - body.top;
                let (cx, cy) = floating_clamp_drag_visible_position(
                    bounds, w, h, body, None, prefer_below, desired_x, desired_y,
                );
                floating_desktop_contact_edge(
                    bounds, w, h, body, None, prefer_below,
                    POINT { x: cx, y: cy }, pointer,
                )
            })
        } else {
            let (cx, cy) = floating_clamp_drag_visible_position(
                bounds, old_w, old_h, old_body, old_edge,
                old_free_below, x, y,
            );
            floating_desktop_contact_edge(
                bounds, old_w, old_h, old_body, old_edge, old_free_below,
                POINT { x: cx, y: cy }, pointer,
            )
        }
    } else {
        floating_drop_edge_at_pointer(
            monitor, bounds, rect, pointer, sc(FLOAT_EDGE_SNAP), [true; 4],
        )
    };
    let reserved_dock = edge.is_some_and(|edge|
        floating_edge_requires_clip(monitor, bounds, edge, pointer));
    diagnose::log_lazy(|| format!(
        "desktop drop pointer=({}, {}) monitor=({}, {}, {}, {}) distances=[L{}, R{}, T{}, B{}] hwnd=({}, {}, {}, {}) work=({}, {}, {}, {}) edge={edge:?} clipped={reserved_dock}",
        pointer.x, pointer.y,
        monitor.left, monitor.top, monitor.right, monitor.bottom,
        pointer.x.abs_diff(bounds.left), pointer.x.abs_diff(bounds.right),
        pointer.y.abs_diff(bounds.top), pointer.y.abs_diff(bounds.bottom),
        rect.left, rect.top, rect.right, rect.bottom,
        bounds.left, bounds.top, bounds.right, bounds.bottom,
    ));
    let free_below = edge.is_none() && prefer_below;
    let (new_w, new_h, new_body) = if edge.is_none() && explicit_desktop {
        floating_free_canvas_geometry(body_w, body_h, free_below)
    } else {
        floating_canvas_geometry(body_w, body_h, edge)
    };
    let desired_x = x + old_body.left - new_body.left;
    let desired_y = y + old_body.top - new_body.top;
    // Rebase the HWND when flipping the free tab, preserving the body pixels.
    let (x, y) = if explicit_desktop && edge.is_none() {
        floating_clamp_drag_visible_position(
            bounds, new_w, new_h, new_body, None, free_below, desired_x, desired_y,
        )
    } else {
        floating_edge_position(
            bounds, desired_x, desired_y, new_w, new_h, edge, false, sc(FLOAT_EDGE_PEEK),
        )
    };
    // On expanded edge-docked desktops the body stays visible, and the
    // six-dot tab must follow the same hover fade as a free desktop tab.
    let grip = floating_layout_grip_rect(new_w, new_h, edge, free_below);
    let grip_hovered = point_in_rect(grip, pointer.x - x, pointer.y - y);
    let body_hovered = point_in_rect(
        RECT {
            left: x + new_body.left, top: y + new_body.top,
            right: x + new_body.right, bottom: y + new_body.bottom,
        }, pointer.x, pointer.y,
    );
    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.floating_x = x;
            s.floating_y = y;
            s.dragging = false;
            s.floating_edge = edge;
            s.floating_free_grip_below = free_below;
            s.floating_reserved_dock = reserved_dock;
            s.floating_drag_glow_detached = false;
            // Reserved edges retract by pixel clipping, not by moving the
            // popup underneath Explorer.
            let retract = edge.is_some() && floating_auto_collapse_allowed(
                s.display_position, s.desktop_auto_collapse,
            );
            s.floating_collapsed = retract;
            s.floating_first_collapse_guard = retract;
            s.floating_reveal = 100;
            s.floating_slide_from = 100;
            s.floating_slide_target = 100;
            s.floating_slide_started = Instant::now();
            let fade_grip = floating_grip_uses_hover_fade(s);
            s.drag_handle_hovered = grip_hovered && fade_grip;
            s.floating_grip_hover = fade_grip && (grip_hovered || body_hovered);
            s.floating_grip_alpha = if floating_grip_should_show(
                edge, floating_auto_collapse_allowed(
                    s.display_position, s.desktop_auto_collapse,
                ), s.floating_grip_hover, false,
            ) { 100 } else { 0 };
            s.floating_fade_from = s.floating_grip_alpha;
            s.floating_fade_target = s.floating_grip_alpha;
            s.floating_fade_started = Instant::now();
        }
    }
    // Publish the resized/reoriented tab and the final HWND coordinates in
    // one layered-window update. The drop may be on the next monitor, so
    // WM_DPICHANGED must not invoke position_at_taskbar() halfway through
    // committing the new edge and lose the pointer-based drop position.
    let _placement_guard = FloatingInitialPlacementGuard::begin();
    render_layered_at(Some(POINT { x, y }));
    // The frosted pair keeps its existing Z order. The dock geometry now
    // avoids taskbar overlap instead of attempting taskbar ownership.
    if lock_state().as_ref().is_some_and(|s| s.composition_blur_active) {
        sync_blur_backdrop_zorder(hwnd);
    }
    if lock_state().as_ref().is_some_and(|s| s.floating_collapsed) {
        unsafe {
            SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_FLOAT_ANIMATION, 16, None);
        }
    }
    save_state_settings();
}

// Source dimensions must be recalculated after choosing the desktop
// monitor DPI. Using the taskbar DPI for a collapsed grip makes its final
// visible pixels miss the docking edge when the displays have different
// scaling.
fn floating_content_size_for_state(s: &AppState, vertical: bool) -> (i32, i32) {
    let preset = if vertical {
        vertical_effective_preset(s.appearance_preset)
    } else {
        effective_appearance_preset(s)
    };
    let width = if vertical {
        sc(VERTICAL_WIDGET_MIN_WIDTH).max(sc(48))
    } else {
        desktop_horizontal_content_width(
            s.language, preset, s.show_session_window, s.show_weekly_window,
            &s.codex_session_text, &s.codex_weekly_text,
        )
    };
    let height = if vertical {
        desktop_vertical_content_height(
            preset, s.show_session_window, s.show_weekly_window,
        )
    } else {
        sc(preset.metrics().widget_height)
    };
    (width, height)
}

/// Reset animation endpoints when entering or reconfiguring a desktop popup.
/// Keep this state-only: never call synchronous Win32 APIs under the state lock.
fn reset_floating_transition_state(s: &mut AppState) {
    let (collapsed, reveal) = initial_floating_visibility(
        s.floating_edge, s.display_position, s.desktop_auto_collapse,
    );
    s.floating_collapsed = collapsed;
    s.floating_reveal = reveal;
    s.floating_slide_from = reveal;
    s.floating_slide_target = reveal;
    s.floating_slide_started = Instant::now();
    s.floating_reserved_drag_reveal = reveal;
    s.floating_grip_hover = false;
    s.floating_first_collapse_guard = false;
    s.floating_grip_alpha = if !floating_grip_uses_hover_fade(s) { 100 } else { 0 };
    s.floating_fade_from = s.floating_grip_alpha;
    s.floating_fade_target = s.floating_grip_alpha;
    s.floating_fade_started = Instant::now();
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
            let preset = vertical_effective_preset(s.appearance_preset);
            sync_floating_preset_and_usage(s, preset);
            s.vertical_widget_width = sc(VERTICAL_WIDGET_MIN_WIDTH).max(sc(48));
            reset_floating_transition_state(s);
            s.floating_fade_started = Instant::now();
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
        refresh_floating_reserved_dock(hwnd);
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
    let (existing, old, x, y, edge, free_below, language, width, height) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        let (width, height) = floating_content_size_for_state(s, vertical);
        (
            s.floating_hwnd.map(|h| h.to_hwnd()), s.hwnd.to_hwnd(),
            s.floating_x, s.floating_y, s.floating_edge,
            floating_free_grip_below_for_state(s), s.language, width, height,
        )
    };
    let (window_width, window_height, _) = if edge.is_none() {
        floating_free_canvas_geometry(width, height, free_below)
    } else {
        floating_canvas_geometry(width, height, edge)
    };
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
    let taskbar_bounds = floating_monitor_bounds(default_point).unwrap_or(RECT {
        left: 0, top: 0, right: 1920, bottom: 1080,
    });
    let saved_anchor = floating_restore_anchor(x, y, window_width, window_height)
        .map(|center| floating_docked_monitor_anchor(edge, x, y, center));
    let saved_monitor_valid = saved_anchor.is_some_and(|point| unsafe {
        MonitorFromPoint(point, MONITOR_DEFAULTTONULL) != HMONITOR::default()
    });
    let bounds = floating_restore_bounds(saved_anchor, taskbar_bounds, |point| {
        saved_monitor_valid.then(|| floating_monitor_bounds(point)).flatten()
    });
    // The target monitor is now known. All pixel dimensions and tab offsets
    // must use its DPI BEFORE placing the hidden popup on that monitor.
    // This also avoids a WM_DPICHANGED reentrant relayout with the taskbar
    // DPI halfway through the transition.
    let target_point = POINT {
        x: bounds.left.saturating_add((bounds.right - bounds.left) / 2),
        y: bounds.top.saturating_add((bounds.bottom - bounds.top) / 2),
    };
    let target_dpi = floating_monitor_dpi(target_point)
        .unwrap_or_else(|| unsafe { GetDpiForWindow(old) }.max(96));
    let old_dpi = CURRENT_DPI.swap(target_dpi, Ordering::Relaxed);
    let (width, height) = {
        let state = lock_state();
        let Some(s) = state.as_ref() else { return };
        floating_content_size_for_state(s, vertical)
    };
    let (window_width, window_height, body) = if edge.is_none() {
        floating_free_canvas_geometry(width, height, free_below)
    } else {
        floating_canvas_geometry(width, height, edge)
    };
    diagnose::log_lazy(|| format!(
        "desktop restore saved=({x},{y}) saved_monitor_valid={saved_monitor_valid} target_dpi={target_dpi} previous_dpi={old_dpi} target_bounds=({}, {}, {}, {}) taskbar_bounds=({}, {}, {}, {})",
        bounds.left, bounds.top, bounds.right, bounds.bottom,
        taskbar_bounds.left, taskbar_bounds.top, taskbar_bounds.right, taskbar_bounds.bottom,
    ));
    let start_x = if x == i32::MIN { bounds.right - window_width - sc(32) } else { x };
    let start_y = if y == i32::MIN {
        bounds.top + (bounds.bottom - bounds.top - window_height) / 2
    } else { y };
    let free_desktop = lock_state().as_ref().is_some_and(|s|
        s.display_position == DisplayPosition::Desktop && edge.is_none()
    );
    let (x, y) = if free_desktop {
        floating_clamp_drag_visible_position(
            bounds, window_width, window_height, body, None,
            free_below, start_x, start_y,
        )
    } else {
        floating_clamp_position(bounds, window_width, window_height, start_x, start_y)
    };
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
        reset_floating_transition_state(s);
        s.floating_fade_started = Instant::now();
        s.floating_x = x;
        s.floating_y = y;
        s.vertical_taskbar_mode = vertical;
        s.small_taskbar_mode = false;
        let preset = if vertical {
            vertical_effective_preset(s.appearance_preset)
        } else {
            effective_appearance_preset(s)
        };
        sync_floating_preset_and_usage(s, preset);
        s.vertical_widget_width = width;
    }
    unsafe { let _ = ShowWindow(old, SW_HIDE); }
    if let Some(backdrop) = blur_backdrop_hwnd() {
        unsafe { let _ = ShowWindow(backdrop, SW_HIDE); }
    }
    // SetWindowPos may synchronously emit WM_DPICHANGED as this HWND enters
    // the saved desktop monitor. The outer transition already selected the
    // correct DPI and will repaint, so suppress nested position_at_taskbar().
    let _placement_guard = FloatingInitialPlacementGuard::begin();
    refresh_floating_reserved_dock(hwnd);
    // Keep the floating HWND hidden until the complete first frame is ready.
    // A reused popup still has the layered bitmap from its previous visit,
    // which otherwise flashes expanded for one frame before contraction.
    move_floating_widget_with_visibility(hwnd, false);
    style_window::set_preview_orientation(vertical);
    render_layered();
    unsafe { let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE); }
    if lock_state().as_ref().is_some_and(|s| s.composition_blur_active) {
        sync_blur_backdrop_zorder(hwnd);
    }
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

    let Some(window_dc) = WindowDc::acquire(hwnd) else { return };
    let screen_dc = window_dc.handle();
    let Some(mut tooltip_surface) = DibSurface::create(screen_dc, width, height) else { return };
    let mem_dc = tooltip_surface.handle();
    let pixel_data = tooltip_surface.pixels();
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

    // Guarded surfaces restore GDI selection and release the HWND DC.
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
    let measured_width = cached_minimal_tooltip_width(&text);

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
            floating_canvas_geometry_for_state(s).2,
            s.vertical_taskbar_mode,
            effective_appearance_preset(s),
            s.show_session_window, s.show_weekly_window,
        ))
    };
    if let Some((width, height, body, vertical, preset, show_session, show_weekly)) = desktop {
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
        WM_NCHITTEST if lock_state().as_ref().is_some_and(|s| s.floating_mode) => {
            // V24.4: both docked and free desktop canvases have a transparent
            // halo gutter; only the actual panel and drag tab accept clicks.
            let (body, grip, hover_active) = {
                let state = lock_state();
                let s = state.as_ref().expect("desktop mode checked above");
                let (_, _, body) = floating_canvas_geometry_for_state(s);
                (
                    if s.floating_reserved_dock {
                        s.floating_edge.map(|edge| floating_slid_canvas_rect(
                            body, floating_canvas_size_for_state(s).0,
                            floating_canvas_size_for_state(s).1, edge,
                            if s.dragging { s.floating_reserved_drag_reveal }
                                else { s.floating_reveal },
                        )).unwrap_or(body)
                    } else { body },
                    floating_state_grip_rect(s),
                    !s.dragging && (s.drag_handle_hovered ||
                        (floating_grip_uses_hover_fade(s) && s.floating_grip_hover)),
                )
            };
            let screen_x = (lparam.0 & 0xFFFF) as i16 as i32;
            let screen_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
            let Some(rect) = native_interop::get_window_rect_safe(hwnd) else {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            };
            let client_x = screen_x - rect.left;
            let client_y = screen_y - rect.top;
            let hit_code = floating_drag_tab_hit_code(body, grip, client_x, client_y);
            if hit_code == -1 && hover_active {
                // Layered transparent pixels do not reliably generate
                // WM_MOUSEMOVE/WM_MOUSELEAVE. Defer repaint until AFTER the
                // native hit test, avoiding reentrant UpdateLayeredWindow.
                let _ = PostMessageW(
                    hwnd, WM_APP_DESKTOP_HOVER_EXIT, WPARAM(0), LPARAM(0),
                );
            }
            LRESULT(hit_code as isize)
        }
        WM_APP_DESKTOP_HOVER_EXIT => {
            if floating_context_menu_blocks_collapse(hwnd) {
                return LRESULT(0);
            }
            // NCHITTEST can be delivered while layered compositing is still
            // active, so only the posted message may mutate/repaint the hover.
            // Recheck the live pointer: a rapid return to the grip or mouse
            // capture during dragging must not cause another release flicker.
            let valid = lock_state().as_ref().is_some_and(|s|
                s.floating_mode && s.hwnd.to_hwnd() == hwnd
                    && !s.dragging
                    && (s.drag_handle_hovered ||
                        (floating_grip_uses_hover_fade(s) && s.floating_grip_hover))
            );
            if valid {
                let docked_hover_fade = lock_state().as_ref().is_some_and(|s|
                    s.floating_mode && s.hwnd.to_hwnd() == hwnd
                        && s.floating_edge.is_some() && floating_grip_uses_hover_fade(s)
                );
                let hovered = if docked_hover_fade {
                    Some(floating_pointer_over_expanded_docked_panel(hwnd))
                } else {
                    let geometry = {
                        let state = lock_state();
                        state.as_ref().map(|s| {
                            floating_state_grip_rect(s)
                        })
                    };
                    geometry.and_then(|grip| {
                        let rect = native_interop::get_window_rect_safe(hwnd)?;
                        let mut cursor = POINT::default();
                        if GetCursorPos(&mut cursor).is_err() { return None; }
                        Some(point_in_rect(
                            grip, cursor.x - rect.left, cursor.y - rect.top,
                        ))
                    })
                };
                if hovered == Some(false) && docked_hover_fade
                    && floating_pointer_in_expanded_docked_transition(hwnd)
                {
                    // HTTRANSPARENT gap: defer exit just as the original edge
                    // popup does, but keep the panel expanded throughout.
                    floating_arm_hover_recheck(hwnd);
                    return LRESULT(0);
                }
                if hovered == Some(false) {
                    if let Some(s) = lock_state().as_mut() {
                        if s.floating_mode && s.hwnd.to_hwnd() == hwnd && !s.dragging {
                            s.drag_handle_hovered = false;
                        }
                    }
                    if docked_hover_fade {
                        set_floating_grip_hover(false);
                    }
                    render_layered();
                }
            }
            LRESULT(0)
        }
        _ if msg == popup_menu::WM_POPUP_MENU_CLOSED => {
            // The menu owns its own HWND and does not run a modal native
            // menu loop. Re-evaluate only when it has truly been destroyed;
            // an old popup's close must not collapse a newly opened menu.
            if popup_menu::is_open_for(hwnd) {
                return LRESULT(0);
            }
            let eligible = lock_state().as_ref().is_some_and(|s|
                s.floating_mode && s.hwnd.to_hwnd() == hwnd
                    && s.floating_edge.is_some() && !s.dragging
                    && !s.floating_collapsed
                    && floating_auto_collapse_allowed(
                        s.display_position, s.desktop_auto_collapse,
                    )
            );
            let fade_grip = lock_state().as_ref().is_some_and(|s|
                s.floating_mode && s.hwnd.to_hwnd() == hwnd
                    && docked_grip_needs_hover_watch(
                        s.floating_edge,
                        floating_auto_collapse_allowed(
                            s.display_position, s.desktop_auto_collapse,
                        ),
                        s.floating_collapsed, s.dragging,
                    )
            );
            if fade_grip {
                // Restore the hover state after menu focus consumed the last
                // mouse-leave event. Never collapse an explicitly expanded dock.
                let hovered = floating_pointer_over_expanded_docked_panel(hwnd);
                let grip_hovered = hovered && floating_pointer_over_actual_grip(hwnd);
                if let Some(s) = lock_state().as_mut() {
                    s.drag_handle_hovered = grip_hovered;
                    if !hovered { s.minimal_hover_target = None; }
                }
                if hovered {
                    floating_arm_hover_recheck(hwnd);
                } else {
                    floating_cancel_hover_recheck(hwnd);
                    hide_minimal_usage_tooltip();
                }
                set_floating_grip_hover(hovered);
                render_layered();
            }
            if eligible {
                if floating_pointer_in_docked_hover_zone(hwnd) {
                    // WM_MOUSELEAVE tracking is consumed while hovering a
                    // separate popup. Restore it for the next real exit.
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                } else {
                    floating_cancel_hover_recheck(hwnd);
                    if let Some(s) = lock_state().as_mut() {
                        s.minimal_hover_target = None;
                        s.drag_handle_hovered = false;
                    }
                    hide_minimal_usage_tooltip();
                    set_floating_collapsed(true);
                    render_layered();
                }
            }
            LRESULT(0)
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
            if msg == WM_DPICHANGED_MSG
                && (FLOATING_INITIAL_PLACEMENT.with(|flag| flag.get())
                    || lock_state().as_ref().is_some_and(|s|
                        s.floating_mode && s.dragging && s.hwnd.to_hwnd() == hwnd
                    ))
            {
                // SetWindowPos can synchronously deliver WM_DPICHANGED as a
                // popup crosses a monitor seam. Restarting desktop placement
                // here resets its canvas and steals the mouse drag anchor.
                // The next sampled pointer move updates DPI and layered
                // geometry together instead.
                return LRESULT(0);
            }
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
                TIMER_FLOAT_HOVER_RECHECK => {
                    let popup_owner = {
                        let state = lock_state();
                        state.as_ref().and_then(|s|
                            (s.floating_mode && s.floating_edge.is_some())
                                .then_some(s.hwnd.to_hwnd())
                        )
                    };
                    if let Some(owner) = popup_owner {
                        if floating_context_menu_blocks_collapse(owner) {
                            floating_cancel_hover_recheck(owner);
                            return LRESULT(0);
                        }
                    }
                    let active = lock_state().as_ref().and_then(|s|
                        (s.floating_mode && s.floating_edge.is_some()
                            && !s.floating_collapsed && !s.dragging)
                            .then(|| (s.hwnd.to_hwnd(), floating_grip_uses_hover_fade(s)))
                    );
                    if let Some((active, tab_fades)) = active {
                        if tab_fades {
                            // Keep checking the actual cursor while hovering.
                            // Transparent layered pixels can miss WM_MOUSELEAVE,
                            // especially at a physical top-screen boundary.
                            if floating_pointer_over_expanded_docked_panel(active) {
                                let over_grip = floating_pointer_over_actual_grip(active);
                                let changed = {
                                    let mut state = lock_state();
                                    if let Some(s) = state.as_mut() {
                                        let changed = s.drag_handle_hovered != over_grip;
                                        s.drag_handle_hovered = over_grip;
                                        changed
                                    } else { false }
                                };
                                set_floating_grip_hover(true);
                                if changed { render_layered(); }
                            } else {
                                // A transparent body-to-tab gap is only a
                                // brief transition corridor, not a persistent
                                // hover target. One timer tick is its grace
                                // period; a stationary pointer there must
                                // still let the grip fade out.
                                floating_cancel_hover_recheck(active);
                                if let Some(s) = lock_state().as_mut() {
                                    s.minimal_hover_target = None;
                                    s.drag_handle_hovered = false;
                                }
                                hide_minimal_usage_tooltip();
                                set_floating_grip_hover(false);
                                render_layered();
                            }
                        } else if !floating_pointer_in_docked_hover_zone(active) {
                            floating_cancel_hover_recheck(active);
                            // Preserve the original automatic-collapse
                            // debounce and animation unchanged.
                            if let Some(s) = lock_state().as_mut() {
                                s.minimal_hover_target = None;
                                s.drag_handle_hovered = false;
                            }
                            hide_minimal_usage_tooltip();
                            set_floating_collapsed(true);
                            render_layered();
                        }
                    } else {
                        let _ = KillTimer(hwnd, TIMER_FLOAT_HOVER_RECHECK);
                    }
                }
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

            floating_cancel_hover_recheck(hwnd);
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
                        s.floating_reserved_drag_reveal = s.floating_reveal;
                        s.floating_drag_glow_detached = false;
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
            let docked_hover = lock_state().as_ref().is_some_and(|s|
                s.floating_mode && s.floating_edge.is_some() && !s.dragging
            );
            let in_docked_hover_zone = docked_hover
                && floating_pointer_in_docked_hover_zone(hwnd);
            if in_docked_hover_zone {
                // For auto-collapsing docks, mouse motion resolves a pending
                // leave. For non-collapsing docks the timer remains a cursor
                // watchdog: transparent layered pixels can lose the final
                // WM_MOUSELEAVE even after ordinary movement has resumed.
                let watch_grip = lock_state().as_ref().is_some_and(|s|
                    docked_grip_needs_hover_watch(
                        s.floating_edge,
                        floating_auto_collapse_allowed(
                            s.display_position, s.desktop_auto_collapse,
                        ),
                        s.floating_collapsed, s.dragging,
                    )
                );
                if !watch_grip {
                    floating_cancel_hover_recheck(hwnd);
                }
            }
            if lock_state().as_ref().is_some_and(|s|
                s.floating_mode && floating_hover_can_reveal(
                    s.floating_collapsed, s.floating_first_collapse_guard, s.dragging,
                )
            ) && (!docked_hover || in_docked_hover_zone) {
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
                if lock_state().as_ref().is_some_and(|s|
                    s.floating_mode && s.hwnd.to_hwnd() == hwnd
                        && docked_grip_needs_hover_watch(
                            s.floating_edge,
                            floating_auto_collapse_allowed(
                                s.display_position, s.desktop_auto_collapse,
                            ),
                            s.floating_collapsed, s.dragging,
                        )
                ) {
                    floating_arm_hover_recheck(hwnd);
                }
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
            if floating_context_menu_blocks_collapse(hwnd) {
                // The cursor is visiting our context menu, not leaving the
                // widget interaction. Its close notification handles collapse.
                floating_cancel_hover_recheck(hwnd);
                return LRESULT(0);
            }
            // A docked HWND can lose alpha hit-testing at its very first
            // physical pixel when expanding (especially at a display seam).
            // Defer collapse until the cursor really leaves the *unchanging*
            // expanded-panel + collapsed-tab envelope. While this ambiguous
            // leave exists, the short recheck timer handles a later exit even
            // if Windows sends no second WM_MOUSELEAVE.
            if lock_state().as_ref().is_some_and(|s|
                s.floating_mode && s.floating_edge.is_some() && !s.floating_collapsed
                    && !floating_grip_uses_hover_fade(s)
            ) {
                floating_arm_hover_recheck(hwnd);
                return LRESULT(0);
            }
            if lock_state().as_ref().is_some_and(|s|
                s.floating_mode && s.floating_edge.is_some() && !s.floating_collapsed
                    && floating_grip_uses_hover_fade(s)
            ) {
                // WM_MOUSELEAVE is asynchronous: by the time it is delivered
                // the pointer may already have crossed the alpha-zero seam
                // into the visible tab. A geometric "still inside" test would
                // merely re-arm TrackMouseEvent and NEVER update
                // drag_handle_hovered if Win32 stopped sending WM_MOUSEMOVE.
                // Always schedule one bounded actual-cursor recheck for this
                // expanded docked mode, including when the cursor has already
                // arrived at the tab. The timer changes only hover state,
                // NEVER panel collapse.
                floating_arm_hover_recheck(hwnd);
                return LRESULT(0);
            }
            if floating_pointer_still_over_free_grip(hwnd)
                || floating_pointer_over_expanded_docked_panel(hwnd)
            {
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
            floating_cancel_hover_recheck(hwnd);
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
            floating_cancel_hover_recheck(hwnd);
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
                // On a docked drop, WM_CAPTURECHANGED can consume the leave
                // tracking before the user moves away. Keep the cursor watch
                // armed after capture ends, even if no WM_MOUSEMOVE follows.
                if lock_state().as_ref().is_some_and(|s|
                    s.floating_mode && s.hwnd.to_hwnd() == hwnd
                        && s.floating_grip_hover
                        && docked_grip_needs_hover_watch(
                            s.floating_edge,
                            floating_auto_collapse_allowed(
                                s.display_position, s.desktop_auto_collapse,
                            ),
                            s.floating_collapsed, s.dragging,
                        )
                ) {
                    floating_arm_hover_recheck(hwnd);
                }
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
                    if lock_state().as_ref().is_some_and(|s| s.floating_mode) {
                        let _ = reset_floating_widget_position();
                        return LRESULT(0);
                    }
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
                        }
                    }
                    save_state_settings();
                    position_at_taskbar();
                    render_layered();
                }
                IDM_THEME_SYSTEM | IDM_THEME_DARK | IDM_THEME_LIGHT
                | IDM_LAYOUT_DEFAULT | IDM_LAYOUT_MINIMAL
                | IDM_QUICK_POSITION_TASKBAR | IDM_QUICK_POSITION_DESKTOP
                | IDM_QUICK_DIRECTION_HORIZONTAL | IDM_QUICK_DIRECTION_VERTICAL
                | IDM_QUICK_COLLAPSE_ON | IDM_QUICK_COLLAPSE_OFF => {
                    // Use the Settings window's established transition handlers
                    // so all entry points share geometry, clamping, saving and sync.
                    let change = {
                        let state = lock_state();
                        state.as_ref().and_then(|s| quick_appearance_change(s, id))
                    };
                    if let Some((msg, value)) = change {
                        // Submenu commands arrive after root dismissal is queued.
                        // Posting the transition also avoids repositioning the
                        // desktop widget while the popup is still being destroyed.
                        let _ = PostMessageW(
                            taskbar_replica_service_hwnd(hwnd),
                            msg,
                            WPARAM(value),
                            LPARAM(0),
                        );
                    }
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
                _ => {}
            }
            LRESULT(0)
        }
        _ if msg == WM_APP_FOCUS_EXISTING_SETTINGS => {
            hide_minimal_usage_tooltip();
            if let Some(s) = lock_state().as_mut() {
                s.minimal_hover_target = None;
            }
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
            sync_desktop_auto_collapse();
            render_layered();
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        _ if msg == style_window::WM_STYLE_DESKTOP_AUTO_COLLAPSE_CHANGE => {
            let enabled = wparam.0 != 0;
            let changed = {
                let mut state = lock_state();
                match state.as_mut() {
                    Some(s) if s.display_position == DisplayPosition::Desktop
                        && s.desktop_auto_collapse != enabled => {
                        s.desktop_auto_collapse = enabled;
                        true
                    }
                    _ => false,
                }
            };
            if changed {
                save_state_settings();
                sync_desktop_auto_collapse();
                style_window::sync(style_settings_snapshot());
            }
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
            sync_desktop_auto_collapse();
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
            refresh_window_title();
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
            layout: match state.appearance_preset {
                AppearancePreset::Default => "default",
                AppearancePreset::Minimal => "minimal",
                    }
            .to_string(),
            position: if state.display_position == DisplayPosition::Desktop {
                "desktop"
            } else { "taskbar" }.to_string(),
            desktop_direction: if state.desktop_direction == DesktopDirection::Vertical {
                "vertical"
            } else { "horizontal" }.to_string(),
            desktop_auto_collapse: state.desktop_auto_collapse,
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
            desktop_auto_collapse: default_desktop_auto_collapse(),
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
            appearance_preset: s.appearance_preset,
            display_position: s.display_position,
            desktop_direction: s.desktop_direction,
            desktop_auto_collapse: s.desktop_auto_collapse,
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
            desktop_auto_collapse: default_desktop_auto_collapse(),
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
        s.desktop_auto_collapse = settings.appearance.desktop_auto_collapse;
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

    refresh_window_title();
    set_startup_enabled(startup_enabled);
    save_state_settings();
    unsafe {
        SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL, poll_interval_ms, None);
    }
    hide_minimal_usage_tooltip();
    position_at_taskbar();
    sync_desktop_auto_collapse();
    render_layered();
    sync_tray_icons(hwnd);
    notify_quota_alerts(hwnd, &alerts);
    style_window::sync(style_settings_snapshot());
    Ok(())
}


fn quick_appearance_change(state: &AppState, id: u16) -> Option<(u32, usize)> {
    match id {
        IDM_THEME_SYSTEM if state.theme_mode != ThemeMode::System =>
            Some((style_window::WM_STYLE_THEME_CHANGE, 0)),
        IDM_THEME_DARK if state.theme_mode != ThemeMode::Dark =>
            Some((style_window::WM_STYLE_THEME_CHANGE, 1)),
        IDM_THEME_LIGHT if state.theme_mode != ThemeMode::Light =>
            Some((style_window::WM_STYLE_THEME_CHANGE, 2)),
        IDM_LAYOUT_DEFAULT if state.appearance_preset != AppearancePreset::Default =>
            Some((style_window::WM_STYLE_LAYOUT_CHANGE, 0)),
        IDM_LAYOUT_MINIMAL if state.appearance_preset != AppearancePreset::Minimal =>
            Some((style_window::WM_STYLE_LAYOUT_CHANGE, 1)),
        IDM_QUICK_POSITION_TASKBAR if state.display_position != DisplayPosition::Taskbar =>
            Some((style_window::WM_STYLE_POSITION_CHANGE, 0)),
        IDM_QUICK_POSITION_DESKTOP if state.display_position != DisplayPosition::Desktop =>
            Some((style_window::WM_STYLE_POSITION_CHANGE, 1)),
        IDM_QUICK_DIRECTION_HORIZONTAL
            if state.display_position == DisplayPosition::Desktop
                && state.desktop_direction != DesktopDirection::Horizontal =>
            Some((style_window::WM_STYLE_DESKTOP_DIRECTION_CHANGE, 0)),
        IDM_QUICK_DIRECTION_VERTICAL
            if state.display_position == DisplayPosition::Desktop
                && state.desktop_direction != DesktopDirection::Vertical =>
            Some((style_window::WM_STYLE_DESKTOP_DIRECTION_CHANGE, 1)),
        IDM_QUICK_COLLAPSE_ON
            if state.display_position == DisplayPosition::Desktop
                && !state.desktop_auto_collapse =>
            Some((style_window::WM_STYLE_DESKTOP_AUTO_COLLAPSE_CHANGE, 1)),
        IDM_QUICK_COLLAPSE_OFF
            if state.display_position == DisplayPosition::Desktop
                && state.desktop_auto_collapse =>
            Some((style_window::WM_STYLE_DESKTOP_AUTO_COLLAPSE_CHANGE, 0)),
        _ => None,
    }
}

fn quick_appearance_submenus(
    zh: bool,
    theme: ThemeMode,
    layout: AppearancePreset,
    position: DisplayPosition,
    direction: DesktopDirection,
    auto_collapse: bool,
) -> [PopupItem; 2] {
    let text = |zh_text: &'static str, en_text: &'static str| -> &'static str {
        if zh { zh_text } else { en_text }
    };
    let appearance = PopupItem::submenu(
        text("外观与布局", "Appearance & Layout"),
        vec![
            PopupItem::radio_command(text("主题 · 系统", "Theme · System"), IDM_THEME_SYSTEM, theme == ThemeMode::System),
            PopupItem::radio_command(text("主题 · 深色", "Theme · Dark"), IDM_THEME_DARK, theme == ThemeMode::Dark),
            PopupItem::radio_command(text("主题 · 浅色", "Theme · Light"), IDM_THEME_LIGHT, theme == ThemeMode::Light),
            PopupItem::separator(),
            PopupItem::radio_command(text("排版 · 默认", "Layout · Default"), IDM_LAYOUT_DEFAULT, layout == AppearancePreset::Default),
            PopupItem::radio_command(text("排版 · 极简", "Layout · Minimal"), IDM_LAYOUT_MINIMAL, layout == AppearancePreset::Minimal),
            PopupItem::separator(),
            PopupItem::radio_command(text("位置 · 任务栏", "Position · Taskbar"), IDM_QUICK_POSITION_TASKBAR, position == DisplayPosition::Taskbar),
            PopupItem::radio_command(text("位置 · 桌面", "Position · Desktop"), IDM_QUICK_POSITION_DESKTOP, position == DisplayPosition::Desktop),
        ],
    );
    let desktop = PopupItem::submenu(
        text("桌面模式", "Desktop Mode"),
        vec![
            PopupItem::radio_command(text("方向 · 横向", "Direction · Horizontal"), IDM_QUICK_DIRECTION_HORIZONTAL, direction == DesktopDirection::Horizontal),
            PopupItem::radio_command(text("方向 · 纵向", "Direction · Vertical"), IDM_QUICK_DIRECTION_VERTICAL, direction == DesktopDirection::Vertical),
            PopupItem::separator(),
            PopupItem::radio_command(text("收缩 · 开启", "Auto-collapse · On"), IDM_QUICK_COLLAPSE_ON, auto_collapse),
            PopupItem::radio_command(text("收缩 · 关闭", "Auto-collapse · Off"), IDM_QUICK_COLLAPSE_OFF, !auto_collapse),
        ],
    ).with_enabled(position == DisplayPosition::Desktop);
    [appearance, desktop]
}

fn show_context_menu(hwnd: HWND, anchor: POINT, preserve_foreground: bool) {
    // Opening a popup takes interaction away from the quota panel. Its
    // independent topmost tooltip HWND will not necessarily receive a leave
    // message, so dismiss both tooltip layers before opening the menu.
    hide_minimal_usage_tooltip();
    if let Some(s) = lock_state().as_mut() {
        s.minimal_hover_target = None;
    }
    let (strings, language, available_update_version, theme_mode, layout, position, direction, auto_collapse) = {
        let state = lock_state();
        match state.as_ref() {
            Some(s) => (
                s.language.strings(),
                s.language,
                s.available_update_version.clone(),
                s.theme_mode,
                s.appearance_preset,
                s.display_position,
                s.desktop_direction,
                s.desktop_auto_collapse,
            ),
            None => (
                LanguageId::English.strings(),
                LanguageId::English,
                None,
                ThemeMode::System,
                AppearancePreset::Default,
                DisplayPosition::Taskbar,
                DesktopDirection::Horizontal,
                default_desktop_auto_collapse(),
            ),
        }
    };

    let zh = language == LanguageId::SimplifiedChinese;
    let settings_text = if zh { "设置..." } else { "Settings..." };
    let check_text = if zh { "检查更新" } else { "Check for updates" };
    let github_text = if zh { "前往 GitHub" } else { "Open GitHub" };
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
    let [appearance_menu, desktop_menu] = quick_appearance_submenus(
        zh, theme_mode, layout, position, direction, auto_collapse,
    );
    let items = vec![
        PopupItem::command(strings.refresh, 1),
        PopupItem::command(strings.reset_position, IDM_RESET_POSITION),
        PopupItem::separator(),
        appearance_menu,
        desktop_menu,
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
mod tests;
