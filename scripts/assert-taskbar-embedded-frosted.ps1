Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($token in @(
  'CODEX_TASKBAR_EMBEDDED_FROSTED_EXPERIMENT',
  'fn embedded_frosted_experiment_enabled()',
  'if was_embedded && embedded_frosted_experiment_enabled()',
  'taskbar embedded dual-window composition blur activated',
  'taskbar embedded dual-window composition blur disabled',
  'taskbar embedded blur target switched; recreate child backdrop',
  'if child_mode { WS_CHILD | WS_CLIPSIBLINGS } else { WS_POPUP }',
  'taskbar_parent.unwrap_or_default()',
  'if !child_mode {',
  'if !embedded_frosted_experiment_enabled() {',
  'SetWindowPos(backdrop_hwnd, foreground_hwnd,',
  'rect.left - tr.left, rect.top - tr.top',
  'SWP_NOACTIVATE | SWP_SHOWWINDOW'
)) {
  if (-not $source.Contains($token)) { throw "Missing embedded frosted experiment contract: $token" }
}
$predicate = [regex]::Match($source, '(?s)fn\s+embedded_frosted_experiment_enabled\(\).*?\n\}')
if (-not $predicate.Success -or
    -not $predicate.Value.Contains('diagnose::is_enabled()') -or
    -not $predicate.Value.Contains('taskbar_replica_enabled()')) {
  throw 'Embedded frosted experiment must require --diagnose and replica mode.'
}
$activate = [regex]::Match($source, '(?s)fn\s+activate_blur_popup\(.*?\n\}')
if (-not $activate.Success -or
    $activate.Value.IndexOf('if was_embedded && embedded_frosted_experiment_enabled()') -gt
    $activate.Value.IndexOf('native_interop::detach_from_taskbar_as_popup(hwnd)')) {
  throw 'Embedded blur must return before any foreground detach.'
}
$create = [regex]::Match($source, '(?s)fn\s+ensure_blur_backdrop\(.*?\n\}')
if (-not $create.Success -or
    -not $create.Value.Contains('native_interop::create_composition_blur') -or
    -not $create.Value.Contains('destroy_blur_backdrop()')) {
  throw 'Child composition backdrop must preserve blur creation and target recreation.'
}
Write-Host 'PASS: embedded frosted mode is opt-in and does not detach the widget.'
