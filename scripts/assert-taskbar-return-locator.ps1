Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($needle in @(
    'fn replica_taskbar_targets_from_existing_windows()',
    'LAST_RECOVERED_REPLICA_TARGET',
    'TASKBAR_PRIMARY_WINDOW.with(',
    'TASKBAR_REPLICA_WINDOWS.with(',
    'GetAncestor(child, GA_PARENT)',
    'native_interop::is_taskbar_window(parent)',
    'native_interop::get_taskbar_rect(parent)',
    'native_interop::monitor_device_name(parent)',
    'taskbar replica recovered missed target from parked HWND',
    'taskbar replica drop-miss cursor='
)) {
    if (-not $src.Contains($needle)) { throw "Missing parked-taskbar fallback: $needle" }
}
$fn = [regex]::Match($src, '(?s)fn replica_taskbar_targets_from_existing_windows\(.*?\n\}')
if (-not $fn.Success -or -not $fn.Value.Contains('if !taskbar_replica_enabled() { return Vec::new(); }')) {
    throw 'Fallback must be off without the opt-in replica experiment.'
}
foreach ($mutator in @(
 'SetParent\s*\(', 'SetWindowPos\s*\(', 'SetWindowLongW\s*\(',
 'SetForegroundWindow\s*\(', 'ShowWindow\s*\(', 'SendInput\s*\(',
 'AttachThreadInput\s*\(', 'CreateWindowExW\s*\('
)) {
    if ($fn.Value -match $mutator) { throw "Read-only fallback calls unsafe mutator: $mutator" }
}
$hit = [regex]::Match($src, '(?s)fn taskbar_at_point\(.*?\n\}')
if (-not $hit.Success) { throw 'Taskbar target locator missing.' }
$h = $hit.Value
$first = $h.IndexOf('native_interop::find_taskbars()')
$replica = $h.IndexOf('replica_taskbar_targets_from_existing_windows()')
$fallback = $h.IndexOf('state.taskbar_hwnd')
if ($first -lt 0 -or $replica -le $first -or $fallback -le $replica) {
    throw 'Exact Windows enumeration must precede parked-child parent fallback and legacy current-only fallback.'
}
if ($h.Contains('point_near_other_taskbar_top') -or $h.Contains('rect.top - extra')) {
    throw 'This fix must not reintroduce taskbar edge tolerance.'
}
if ($src.Contains('set_popup_owner_regular_band')) {
    throw 'Previously reverted Start-menu z-order experiment must stay absent.'
}
Write-Host 'PASS: existing-window-only return-taskbar locator with exact rect and safe HWND verification.'
