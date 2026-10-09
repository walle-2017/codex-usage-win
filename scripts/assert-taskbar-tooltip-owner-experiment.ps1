Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($part in @(
  'CODEX_TASKBAR_TOOLTIP_OWNER_EXPERIMENT',
  'CODEX_TASKBAR_DISABLE_RESET_TOOLTIP',
  'tooltip_owner_experiment_enabled',
  'tooltip_disable_experiment_enabled',
  'experiment_bind_tooltip_owner(tooltip)',
  'LAST_TOOLTIP_OWNER_ATTEMPT',
  'taskbar tooltip owner creation',
  'taskbar tooltip owner transition',
  'SetLastError(WIN32_ERROR(0))',
  'raw_parent_owner=',
  'EVENT_OBJECT_SHOW_ZORDER',
  'EVENT_OBJECT_HIDE_ZORDER',
  'EVENT_OBJECT_REORDER_ZORDER',
  'taskbar zorder event=',
  'log_taskbar_zorder_snapshot("win-event-zorder"',
  'SetWinEventHook(first, last, None, Some(on_explorer_activation_event), 0, 0, 0)'
)) {
    if (-not $src.Contains($part)) { throw "Missing tooltip isolation/owner diagnostic: $part" }
}
$func = [regex]::Match($src, '(?s)fn\s+experiment_bind_tooltip_owner\(.*?\n\}')
if (-not $func.Success) { throw 'Missing tooltip owner helper.' }
if ($func.Value -notmatch 'if\s+!tooltip_owner_experiment_enabled\(\)\s*\{\s*return;') {
    throw 'Owner experiment must be opt-in.'
}
foreach ($forbidden in @('SetForegroundWindow\s*\(', 'SetParent\s*\(', 'SetWindowPos\s*\(', 'ShowWindow\s*\(', 'AttachThreadInput\s*\(')) {
    if ($func.Value -match $forbidden) { throw "Owner helper changed unrelated taskbar/input behavior: $forbidden" }
}
$show = [regex]::Match($src, '(?s)fn\s+show_minimal_usage_tooltip\(.*?\n\}')
if (-not $show.Success) { throw 'Missing tooltip show function.' }
if ($show.Value -notmatch 'if\s+tooltip_disable_experiment_enabled\(\)\s*\{\s*return;\s*\}') {
    throw 'Tooltip isolation must return before HWND creation.'
}
$creation = [regex]::Match($src, '(?s)fn\s+minimal_tooltip_hwnd\(.*?\n\}')
if (-not $creation.Success -or $creation.Value -notmatch 'WS_POPUP.*') {
    throw 'Missing WS_POPUP tooltip creation.'
}
if ($creation.Value -notmatch 'desired_owner,\s*HMENU::default\(\)') {
    throw 'Tooltip owner must be supplied at popup creation.'
}
if ($src -notmatch 'if\s+zorder_diagnostic_enabled\(\)\s*\{\s*ranges\.push\(\(EVENT_OBJECT_SHOW_ZORDER, EVENT_OBJECT_REORDER_ZORDER\)\)') {
    throw 'Reorder event hook must remain opt-in.'
}
Write-Host 'PASS: tooltip creation owner, disabled-popup isolation and read-only event tracing.'
