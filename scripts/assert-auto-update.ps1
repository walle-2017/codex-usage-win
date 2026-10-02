Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$WindowPath = Join-Path $PSScriptRoot '..\src\window.rs'
$MainPath = Join-Path $PSScriptRoot '..\src\main.rs'
$UpdaterPath = Join-Path $PSScriptRoot '..\src\updater.rs'
$TrayPath = Join-Path $PSScriptRoot '..\src\tray_icon.rs'
$CargoPath = Join-Path $PSScriptRoot '..\Cargo.toml'
$LocalizationPath = Join-Path $PSScriptRoot '..\src\localization\mod.rs'
$PopupMenuPath = Join-Path $PSScriptRoot '..\src\popup_menu.rs'

$window = Get-Content -Raw -LiteralPath $WindowPath
$main = Get-Content -Raw -LiteralPath $MainPath
$updater = Get-Content -Raw -LiteralPath $UpdaterPath
$tray = Get-Content -Raw -LiteralPath $TrayPath
$cargo = Get-Content -Raw -LiteralPath $CargoPath
$localization = Get-Content -Raw -LiteralPath $LocalizationPath
$popupMenu = Get-Content -Raw -LiteralPath $PopupMenuPath

function Assert-Match {
    param([string]$Text, [string]$Pattern, [string]$Message)
    if ($Text -notmatch $Pattern) { throw $Message }
}
function Assert-NoMatch {
    param([string]$Text, [string]$Pattern, [string]$Message)
    if ($Text -match $Pattern) { throw $Message }
}

# Existing secure updater contracts.
Assert-Match $window 'const\s+IDM_CHECK_UPDATE\s*:' 'window.rs must define IDM_CHECK_UPDATE.'
Assert-Match $window 'updater::start_update' 'The version command must start updater asynchronously.'
Assert-Match $popupMenu 'WM_MOUSEACTIVATE\s*=>' 'Popup submenus must explicitly suppress activation transfer.'
Assert-Match $popupMenu 'MA_NOACTIVATE' 'Popup submenus must return MA_NOACTIVATE.'
Assert-Match $popupMenu '(?s)WM_LBUTTONDOWN\s*=>.*?!state\.is_root.*?PopupAction::Command\(command\).*?dispatch_submenu_command\(state,\s*command\)' 'Submenu commands must dispatch from mouse-down because Explorer-hosted no-activate popups can lose mouse-up.'
Assert-Match $popupMenu '(?s)fn\s+dispatch_submenu_command\([^)]*\).*?PostMessageW\(root,\s*WM_CLOSE.*?PostMessageW\(\s*command_target,\s*WM_COMMAND' 'Submenu command dispatch must close the popup chain before posting WM_COMMAND to the app window.'
Assert-Match $popupMenu '(?s)WM_LBUTTONUP\s*=>.*?if\s+state\.is_root.*?SendMessageW\(\s*command_target,\s*WM_COMMAND' 'Root-menu commands must retain standard mouse-up activation.'
Assert-Match $window '(?s)fn\s+open_github_releases\([^)]*\).*?ShellExecuteW\(\s*HWND::default\(\)' 'Open GitHub must not use the Explorer-embedded child HWND as the ShellExecute owner.'
Assert-Match $window 'open GitHub Releases failed ShellExecute code=' 'Open GitHub failures must leave a diagnostic instead of failing silently.'
Assert-Match $window 'updater::WM_APP_UPDATE_RESULT\s*=>' 'Update results must return to the UI thread.'
Assert-Match $main '(?m)^mod\s+updater;' 'main.rs must register updater.'
Assert-Match $updater 'https://github\.com/walle-2017/codex-usage-win/releases/latest' 'Latest discovery must use the fork github.com redirect endpoint.'
Assert-NoMatch $updater 'https://api\.github\.com/repos/walle-2017/codex-usage-win/releases/latest' 'Normal discovery must not use the rate-limited REST latest-release API.'
Assert-Match $updater '\.redirects\(0\)' 'Latest discovery must inspect Location instead of blindly following.'
Assert-Match $updater 'https://github\.com/walle-2017/codex-usage-win/releases/download/' 'Release assets must stay pinned to this fork.'
Assert-Match $updater 'AtomicBool' 'Updater must guard concurrent operations.'
Assert-Match $updater 'compare_exchange' 'Updater must atomically reject duplicate starts.'
Assert-Match $updater 'std::thread::spawn' 'Network/disk work must stay off UI thread.'
Assert-Match $updater 'codex-usage-win\.exe\.sha256' 'Updater must require checksum asset.'
Assert-Match $updater '(?i)sha256' 'Updater must verify SHA256.'
Assert-Match $updater '\.old' 'Updater helper must retain rollback backup.'
Assert-Match $updater 'fn\s+github_status_detail\s*\(' 'HTTP failures must retain bounded diagnostics.'
Assert-Match $updater 'redact_url_userinfo' 'HTTP diagnostics must redact URL credentials.'

# Concise notification identity and severity.
Assert-Match $cargo 'FileDescription\s*=\s*"Codex Usage Win"' 'Windows FileDescription must be the Codex Usage Win name.'
Assert-Match $tray 'NIIF_NONE' 'Routine notifications must support no-warning-icon style.'
Assert-Match $tray 'pub\s+fn\s+notify_info\s*\(' 'Tray API must expose a non-warning info notification.'
Assert-Match $tray 'pub\s+fn\s+notify_warning\s*\(' 'Tray API must keep a warning path for real failures.'
Assert-Match $tray 'pub\s+fn\s+clear_notification\s*\(' 'Tray API must support clearing stale progress notifications.'
Assert-Match $window 'notify_info\(' 'Routine update/quota notifications must use the info style.'
Assert-Match $window 'notify_warning\(' 'Update failures must use the warning style.'

# Update UI must not queue obsolete progress messages or delay replacement.
Assert-Match $updater 'UPDATE_CHECK_NOTIFY_DELAY_MS' 'Checking notification must be delayed so fast checks stay silent.'
Assert-Match $updater 'UpdateProgress::Checking' 'Slow checks must still be able to show a checking message.'
Assert-Match $updater 'Updating\s*\{\s*version:\s*String\s*\}' 'Discovered updates must collapse download/checksum work into one Updating state.'
Assert-NoMatch $updater 'Restarting\s*\{\s*version:\s*String\s*\}' 'Updater must not queue a separate restarting notification.'
Assert-Match $window 'UpdateProgress::Updating' 'Window must display the single active update progress notification.'
Assert-Match $window 'clear_notification\(hwnd\)' 'Window must clear obsolete progress notification before final state/restart.'
Assert-NoMatch $window 'Duration::from_millis\(1200\)' 'Replacement must not be artificially delayed for notification visibility.'

# Successful update handoff is process-local; no marker file may remain beside the EXE.
Assert-Match $updater 'UPDATE_SUCCESS_ARG_PREFIX\s*:\s*&str\s*=\s*"--codex-usage-win-updated-to="' 'Updater must define an internal one-shot success argument.'
Assert-Match $updater 'fn\s+successful_update_version_from_args\s*\(' 'New process must validate the internal success argument.'
Assert-Match $updater 'fn\s+is_internal_update_arg\s*\(' 'Internal update arguments must be identifiable for filtering.'
Assert-Match $updater 'filter\([^\r\n]*is_internal_update_arg|filter_map\(' 'Preserved relaunch args must filter updater-internal state.'
Assert-Match $updater '\$LaunchArgs\s*=\s*@\(\$RelaunchArgs\)\s*\+\s*@\(\$SuccessArg\)' 'Replacement helper must append the one-shot success argument to preserved user arguments.'
Assert-Match $updater 'Start-Process\s+-FilePath\s+\$Target\s+-ArgumentList\s+\$LaunchArgs' 'Replacement helper must relaunch the updated executable with the one-shot success argument.'
Assert-NoMatch $updater 'UPDATE_SUCCESS_MARKER_SUFFIX|SuccessMarker|update-success|Set-Content\s+-LiteralPath' 'Updater must not use a persistent success marker file.'
Assert-Match $window 'successful_update_version_from_args' 'Window startup must consume the process-local update success argument.'
Assert-Match $window 'is_internal_update_arg' 'Explorer relaunch must not propagate updater-internal success arguments.'

# Quota warnings use the same concise style and aggregate one polling cycle into one notification.
Assert-Match $window 'fn\s+notify_quota_alerts\s*\(' 'Quota alerts must be aggregated by one notification helper.'
Assert-Match $window 'notify_quota_alerts\(hwnd,' 'Polling and threshold changes must call the aggregated quota notifier.'
Assert-Match $window 'join\("\\n"\)' 'Multiple quota windows must be combined into one multiline message.'
Assert-NoMatch $window 'for\s+alert\s+in\s+&alerts\s*\{\s*tray_icon::notify' 'Quota alerts must not emit one Shell notification per window.'

foreach ($field in @('update_checking','update_downloading','update_success','update_available')) {
    Assert-Match $localization ("pub " + $field + ":") "Localization Strings must define $field."
}
Assert-NoMatch $localization 'pub\s+update_restarting\s*:' 'Restarting localization is obsolete when no restart notification is queued.'

if ($updater -match 'upstream-ray/codex-usage-monitor|ShumTin/CodexTray') {
    throw 'Updater must never use upstream/original repositories.'
}
foreach ($pattern in @('codex login','codex auth','refresh_token','auth\.json.*write','Command::new\("codex"\)','Command::new\("codex\.exe"\)')) {
    if ($updater -match $pattern) { throw "Forbidden credential/CLI behavior: $pattern" }
}

# Final verification trigger after concise quota notification assertion alignment.
Write-Host 'PASS: updater uses concise non-blocking notifications, process-local success handoff, aggregated quota alerts, and secure fork-only updates.'
