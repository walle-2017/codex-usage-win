Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
foreach ($check in @(
    'fn refreshed_drag_taskbar_targets()',
    'fn point_near_other_taskbar_top(',
    'refreshed_drag_taskbar_targets();',
    'taskbar drag targets cached count=',
    'taskbar drag near-edge target=',
    'taskbar drag drop-miss cursor=',
    'target.rect.top',
    'native_interop::set_popup_owner_regular_band(backdrop_hwnd',
    'native_interop::set_popup_owner_regular_band(foreground_hwnd',
    'fn sync_blur_backdrop_zorder('
)) {
    if (-not $window.Contains($check)) { throw "Missing Start-menu safeguard: $check" }
}
foreach ($check in @(
    'pub fn set_popup_owner_regular_band(',
    'GWLP_HWNDPARENT',
    'HWND_NOTOPMOST'
)) {
    if (-not $native.Contains($check)) { throw "Missing native owner safeguard: $check" }
}
$zorder = [regex]::Match($window, '(?s)fn sync_blur_backdrop_zorder\(.*?\n\}')
if (-not $zorder.Success -or $zorder.Value.Contains('HWND_TOPMOST')) {
    throw 'Frosted popup must not force HWND_TOPMOST over Start.'
}
$bounds = [regex]::Match($window, '(?s)fn point_near_other_taskbar_top\(.*?\n\}')
if (-not $bounds.Success -or -not $bounds.Value.Contains('is_other_taskbar')
    -or -not $bounds.Value.Contains('pt.y < rect.top')) {
    throw 'Drop tolerance must be restricted to the other taskbar upper edge.'
}
$hit = [regex]::Match($window, '(?s)fn taskbar_at_point\(.*?\n\}')
if (-not $hit.Success -or $hit.Value.IndexOf('point_is_in_taskbar_rect') -gt $hit.Value.IndexOf('point_near_other_taskbar_top')) {
    throw 'Exact taskbar hit testing must take precedence over edge tolerance.'
}
Write-Host 'PASS: Start-menu popup normal band, validated multi-taskbar cache, bounded drag edge.'
