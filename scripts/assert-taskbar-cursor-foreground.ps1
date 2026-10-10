Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($needle in @(
  'CODEX_TASKBAR_CURSOR_FOREGROUND_VERIFY',
  'TIMER_CURSOR_FOREGROUND_VERIFY',
  'fn log_cursor_foreground_snapshot(',
  'fn cursor_in_rect(',
  'GetCursorPos(&mut cursor)',
  'WindowFromPoint(cursor)',
  'GetWindowRect(hwnd, &mut rect)',
  'taskbar cursor-foreground stage=',
  'last_external_in_source=',
  'log_cursor_foreground_snapshot("cursor-region-change", None, None, true)',
  'log_cursor_foreground_snapshot("widget-mouseleave", None, None, false)',
  'log_cursor_foreground_snapshot("before-reparent", None, Some(taskbar.hwnd), false)',
  'log_cursor_foreground_snapshot("reparent-failed", None, Some(taskbar.hwnd), false)',
  'log_cursor_foreground_snapshot("reparent-success", None, Some(taskbar.hwnd), false)',
  '"foreground-event", Some((event_hwnd, event_time_ms)), None, false'
)) {
  if (-not $src.Contains($needle)) { throw "Missing cursor/foreground diagnostic: $needle" }
}
$fn = [regex]::Match($src, '(?s)fn\s+log_cursor_foreground_snapshot\(.*?\n\}')
if (-not $fn.Success -or $fn.Value -notmatch 'if\s+!cursor_foreground_verify_enabled\(\)\s*\{\s*return;\s*\}') {
  throw 'Cursor observer is not opt-in.'
}
foreach ($bad in @('SetForegroundWindow\s*\(', 'SetParent\s*\(', 'SetWindowPos\s*\(', 'TrackMouseEvent\s*\(', 'AttachThreadInput\s*\(', 'SetCapture\s*\(', 'SendInput\s*\(')) {
  if ($fn.Value -match $bad) { throw "Cursor observer must be read-only: $bad" }
}
if ($src -notmatch 'if\s+cursor_foreground_verify_enabled\(\)\s*\{\s*SetTimer\(hwnd, TIMER_CURSOR_FOREGROUND_VERIFY, 50, None\)') {
  throw 'Cursor observer timer must be opt-in.'
}
if ($src -notmatch 'if\s+explorer_activation_events_enabled\(\)\s*\|\|\s*taskbar_activation_edge_verify_enabled\(\)\s*\|\|\s*cursor_foreground_verify_enabled\(\)') {
  throw 'Foreground hook must be opt-in and include cursor diagnostic.'
}
Write-Host 'PASS: read-only cursor/foreground state tied to asynchronous foreground events.'
