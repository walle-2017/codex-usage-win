Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($part in @(
  'CODEX_TASKBAR_TOOLTIP_OWNER_EXPERIMENT',
  'tooltip_owner_experiment_enabled',
  'experiment_bind_tooltip_owner(tooltip)',
  'GWLP_HWNDPARENT',
  'EVENT_OBJECT_SHOW_ZORDER',
  'EVENT_OBJECT_HIDE_ZORDER',
  'EVENT_OBJECT_REORDER_ZORDER',
  'taskbar zorder event=',
  'log_taskbar_zorder_snapshot("win-event-zorder"',
  'SetWinEventHook(first, last, None, Some(on_explorer_activation_event), 0, 0, 0)'
)) {
    if (-not $src.Contains($part)) { throw "Missing popup owner or reorder tracing: $part" }
}
$behavior = [regex]::Match($src, '(?s)fn\s+experiment_bind_tooltip_owner\(.*?\n\}')
if (-not $behavior.Success) { throw 'Owner experiment helper not found.' }
if ($behavior.Value -notmatch 'if\s+!tooltip_owner_experiment_enabled\(\)\s*\{\s*return;') {
    throw 'Owner change must be strictly opt-in.'
}
foreach ($notAllowed in @('SetForegroundWindow\s*\(', 'SetParent\s*\(', 'SetWindowPos\s*\(', 'ShowWindow\s*\(', 'AttachThreadInput\s*\(')) {
    if ($behavior.Value -match $notAllowed) { throw "Owner experiment must not change focus, parenting or order: $notAllowed" }
}
if ($src -notmatch 'if\s+zorder_diagnostic_enabled\(\)\s*\{\s*ranges\.push\(\(EVENT_OBJECT_SHOW_ZORDER, EVENT_OBJECT_REORDER_ZORDER\)\)') {
    throw 'Reorder hook must be enabled only with z-order diagnostics.'
}
if ($src -notmatch 'if\s+!zorder_diagnostic_enabled\(\)\s*\|\|\s*object_id\s*!=\s*0\s*\|\|\s*child_id\s*!=\s*0') {
    throw 'Reorder event filtering must reject non-window accessibility events.'
}
Write-Host 'PASS: owner is opt-in; z-order events are read-only and filtered.'
