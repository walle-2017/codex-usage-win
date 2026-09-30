$ErrorActionPreference = 'Stop'

$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$windowProduction = ($window -split '#\[cfg\(test\)\]', 2)[0]

if ($windowProduction -notmatch 'hit_center_x\s*=\s*sc\(DRAG_HANDLE_HIT_W\)\s*/\s*2' -or
    $windowProduction -notmatch 'origin_x\s*=\s*hit_center_x\s*\+\s*sc\(2\)\s*-\s*matrix_w\s*/\s*2') {
    throw 'Dotted drag handle must keep its hit strip while using the small inward visual offset.'
}
if ($windowProduction -notmatch 'const\s+SMALL_TASKBAR_THRESHOLD:\s*i32\s*=\s*34') {
    throw 'Small-taskbar mode must use the defined 34px logical threshold.'
}
if ($windowProduction -notmatch 'const\s+SMALL_WIDGET_HEIGHT:\s*i32\s*=\s*28') {
    throw 'Small-taskbar one-row widget height must be 28 logical pixels.'
}
if ($windowProduction -notmatch 'small_taskbar_mode:\s*bool' -or $windowProduction -notmatch 'small_show_weekly:\s*bool') {
    throw 'App state must track small-taskbar mode and the selected 5H/7D row.'
}
$settingsBlock = [regex]::Match($windowProduction, '(?s)struct SettingsFile \{.*?\n\}').Value
if ($settingsBlock -match 'small_taskbar_mode|small_show_weekly') {
    throw 'Small-taskbar mode/selection are transient and must not be persisted in SettingsFile.'
}
if ($windowProduction -notmatch 'fn\s+is_small_taskbar_height_at_dpi\s*\(' -or
    $windowProduction -notmatch 'CURRENT_DPI\.load\(Ordering::Relaxed\)') {
    throw 'Small-taskbar detection must use a pure DPI-aware helper and the runtime DPI.'
}
if ($windowProduction -notmatch 'map\(widget_height_for_state\)') {
    throw 'Drag-handle hit testing must use the actual runtime widget height in small mode.'
}
if ($windowProduction -notmatch 'anchor_top\s*\+\s*\(anchor_height\s*-\s*widget_height\)\.max\(0\)\s*/\s*2') {
    throw 'Widget must be vertically centered within the taskbar.'
}
if ($windowProduction -notmatch 'if\s+small_mode\s*\{\s*s\.small_show_weekly\s*=\s*false') {
    throw 'Entering small-taskbar mode must default to the 5H row.'
}
if ($windowProduction -notmatch 's\.small_show_weekly\s*=\s*!s\.small_show_weekly') {
    throw 'Left-clicking content in small-taskbar mode must toggle 5H/7D.'
}
if ($windowProduction -notmatch 'if\s*!is_drag_handle_point\(client_x,\s*client_y\)') {
    throw 'Small-mode click toggle must stay outside the drag-handle path.'
}
if ($windowProduction -notmatch 'effective_show_session\s*=\s*if\s+small_taskbar_mode' -or
    $windowProduction -notmatch 'effective_show_weekly\s*=\s*if\s+small_taskbar_mode') {
    throw 'Paint path must render only the selected row in small-taskbar mode.'
}

Write-Host 'PASS: small-taskbar UI and vertical-centering contract is satisfied.'
