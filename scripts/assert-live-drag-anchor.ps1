$ErrorActionPreference = 'Stop'
$source = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')

$required = @(
    'drag_anchor_logical_x',
    'drag_anchor_px_for_dpi',
    'drag_left_from_cursor',
    'offset_for_drag_left'
)
foreach ($name in $required) {
    if ($source -notmatch $name) {
        throw "Missing live-drag anchor contract: $name"
    }
}

$move = [regex]::Match($source, '(?s)WM_MOUSEMOVE\s*=>\s*\{(?<body>.*?)\n\s*WM_CANCELMODE\s*=>')
if (-not $move.Success) { throw 'Unable to locate WM_MOUSEMOVE.' }
$body = $move.Groups['body'].Value
if ($body -notmatch 'GetDpiForWindow\s*\(\s*hovered_taskbar\.hwnd\s*\)') { throw 'Target taskbar DPI is not used.' }
if ($body -notmatch 'drag_left_from_cursor\s*\(') { throw 'Cursor-anchored absolute positioning is not used.' }
if ($body -match 'offset_for_drop_point\s*\(') { throw 'Live handoff still uses docked drop-point offset geometry.' }
if ($body -match 'drag_start_mouse_x\s*-\s*pt\.x') { throw 'Live dragging still uses the old delta model.' }

$up = [regex]::Match($source, '(?s)WM_LBUTTONUP\s*=>\s*\{(?<body>.*?)\n\s*WM_RBUTTONUP\s*=>')
if (-not $up.Success) { throw 'Unable to locate WM_LBUTTONUP.' }
if ($up.Groups['body'].Value -notmatch 'offset_for_drag_left\s*\(') { throw 'Docked offset is not finalized from the final drag-left position.' }

Write-Host 'PASS: live drag anchor contract.'
