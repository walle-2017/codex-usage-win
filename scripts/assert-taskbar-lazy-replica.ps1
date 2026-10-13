Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$src=Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach($needle in @(
 'CODEX_TASKBAR_LAZY_REPLICA_EXPERIMENT',
 'TASKBAR_REPLICA_MAX_WINDOWS: usize = 4',
 'TASKBAR_REPLICA_MAX_PRIVATE_DELTA',
 'fn replica_resources_acceptable(',
 'fn taskbar_replica_service_hwnd(',
 'taskbar replica cache pruned stale_slots=',
 'taskbar replica health active=',
 'fn replica_wnd_proc(',
 'fn register_taskbar_replica_class(',
 'fn try_taskbar_replica_handoff(',
 'taskbar replica resource delta',
 'taskbar replica switch success',
 'CreateWindowExW(',
 'taskbar replica popup created hwnd='
 'taskbar replica initial-embed target='
 'taskbar replica initial embed refused, source window unchanged'
 'WS_POPUP',
 'taskbar replica show failed, original HWND restored',
 'native_interop::set_tray_event_hook(thread_id, on_tray_location_changed)',
 'if let Some(switched) = try_taskbar_replica_handoff(hwnd, index, taskbar)',
 'TASKBAR_PRIMARY_WINDOW.with(|cell| cell.set(hwnd.0 as isize))'
)) { if(-not $src.Contains($needle)){throw "Missing replica safeguard: $needle"} }
$start=$src.IndexOf('fn try_taskbar_replica_handoff(')
$end=$src.IndexOf('fn attach_to_taskbar_window(', $start)
if($start -lt 0 -or $end -le $start){ throw 'Replica handoff not found.' }
$method=$src.Substring($start,$end-$start)
foreach($unsafePath in @('SetParent\s*\(', 'AttachThreadInput\s*\(', 'SetForegroundWindow\s*\(', 'SendInput\s*\(', 'std::thread::spawn\s*\(')) {
 if($method -match $unsafePath){throw "Replica handoff must not use $unsafePath"}
}
if ($method -notmatch 'if !embedded \|\| !dragging \|\| parent == taskbar.hwnd' -or $method -notmatch 'if frosted') {
 throw 'Replica handoff must be restricted to embedded non-frosted cross-monitor drag.'
}
if (($method -notmatch 'native_interop::embed_in_taskbar\(created, taskbar.hwnd\)') -or
    ($method -notmatch 'DestroyWindow\(created\)')) {
 throw 'A new popup must be initially embedded or destroyed; source cannot be reparented.'
}
if ($src -notmatch 'SetCapture\(active_hwnd\)' -or $src -notmatch 'move_window_without_repaint\(\s*drag_hwnd,') {
 throw 'Replica handoff must transfer capture and drag position to the active HWND.'
}
if ($src -notmatch 'if !taskbar_replica_enabled\(\) \{ return None; \}') {
 throw 'Replica must be opt-in.'
}

# The primary window must own callbacks and timers even after AppState.hwnd
# points at an active replica. Creating a second poll timer is forbidden.
foreach($needle in @(
    'let hwnd = taskbar_replica_service_hwnd(s.hwnd.to_hwnd());',
    'tray_icon::sync(taskbar_replica_service_hwnd(hwnd), icon.as_ref());',
    'let hwnd = taskbar_replica_service_hwnd(hwnd);',
    'SendHwnd::from_hwnd(taskbar_replica_service_hwnd(hwnd))',
    'updater::start_update(taskbar_replica_service_hwnd(hwnd))',
    'SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_POLL,',
    'SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_COUNTDOWN,',
    'SetTimer(taskbar_replica_service_hwnd(hwnd), TIMER_RESET_POLL,',
    'style_window::open_or_focus('
)) {
    if(-not $src.Contains($needle)) { throw "Service owner regression: $needle" }
}
if($src -match 'SetTimer\(s.hwnd.to_hwnd\(\), TIMER_') {
    throw 'AppState current visible HWND must not own service timers.'
}

Write-Host 'PASS: lazy replica handoff, stable timer/tray owner, cache pruning and resource telemetry.'
