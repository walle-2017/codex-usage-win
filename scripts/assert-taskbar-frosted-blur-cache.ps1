Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($token in @(
  'struct ParkedBlurBackdrop',
  'static PARKED_BLUR_BACKDROPS:',
  'fn park_embedded_blur_backdrop()',
  'fn restore_parked_embedded_blur_backdrop(parent: HWND) -> bool',
  'fn destroy_parked_blur_backdrops()',
  'TASKBAR_REPLICA_MAX_WINDOWS.saturating_sub(1)',
  'ShowWindow(hwnd, SW_HIDE)',
  'ShowWindow(hwnd, SW_SHOWNOACTIVATE)',
  'taskbar embedded blur cache parked',
  'taskbar embedded blur cache restored',
  'taskbar embedded blur cache invalid',
  'taskbar embedded blur CreateWindowExW failed',
  'taskbar embedded blur DirectComposition create failed',
  'destroy_parked_blur_backdrops();'
)) {
  if (-not $source.Contains($token)) { throw "Missing persistent Composition blur cache guard: $token" }
}
$start = $source.IndexOf('fn park_embedded_blur_backdrop()')
$end = $source.IndexOf('fn blur_amount_for_strength(', $start)
if ($start -lt 0 -or $end -le $start) { throw 'Blur cache helpers not found.' }
$cache = $source.Substring($start, $end - $start)
foreach ($forbidden in @('SetParent\s*\(', 'SetForegroundWindow\s*\(', 'AttachThreadInput\s*\(')) {
  if ($cache -match $forbidden) { throw "Blur cache must not reparent or activate: $forbidden" }
}
if (-not $cache.Contains('GetAncestor(hwnd, GA_PARENT) == parent') -or
    -not $cache.Contains('native_interop::destroy_composition_blur(entry.context)')) {
  throw 'Cached blur must verify its taskbar parent and release failed/stale Composition targets.'
}
$ensure = [regex]::Match($source, '(?s)fn\s+ensure_blur_backdrop\(.*?\n\}')
if (-not $ensure.Success -or
    -not $ensure.Value.Contains('park_embedded_blur_backdrop();') -or
    -not $ensure.Value.Contains('restore_parked_embedded_blur_backdrop(target)')) {
  throw 'Taskbar switch must park and restore existing Composition targets.'
}
Write-Host 'PASS: bounded taskbar-owned Composition blur cache is reused across return handoffs.'
