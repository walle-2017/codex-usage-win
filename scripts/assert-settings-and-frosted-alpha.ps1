# Focused regression guard: settings reuse + Alpha-gated visible blur with prewarmed backdrop.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$style = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style.rs')
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$replica = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\taskbar_replica.rs')
$settings = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window.rs')

foreach ($token in @(
    'pub fn panel_frosted_active(&self) -> bool',
    'pub fn tooltip_frosted_active(&self) -> bool',
    'StyleColorTarget::PanelBackground).a < u8::MAX',
    'StyleColorTarget::TooltipBackground).a < u8::MAX'
)) {
    if (-not $style.Contains($token)) {
        throw "Alpha-gated style predicate missing: $token"
    }
}

foreach ($token in @(
    'let blur_requested = panel_blur_backdrop_needed(&style);',
    'if composition_blur_active && background.a < u8::MAX',
    'style.tooltip_frosted_active()',
    'WM_APP_FOCUS_EXISTING_SETTINGS',
    'style_window::open_or_focus('
)) {
    if (-not $window.Contains($token)) {
        throw "Panel/tooltip or deferred settings path missing: $token"
    }
}
# At alpha=255 the Composition backdrop remains allocated but the layered
# foreground stays opaque. Crossing to 254 reuses the prepared background.
# Do not regress to first creating the asynchronous blur visual at that edge.
if ($window -notmatch '(?s)fn panel_blur_backdrop_needed\(style: &ThemeStyle\) -> bool\s*\{\s*style\.panel_frosted_strength > 0\s*\}') {
    throw 'Frosted Composition visual must be prepared whenever strength > 0.'
}
if ($window -notmatch 'fn frosted_backdrop_is_prepared_before_panel_alpha_becomes_translucent\(') {
    throw 'Alpha 255 to 254 transition must retain its regression test.'
}
if ($replica -notmatch 's\.styles\.active\(s\.is_dark\)\.panel_frosted_active\(\)') {
    throw 'Replica handoff must use the same Alpha-gated frosted predicate.'
}

foreach ($token in @(
    'paint_frosted_alpha_hint(hdc, hwnd, snapshot.language, secondary)',
    'Alpha < 255',
    'SW_RESTORE',
    'HWND_TOP,',
    'SetForegroundWindow(hwnd)'
)) {
    if (-not $settings.Contains($token)) {
        throw "Settings reactivation or frosted hint missing: $token"
    }
}

$open = [regex]::Match($settings, '(?s)pub fn open_or_focus\(.*?\n\}\s*\n\s*pub fn sync\(')
if (-not $open.Success -or $open.Value -match 'HWND_TOPMOST' -or
    $open.Value -notmatch 'IsWindow\(hwnd\)' -or
    $open.Value -notmatch 'IsIconic\(hwnd\)') {
    throw 'Existing settings HWND must be restored and raised normally, never permanently topmost.'
}
if ($window -notmatch '(?s)IDM_STYLE_SETTINGS\s*=>.*?PostMessageW\(.*?WM_APP_FOCUS_EXISTING_SETTINGS') {
    throw 'Menu commands must defer settings focus until popup dismissal.'
}

Write-Host 'PASS: settings singleton/foreground, Alpha-gated visible blur and prewarmed panel backdrop.'
