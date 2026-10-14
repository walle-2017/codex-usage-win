Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($needle in @(
    'POPUP_VISITED_TASKBAR_LIMIT: usize = 4',
    'fn remember_popup_visited_taskbar(',
    'fn popup_visited_taskbar_at_point(',
    'popup_visited_taskbar_at_point(pt)',
    'remember_popup_visited_taskbar(taskbar);',
    'remember_popup_visited_taskbar(found);',
    'native_interop::is_taskbar_window(entry.hwnd)',
    'native_interop::get_taskbar_rect(entry.hwnd)',
    'monitor != entry.monitor_device',
    'taskbar popup recovered visited target',
    'popup_visited_targets={:?}'
)) {
    if (-not $source.Contains($needle)) { throw "Missing popup return-target safeguard: $needle" }
}
$finder = [regex]::Match($source, '(?s)fn popup_visited_taskbar_at_point\(.*?\n\}')
if (-not $finder.Success) { throw 'Popup return-taskbar locator missing.' }
$locator = $finder.Value
foreach ($needle in @(
    'if !taskbar_replica_enabled()',
    'is_some_and(|s| !s.embedded)',
    'point_is_in_taskbar_rect(pt, entry.rect)',
    'targets.retain_mut'
)) {
    if (-not $locator.Contains($needle)) { throw "Unsafe popup cache lookup: $needle" }
}
foreach ($mutator in @(
    'SetParent\s*\(', 'SetWindowPos\s*\(', 'SetWindowLongW\s*\(',
    'SetForegroundWindow\s*\(', 'SendInput\s*\(', 'CreateWindowExW\s*\(',
    'ShowWindow\s*\(', 'AttachThreadInput\s*\('
)) {
    if ($locator -match $mutator) { throw "Popup locator must remain read-only: $mutator" }
}
$atPoint = [regex]::Match($source, '(?s)fn taskbar_at_point\(.*?\n\}')
if (-not $atPoint.Success) { throw 'Taskbar hit finder missing.' }
$at = $atPoint.Value
$a = $at.IndexOf('native_interop::find_taskbars()')
$b = $at.IndexOf('replica_taskbar_targets_from_existing_windows()')
$c = $at.IndexOf('popup_visited_taskbar_at_point(pt)')
$d = $at.IndexOf('state.taskbar_hwnd')
if ($a -lt 0 -or $b -le $a -or $c -le $b -or $d -le $c) {
    throw 'Exact enumeration, parked child, popup-visited and current-only fallback order changed.'
}
if ($source.Contains('set_popup_owner_regular_band') -or
    $at.Contains('point_near_other_taskbar_top')) {
    throw 'Previous z-order and fuzzy edge experiment must not return.'
}
Write-Host 'PASS: opt-in frosted popup visited-taskbar cache uses exact verified rectangles only.'
