Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')

foreach ($token in @(
    'CODEX_TASKBAR_POPUP_OWNER_RETRY_EXPERIMENT',
    'fn popup_owner_retry_experiment_enabled()',
    'TIMER_POPUP_OWNER_RETRY',
    'POPUP_OWNER_RETRY_MAX_ATTEMPTS',
    'fn arm_popup_owner_retry(',
    'fn poll_popup_owner_retry(',
    'arm_popup_owner_retry(foreground_hwnd, taskbar.hwnd);',
    'sync_blur_backdrop_zorder(foreground);',
    'taskbar popup owner retry recovered',
    'taskbar popup owner retry exhausted'
)) {
    if (-not $window.Contains($token)) { throw "Missing owner retry guard: $token" }
}
foreach ($token in @(
    'pub fn popup_owner_matches(',
    'pub fn try_rebind_popup_owner(',
    'get_popup_owner_diagnostic(hwnd, GW_OWNER_POPUP_DIAGNOSTIC)',
    'taskbar popup owner retry hwnd='
)) {
    if (-not $native.Contains($token)) { throw "Missing native owner retry guard: $token" }
}
$predicate = [regex]::Match($window, '(?s)fn\s+popup_owner_retry_experiment_enabled\(\).*?\n\}')
if (-not $predicate.Success -or
    -not $predicate.Value.Contains('diagnose::is_enabled()') -or
    -not $predicate.Value.Contains('taskbar_replica_enabled()') -or
    -not $predicate.Value.Contains('CODEX_TASKBAR_POPUP_OWNER_RETRY_EXPERIMENT')) {
    throw 'Owner retry must be diagnose-only and opt-in under replica experiment.'
}
$retry = [regex]::Match($native, '(?s)pub fn\s+try_rebind_popup_owner\(.*?\n\}')
if (-not $retry.Success) { throw 'Missing native owner retry.' }
foreach ($forbidden in @('SetWindowPos\s*\(', 'SetParent\s*\(', 'SetForegroundWindow\s*\(')) {
    if ($retry.Value -match $forbidden) { throw "Owner retry must not manipulate other window state: $forbidden" }
}
$poll = [regex]::Match($window, '(?s)fn\s+poll_popup_owner_retry\(.*?\n\}')
if (-not $poll.Success) { throw 'Missing owner timer callback.' }
if ($poll.Value -notmatch 'if backdrop_ok && foreground_ok \{\s*stop_popup_owner_retry\(hwnd\);') {
    throw 'Both popup owners must be verified before any recovery z-order sync.'
}
if ($poll.Value -notmatch 'if dragging \{\s*return;') {
    throw 'Owner recovery must not modify windows during a drag.'
}
Write-Host 'PASS: popup owner recovery is bounded, opt-in, and keeps z-order stable on errors.'
