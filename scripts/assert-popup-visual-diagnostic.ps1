Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')

foreach ($token in @(
    'CODEX_TASKBAR_POPUP_VISUAL_DIAGNOSTIC',
    'fn popup_visual_diagnostic_enabled()',
    'fn log_popup_visual_snapshot(',
    'DwmGetWindowAttribute',
    'DWMWA_CLOAKED_POPUP_VISUAL',
    'DWMWA_EXTENDED_FRAME_BOUNDS_POPUP_VISUAL',
    'WindowFromPoint(point)',
    'foreground_above_taskbar=',
    'center_root_class=',
    'log_popup_visual_snapshot("popup-switch-after-owner")',
    'log_popup_visual_snapshot("owner-retry-recovered")',
    'log_popup_visual_snapshot("shell-foreground-or-menuend")'
)) {
    if (-not $source.Contains($token)) { throw "Missing visual diagnostic contract: $token" }
}
$predicate = [regex]::Match($source, '(?s)fn\s+popup_visual_diagnostic_enabled\(\).*?\n\}')
if (-not $predicate.Success -or -not $predicate.Value.Contains('diagnose::is_enabled()') -or
    -not $predicate.Value.Contains('CODEX_TASKBAR_POPUP_VISUAL_DIAGNOSTIC')) {
    throw 'Visual diagnostics must require --diagnose and explicit opt-in.'
}
$observer = [regex]::Match($source, '(?s)fn\s+log_popup_visual_snapshot\(.*?\n\}')
if (-not $observer.Success) { throw 'Missing visual diagnostic function.' }
foreach ($bad in @(
    'SetWindowPos\s*\(', 'SetParent\s*\(', 'SetWindowLongPtrW\s*\(',
    'SetForegroundWindow\s*\(', 'SendInput\s*\(', 'DwmSetWindowAttribute\s*\('
)) {
    if ($observer.Value -match $bad) { throw "Visual observer must remain read-only: $bad" }
}
Write-Host 'PASS: popup visual diagnostic is opt-in and read-only, including DWM checks.'
