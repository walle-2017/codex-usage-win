$ErrorActionPreference = 'Stop'

$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$appearance = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\appearance.rs')
$styleWindow = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window.rs')
$settings = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\settings_model.rs')

foreach ($required in @(
    'free_spans_from_occupied',
    'taskbar_free_spans',
    'nearest_free_left_offset',
    'placement_for_free_spans',
    'hidden_for_taskbar_space',
    'spawn_taskbar_layout_worker',
    'WM_APP_TASKBAR_LAYOUT_UPDATED',
    'TASKBAR_CONTROL_MARGIN_LOGICAL',
    'show_taskbar_space_warning',
    'MessageBoxW'
)) {
    if ($window -notmatch [regex]::Escape($required)) {
        throw "Missing adaptive taskbar placement contract: $required"
    }
}

if ($native -notmatch 'codex_taskbar_control_rects' -or
    $native -notmatch 'taskbar_control_rects\(') {
    throw 'Taskbar occupied controls must be queried through the native UI Automation bridge.'
}

if ($appearance -notmatch '(?m)^\s*Adaptive,\s*$' -or
    $settings -notmatch '"adaptive"\s*=>\s*AppearancePreset::Adaptive' -or
    $styleWindow -notmatch 'AppearancePreset::Adaptive' -or
    $styleWindow -notmatch '"自适应"') {
    throw 'Adaptive must be a persisted and selectable layout option.'
}

$placement = [regex]::Match(
    $window,
    '(?s)fn\s+placement_for_free_spans\s*\(.*?\n\}'
).Value
$defaultWidthPos = $placement.IndexOf('span.width() >= default_width')
$minimalWidthPos = $placement.IndexOf('span.width() >= minimal_width')
if ($defaultWidthPos -lt 0 -or
    $minimalWidthPos -lt 0 -or
    $defaultWidthPos -ge $minimalWidthPos -or
    $placement -notmatch 'AppearancePreset::Default,\s*default_width' -or
    $placement -notmatch 'AppearancePreset::Minimal,\s*minimal_width') {
    throw 'Adaptive layout must use Default for a sufficiently wide target gap and fall back to Minimal for a narrower target gap.'
}

$position = [regex]::Match(
    $window,
    '(?s)fn\s+position_at_taskbar\s*\(\)\s*\{.*?\n\}'
).Value
if ($position -notmatch 'hide_widget_for_taskbar_space' -or
    $position -notmatch 'placement_for_free_spans' -or
    $position -notmatch 'effective_appearance_preset') {
    throw 'Positioning must hide on insufficient space and apply the resolved adaptive layout.'
}

$worker = [regex]::Match(
    $window,
    '(?s)fn\s+spawn_taskbar_layout_worker\s*\(.*?\n\}'
).Value
if ($worker -notmatch 'native_interop::taskbar_control_rects' -or
    $worker -notmatch 'PostMessageW' -or
    $window -notmatch 'WM_APP_TASKBAR_LAYOUT_UPDATED\s*=>|msg\s*==\s*WM_APP_TASKBAR_LAYOUT_UPDATED') {
    throw 'Taskbar UI Automation must run on a background worker and notify the UI thread asynchronously.'
}
if ($position -match 'native_interop::taskbar_control_rects') {
    throw 'The UI thread must never synchronously query taskbar UI Automation while embedded in Explorer.'
}
if ($worker -notmatch 'merge_taskbar_layout_cache' -or
    $worker -notmatch 'preserving last-known-good layout' -or
    $worker -match '\*cache\s*=\s*next_cache') {
    throw 'Transient UI Automation failures or empty results must preserve the last-known-good taskbar layout cache.'
}
if ((Get-Content -Raw (Join-Path $PSScriptRoot '..\native\taskbar_layout.cpp')) -notmatch 'COINIT_MULTITHREADED') {
    throw 'The taskbar UI Automation worker must initialize COM as MTA.'
}

Write-Host 'PASS: adaptive taskbar free-space placement contract.'
