# Regression contract for V24.8: desktop popups remain above the taskbar.
$ErrorActionPreference = 'Stop'

$widget = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$glass = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\blur_backdrop.rs')

if ($widget -notmatch '(?s)fn sync_desktop_taskbar_owner\(hwnd: HWND\).*?is_taskbar_window\(\*owner\).*?GetWindow\(hwnd, GW_OWNER\).*?set_popup_owner\(hwnd, Some\(owner\)\)') {
    throw 'Desktop foreground must follow the Explorer taskbar owner without changing non-taskbar windows.'
}
if (([regex]::Matches($widget, 'sync_desktop_taskbar_owner\(hwnd\);')).Count -lt 2) {
    throw 'Desktop popup ownership must be synchronized both on initial creation and when reusing it.'
}
if ($glass -notmatch 'taskbar_parent\.or\(desktop_owner\)\.unwrap_or_default\(\)') {
    throw 'The desktop DirectComposition backdrop must be taskbar-owned at creation.'
}
if ($glass -notmatch '(?s)GetWindow\(backdrop_hwnd, GW_OWNER\).*?set_popup_owner\(backdrop_hwnd, Some\(owner\)\).*?GetWindow\(foreground_hwnd, GW_OWNER\).*?set_popup_owner\(foreground_hwnd, Some\(owner\)\)') {
    throw 'The desktop glass and its foreground must recover the same Explorer owner.'
}
if ($glass -notmatch '(?s)fn move_frosted_pair\(.*?SWP_NOZORDER \| SWP_NOACTIVATE') {
    throw 'Dragging a frosted desktop pair must preserve z-order to avoid flicker.'
}

Write-Host 'PASS: desktop widget and frosted backdrop are taskbar-owned, with drag-time z-order preserved.'
