Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$src=Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach($needle in @(
 'CODEX_TASKBAR_LAZY_REPLICA_EXPERIMENT',
 'TASKBAR_REPLICA_MAX_WINDOWS: usize = 4',
 'TASKBAR_REPLICA_MAX_PRIVATE_DELTA',
 'fn replica_resources_acceptable(',
 'fn replica_wnd_proc(',
 'fn register_taskbar_replica_class(',
 'fn try_taskbar_replica_handoff(',
 'taskbar replica resource delta',
 'taskbar replica switch success',
 'CreateWindowExW(',
 'WS_CHILD | WS_CLIPSIBLINGS',
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
if ($src -notmatch 'if !taskbar_replica_enabled\(\) \{ return None; \}') {
 throw 'Replica must be opt-in.'
}
Write-Host 'PASS: lazy one-process cached HWND handoff, resource guard, no cross-taskbar SetParent.'
