Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($t in @(
    'CODEX_TASKBAR_EXTERNAL_MOUSE_DIAGNOSTIC',
    'fn external_mouse_diagnostic_enabled()',
    'fn on_external_mouse_click(',
    'fn drain_external_clicks(',
    'fn log_external_click_context(',
    'fn log_reparent_window_state(',
    'WH_MOUSE_LL',
    'CallNextHookEx(None, code, wparam, lparam)',
    'TIMER_EXTERNAL_MOUSE_DIAGNOSTIC',
    'UnhookWindowsHookEx(hook)',
    'taskbar external-click context stage=',
    'taskbar window-state stage=',
    'log_external_click_context("before-reparent")',
    'log_reparent_window_state("reparent-failed"',
    'log_reparent_window_state("reparent-success"'
)) { if (-not $src.Contains($t)) { throw "Missing external mouse / reparent diagnostic: $t" } }
$callback = [regex]::Match($src, '(?s)unsafe extern "system" fn on_external_mouse_click\(.*?\n\}')
if (-not $callback.Success) { throw 'Missing low-level callback.' }
if ($callback.Value -notmatch 'wparam\.0 as u32 == WM_LBUTTONDOWN') {
    throw 'Callback must observe left-down events only.'
}
foreach ($forbidden in @('diagnose::log\s*\(', 'SetParent\s*\(', 'SetForegroundWindow\s*\(', 'SetCapture\s*\(', 'SendInput\s*\(', 'SetWindowPos\s*\(', 'PostMessageW\s*\(')) {
    if ($callback.Value -match $forbidden) { throw "Callback must not perform I/O or change window/input state: $forbidden" }
}
$context = [regex]::Match($src, '(?s)fn log_reparent_window_state\(.*?\n\}')
if (-not $context.Success -or $context.Value -notmatch 'if\s+!external_mouse_diagnostic_enabled\(\)\s*\{\s*return;') {
    throw 'Window state logging must be explicitly opt-in.'
}
if ($src -notmatch 'if\s+external_mouse_diagnostic_enabled\(\)\s*\{\s*SetTimer\(hwnd, TIMER_EXTERNAL_MOUSE_DIAGNOSTIC, 50, None\)') {
    throw 'Timer must be gated.'
}
Write-Host 'PASS: left-button hook is opt-in, bounded and does not consume or synthesize input.'
