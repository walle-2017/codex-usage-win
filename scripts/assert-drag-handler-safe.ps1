# Regression contract for the v1.0.1 live cross-taskbar mouse-capture fix.
$ErrorActionPreference = 'Stop'

$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')

$moveMatch = [regex]::Match(
    $source,
    '(?s)WM_MOUSEMOVE\s*=>\s*\{(?<body>.*?)\n\s*WM_CANCELMODE\s*=>'
)
if (-not $moveMatch.Success) {
    throw 'Unable to locate WM_MOUSEMOVE handler.'
}

$moveBody = $moveMatch.Groups['body'].Value
if ($moveBody -match 'current_appearance_preset\s*\(') {
    throw 'WM_MOUSEMOVE must not re-lock STATE through current_appearance_preset() while dragging.'
}

if ($moveBody -notmatch 'taskbar_at_point\s*\(') {
    throw 'WM_MOUSEMOVE must detect the taskbar under the cursor while dragging.'
}
if ($moveBody -notmatch 'attach_to_taskbar_window\s*\(' -or
    $moveBody -notmatch 'hovered_taskbar\.hwnd') {
    throw 'WM_MOUSEMOVE must reattach directly to the exact taskbar HWND under the cursor.'
}
if ($moveBody -notmatch 'drag_left_from_cursor\s*\(') {
    throw 'WM_MOUSEMOVE must preserve the cursor grab point with cursor-anchored geometry when switching taskbars.'
}
if ($moveBody -notmatch 'SetCapture\s*\(\s*hwnd\s*\)') {
    throw 'WM_MOUSEMOVE must restore mouse capture after a live taskbar switch.'
}

# A captured child window must not be reparented directly between Explorer taskbars.
# Release capture before SetParent/attach, mark the expected capture transition, then
# restore capture only after the widget is attached to the new taskbar.
$releaseBeforeAttach = [regex]::Match(
    $moveBody,
    '(?s)drag_reparenting\s*=\s*true.*?ReleaseCapture\s*\(\s*\).*?attach_to_taskbar_window\s*\('
)
if (-not $releaseBeforeAttach.Success) {
    throw 'Live taskbar switching must mark internal reparenting and release mouse capture before exact taskbar attachment.'
}
if ($moveBody -notmatch '(?s)attach_to_taskbar_window\s*\(.*?drag_reparenting\s*=\s*false.*?SetCapture\s*\(\s*hwnd\s*\)') {
    throw 'Live taskbar switching must clear the reparent marker and restore capture only after exact taskbar attachment.'
}

$hitTestMatch = [regex]::Match(
    $source,
    '(?s)fn\s+is_drag_handle_point\s*\([^)]*\)\s*->\s*bool\s*\{(?<body>.*?)\n\}'
)
if (-not $hitTestMatch.Success) {
    throw 'Unable to locate is_drag_handle_point().'
}
if ($hitTestMatch.Groups['body'].Value -match 'current_appearance_preset\s*\(') {
    throw 'is_drag_handle_point() must not re-lock STATE through current_appearance_preset().'
}

if ($source -match 'map\(widget_height_for_state\)\s*\.unwrap_or\(sc\(current_appearance_preset\(\)\.metrics\(\)\.widget_height\)\)') {
    throw 'Widget-height fallback must not eagerly re-lock STATE through current_appearance_preset().'
}

$setCursorMatch = [regex]::Match(
    $source,
    '(?s)WM_SETCURSOR\s*=>\s*\{(?<body>.*?)\n\s*WM_LBUTTONDOWN\s*=>'
)
if (-not $setCursorMatch.Success) {
    throw 'Unable to locate WM_SETCURSOR handler.'
}
$setCursorBody = $setCursorMatch.Groups['body'].Value
if ($setCursorBody -notmatch 'IDC_SIZEALL') {
    throw 'Drag handle hover/drag cursor must use the four-way move cursor IDC_SIZEALL.'
}
if ($setCursorBody -match 'IDC_SIZEWE') {
    throw 'Drag handle must not use the horizontal resize cursor IDC_SIZEWE.'
}

$captureChangedMatch = [regex]::Match(
    $source,
    '(?s)WM_CAPTURECHANGED\s*=>\s*\{(?<body>.*?)\n\s*WM_LBUTTONUP\s*=>'
)
if (-not $captureChangedMatch.Success) {
    throw 'Drag handling must process WM_CAPTURECHANGED.'
}
$captureChangedBody = $captureChangedMatch.Groups['body'].Value
if ($captureChangedBody -notmatch 'drag_reparenting') {
    throw 'WM_CAPTURECHANGED must distinguish an intentional live-reparent capture transition from a real capture loss.'
}
if ($captureChangedBody -notmatch '!s\.drag_reparenting') {
    throw 'WM_CAPTURECHANGED must keep the drag session alive during an intentional live reparent.'
}

$buttonUpMatch = [regex]::Match(
    $source,
    '(?s)WM_LBUTTONUP\s*=>\s*\{(?<body>.*?)\n\s*WM_RBUTTONUP\s*=>'
)
if (-not $buttonUpMatch.Success) {
    throw 'Unable to locate WM_LBUTTONUP handler.'
}
$buttonUpBody = $buttonUpMatch.Groups['body'].Value
$releasePos = $buttonUpBody.IndexOf('ReleaseCapture')
$dragResultBranchPos = $buttonUpBody.IndexOf('if let Some((current_taskbar_hwnd')
if ($releasePos -lt 0 -or ($dragResultBranchPos -ge 0 -and $releasePos -gt $dragResultBranchPos)) {
    throw 'WM_LBUTTONUP must release mouse capture unconditionally before branching on dragging state.'
}

if ($source -notmatch 'WM_CANCELMODE') {
    throw 'Drag handling must clear dragging state when Windows cancels the interaction (WM_CANCELMODE).'
}

Write-Host 'PASS: taskbar dragging releases capture before live reparent, preserves intentional capture transitions, restores capture safely, and always releases on button-up.'
