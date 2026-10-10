Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$src = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$win = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($needle in @(
    'CODEX_TASKBAR_HANDOFF_DEEP_VERIFY',
    'fn handoff_deep_verify_enabled()',
    'pub fn log_handoff_deep_snapshot(',
    'taskbar handoff-deep stage=',
    'taskbar handoff-deep window stage=',
    'taskbar handoff-deep gui stage=',
    'source_target_same_thread=',
    'GetGUIThreadInfo(tid, &mut gui)',
    'log_handoff_deep_snapshot("native-before-setparent", hwnd, taskbar_hwnd)',
    'log_handoff_deep_snapshot("native-after-setparent", hwnd, taskbar_hwnd)'
)) { if (-not $src.Contains($needle)) { throw "Missing handoff deep diagnostic: $needle" } }
foreach ($needle in @(
    '"drag-before-release"',
    '"drag-after-release"',
    '"drag-after-recapture-success"',
    '"drag-after-recapture-failure"'
)) { if (-not $win.Contains($needle)) { throw "Missing handoff stage: $needle" } }
$fn = [regex]::Match($src, '(?s)pub fn log_handoff_deep_snapshot\(.*?\n\}')
if (-not $fn.Success -or $fn.Value -notmatch 'if !handoff_deep_verify_enabled\(\) \{ return; \}') {
    throw 'Snapshot must be explicitly opt-in.'
}
foreach ($unsafeWrite in @(
    'SetParent\s*\(', 'SetCapture\s*\(', 'ReleaseCapture\s*\(',
    'SetForegroundWindow\s*\(', 'AttachThreadInput\s*\(',
    'SetWindowLongW\s*\(', 'SetWindowPos\s*\(', 'SendInput\s*\(',
    'PostMessageW\s*\(', 'TrackMouseEvent\s*\('
)) {
    if ($fn.Value -match $unsafeWrite) {
        throw "Read-only diagnostic contains window/input mutation: $unsafeWrite"
    }
}
if ($src.Contains('CODEX_TASKBAR_DETACHED_BRIDGE_EXPERIMENT')) {
    throw 'Failed detached-bridge code must remain removed.'
}
Write-Host 'PASS: opt-in read-only deep handoff stages and GUI thread snapshots.'
