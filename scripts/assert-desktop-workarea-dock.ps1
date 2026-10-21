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
if ($window -notmatch 'fn floating_slide_offset\(' -or
    $window -notmatch 'fn floating_slide_layered_bitmap\(' -or
    $window -notmatch 'fn floating_slid_canvas_rect\(' -or
    $window -notmatch 'floating_slide_layered_bitmap\(' -or
    $window -notmatch '(?s)if reserved \{\s*// For Explorer or adjacent-monitor edges, slide bitmap pixels' -or
    $window -notmatch 'floating_reserved_drag_reveal = 100') {
    throw 'Taskbar docking must use physical-edge slide offsets and alpha-clipped canvas, including zero-jump drag rebasing.'
}
if ($blur -notmatch 'floating_slid_canvas_rect\(' -or
    $blur -notmatch 'floating_state_grip_rect\(s\)') {
    throw 'Frosted panel and grip must use the same slide offset and work-area clip.'
}
if ($window -match 'fn floating_reserved_visible_body\(' -or
    $window -match 'fn floating_reserved_grip_rect\(') {
    throw 'Obsolete taskbar-only contraction and grip geometry must not return.'
}
if ($window -notmatch 'fn floating_drop_edge_at_pointer\(' -or
    $window -notmatch 'let edge = floating_drop_edge_at_pointer\(' -or
    $window -notmatch 'desktop_taskbar_drop_with_entire_canvas_beyond_work_area_still_snaps' -or
    $window -notmatch 'desktop_taskbar_drop_overshoot_detects_all_four_reserved_edges' -or
    $window -notmatch 'desktop_taskbar_drop_does_not_snap_when_cursor_not_in_reserved_band') {
    throw 'Taskbar drop must use the cursor reserved band even when the entire dragged HWND is beyond rcWork.'
}
if ($window -notmatch 'fn floating_drag_has_crossed_taskbar_edge\(' -or
    $window -notmatch '(?s)let should_unlock = !unlocked.*?floating_drag_has_cleared_edge\(.*?\|\| \(reserved && floating_drag_has_crossed_taskbar_edge\(' -or
    $window -notmatch 'desktop_taskbar_drag_restores_clipped_glow_in_both_directions' -or
    $window -notmatch 'desktop_taskbar_drag_does_not_unlock_from_small_jitter') {
    throw 'Dragging into Explorer must restore the clipped external glow, without changing physical-edge behavior.'
}
if ($window -notmatch '(?s)fn floating_drop_edge_at_pointer\(.*?pointer\.x\.abs_diff\(bounds\.left\).*?pointer\.y\.abs_diff\(bounds\.bottom\).*?\.min_by_key\(\|\(_, _, _, _, distance\)\| \*distance\)' -or
    $window -notmatch 'desktop_corner_snap_picks_nearest_mouse_edge_at_all_four_corners' -or
    $window -notmatch 'desktop_taskbar_corner_snap_uses_actual_inner_taskbar_edge_distances' -or
    $window -notmatch 'desktop_shared_seam_corner_uses_nearest_mouse_edge_including_taskbar') {
    throw 'Corner docking must compare cursor distances even where a display seam meets a taskbar.'
}
if ($window -notmatch 'fn floating_edge_needs_clip\(' -or
    $window -notmatch 'fn floating_edge_requires_clip\(' -or
    $window -notmatch '(?s)fn finish_floating_drag\(.*?floating_monitor_geometry\(pointer\).*?floating_drop_edge_at_pointer\(\s*monitor, bounds, rect, pointer, sc\(FLOAT_EDGE_SNAP\), \[true; 4\]' -or
    $window -match 'let blocked_seam =' -or
    $window -notmatch 'desktop_shared_seam_docks_to_monitor_under_cursor_and_auto_hides' -or
    $window -notmatch 'desktop_shared_seam_dock_clips_sliding_pixels_inside_chosen_monitor') {
    throw 'Each monitor seam must be dockable, use the mouse-selected monitor, and clip autohide instead of crossing screens.'
}
if ($window -notmatch 'desktop_taskbar_rect_corrects_rcwork_gap_on_every_screen_edge' -or
    $window -notmatch 'desktop_taskbar_and_physical_edges_use_identical_slide_offsets' -or
    $window -notmatch 'desktop_taskbar_bitmap_slide_is_exact_translation_clipped_on_all_four_sides' -or
    $window -notmatch 'desktop_taskbar_tab_grab_anchor_stays_fixed_when_virtual_slide_becomes_real') {
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
Write-Host 'PASS: cursor-selected monitor seam docking, nearest corner, cross-screen clipping, unified slide and glass synchronization.'
