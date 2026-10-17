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
    'vertical_placement_for_gaps',
    'enable_floating_widget',
    'restore_floating_widget_to_taskbar',
    'floating_nearest_edge',
    'floating_edge_position'
)) {
    if ($window -notmatch [regex]::Escape($required)) {
        throw "Missing adaptive taskbar placement contract: $required"
    }
}

if ($native -notmatch 'codex_taskbar_control_rects' -or
    $native -notmatch 'taskbar_control_rects\(') {
    throw 'Taskbar occupied controls must be queried through the native UI Automation bridge.'
}

if ($appearance -notmatch 'pub enum DisplayPosition' -or
    $appearance -notmatch 'Self::Adaptive\s*=>\s*Self::Default' -or
    $settings -notmatch 'appearance.position: allowed values are taskbar, desktop' -or
    $styleWindow -notmatch 'HitTarget::Position\(position\)' -or
    $styleWindow -notmatch 'DisplayPosition::Taskbar, DisplayPosition::Desktop' -or
    $styleWindow -match 'for preset in \[AppearancePreset::Default, AppearancePreset::Minimal, AppearancePreset::Adaptive\]') {
    throw 'Appearance must provide Default and Minimal layouts with a separate persisted Taskbar/Desktop location selector.'
}

$placement = [regex]::Match(
    $window,
    '(?s)fn\s+placement_for_free_spans\s*\(.*?\n\}'
).Value
if ($placement -notmatch 'AppearancePreset::Default\s*=>\s*place\(AppearancePreset::Default\)' -or
    $placement -notmatch 'or_else\(\|\| place\(AppearancePreset::Minimal\)\)' -or
    $placement -notmatch 'AppearancePreset::Minimal\s*=>\s*place\(AppearancePreset::Minimal\)') {
    throw 'Default layout must try full width before falling back to Minimal; explicit Minimal must remain unchanged.'
}

$position = [regex]::Match(
    $window,
    '(?s)fn\s+position_at_taskbar\s*\(\)\s*\{.*?\n\}'
).Value
if ($position -notmatch 'hide_widget_for_taskbar_space' -or
    $position -notmatch 'placement_for_free_spans' -or
    $position -notmatch 'effective_appearance_preset' -or
    $window -notmatch 'vertical_placement_for_gaps' -or
    $window -notmatch 'AppearancePreset::Default\s*=>\s*\[' -or
    $window -notmatch 'DisplayPosition::Desktop' -or
    $window -notmatch 'restore_floating_widget_to_taskbar\(' -or
    $window -notmatch 'enable_floating_widget\(' -or
    $window -match 'fn\s+show_taskbar_space_warning\(') {
    throw 'Positioning must fall back Default -> Minimal -> desktop popup without modal space warnings and restore taskbar placement when possible.'
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
$watchdog = [regex]::Match(
    $window,
    '(?s)fn\s+spawn_taskbar_watchdog\s*\(\)\s*\{.*?\n\}'
).Value
if ($worker -notmatch 'preserve_valid_cached_taskbars' -or
    $worker -notmatch 'is_taskbar_window' -or
    $watchdog -notmatch 'is_taskbar_window\(old\)') {
    throw 'A transient EnumWindows gap must not be treated as taskbar destruction while the cached taskbar HWND/class is still valid.'
}
if ($native -notmatch 'pub fn is_taskbar_window' -or
    $native -notmatch 'IsWindow\(hwnd\)' -or
    $native -notmatch 'Shell_SecondaryTrayWnd') {
    throw 'Persisted taskbar handles must be validated directly by HWND and taskbar class.'
}
if ((Get-Content -Raw (Join-Path $PSScriptRoot '..\native\taskbar_layout.cpp')) -notmatch 'COINIT_MULTITHREADED') {
    throw 'The taskbar UI Automation worker must initialize COM as MTA.'
}

Write-Host 'PASS: default adaptive taskbar placement and persistent taskbar/desktop selection contract.'
