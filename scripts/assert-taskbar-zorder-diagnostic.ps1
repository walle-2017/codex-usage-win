Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
function Assert-Term([string]$value) {
    if ($window -notmatch [regex]::Escape($value)) {
        throw "Missing z-order diagnostic contract: $value"
    }
}
foreach ($required in @(
    'CODEX_TASKBAR_ZORDER_DIAGNOSTIC',
    'fn zorder_diagnostic_enabled()',
    'fn log_taskbar_zorder_snapshot(',
    'fn zorder_above_relation(',
    'GW_OWNER_ZORDER_DIAGNOSTIC',
    'GW_PREV_ZORDER_DIAGNOSTIC',
    'tooltip_vs_taskbar=',
    'blur_vs_taskbar=',
    'tooltip_setwindowpos_ok=',
    'backdrop_setwindowpos_ok=',
    'log_taskbar_zorder_snapshot("before-reparent"',
    'log_taskbar_zorder_snapshot("reparent-failed"',
    'log_taskbar_zorder_snapshot("reparent-success"',
    'log_taskbar_zorder_snapshot("win-event-foreground"',
    'log_taskbar_zorder_snapshot("tooltip-hidden"',
    'log_tooltip_zorder_show(tooltip_placement_ok, backdrop_placement_ok)'
)) {
    Assert-Term $required
}
$observer = [regex]::Match($window, '(?s)fn\s+log_taskbar_zorder_snapshot\(.*?\n\}')
if (-not $observer.Success) { throw 'Missing z-order observer.' }
foreach ($forbidden in @('SetWindowPos\s*\(', 'SetForegroundWindow\s*\(', 'SetParent\s*\(', 'AttachThreadInput\s*\(', 'SendInput\s*\(')) {
    if ($observer.Value -match $forbidden) {
        throw "z-order observer must be read-only: $forbidden"
    }
}
if ($window -notmatch 'diagnose::is_enabled\(\)\s*&&\s*std::env::var_os\("CODEX_TASKBAR_ZORDER_DIAGNOSTIC"\)') {
    throw 'Must require --diagnose and explicit opt-in.'
}
# Popup owner investigation must remain opt-in and have no repair side effects.
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
foreach ($required in @(
    'CODEX_TASKBAR_POPUP_OWNER_DIAGNOSTIC',
    'fn popup_owner_diagnostic_enabled()',
    'fn log_popup_owner_snapshot(',
    'popup-switch-before-owner',
    'popup-switch-after-owner',
    'shell-foreground-or-menuend',
    'foreground_vs_backdrop=',
    'owner_matches_target=',
    'log_popup_owner_snapshot("shell-foreground-or-menuend", taskbar)'
)) {
    Assert-Term $required
}
foreach ($required in @(
    'taskbar popup owner transition',
    'before_owner={:?}',
    'after_owner={:?}',
    'set_last_error={:?}',
    'owner_matches={}',
    'raw_matches={}',
    'topmost_result={:?}'
)) {
    if (-not $native.Contains($required)) {
        throw "Missing native popup owner transition diagnostic: $required"
    }
}
$popupObserver = [regex]::Match($window, '(?s)fn log_popup_owner_snapshot\(.*?\n\}')
if (-not $popupObserver.Success) { throw 'Missing popup owner observer.' }
foreach ($forbidden in @(
    'SetWindowPos\s*\(', 'SetForegroundWindow\s*\(',
    'SetParent\s*\(', 'SetWindowLongPtrW\s*\(',
    'AttachThreadInput\s*\(', 'SendInput\s*\('
)) {
    if ($popupObserver.Value -match $forbidden) {
        throw "Popup owner snapshot must be read-only: $forbidden"
    }
}
if ($window -notmatch 'diagnose::is_enabled\(\)\s*&&\s*std::env::var_os\("CODEX_TASKBAR_POPUP_OWNER_DIAGNOSTIC"\)') {
    throw 'Popup owner diagnostic must require --diagnose and explicit opt-in.'
}
Write-Host 'PASS: taskbar/tooltip z-order is opt-in, read-only and correlated with SetParent.'
