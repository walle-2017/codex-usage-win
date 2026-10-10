Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$win = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($s in @(
    'CODEX_TASKBAR_DETACHED_BRIDGE_EXPERIMENT',
    'fn taskbar_bridge_experiment_enabled()',
    'fn try_detached_taskbar_bridge(',
    'taskbar detached-bridge begin',
    'taskbar detached-bridge target_result=',
    'taskbar detached-bridge end',
    'SetParent(hwnd, HWND::default())',
    'GetAncestor(hwnd, GA_PARENT) == source',
    'BRIDGE_RESCUED_AS_POPUP',
    'SWP_NOACTIVATE',
    'taskbar detached-bridge emergency popup rescue'
)) { if (-not $src.Contains($s)) { throw "Missing bridge experiment safeguard: $s" } }
foreach ($s in @(
    'native_interop::take_bridge_popup_rescue()',
    's.embedded = false;',
    'taskbar detached-bridge popup rescue'
)) { if (-not $win.Contains($s)) { throw "Missing window rescue state: $s" } }
$fn = [regex]::Match($src, '(?s)unsafe fn try_detached_taskbar_bridge\(.*?\n\}')
if (-not $fn.Success) { throw 'Missing bridge implementation.' }
foreach ($bad in @('SendInput\s*\(', 'SetForegroundWindow\s*\(', 'AttachThreadInput\s*\(', 'SetCapture\s*\(', 'ReleaseCapture\s*\(')) {
    if ($fn.Value -match $bad) { throw "Bridge may not alter focus/input: $bad" }
}
if ($src -notmatch 'if set_parent_result\.is_err\(\)\s*&& taskbar_bridge_experiment_enabled\(\)') {
    throw 'Bridge must be opt-in and run only after the direct path fails.'
}
if ($src -notmatch 'source_pid != 0\s*&& source_pid == target_pid') {
    throw 'Bridge must only run between windows in the same Explorer process.'
}
if ($src -notmatch 'for _ in 0\.\.2\s*\{\s*let rollback = SetParent\(hwnd, source\)') {
    throw 'Bridge rollback must verify the original parent.'
}
Write-Host 'PASS: opt-in detached bridge, parent verification, rollback, and popup rescue.'
