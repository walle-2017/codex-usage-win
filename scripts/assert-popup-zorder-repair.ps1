Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')

foreach ($token in @(
  'CODEX_TASKBAR_POPUP_ZORDER_REPAIR_EXPERIMENT',
  'fn popup_zorder_repair_experiment_enabled()',
  'fn arm_popup_zorder_repair()',
  'fn poll_popup_zorder_repair(',
  'TIMER_POPUP_ZORDER_REPAIR',
  'POPUP_ZORDER_REPAIR_MAX_CHECKS',
  'POPUP_ZORDER_REPAIR_MAX_ACTIONS',
  'arm_popup_zorder_repair();',
  'taskbar popup zorder repair attempt=',
  'taskbar popup zorder repair finished',
  'log_popup_visual_snapshot("zorder-repair-attempt")'
)) {
  if (-not $source.Contains($token)) { throw "Missing opt-in popup z-order repair contract: $token" }
}
$predicate = [regex]::Match($source, '(?s)fn\s+popup_zorder_repair_experiment_enabled\(\).*?\n\}')
if (-not $predicate.Success -or
    -not $predicate.Value.Contains('diagnose::is_enabled()') -or
    -not $predicate.Value.Contains('taskbar_replica_enabled()') -or
    -not $predicate.Value.Contains('CODEX_TASKBAR_POPUP_ZORDER_REPAIR_EXPERIMENT')) {
  throw 'Popup z-order repair must require --diagnose, replica mode and explicit opt-in.'
}
$callback = [regex]::Match($source, '(?s)fn\s+poll_popup_zorder_repair\(.*?\n\}')
if (-not $callback.Success) { throw 'Missing z-order repair callback.' }
foreach ($forbidden in @('SetWindowLongPtrW\s*\(', 'SetParent\s*\(', 'SetForegroundWindow\s*\(', 'SendInput\s*\(')) {
  if ($callback.Value -match $forbidden) { throw "Z-order repair must not change ownership, parenting or activation: $forbidden" }
}
if ($callback.Value -notmatch 'before_foreground == "below" \|\| before_backdrop == "below"') {
  throw 'Popup z-order repair must only act on an observed below-taskbar relation.'
}
if ($callback.Value -notmatch 'if !dragging && !shell_menu' -or
    $callback.Value -notmatch 'SWP_NOMOVE \| SWP_NOSIZE \| SWP_NOACTIVATE') {
  throw 'Popup repair must not activate or move windows or reorder during drag/menu.'
}
if ($callback.Value -notmatch 'SetWindowPos\(backdrop, HWND_TOPMOST' -or
    $callback.Value -notmatch 'SetWindowPos\(foreground, HWND_TOPMOST') {
  throw 'Repair must restore both backdrop and foreground in that order.'
}
Write-Host 'PASS: popup z-order repair is bounded, opt-in, and limited to observed inversions.'
