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
    'TIMER_TASKBAR_LAYOUT',
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
$defaultPos = $placement.IndexOf('Some(AppearancePreset::Default)')
$minimalPos = $placement.IndexOf('Some(AppearancePreset::Minimal)')
if ($defaultPos -lt 0 -or $minimalPos -lt 0 -or $defaultPos -ge $minimalPos) {
    throw 'Adaptive layout must prefer Default and fall back to Minimal.'
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

if ($window -notmatch 'TIMER_TASKBAR_LAYOUT\s*=>\s*\{[\s\S]*?position_at_taskbar\(\)[\s\S]*?render_layered\(\)') {
    throw 'Taskbar layout must be polled so hidden widgets restore automatically.'
}

Write-Host 'PASS: adaptive taskbar free-space placement contract.'
