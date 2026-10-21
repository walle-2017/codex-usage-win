# Desktop docking follows Explorer's true edge and retracts without covering it.
$ErrorActionPreference = 'Stop'
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$blur = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\blur_backdrop.rs')

if ($window -notmatch 'fn floating_bounds_at_taskbar\(monitor: RECT, work: RECT, taskbars: &\[RECT\]\)' -or
    $window -notmatch 'TASKBAR_EDGE_RECTS' -or
    $window -notmatch 'taskbars\.iter\(\)\.map\(\|bar\| bar\.rect\)\.collect\(\)') {
    throw 'Snap bounds must use the cached actual Explorer taskbar rectangle, not just rcWork.'
}
if ($window -match 'RECT \{ bottom: monitor\.bottom, \.\.work \}') {
    throw 'Physical bottom-edge override would reintroduce taskbar overlap.'
}
if ($window -notmatch 'floating_reserved_dock: bool' -or
    $window -notmatch 's\.floating_reserved_dock = reserved_dock' -or
    $window -match '\(collapsed && s\.floating_reserved_dock\)') {
    throw 'Reserved taskbar-side docks must be allowed to retract normally.'
}
if ($window -notmatch 'fn floating_reserved_visible_body\(' -or
    $window -notmatch 'fn floating_reserved_grip_rect\(' -or
    $window -notmatch 'render_floating_handle_bitmap_at\(' -or
    $window -notmatch '(?s)if reserved \{\s*// Keep HWND fixed within the taskbar-free work area\.') {
    throw 'Reserved retract must clip content inside a stationary window and move its grip.'
}
if ($blur -notmatch 'floating_reserved_visible_body\(' -or
    $blur -notmatch 'floating_state_grip_rect\(s\)') {
    throw 'Frosted panel and tab must be clipped and moved with the same geometry.'
}
if ($window -notmatch 'fn floating_drop_edge_at_pointer\(' -or
    $window -notmatch 'let edge = floating_drop_edge_at_pointer\(' -or
    $window -notmatch 'desktop_taskbar_drop_with_entire_canvas_beyond_work_area_still_snaps' -or
    $window -notmatch 'desktop_taskbar_drop_overshoot_detects_all_four_reserved_edges' -or
    $window -notmatch 'desktop_taskbar_drop_does_not_snap_when_cursor_not_in_reserved_band') {
    throw 'Taskbar drop must use the cursor reserved band even when the entire dragged HWND is beyond rcWork.'
}
if ($window -notmatch 'desktop_taskbar_rect_corrects_rcwork_gap_on_every_screen_edge' -or
    $window -notmatch 'desktop_reserved_tabs_retract_flush_to_taskbar_all_four_edges') {
    throw 'Missing exact-gap and four-edge animation regressions.'
}
if ($window -notmatch 'geometry_changed = geometry != last_taskbar_geometry' -or
    $window -notmatch 'if changed \|\| geometry_changed') {
    throw 'Taskbar relocation must invalidate desktop docking geometry.'
}
if ($window -match 'sync_desktop_taskbar_owner|desktop_taskbar_overlap_area|desktop_taskbar_owner_for_rect' -or
    $blur -match 'desktop_owner|set_popup_owner\(backdrop_hwnd|set_popup_owner\(foreground_hwnd') {
    throw 'Obsolete taskbar Z-order workaround must remain removed.'
}
if ($blur -notmatch '(?s)fn move_frosted_pair\(.*?SWP_NOZORDER \| SWP_NOACTIVATE') {
    throw 'Drag-time frosted positioning must retain its no-flicker Z-order.'
}
Write-Host 'PASS: exact Explorer-edge snap, in-place retract, four-edge grip and blur synchronization.'
