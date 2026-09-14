$ErrorActionPreference = 'Stop'

$appearance = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\appearance.rs')
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$style = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style.rs')
$simplifiedChinese = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\localization\simplified_chinese.rs')
$appearanceProduction = ($appearance -split '#\[cfg\(test\)\]', 2)[0]
$windowProduction = ($window -split '#\[cfg\(test\)\]', 2)[0]
$styleProduction = ($style -split '#\[cfg\(test\)\]', 2)[0]

if ($appearanceProduction -match '(?m)^\s*Default,\s*$') {
    throw 'Legacy Default appearance variant must be removed from the active preset enum.'
}
if ($windowProduction -match 'IDM_APPEARANCE_DEFAULT') {
    throw 'Layout menu must only expose Compact and Minimal.'
}
if ($windowProduction -match '(?m)^const SEGMENT_COUNT:') {
    throw 'Obsolete Default-preset SEGMENT_COUNT constant must be removed.'
}
if (($appearanceProduction + $windowProduction) -match '↻') {
    throw 'Reset-time icon must not appear in the taskbar UI.'
}
if ($windowProduction -notmatch 'format!\("5H ') {
    throw 'Tooltip 5-hour labels must use uppercase 5H.'
}
if ($windowProduction -notmatch 'format!\("7D ') {
    throw 'Tooltip 7-day labels must use uppercase 7D.'
}
if ($simplifiedChinese -notmatch 'session_window:\s*"5H"') {
    throw 'Simplified-Chinese taskbar session label must use uppercase 5H.'
}
if ($simplifiedChinese -notmatch 'weekly_window:\s*"7D"') {
    throw 'Simplified-Chinese taskbar weekly label must use uppercase 7D.'
}

foreach ($field in @('outer_padding', 'label_bar_gap', 'bar_percent_gap', 'percent_width', 'percent_reset_gap', 'reset_width')) {
    if ($appearanceProduction -notmatch ("pub\s+" + $field + ":\s*i32")) {
        throw "Appearance metrics must expose $field."
    }
}
if (($appearanceProduction | Select-String -Pattern 'outer_padding:\s*6' -AllMatches).Matches.Count -lt 2) {
    throw 'Compact and Minimal presets must use symmetric 6px horizontal outer padding.'
}
if (($appearanceProduction | Select-String -Pattern 'label_bar_gap:\s*6' -AllMatches).Matches.Count -lt 2) {
    throw 'Q1-to-Q2 gap must remain 6px in both presets.'
}
if (($appearanceProduction | Select-String -Pattern 'percent_reset_gap:\s*3' -AllMatches).Matches.Count -lt 2) {
    throw 'Q3-to-Q4 gap must remain 3px.'
}
if (($appearanceProduction | Select-String -Pattern 'bar_percent_gap:\s*4' -AllMatches).Matches.Count -lt 2) {
    throw 'Q2-to-Q3 gap must remain 4px.'
}
if (($appearanceProduction | Select-String -Pattern 'percent_width:\s*36' -AllMatches).Matches.Count -lt 2) {
    throw 'Q3 percentage slot must remain a fixed 36px wide.'
}
if ($appearanceProduction -notmatch 'reset_width:\s*34') {
    throw 'Compact Q4 reset slot must remain 34px.'
}
if ($appearanceProduction -notmatch 'secondary_font_height:\s*-11') {
    throw 'Compact reset time/date font must remain -11.'
}
if ($windowProduction -notmatch 'DT_LEFT\s*\|\s*DT_VCENTER\s*\|\s*DT_SINGLELINE') {
    throw 'Percentage values must remain left aligned.'
}

# Progress track/fill stays square-cornered.
if ($windowProduction -match 'CreateRoundRectRgn\([\s\S]{0,250}?bar_rect') {
    throw 'Progress bars must not use rounded clipping.'
}
if ($windowProduction -notmatch 'FillRect\(hdc,\s*&bar_rect') {
    throw 'Square progress track must be drawn with FillRect.'
}

# Semantic quota thresholds remain unchanged; palette defaults moved to style.rs.
if ($windowProduction -notmatch 'fn\s+quota_bar_color\s*\(') {
    throw 'Quota bar color must be selected by a dedicated semantic helper.'
}
foreach ($target in @('ProgressHigh', 'ProgressMedium', 'ProgressLow')) {
    if ($windowProduction -notmatch ("StyleColorTarget::" + $target)) {
        throw "Quota bar helper must use StyleColorTarget::$target."
    }
}
foreach ($hex in @('#55A8F2FF', '#E6B84AFF', '#D95C5CFF')) {
    if ($styleProduction -notmatch [regex]::Escape($hex)) {
        throw "Default quota palette must contain $hex."
    }
}
if ($windowProduction -notmatch 'remaining\s*>\s*50\.0' -or $windowProduction -notmatch 'remaining\s*>\s*20\.0') {
    throw 'Quota thresholds must remain >50%, 21-50%, and <=20% remaining.'
}
if ($windowProduction -notmatch 'StyleColorTarget::Remaining' -or $windowProduction -notmatch 'StyleColorTarget::Error') {
    throw 'Primary usage text must use configurable remaining/error colors.'
}

# Panel remains square; only the six-dot drag handle may use rounded dot regions.
if (($appearanceProduction | Select-String -Pattern 'panel_radius:\s*0' -AllMatches).Matches.Count -lt 2) {
    throw 'Compact and Minimal panels must both remain square.'
}
if ($windowProduction -notmatch 'fn\s+draw_panel\s*\(') {
    throw 'Widget must draw a dedicated square panel.'
}
$panelBlock = [regex]::Match($windowProduction, '(?s)fn\s+draw_panel\s*\(.*?\n\}').Value
if ($panelBlock -notmatch 'FillRect\(hdc,\s*&outer' -or $panelBlock -notmatch 'FillRect\(hdc,\s*&inner') {
    throw 'Panel border and fill must use square FillRect drawing.'
}
if ($panelBlock -match 'CreateRoundRectRgn') {
    throw 'Panel must not use rounded clipping.'
}
if ($windowProduction -notmatch 'fn\s+draw_drag_handle\s*\(') {
    throw 'Widget must draw a dedicated dotted drag handle.'
}
if (($windowProduction | Select-String -Pattern 'for\s+row\s+in\s+0\.\.3' -AllMatches).Matches.Count -lt 1 -or
    ($windowProduction | Select-String -Pattern 'for\s+col\s+in\s+0\.\.2' -AllMatches).Matches.Count -lt 1) {
    throw 'Drag handle must use a 2x3 dot matrix.'
}
if ($windowProduction -notmatch 'DRAG_HANDLE_VISUAL_INSET_X:\s*i32\s*=\s*7') {
    throw 'Drag-handle dots must retain their inset.'
}

Write-Host 'PASS: compact layout and square themed taskbar UI contract is satisfied.'
