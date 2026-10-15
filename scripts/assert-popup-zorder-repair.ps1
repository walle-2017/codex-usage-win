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
if ($callback.Value -notmatch 'taskbar popup zorder repair deferred check=' -or
    $callback.Value -notmatch 'active_class=') {
  throw 'Blocked repairs must explain active window and drag/menu gates.'
}
if ($source -notmatch 'const POPUP_ZORDER_REPAIR_MAX_CHECKS: u8 = 120;' -or
    $source -notmatch 'const POPUP_ZORDER_REPAIR_MAX_ACTIONS: u8 = 3;') {
  throw 'Observation must be bounded and correction cap must not increase.'
}
$unknownHit = [regex]::Match($source, '(?s)fn\s+popup_unknown_order_hit_taskbar\(.*?\n\}')
if (-not $unknownHit.Success -or
    -not $unknownHit.Value.Contains('WindowFromPoint') -or
    -not $unknownHit.Value.Contains('root == taskbar') -or
    -not $unknownHit.Value.Contains('hits >= 2')) {
  throw 'Unknown-order fallback must use multiple observed taskbar root hits.'
}
if ($callback.Value -notmatch 'before_foreground == "unknown"' -or
    $callback.Value -notmatch 'before_backdrop == "unknown"' -or
    $callback.Value -notmatch 'stable_unknown_occlusion = pending.unknown_hit_checks >= 2' -or
    $callback.Value -notmatch '\|\| stable_unknown_occlusion') {
  throw 'Unknown-order fallback must require both popup relations unknown and repeated hits.'
}
if ($callback.Value -notmatch 'taskbar popup zorder repair unknown-hit' -or
    $callback.Value -notmatch 'unknown_occlusion=') {
  throw 'Unknown-order recovery must provide precise diagnostic evidence.'
}
Write-Host 'PASS: popup z-order repair is bounded, opt-in, and limited to observed inversions.'
