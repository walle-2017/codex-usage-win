Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($term in @(
    'CODEX_TASKBAR_ACTIVATION_EDGE_VERIFY',
    'TIMER_TASKBAR_ACTIVATION_EDGE',
    'TaskbarActivationEdgeTrack',
    'fn observe_taskbar_activation_edge(',
    'taskbar activation-edge stage=',
    'since_failed_returned=',
    'last_inactive_duration_ms=',
    'observe_taskbar_activation_edge("foreground-event", true)',
    'observe_taskbar_activation_edge("mouse-down", true)',
    'observe_taskbar_activation_edge("before-reparent", true)',
    'observe_taskbar_activation_edge("reparent-failed", true)',
    'observe_taskbar_activation_edge("reparent-success", true)',
    'observe_taskbar_activation_edge("mouse-up", true)'
)) {
    if (-not $src.Contains($term)) { throw "Missing activation edge observer: $term" }
}
$observer = [regex]::Match($src, '(?s)fn\s+observe_taskbar_activation_edge\(.*?\n\}')
if (-not $observer.Success) { throw 'No activation-edge observer function.' }
if ($observer.Value -notmatch 'if\s+!taskbar_activation_edge_verify_enabled\(\)\s*\{\s*return;') {
    throw 'Activation edge observer must be opt-in.'
}
foreach ($forbidden in @('SetForegroundWindow\s*\(', 'SetParent\s*\(', 'SetWindowPos\s*\(', 'AttachThreadInput\s*\(', 'SendInput\s*\(', 'SetCapture\s*\(')) {
    if ($observer.Value -match $forbidden) { throw "Activation edge observer must be read-only: $forbidden" }
}
if ($src -notmatch 'if\s+taskbar_activation_edge_verify_enabled\(\)\s*\{\s*SetTimer\(hwnd, TIMER_TASKBAR_ACTIVATION_EDGE, 50, None\)') {
    throw 'Sampling timer must be opt-in.'
}
if ($src -notmatch 'if\s+explorer_activation_events_enabled\(\)\s*\|\|\s*taskbar_activation_edge_verify_enabled\(\)') {
    throw 'Activation edge diagnostic must register foreground observer.'
}
Write-Host 'PASS: activation-edge observer is read-only and only enabled explicitly.'
