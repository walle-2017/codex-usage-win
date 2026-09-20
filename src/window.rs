use std::collections::BTreeSet;
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

use crate::appearance::{self, AppearancePreset};
use crate::diagnose;
use crate::localization::{self, LanguageId, Strings};
use crate::models::AppUsageData;
use crate::native_interop::{
    self, Color, TIMER_COUNTDOWN, TIMER_POLL, TIMER_RESET_POLL, WM_APP_TRAY, WM_APP_USAGE_UPDATED,
};
use crate::poller;
use crate::style::{
    StyleColorTarget, StyleSettings, ThemeMode, ThemeStyle, FROSTED_STRENGTH_MAX,
};
use crate::style_window;
use crate::theme;
use crate::tray_icon;
use crate::updater;

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
    theme_mode: ThemeMode,
    styles: StyleSettings,
    composition_blur_active: bool,
    frosted_popup_session: bool,
    small_taskbar_mode: bool,
    small_show_weekly: bool,
    minimal_hover_target: Option<MinimalHoverTarget>,

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
    tray_offset: i32,
    dragging: bool,
    drag_anchor_logical_x: i32,
    drag_reparenting: bool,
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

const GITHUB_RELEASES_URL: &str =
    "https://github.com/walle-2017/codex-usage-win/releases";
const WM_DPICHANGED_MSG: u32 = 0x02E0;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const MINIMAL_TOOLTIP_CLASS: &str = "CodexUsageMinimalTooltip";
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
static LAST_STYLE_PREVIEW_RENDER: Mutex<Option<Instant>> = Mutex::new(None);
static LAST_DRAG_FRAME: Mutex<Option<Instant>> = Mutex::new(None);
static MINIMAL_TOOLTIP_HWND: Mutex<Option<SendHwnd>> = Mutex::new(None);
static MINIMAL_TOOLTIP_TEXT: Mutex<String> = Mutex::new(String::new());

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

fn text_quality_for_layered_surface(panel_alpha: u8, composition_blur_active: bool) -> u32 {
    if composition_blur_active || panel_alpha < u8::MAX {
        // ClearType/antialiased GDI glyph edges are pre-blended against the
        // panel RGB. The layered-window finalizer later promotes changed pixels
        // to opaque foreground, turning those edge blends into visible halos.
        NONANTIALIASED_QUALITY.0 as u32
    } else {
        CLEARTYPE_QUALITY.0 as u32
    }
}

fn widget_text_quality() -> u32 {
    let state = lock_state();
    let Some(s) = state.as_ref() else {
        return CLEARTYPE_QUALITY.0 as u32;
    };
    let panel_alpha = s
        .styles
        .active(s.is_dark)
        .color(StyleColorTarget::PanelBackground)
        .a;
    text_quality_for_layered_surface(panel_alpha, s.composition_blur_active)
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
            s.taskbar_hwnd.unwrap_or_else(|| s.hwnd.to_hwnd())
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
            state.as_ref().and_then(|s| s.taskbar_hwnd)
        };
        // Only relevant once we have embedded into a taskbar at least once.
        let Some(old) = stored else {
            continue;
        };
        let taskbars = native_interop::find_taskbars();
        if !taskbars.is_empty() && !taskbars.iter().any(|taskbar| taskbar.hwnd == old) {
            let new = taskbars[0].hwnd;
            diagnose::log(format!(
                "watchdog: taskbar changed old={:?} new={:?} -> relaunching",
                old.0, new.0
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
    #[serde(default = "default_poll_interval")]
    poll_interval_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(default)]
    appearance_preset: AppearancePreset,
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
            poll_interval_ms: default_poll_interval(),
            language: None,
            appearance_preset: AppearancePreset::Default,
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
    let state = lock_state();
    if let Some(s) = state.as_ref() {
        save_settings(&SettingsFile {
            tray_offset: s.tray_offset,
            taskbar_index: s.taskbar_index,
            poll_interval_ms: s.poll_interval_ms,
            language: s
                .language_override
                .map(|language| language.code().to_string()),
            appearance_preset: s.appearance_preset,
            theme_mode: s.theme_mode,
            styles: s.styles.clone(),
            show_session_window: s.show_session_window,
            show_weekly_window: s.show_weekly_window,
            alert_threshold_percent: s.alert_threshold_percent,
            notified_quota_windows: s.notified_quota_windows.iter().cloned().collect(),
        });
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
    tray_icon::sync(hwnd, icon.as_ref());
}

fn attach_to_taskbar(hwnd: HWND, requested_index: usize) -> bool {
    let taskbars = native_interop::find_taskbars();
    if taskbars.is_empty() {
        diagnose::log("taskbar not found; using fallback popup window");
        return false;
    }

    let index = requested_index.min(taskbars.len().saturating_sub(1));
    let taskbar = taskbars[index];
    diagnose::log(format!(
        "taskbar selected index={index} count={} hwnd={:?} rect=({}, {}, {}, {})",
        taskbars.len(),
        taskbar.hwnd,
        taskbar.rect.left,
        taskbar.rect.top,
        taskbar.rect.right,
        taskbar.rect.bottom
    ));

    let old_hook = {
        let mut state = lock_state();
        state.as_mut().and_then(|s| s.win_event_hook.take())
    };
    if let Some(hook) = old_hook {
        native_interop::unhook_win_event(hook);
    }

    native_interop::embed_in_taskbar(hwnd, taskbar.hwnd);

    let tray_notify = native_interop::find_child_window(taskbar.hwnd, "TrayNotifyWnd");
    if tray_notify.is_some() {
        diagnose::log("TrayNotifyWnd found");
    } else {
        diagnose::log("TrayNotifyWnd not found");
    }

    let hook = tray_notify.and_then(|tray_hwnd| {
        let thread_id = native_interop::get_window_thread_id(tray_hwnd);
        native_interop::set_tray_event_hook(thread_id, on_tray_location_changed)
    });
    if hook.is_some() {
        diagnose::log("tray event hook installed");
    } else {
        diagnose::log("tray event hook could not be installed");
    }

    let mut state = lock_state();
    if let Some(s) = state.as_mut() {
        s.taskbar_hwnd = Some(taskbar.hwnd);
        s.tray_notify_hwnd = tray_notify;
        s.win_event_hook = hook;
        s.taskbar_index = index;
        s.embedded = true;
    }
    true
}

fn select_taskbar_for_popup(requested_index: usize) -> bool {
    let taskbars = native_interop::find_taskbars();
    if taskbars.is_empty() {
        return false;
    }
    let index = requested_index.min(taskbars.len().saturating_sub(1));
    let taskbar = taskbars[index];

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

    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.taskbar_hwnd = Some(taskbar.hwnd);
            s.tray_notify_hwnd = tray_notify;
            s.win_event_hook = hook;
            s.taskbar_index = index;
            s.embedded = false;
        }
    }

    let (foreground_hwnd, blur_active) = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| (Some(s.hwnd.to_hwnd()), s.composition_blur_active))
            .unwrap_or((None, false))
    };
    if let Some(foreground_hwnd) = foreground_hwnd {
        if blur_active {
            // Rebinding both top-level popups to another taskbar can change
            // their relative z-order. Reassert it once per taskbar switch, not
            // on every drag frame.
            sync_blur_backdrop_zorder(foreground_hwnd);
            diagnose::log(
                "popup taskbar switched; reasserted blur backdrop behind foreground",
            );
        } else {
            bind_popup_windows_to_taskbar_owner(foreground_hwnd);
        }
    }
    true
}

fn taskbar_at_point(pt: POINT) -> Option<(usize, native_interop::TaskbarWindow)> {
    native_interop::find_taskbars()
        .into_iter()
        .enumerate()
        .find(|(_, taskbar)| {
            pt.x >= taskbar.rect.left
                && pt.x < taskbar.rect.right
                && pt.y >= taskbar.rect.top
                && pt.y < taskbar.rect.bottom
        })
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

fn clamp_offset_for_taskbar(taskbar_hwnd: HWND, taskbar_rect: RECT, offset: i32) -> i32 {
    let tray_left = tray_left_for_taskbar(taskbar_hwnd, taskbar_rect);
    let max_offset = (tray_left - taskbar_rect.left - total_widget_width()).max(0);
    offset.clamp(0, max_offset)
}

fn drag_anchor_px_for_dpi(logical_x: i32, dpi: u32) -> i32 {
    let dpi = dpi.max(1);
    (logical_x as f64 * dpi as f64 / 96.0).round() as i32
}

fn drag_left_from_cursor(taskbar_rect: RECT, pt: POINT, anchor_px: i32) -> i32 {
    pt.x - taskbar_rect.left - anchor_px
}

fn offset_for_drag_left(taskbar_hwnd: HWND, taskbar_rect: RECT, drag_left: i32) -> i32 {
    let tray_left = tray_left_for_taskbar(taskbar_hwnd, taskbar_rect);
    let offset = tray_left - taskbar_rect.left - total_widget_width() - drag_left;
    clamp_offset_for_taskbar(taskbar_hwnd, taskbar_rect, offset)
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
    state.codex_session_text = appearance::taskbar_line(
        state.appearance_preset,
        state.language,
        &codex.session,
        poller::UsageWindowKind::Session,
    );
    state.codex_weekly_text = appearance::taskbar_line(
        state.appearance_preset,
        state.language,
        &codex.weekly,
        poller::UsageWindowKind::Weekly,
    );
}

fn github_releases_menu_label(language: LanguageId) -> &'static str {
    match language {
        LanguageId::English => "Open GitHub Releases",
        LanguageId::Dutch => "GitHub Releases openen",
        LanguageId::Spanish => "Abrir GitHub Releases",
        LanguageId::French => "Ouvrir GitHub Releases",
        LanguageId::German => "GitHub Releases öffnen",
        LanguageId::Japanese => "GitHub Releases を開く",
        LanguageId::Korean => "GitHub Releases 열기",
        LanguageId::SimplifiedChinese => "前往 GitHub Releases",
        LanguageId::TraditionalChinese => "前往 GitHub Releases",
        LanguageId::Russian => "Открыть GitHub Releases",
        LanguageId::PortugueseBrazil => "Abrir GitHub Releases",
    }
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

fn open_github_releases(hwnd: HWND) {
    unsafe {
        let operation = native_interop::wide_str("open");
        let url = native_interop::wide_str(GITHUB_RELEASES_URL);
        let _ = ShellExecuteW(
            hwnd,
            PCWSTR::from_raw(operation.as_ptr()),
            PCWSTR::from_raw(url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn set_window_title(hwnd: HWND, strings: Strings) {
    unsafe {
        let title = native_interop::wide_str(strings.window_title);
        let _ = SetWindowTextW(hwnd, PCWSTR::from_raw(title.as_ptr()));
    }
}

fn apply_language_to_state(state: &mut AppState, language_override: Option<LanguageId>) {
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
const DRAG_HANDLE_VISUAL_INSET_X: i32 = 7;
const SMALL_TASKBAR_THRESHOLD: i32 = 34;
const SMALL_WIDGET_HEIGHT: i32 = 28;
const DRAG_HANDLE_HIT_H: i32 = 24;

fn is_drag_handle_point(client_x: i32, client_y: i32) -> bool {
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

fn widget_height_for_state(state: &AppState) -> i32 {
    if state.small_taskbar_mode {
        sc(SMALL_WIDGET_HEIGHT)
    } else {
        sc(state.appearance_preset.metrics().widget_height)
    }
}
fn current_appearance_preset() -> AppearancePreset {
    let state = lock_state();
    state
        .as_ref()
        .map(|s| s.appearance_preset)
        .unwrap_or_default()
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

fn total_widget_width_for_state(state: &AppState) -> i32 {
    total_widget_width_for_preset(state.language, state.appearance_preset)
}

fn total_widget_width() -> i32 {
    let (language, preset) = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| (s.language, s.appearance_preset))
            .unwrap_or((LanguageId::English, AppearancePreset::Default))
    };
    total_widget_width_for_preset(language, preset)
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
        let language_override = settings.language.as_deref().and_then(LanguageId::from_code);
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
                theme_mode: settings.theme_mode,
                styles: settings.styles.clone(),
                composition_blur_active: false,
                frosted_popup_session: false,
                small_taskbar_mode: false,
                small_show_weekly: false,
                minimal_hover_target: None,
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
                tray_offset: settings.tray_offset,
                dragging: false,
                drag_anchor_logical_x: 0,
                drag_reparenting: false,
            });
        }

        // Try to embed in taskbar
        if attach_to_taskbar(hwnd, settings.taskbar_index) {
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

        // Position and show. While the process runs, the taskbar widget is always visible.
        position_at_taskbar();
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        diagnose::log("window shown");

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
        SetTimer(hwnd, TIMER_POLL, initial_poll_ms, None);

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

fn bind_popup_windows_to_taskbar_owner(foreground_hwnd: HWND) {
    let taskbar_hwnd = {
        let state = lock_state();
        state.as_ref().and_then(|s| s.taskbar_hwnd)
    };
    if let Some(taskbar_hwnd) = taskbar_hwnd {
        // set_popup_owner() also promotes the popup to HWND_TOPMOST. Bind the
        // backdrop first and the layered foreground last so owner changes can
        // never leave the blur surface above text/progress content.
        if let Some(backdrop_hwnd) = blur_backdrop_hwnd() {
            native_interop::set_popup_owner(backdrop_hwnd, Some(taskbar_hwnd));
        }
        native_interop::set_popup_owner(foreground_hwnd, Some(taskbar_hwnd));
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

fn blur_backdrop_hwnd() -> Option<HWND> {
    let state = BLUR_BACKDROP_HWND
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    state.as_ref().map(|h| h.to_hwnd())
}

fn blur_backdrop_context() -> Option<usize> {
    let state = BLUR_BACKDROP_CONTEXT
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *state
}

fn sync_composition_blur_bounds(width: i32, height: i32) -> bool {
    let Some(context) = blur_backdrop_context() else {
        return false;
    };
    native_interop::set_composition_blur_bounds(context, width, height)
}

fn destroy_blur_backdrop() {
    let context = {
        let mut state = BLUR_BACKDROP_CONTEXT
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.take()
    };
    if let Some(context) = context {
        native_interop::destroy_composition_blur(context);
    }

    {
        let mut params = BLUR_BACKDROP_PARAMS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *params = None;
    }

    let hwnd = {
        let mut state = BLUR_BACKDROP_HWND
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        state.take().map(|h| h.to_hwnd())
    };
    if let Some(hwnd) = hwnd {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        diagnose::log("composition gaussian backdrop destroyed");
    }
}

fn blur_amount_for_strength(strength: u8) -> f32 {
    FROSTED_MAX_BLUR_PX * f32::from(strength.min(FROSTED_STRENGTH_MAX))
        / f32::from(FROSTED_STRENGTH_MAX)
}

fn ensure_blur_backdrop(blur_amount: f32, tint: Color) -> Option<HWND> {
    let params = BlurBackdropParams {
        blur_bits: blur_amount.to_bits(),
        tint,
    };

    let existing_hwnd = blur_backdrop_hwnd();
    let existing_context = blur_backdrop_context();

    if let (Some(hwnd), Some(context)) = (existing_hwnd, existing_context) {
        let unchanged = {
            let cached = BLUR_BACKDROP_PARAMS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *cached == Some(params)
        };
        if unchanged {
            return Some(hwnd);
        }

        let amount_ok = native_interop::set_composition_blur_amount(context, blur_amount);
        let tint_ok = native_interop::set_composition_blur_tint(context, tint);
        if amount_ok && tint_ok {
            let mut cached = BLUR_BACKDROP_PARAMS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *cached = Some(params);
            return Some(hwnd);
        }

        destroy_blur_backdrop();
    } else if existing_hwnd.is_some() || existing_context.is_some() {
        // A partial backend state is not reusable. Recreate it atomically.
        destroy_blur_backdrop();
    }

    register_blur_backdrop_class();
    unsafe {
        let class_name = native_interop::wide_str("CodexUsageBlurBackdrop");
        let title = native_interop::wide_str("");
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW
                | WS_EX_TOPMOST
                | WS_EX_NOACTIVATE
                | WS_EX_TRANSPARENT
                | WS_EX_NOREDIRECTIONBITMAP,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(title.as_ptr()),
            WS_POPUP,
            0,
            0,
            1,
            1,
            HWND::default(),
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        )
        .ok()?;

        let Some(context) =
            native_interop::create_composition_blur(hwnd, blur_amount, tint)
        else {
            let _ = DestroyWindow(hwnd);
            return None;
        };

        {
            let mut state = BLUR_BACKDROP_CONTEXT
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *state = Some(context);
        }
        {
            let mut cached = BLUR_BACKDROP_PARAMS
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *cached = Some(params);
        }
        {
            let mut state = BLUR_BACKDROP_HWND
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *state = Some(SendHwnd::from_hwnd(hwnd));
        }

        let owner = {
            let state = lock_state();
            state.as_ref().and_then(|s| s.taskbar_hwnd)
        };
        native_interop::set_popup_owner(hwnd, owner);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        diagnose::log("composition gaussian backdrop created");
        Some(hwnd)
    }
}

fn sync_blur_backdrop_zorder(foreground_hwnd: HWND) {
    bind_popup_windows_to_taskbar_owner(foreground_hwnd);
    let Some(backdrop_hwnd) = blur_backdrop_hwnd() else {
        return;
    };
    let Some(rect) = native_interop::get_window_rect_safe(foreground_hwnd) else {
        return;
    };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }

    if !sync_composition_blur_bounds(width, height) {
        diagnose::log("composition blur bounds sync failed during z-order update");
    }

    unsafe {
        let _ = SetWindowPos(
            backdrop_hwnd,
            HWND_TOPMOST,
            rect.left,
            rect.top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = SetWindowPos(
            foreground_hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

fn sync_blur_backdrop_geometry(foreground_hwnd: HWND) {
    let Some(backdrop_hwnd) = blur_backdrop_hwnd() else {
        return;
    };
    let Some(rect) = native_interop::get_window_rect_safe(foreground_hwnd) else {
        return;
    };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return;
    }
    if !sync_composition_blur_bounds(width, height) {
        diagnose::log("composition blur bounds sync failed during geometry update");
    }
    unsafe {
        let _ = SetWindowPos(
            backdrop_hwnd,
            HWND::default(),
            rect.left,
            rect.top,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn move_window_without_repaint(hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn move_frosted_pair(foreground_hwnd: HWND, x: i32, y: i32, width: i32, height: i32) {
    let Some(backdrop_hwnd) = blur_backdrop_hwnd() else {
        move_window_without_repaint(foreground_hwnd, x, y, width, height);
        return;
    };
    // Shrink/expand the visual tree first. The current HWND bounds clip an
    // expansion, while an early visual shrink prevents a stale-DPI blur tail.
    if !sync_composition_blur_bounds(width, height) {
        diagnose::log("composition blur bounds sync failed during drag");
    }
    unsafe {
        // Preserve existing owner/z-order. Re-ordering two top-level windows on
        // every mouse move causes visible DWM flicker.
        let flags = SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(
            backdrop_hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            flags,
        );
        let _ = SetWindowPos(
            foreground_hwnd,
            HWND::default(),
            x,
            y,
            width,
            height,
            flags,
        );
    }
}

fn activate_blur_popup(
    hwnd: HWND,
    blur_amount: f32,
    tint: Color,
) -> bool {
    let was_embedded = {
        let state = lock_state();
        state.as_ref().map(|s| s.embedded).unwrap_or(false)
    };

    // Keep the foreground widget layered at all times. Only detach it from
    // Explorer so the independent Composition backdrop can sit behind it.
    if was_embedded {
        native_interop::detach_from_taskbar_as_popup(hwnd);
        native_interop::set_layered_style(hwnd, true);
        {
            let mut state = lock_state();
            if let Some(s) = state.as_mut() {
                s.embedded = false;
                // Reparenting a layered HWND back into Explorer after entering
                // backdrop mode is unreliable on affected Windows 11 builds.
                // Keep this foreground as a top-level popup for the process.
                s.frosted_popup_session = true;
            }
        }
        bind_popup_windows_to_taskbar_owner(hwnd);
        position_at_taskbar();
    }

    let Some(_) = ensure_blur_backdrop(blur_amount, tint) else {
        diagnose::log(
            "composition blur backdrop activation failed; restoring layered mode",
        );
        restore_layered_taskbar_mode(hwnd);
        return false;
    };

    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.composition_blur_active = true;
        }
    }

    bind_popup_windows_to_taskbar_owner(hwnd);
    sync_blur_backdrop_zorder(hwnd);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    diagnose::log("dual-window composition gaussian blur activated");
    true
}

fn restore_layered_taskbar_mode(hwnd: HWND) {
    let frosted_popup_session = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| s.frosted_popup_session)
            .unwrap_or(false)
    };

    destroy_blur_backdrop();
    native_interop::set_layered_style(hwnd, true);

    {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            s.composition_blur_active = false;
        }
    }

    if frosted_popup_session {
        // Do NOT SetParent() this HWND back into Explorer during the same
        // process lifetime. Reparenting is what invalidates the layered surface
        // on affected Windows 11 builds. Keep the same top-level layered HWND
        // and simply render the normal opaque/transparent panel again.
        bind_popup_windows_to_taskbar_owner(hwnd);
        position_at_taskbar();
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
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        diagnose::log("frosted glass disabled; keeping foreground in stable layered popup mode");
        return;
    }

    // This path is only for an activation failure that happened before the
    // foreground ever entered the frosted popup session.
    let taskbar_index = {
        let state = lock_state();
        state.as_ref().map(|s| s.taskbar_index).unwrap_or(0)
    };
    if attach_to_taskbar(hwnd, taskbar_index) {
        position_at_taskbar();
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
    } else {
        native_interop::detach_from_taskbar_as_popup(hwnd);
        native_interop::set_layered_style(hwnd, true);
        {
            let mut state = lock_state();
            if let Some(s) = state.as_mut() {
                s.embedded = false;
                s.frosted_popup_session = true;
            }
        }
        position_at_taskbar();
    }
}

/// Render the foreground widget through UpdateLayeredWindow. Frosted mode keeps
/// the foreground layered and uses a separate Windows Composition Gaussian backdrop.
fn render_layered() {
    refresh_dpi();
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
    let blur_requested = frosted_strength > 0;
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
                activate_blur_popup(hwnd, blur_amount, blur_tint);
        }
    } else if composition_blur_active {
        restore_layered_taskbar_mode(hwnd);
        composition_blur_active = false;
    }

    let (embedded, frosted_popup_session) = {
        let state = lock_state();
        state
            .as_ref()
            .map(|s| (s.embedded, s.frosted_popup_session))
            .unwrap_or((false, false))
    };
    if !embedded && !frosted_popup_session && !composition_blur_active {
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

        let pt_src = POINT { x: 0, y: 0 };
        let sz = SIZE {
            cx: width,
            cy: height,
        };
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
            Some(&sz),
            mem_dc,
            Some(&pt_src),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        SelectObject(mem_dc, old_bmp);
        let _ = DeleteObject(dib);
        let _ = DeleteDC(mem_dc);
        ReleaseDC(hwnd, screen_dc);
    }

    if composition_blur_active {
        sync_blur_backdrop_geometry(hwnd);
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

fn blend_panel_bitmap(pixels: &mut [u32], width: i32, height: i32, style: &ThemeStyle) {
    let outer_inset = sc(1).max(1);
    let inner_inset = outer_inset + PANEL_BORDER_WIDTH_PX;
    let border = style.color(StyleColorTarget::PanelBorder);
    let fill = style.color(StyleColorTarget::PanelBackground);
    for y in outer_inset..(height - outer_inset).max(outer_inset) {
        for x in outer_inset..(width - outer_inset).max(outer_inset) {
            let idx = (y * width + x) as usize;
            let color = if x >= inner_inset
                && x < width - inner_inset
                && y >= inner_inset
                && y < height - inner_inset
            {
                fill
            } else {
                border
            };
            pixels[idx] = blend_pixel(pixels[idx], color);
        }
    }
}

fn panel_color_at(style: &ThemeStyle, width: i32, height: i32, x: i32, y: i32) -> Option<Color> {
    let outer_inset = sc(1).max(1);
    if x < outer_inset
        || x >= width - outer_inset
        || y < outer_inset
        || y >= height - outer_inset
    {
        return None;
    }
    let inner_inset = outer_inset + PANEL_BORDER_WIDTH_PX;
    if x >= inner_inset
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

            // Foreground content is kept opaque. Panel pixels retain the
            // configured RGBA alpha in the normal layered renderer.
            if pixels[idx] != panel_pixels[idx] {
                pixels[idx] = (pixels[idx] & 0x00FFFFFF) | 0xFF000000;
            } else {
                pixels[idx] = premultiplied_pixel(panel_color);
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

            let border = style.color(StyleColorTarget::PanelBorder).blend_over(*bg);
            draw_panel(hdc, width, height, &border, &panel_base);
        }

        draw_drag_handle(hdc, height, &drag_color);

        let content_x = sc(DRAG_HANDLE_HIT_W) + sc(metrics.outer_padding);
        let row2_y = height - sc(4) - sc(SEGMENT_H);
        let row1_y = row2_y - sc(metrics.row_gap) - sc(SEGMENT_H);
        let single_row_y = (height - sc(SEGMENT_H)) / 2;

        let _ = SetBkMode(hdc, TRANSPARENT);
        let font_name = native_interop::wide_str("Segoe UI");
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
            widget_text_quality(),
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
                        let _ = KillTimer(hwnd, TIMER_RESET_POLL);
                    }
                }
                quota_alerts = collect_low_quota_alerts(s, &data);
                s.data = Some(data);
                s.last_poll_ok = true;
                refresh_usage_texts(s);
                if s.retry_count > 0 {
                    s.retry_count = 0;
                    unsafe {
                        SetTimer(hwnd, TIMER_POLL, s.poll_interval_ms, None);
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
                            let _ = KillTimer(hwnd, TIMER_POLL);
                            let _ = KillTimer(hwnd, TIMER_RESET_POLL);
                            let _ = KillTimer(hwnd, TIMER_COUNTDOWN);
                            SetTimer(hwnd, TIMER_POLL, s.poll_interval_ms, None);
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
                            let _ = KillTimer(hwnd, TIMER_RESET_POLL);
                            SetTimer(hwnd, TIMER_POLL, backoff.min(s.poll_interval_ms), None);
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

    let hwnd = s.hwnd.to_hwnd();
    if !s.last_poll_ok {
        unsafe {
            let _ = KillTimer(hwnd, TIMER_COUNTDOWN);
            let _ = KillTimer(hwnd, TIMER_RESET_POLL);
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
            SetTimer(hwnd, TIMER_RESET_POLL, 5_000, None);
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
        SetTimer(hwnd, TIMER_COUNTDOWN, ms, None);
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

fn position_at_taskbar() {
    refresh_dpi();
    let (hwnd, embedded, tray_offset, taskbar_hwnd, blur_active) = {
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
            s.tray_offset,
            taskbar_hwnd,
            s.composition_blur_active,
        )
    };

    let taskbar_rect = match native_interop::get_taskbar_rect(taskbar_hwnd) {
        Some(r) => r,
        None => {
            diagnose::log("position_at_taskbar skipped: unable to query taskbar rect");
            return;
        }
    };
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

    let mut tray_left = taskbar_rect.right;
    let anchor_top = taskbar_rect.top;
    let anchor_height = taskbar_height;
    if let Some(tray_hwnd) = native_interop::find_child_window(taskbar_hwnd, "TrayNotifyWnd") {
        if let Some(tray_rect) = native_interop::get_window_rect_safe(tray_hwnd) {
            tray_left = tray_rect.left;
        }
    }

    let widget_width = total_widget_width();
    let max_offset = (tray_left - taskbar_rect.left - widget_width).max(0);
    let tray_offset = tray_offset.clamp(0, max_offset);
    let offset_changed = {
        let mut state = lock_state();
        if let Some(s) = state.as_mut() {
            if s.tray_offset != tray_offset {
                s.tray_offset = tray_offset;
                true
            } else {
                false
            }
        } else {
            false
        }
    };
    if offset_changed {
        save_state_settings();
    }

    let widget_height = {
        let state = lock_state();
        state
            .as_ref()
            .map(widget_height_for_state)
            .unwrap_or(sc(AppearancePreset::Default.metrics().widget_height))
    };
    let y = compute_anchor_y(anchor_top, anchor_height, widget_height);
    if embedded {
        let x = tray_left - taskbar_rect.left - widget_width - tray_offset;
        native_interop::move_window(hwnd, x, y - taskbar_rect.top, widget_width, widget_height);
        diagnose::log(format!(
            "positioned embedded widget at x={x} y={} w={widget_width} h={widget_height}",
            y - taskbar_rect.top
        ));
    } else {
        let x = tray_left - widget_width - tray_offset;
        if blur_active {
            move_frosted_pair(hwnd, x, y, widget_width, widget_height);
        } else {
            native_interop::move_window(hwnd, x, y, widget_width, widget_height);
        }
        diagnose::log(format!(
            "positioned fallback widget at x={x} y={y} w={widget_width} h={widget_height}"
        ));
    }
    if !embedded && !blur_active {
        bind_popup_windows_to_taskbar_owner(hwnd);
    }
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

/// Main window procedure

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
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);

            let background = CreateSolidBrush(COLORREF(Color::from_hex("#30343CFF").to_colorref()));
            let border = CreateSolidBrush(COLORREF(Color::from_hex("#626A76FF").to_colorref()));
            FillRect(hdc, &client, background);
            FrameRect(hdc, &client, border);
            let _ = DeleteObject(background);
            let _ = DeleteObject(border);

            let font_name = native_interop::wide_str("Segoe UI");
            let font = CreateFontW(
                sc(-12),
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
            let old_font = SelectObject(hdc, font);
            let _ = SetBkMode(hdc, TRANSPARENT);
            let _ = SetTextColor(hdc, COLORREF(Color::from_hex("#F4F6F8FF").to_colorref()));

            let text = MINIMAL_TOOLTIP_TEXT
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            let mut text_rect = client;
            text_rect.left += sc(8);
            text_rect.right -= sc(8);
            let _ = DrawTextW(
                hdc,
                &mut wide,
                &mut text_rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );

            let _ = SelectObject(hdc, old_font);
            let _ = DeleteObject(font);
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
        let state = MINIMAL_TOOLTIP_HWND
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(hwnd) = *state {
            return Some(hwnd.to_hwnd());
        }
    }

    register_minimal_tooltip_class();
    unsafe {
        let class_name = native_interop::wide_str(MINIMAL_TOOLTIP_CLASS);
        let title = native_interop::wide_str("");
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
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
    let (label, section, window) = match target {
        MinimalHoverTarget::Session => (
            strings.session_window,
            &codex.session,
            poller::UsageWindowKind::Session,
        ),
        MinimalHoverTarget::Weekly => (
            strings.weekly_window,
            &codex.weekly,
            poller::UsageWindowKind::Weekly,
        ),
    };
    let reset = appearance::taskbar_value_text(
        AppearancePreset::Default,
        s.language,
        section,
        window,
    )
    .secondary
    .unwrap_or_else(|| "--".to_string());
    Some(format!("{label} · {reset}"))
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
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }
}

fn show_minimal_usage_tooltip(owner: HWND, target: MinimalHoverTarget, hit_rect: RECT) {
    let Some(text) = minimal_hover_text(target) else {
        hide_minimal_usage_tooltip();
        return;
    };
    let Some(tooltip) = minimal_tooltip_hwnd() else {
        return;
    };

    {
        let mut tooltip_text = MINIMAL_TOOLTIP_TEXT
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *tooltip_text = text;
    }

    unsafe {
        let width = sc(112);
        let height = sc(28);
        let mut owner_rect = RECT::default();
        let _ = GetWindowRect(owner, &mut owner_rect);
        let mut anchor = POINT {
            x: (hit_rect.left + hit_rect.right) / 2,
            y: hit_rect.top,
        };
        let _ = ClientToScreen(owner, &mut anchor);

        let y = if owner_rect.top >= height + sc(6) {
            owner_rect.top - height - sc(6)
        } else {
            owner_rect.bottom + sc(6)
        };
        let x = anchor.x - width / 2;

        let _ = SetWindowPos(
            tooltip,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(tooltip, None, false);
        let _ = UpdateWindow(tooltip);
    }
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
            s.appearance_preset,
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
    let content_x = sc(DRAG_HANDLE_HIT_W) + sc(metrics.outer_padding);
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
        if pt_in_rect(rect, x, y) {
            return Some((MinimalHoverTarget::Session, rect));
        }
    }
    if effective_show_weekly {
        let rect = make_rect(if effective_show_session { row2_y } else { single_row_y });
        if pt_in_rect(rect, x, y) {
            return Some((MinimalHoverTarget::Weekly, rect));
        }
    }
    None
}

fn update_minimal_hover(hwnd: HWND, x: i32, y: i32) {
    let hit = minimal_percent_hit(hwnd, x, y);
    let target = hit.map(|(target, _)| target);
    let changed = {
        let mut state = lock_state();
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.minimal_hover_target == target {
            false
        } else {
            s.minimal_hover_target = target;
            true
        }
    };

    if !changed {
        return;
    }
    if let Some((target, rect)) = hit {
        show_minimal_usage_tooltip(hwnd, target, rect);
    } else {
        hide_minimal_usage_tooltip();
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            let (embedded, frosted_active) = {
                let state = lock_state();
                state
                    .as_ref()
                    .map(|s| (s.embedded, s.composition_blur_active))
                    .unwrap_or((false, false))
            };
            if embedded || frosted_active {
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

            let mut state = lock_state();
            if let Some(s) = state.as_mut() {
                s.dragging = true;
                s.drag_anchor_logical_x = anchor_logical_x;
            }
            {
                let mut last = LAST_DRAG_FRAME.lock().unwrap_or_else(|e| e.into_inner());
                *last = None;
            }
            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let is_dragging = {
                let state = lock_state();
                state.as_ref().map(|s| s.dragging).unwrap_or(false)
            };
            if is_dragging {
                hide_minimal_usage_tooltip();
            } else {
                let client_x = (lparam.0 & 0xFFFF) as i16 as i32;
                let client_y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                update_minimal_hover(hwnd, client_x, client_y);
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
                let _ = GetCursorPos(&mut pt);

                let (current_taskbar_index, embedded, blur_active) = {
                    let state = lock_state();
                    state
                        .as_ref()
                        .map(|s| (Some(s.taskbar_index), s.embedded, s.composition_blur_active))
                        .unwrap_or((None, false, false))
                };

                // A popup uses absolute screen coordinates. If the pointer has
                // left every taskbar, combining its X coordinate with the old
                // taskbar's Y coordinate can place the widget in the middle of
                // another desktop (especially with vertically offset monitors).
                // Freeze at the last valid taskbar position until the pointer
                // enters a taskbar again.
                let Some((hovered_taskbar_index, hovered_taskbar)) = taskbar_at_point(pt) else {
                    return LRESULT(0);
                };

                let mut switched_taskbar = false;
                if let Some(current_index) = current_taskbar_index {
                    if hovered_taskbar_index != current_index {
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

                        let switched = if embedded {
                            attach_to_taskbar(hwnd, hovered_taskbar_index)
                        } else {
                            select_taskbar_for_popup(hovered_taskbar_index)
                        };
                        if switched {
                            {
                                let mut state = lock_state();
                                if let Some(s) = state.as_mut() {
                                    s.dragging = true;
                                    s.drag_reparenting = false;
                                }
                            }
                            SetCapture(hwnd);
                            switched_taskbar = true;
                        } else {
                            CURRENT_DPI.store(previous_dpi, Ordering::Relaxed);
                            let mut state = lock_state();
                            if let Some(s) = state.as_mut() {
                                s.drag_reparenting = false;
                            }
                            SetCapture(hwnd);
                        }
                    }
                }

                let drag_context = {
                    let state = lock_state();
                    state.as_ref().and_then(|s| {
                        s.taskbar_hwnd.map(|taskbar_hwnd| {
                            (taskbar_hwnd, s.drag_anchor_logical_x, s.embedded)
                        })
                    })
                };

                if let Some((taskbar_hwnd, anchor_logical_x, embedded)) = drag_context {
                    if let Some(taskbar_rect) = native_interop::get_taskbar_rect(taskbar_hwnd) {
                        let dpi = GetDpiForWindow(taskbar_hwnd);
                        if dpi > 0 {
                            CURRENT_DPI.store(dpi, Ordering::Relaxed);
                        }
                        let effective_dpi = CURRENT_DPI.load(Ordering::Relaxed).max(1);
                        let anchor_px = drag_anchor_px_for_dpi(anchor_logical_x, effective_dpi);
                        let drag_left = drag_left_from_cursor(taskbar_rect, pt, anchor_px);
                        let taskbar_height = taskbar_rect.bottom - taskbar_rect.top;
                        let small_mode = is_small_taskbar_height(taskbar_height);

                        let (widget_width, widget_height, small_mode_changed) = {
                            let mut state = lock_state();
                            let s = match state.as_mut() {
                                Some(s) => s,
                                None => return LRESULT(0),
                            };
                            let changed = s.small_taskbar_mode != small_mode;
                            if changed {
                                s.small_taskbar_mode = small_mode;
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
                        let anchor_y =
                            compute_anchor_y(taskbar_rect.top, taskbar_height, widget_height);
                        let (window_x, window_y) = if embedded {
                            (drag_left, anchor_y - taskbar_rect.top)
                        } else {
                            (taskbar_rect.left + drag_left, anchor_y)
                        };

                        // Embedded mode uses taskbar-client coordinates; Composition blur popup
                        // mode uses absolute screen coordinates.
                        if blur_active {
                            move_frosted_pair(
                                hwnd,
                                window_x,
                                window_y,
                                widget_width,
                                widget_height,
                            );
                        } else {
                            move_window_without_repaint(
                                hwnd,
                                window_x,
                                window_y,
                                widget_width,
                                widget_height,
                            );
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
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.minimal_hover_target = None;
                }
            }
            hide_minimal_usage_tooltip();
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
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let drag_result = {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.drag_reparenting = false;
                    let was_dragging = s.dragging;
                    s.dragging = false;
                    if was_dragging {
                        Some((s.taskbar_index, s.drag_anchor_logical_x, s.embedded))
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

            if let Some((current_taskbar_index, anchor_logical_x, embedded)) = drag_result {
                let release_taskbar = taskbar_at_point(pt);
                if let Some((target_index, _)) = release_taskbar {
                    if target_index != current_taskbar_index {
                        if embedded {
                            let _ = attach_to_taskbar(hwnd, target_index);
                        } else {
                            let _ = select_taskbar_for_popup(target_index);
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
                            let anchor_px =
                                drag_anchor_px_for_dpi(anchor_logical_x, effective_dpi);
                            let final_drag_left =
                                drag_left_from_cursor(taskbar_rect, pt, anchor_px);
                            let new_offset =
                                offset_for_drag_left(taskbar_hwnd, taskbar_rect, final_drag_left);
                            {
                                let mut state = lock_state();
                                if let Some(s) = state.as_mut() {
                                    s.tray_offset = new_offset;
                                }
                            }
                            position_at_taskbar();
                            render_layered();
                        }
                    }
                    save_state_settings();
                } else {
                    // Releasing over desktop is not a valid drop target. Keep the
                    // current taskbar selection and restore its persisted offset.
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
            show_context_menu(hwnd);
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
                    let sh = SendHwnd::from_hwnd(hwnd);
                    std::thread::spawn(move || {
                        do_poll(sh);
                    });
                }
                2 => {
                    destroy_blur_backdrop();
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
                        let _ = updater::start_update(hwnd);
                    }
                }
                IDM_OPEN_RELEASES => {
                    open_github_releases(hwnd);
                }
                IDM_RESET_POSITION => {
                    {
                        let mut state = lock_state();
                        if let Some(s) = state.as_mut() {
                            s.tray_offset = 0;
                        }
                    }
                    save_state_settings();
                    position_at_taskbar();
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
                    SetTimer(hwnd, TIMER_POLL, new_interval, None);
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
                IDM_LAYOUT_DEFAULT | IDM_LAYOUT_MINIMAL => {
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
                    close_style_editors();
                    style_window::open_or_focus(hwnd, style_settings_snapshot());
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
        _ if msg == WM_APP_TRAY => {
            match tray_icon::handle_message(lparam) {
                tray_icon::TrayAction::ShowContextMenu => {
                    show_context_menu(hwnd);
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
                    s.appearance_preset = if wparam.0 == 1 {
                        AppearancePreset::Minimal
                    } else {
                        AppearancePreset::Default
                    };
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
        _ if msg == style_window::WM_STYLE_RESET_CURRENT => {
            {
                let mut state = lock_state();
                if let Some(s) = state.as_mut() {
                    s.styles.reset_active(s.is_dark);
                }
            }
            save_state_settings();
            render_layered();
            style_window::sync(style_settings_snapshot());
            LRESULT(0)
        }
        WM_DESTROY => {
            hide_minimal_usage_tooltip();

            destroy_blur_backdrop();
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

fn style_settings_snapshot() -> style_window::StyleWindowSnapshot {
    let state = lock_state();
    if let Some(s) = state.as_ref() {
        style_window::StyleWindowSnapshot {
            language: s.language,
            theme_mode: s.theme_mode,
            is_dark: s.is_dark,
            appearance_preset: s.appearance_preset,
            active_style: s.styles.active(s.is_dark).clone(),
        }
    } else {
        style_window::StyleWindowSnapshot {
            language: LanguageId::English,
            theme_mode: ThemeMode::System,
            is_dark: true,
            appearance_preset: AppearancePreset::Default,
            active_style: ThemeStyle::dark_default(),
        }
    }
}


fn show_context_menu(hwnd: HWND) {
    unsafe {
        let (
            current_interval,
            strings,
            language,
            language_override,
            show_session_window,
            show_weekly_window,
            alert_threshold_percent,
            available_update_version,
        ) = {
            let state = lock_state();
            match state.as_ref() {
                Some(s) => (
                    s.poll_interval_ms,
                    s.language.strings(),
                    s.language,
                    s.language_override,
                    s.show_session_window,
                    s.show_weekly_window,
                    s.alert_threshold_percent,
                    s.available_update_version.clone(),
                ),
                None => (
                    POLL_15_MIN,
                    LanguageId::English.strings(),
                    LanguageId::English,
                    None,
                    true,
                    true,
                    0,
                    None,
                ),
            }
        };

        let menu = CreatePopupMenu().unwrap();

        let refresh_str = native_interop::wide_str(strings.refresh);
        let _ = AppendMenuW(
            menu,
            MENU_ITEM_FLAGS(0),
            1,
            PCWSTR::from_raw(refresh_str.as_ptr()),
        );

        // Update Frequency submenu
        let freq_menu = CreatePopupMenu().unwrap();
        let freq_items: [(u16, u32, &str); 4] = [
            (IDM_FREQ_1MIN, POLL_1_MIN, strings.one_minute),
            (IDM_FREQ_5MIN, POLL_5_MIN, strings.five_minutes),
            (IDM_FREQ_15MIN, POLL_15_MIN, strings.fifteen_minutes),
            (IDM_FREQ_1HOUR, POLL_1_HOUR, strings.one_hour),
        ];
        for (id, interval, label) in freq_items {
            let label_str = native_interop::wide_str(label);
            let flags = if interval == current_interval {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            };
            let _ = AppendMenuW(
                freq_menu,
                flags,
                id as usize,
                PCWSTR::from_raw(label_str.as_ptr()),
            );
        }

        let freq_label = native_interop::wide_str(strings.update_frequency);
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            freq_menu.0 as usize,
            PCWSTR::from_raw(freq_label.as_ptr()),
        );

        // Usage window visibility submenu. Keep at least one window enabled.
        let usage_menu = CreatePopupMenu().unwrap();
        let session_label =
            native_interop::wide_str(if language == LanguageId::SimplifiedChinese {
                "5 小时额度"
            } else {
                "5-hour quota"
            });
        let session_flags = if show_session_window {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        let _ = AppendMenuW(
            usage_menu,
            session_flags,
            IDM_SHOW_SESSION_WINDOW as usize,
            PCWSTR::from_raw(session_label.as_ptr()),
        );
        let weekly_label = native_interop::wide_str(if language == LanguageId::SimplifiedChinese {
            "每周额度"
        } else {
            "Weekly quota"
        });
        let weekly_flags = if show_weekly_window {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        let _ = AppendMenuW(
            usage_menu,
            weekly_flags,
            IDM_SHOW_WEEKLY_WINDOW as usize,
            PCWSTR::from_raw(weekly_label.as_ptr()),
        );
        let usage_label = native_interop::wide_str(if language == LanguageId::SimplifiedChinese {
            "显示用量"
        } else {
            "Usage display"
        });
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            usage_menu.0 as usize,
            PCWSTR::from_raw(usage_label.as_ptr()),
        );

        // Low-quota alert threshold submenu. Zero means opt-out.
        let alert_menu = CreatePopupMenu().unwrap();
        let alert_items = [
            (
                IDM_ALERT_OFF,
                0u8,
                if language == LanguageId::SimplifiedChinese {
                    "关闭"
                } else {
                    "Off"
                },
            ),
            (
                IDM_ALERT_10,
                10u8,
                if language == LanguageId::SimplifiedChinese {
                    "剩余 10%"
                } else {
                    "10% remaining"
                },
            ),
            (
                IDM_ALERT_20,
                20u8,
                if language == LanguageId::SimplifiedChinese {
                    "剩余 20%"
                } else {
                    "20% remaining"
                },
            ),
            (
                IDM_ALERT_30,
                30u8,
                if language == LanguageId::SimplifiedChinese {
                    "剩余 30%"
                } else {
                    "30% remaining"
                },
            ),
        ];
        for (id, threshold, label) in alert_items {
            let label = native_interop::wide_str(label);
            let flags = if alert_threshold_percent == threshold {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            };
            let _ = AppendMenuW(
                alert_menu,
                flags,
                id as usize,
                PCWSTR::from_raw(label.as_ptr()),
            );
        }
        let alert_label = native_interop::wide_str(if language == LanguageId::SimplifiedChinese {
            "额度提醒"
        } else {
            "Quota alerts"
        });
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            alert_menu.0 as usize,
            PCWSTR::from_raw(alert_label.as_ptr()),
        );

        // Settings submenu
        let settings_menu = CreatePopupMenu().unwrap();

        let startup_str = native_interop::wide_str(strings.start_with_windows);
        let startup_flags = if is_startup_enabled() {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        let _ = AppendMenuW(
            settings_menu,
            startup_flags,
            IDM_START_WITH_WINDOWS as usize,
            PCWSTR::from_raw(startup_str.as_ptr()),
        );

        let reset_pos_str = native_interop::wide_str(strings.reset_position);
        let _ = AppendMenuW(
            settings_menu,
            MENU_ITEM_FLAGS(0),
            IDM_RESET_POSITION as usize,
            PCWSTR::from_raw(reset_pos_str.as_ptr()),
        );

        let language_menu = CreatePopupMenu().unwrap();
        let system_label = native_interop::wide_str(strings.system_default);
        let system_flags = if language_override.is_none() {
            MF_CHECKED
        } else {
            MENU_ITEM_FLAGS(0)
        };
        let _ = AppendMenuW(
            language_menu,
            system_flags,
            IDM_LANG_SYSTEM as usize,
            PCWSTR::from_raw(system_label.as_ptr()),
        );

        for language in LanguageId::ALL {
            let id = match language {
                LanguageId::English => IDM_LANG_ENGLISH,
                LanguageId::Dutch => IDM_LANG_DUTCH,
                LanguageId::Spanish => IDM_LANG_SPANISH,
                LanguageId::French => IDM_LANG_FRENCH,
                LanguageId::German => IDM_LANG_GERMAN,
                LanguageId::Japanese => IDM_LANG_JAPANESE,
                LanguageId::Korean => IDM_LANG_KOREAN,
                LanguageId::SimplifiedChinese => IDM_LANG_SIMPLIFIED_CHINESE,
                LanguageId::TraditionalChinese => IDM_LANG_TRADITIONAL_CHINESE,
                LanguageId::Russian => IDM_LANG_RUSSIAN,
                LanguageId::PortugueseBrazil => IDM_LANG_PORTUGUESE_BRAZIL,
            };
            let label_str = native_interop::wide_str(language.native_name());
            let flags = if language_override == Some(language) {
                MF_CHECKED
            } else {
                MENU_ITEM_FLAGS(0)
            };
            let _ = AppendMenuW(
                language_menu,
                flags,
                id as usize,
                PCWSTR::from_raw(label_str.as_ptr()),
            );
        }

        let language_label = native_interop::wide_str(strings.language);
        let _ = AppendMenuW(
            settings_menu,
            MF_POPUP,
            language_menu.0 as usize,
            PCWSTR::from_raw(language_label.as_ptr()),
        );

        let style_settings_label = native_interop::wide_str(match language {
            LanguageId::SimplifiedChinese => "样式...",
            LanguageId::TraditionalChinese => "樣式...",
            LanguageId::Japanese => "スタイル...",
            LanguageId::Korean => "스타일...",
            LanguageId::Dutch => "Stijl...",
            LanguageId::Spanish => "Estilo...",
            LanguageId::French => "Style...",
            LanguageId::German => "Stil...",
            LanguageId::Russian => "Стиль...",
            LanguageId::PortugueseBrazil => "Estilo...",
            LanguageId::English => "Style...",
        });
        let _ = AppendMenuW(
            settings_menu,
            MENU_ITEM_FLAGS(0),
            IDM_STYLE_SETTINGS as usize,
            PCWSTR::from_raw(style_settings_label.as_ptr()),
        );

        let _ = AppendMenuW(settings_menu, MF_SEPARATOR, 0, PCWSTR::null());
        let version_label_text = if cfg!(feature = "github-update") {
            match available_update_version.as_deref() {
                Some(latest) => format!("v{} --> v{}", env!("CARGO_PKG_VERSION"), latest),
                None => format!("v{}", env!("CARGO_PKG_VERSION")),
            }
        } else {
            format!("v{} (Microsoft Store)", env!("CARGO_PKG_VERSION"))
        };
        let version_label = native_interop::wide_str(&version_label_text);
        let version_flags = if cfg!(feature = "github-update") {
            MENU_ITEM_FLAGS(0)
        } else {
            MF_GRAYED
        };
        let _ = AppendMenuW(
            settings_menu,
            version_flags,
            IDM_CHECK_UPDATE as usize,
            PCWSTR::from_raw(version_label.as_ptr()),
        );
        let releases_label = native_interop::wide_str(github_releases_menu_label(language));
        let _ = AppendMenuW(
            settings_menu,
            MENU_ITEM_FLAGS(0),
            IDM_OPEN_RELEASES as usize,
            PCWSTR::from_raw(releases_label.as_ptr()),
        );

        let settings_label = native_interop::wide_str(strings.settings);
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            settings_menu.0 as usize,
            PCWSTR::from_raw(settings_label.as_ptr()),
        );

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        let exit_str = native_interop::wide_str(strings.exit);
        let _ = AppendMenuW(
            menu,
            MENU_ITEM_FLAGS(0),
            2,
            PCWSTR::from_raw(exit_str.as_ptr()),
        );

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let _ = SetForegroundWindow(hwnd);
        let _ = TrackPopupMenu(menu, TPM_RIGHTBUTTON, pt.x, pt.y, 0, hwnd, None);
        let _ = DestroyMenu(menu);
    }
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
        let track_brush = CreateSolidBrush(COLORREF(track.to_colorref()));
        FillRect(hdc, &bar_rect, track_brush);
        let _ = DeleteObject(track_brush);

        let fill_width = (progress_width as f64 * percent_clamped / 100.0).round() as i32;
        if fill_width > 0 {
            let fill_rect = RECT {
                left: bar_x,
                top: bar_y,
                right: bar_x + fill_width,
                bottom: bar_y + bar_h,
            };
            let fill_brush = CreateSolidBrush(COLORREF(accent.to_colorref()));
            FillRect(hdc, &fill_rect, fill_brush);
            let _ = DeleteObject(fill_brush);
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
        let font_name = native_interop::wide_str("Segoe UI");
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
            widget_text_quality(),
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
                widget_text_quality(),
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

fn draw_panel(hdc: HDC, width: i32, height: i32, border: &Color, fill: &Color) {
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
        let border_brush = CreateSolidBrush(COLORREF(border.to_colorref()));
        FillRect(hdc, &outer, border_brush);
        let _ = DeleteObject(border_brush);

        let fill_brush = CreateSolidBrush(COLORREF(fill.to_colorref()));
        FillRect(hdc, &inner, fill_brush);
        let _ = DeleteObject(fill_brush);
    }
}

fn draw_drag_handle(hdc: HDC, height: i32, color: &Color) {
    let dot = sc(2).max(1);
    let gap_x = sc(1).max(1);
    let gap_y = sc(2).max(1);
    let matrix_h = dot * 3 + gap_y * 2;
    let origin_x = sc(DRAG_HANDLE_VISUAL_INSET_X);
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
            draw_rounded_rect(hdc, &rect, color, sc(1).max(1));
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
    fn transparent_layered_text_avoids_cleartype_background_fringe() {
        assert_eq!(
            text_quality_for_layered_surface(255, false),
            CLEARTYPE_QUALITY.0 as u32
        );
        assert_eq!(
            text_quality_for_layered_surface(254, false),
            NONANTIALIASED_QUALITY.0 as u32
        );
        assert_eq!(
            text_quality_for_layered_surface(255, true),
            NONANTIALIASED_QUALITY.0 as u32
        );
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
