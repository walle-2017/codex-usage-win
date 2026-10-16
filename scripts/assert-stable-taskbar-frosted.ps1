Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$src += "`n" + (Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\taskbar_replica.rs'))
$src += "`n" + (Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\blur_backdrop.rs'))
if ($src -notmatch 'mod taskbar_replica;' -or $src -notmatch 'mod blur_backdrop;') {
  throw 'Permanent taskbar modules must be explicitly declared.'
}
$interop = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$main = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\main.rs')

if ($src.Contains('CODEX_TASKBAR_') -or $interop.Contains('CODEX_TASKBAR_')) {
  throw 'Legacy taskbar environment variable remains in runtime source.'
}
foreach ($pattern in @(
  'frosted_popup_session',
  'popup_owner_retry', 'popup_zorder_repair', 'popup_ownerless_frosted',
  'select_taskbar_for_popup_window', 'detach_from_taskbar_as_popup',
  'TIMER_FOCUS_EXPERIMENT', 'TIMER_EXPLORER_STATE_VERIFY',
  'TIMER_TASKBAR_ACTIVATION_EDGE', 'TIMER_EXTERNAL_MOUSE_DIAGNOSTIC',
  'TIMER_CURSOR_FOREGROUND_VERIFY', 'log_handoff_deep_snapshot',
  'reparent_diagnostic_active', 'GetGUIThreadInfo', 'AttachThreadInput'
)) {
  if (($src + $interop) -match $pattern) {
    throw "Removed experimental API is still referenced: $pattern"
  }
}
foreach ($token in @(
  'fn taskbar_replica_enabled() -> bool',
  'fn try_taskbar_replica_handoff(',
  'fn replica_taskbar_targets_from_existing_windows()',
  'fn taskbar_at_point(',
  'fn activate_blur_backdrop(',
  'fn park_embedded_blur_backdrop()',
  'fn restore_parked_embedded_blur_backdrop(parent: HWND) -> bool',
  'fn destroy_parked_blur_backdrops()',
  'TASKBAR_REPLICA_MAX_WINDOWS',
  'TASKBAR_REPLICA_MAX_PRIVATE_DELTA',
  'WS_CHILD | WS_CLIPSIBLINGS',
  'GetAncestor(blur, GA_PARENT) == taskbar.hwnd',
  'let shown = after_render_ok && backdrop_ready',
  'taskbar replica show failed, original HWND restored',
  'taskbar embedded blur cache restored'
)) {
  if (-not $src.Contains($token)) { throw "Stable taskbar invariant missing: $token" }
}
# Both tooltip windows must follow the active taskbar without changing the
# taskbar-embedded foreground replica or blur-sibling ownership.
foreach ($token in @(
  'fn sync_minimal_tooltip_owner(hwnd: HWND, taskbar_owner: Option<HWND>)',
  'GetWindow(hwnd, GW_OWNER).ok() != Some(owner)',
  'sync_minimal_tooltip_owner(tooltip, taskbar_owner)',
  'sync_minimal_tooltip_owner(backdrop, taskbar_owner)'
)) {
  if (-not $src.Contains($token)) {
    throw "Reset-time tooltip/taskbar ownership invariant missing: $token"
  }
}
$replica = [regex]::Match($src, '(?s)fn\s+taskbar_replica_enabled\(\).*?\n\}')
if (-not $replica.Success -or -not ($replica.Value -match '\btrue\b') -or
    $replica.Value.Contains('diagnose::is_enabled()')) {
  throw 'Taskbar replica must be enabled without --diagnose.'
}
$activate = [regex]::Match($src, '(?s)fn\s+activate_blur_backdrop\(.*?\n\}')
if (-not $activate.Success -or -not $activate.Value.Contains('ensure_blur_backdrop(') -or
    $activate.Value.Contains('SetParent(')) {
  throw 'Frosted mode must use the embedded Composition target.'
}
if ($main -notmatch '"--diagnose"') { throw 'Diagnostic-only logging switch must be retained.' }
Write-Host 'PASS: stable taskbar replica, embedded blur cache, guarded handoff and diagnose-independent behavior.'
