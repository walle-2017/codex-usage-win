$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
foreach ($term in @(
    'CODEX_TASKBAR_ACTIVE_STATE_VERIFY',
    'TIMER_EXPLORER_STATE_VERIFY',
    'sample_explorer_active_state',
    'EXPLORER_ACTIVE_TRACK',
    'GetGUIThreadInfo',
    'handoff_begin attempt=',
    'handoff_result attempt=',
    'consecutive_samples=',
    'last_sample_gap_ms='
)) {
    if ($source -notmatch [regex]::Escape($term)) {
        throw "Missing read-only taskbar state verification: $term"
    }
}
$sample = [regex]::Match($source, '(?s)fn\s+sample_explorer_active_state\(.*?\n\}')
if (-not $sample.Success) { throw 'Missing Explorer state sampler.' }
foreach ($forbidden in @(
    'SetForegroundWindow\s*\(',
    'SetParent\s*\(',
    'AttachThreadInput\s*\(',
    'SetActiveWindow\s*\(',
    'SetFocus\s*\(',
    'SendInput\s*\('
)) {
    if ($sample.Value -match $forbidden) {
        throw "Explorer state sampling must be read-only: $forbidden"
    }
}
if ($source -notmatch 'explorer_active_state_verify_enabled\(\)\s*\{\s*SetTimer\(') {
    throw 'Explorer state timer must be opt-in.'
}
if ($source -notmatch 'source_taskbar\.is_some\(\)\s*&&\s*source_taskbar\s*!=\s*Some\(taskbar\.hwnd\)') {
    throw 'Handoff attempt IDs must exclude initial taskbar embedding.'
}
Write-Host 'PASS: taskbar active-state correlation verification is read-only and opt-in.'
