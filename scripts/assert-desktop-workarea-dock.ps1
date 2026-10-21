# The desktop widget must not overlap an Explorer taskbar on a reserved edge.
$ErrorActionPreference = 'Stop'
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$blur = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\blur_backdrop.rs')

if ($window -notmatch '(?s)fn floating_usable_bounds\(monitor: RECT, work: RECT\).*?\n\s*work\s*\n') {
    throw 'Desktop geometry must honor the complete Windows rcWork, including its bottom edge.'
}
if ($window -match 'RECT \{ bottom: monitor\.bottom, \.\.work \}') {
    throw 'Physical bottom-edge override reintroduces taskbar overlap.'
}
if ($window -notmatch 'fn floating_edge_is_reserved' -or
    $window -notmatch 'floating_reserved_dock: bool' -or
    $window -notmatch 's\.floating_reserved_dock = reserved_dock' -or
    $window -notmatch '\(collapsed && s\.floating_reserved_dock\)') {
    throw 'Reserved taskbar-side docks must remain fully expanded.'
}
if ($window -notmatch '(?s)let exposed = \[.*?floating_edge_is_reserved\(monitor, bounds, FloatingEdge::Bottom\).*?\];') {
    throw 'Taskbar work-area edges must qualify for snapping even at shared monitor seams.'
}
if ($window -notmatch 'geometry_changed = geometry != last_taskbar_geometry' -or
    $window -notmatch 'if changed \|\| geometry_changed') {
    throw 'Taskbar relocation must invalidate desktop geometry when HWND rectangles change.'
}
if ($window -notmatch 'refresh_floating_reserved_dock\(hwnd\)' -or
    $window -notmatch 'desktop_taskbar_dock_respects_top_right_and_left_work_boundaries' -or
    $window -notmatch 'desktop_bottom_taskbar_docks_above_taskbar_without_sliding_into_it') {
    throw 'Missing desktop refresh or four-edge regression test coverage.'
}
if ($window -match 'sync_desktop_taskbar_owner|desktop_taskbar_overlap_area|desktop_taskbar_owner_for_rect' -or
    $blur -match 'desktop_owner|set_popup_owner\(backdrop_hwnd|set_popup_owner\(foreground_hwnd') {
    throw 'Obsolete V24.8/V24.9 taskbar-owner workaround must be removed.'
}
if ($blur -notmatch '(?s)fn move_frosted_pair\(.*?SWP_NOZORDER \| SWP_NOACTIVATE') {
    throw 'Drag-time frosted pair motion must preserve its Z order.'
}
Write-Host 'PASS: desktop edge docking avoids reserved taskbar space, tracks taskbar relocation, and removes owner hacks.'
