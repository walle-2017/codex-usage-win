# V24.9 regression: desktop popup follows the taskbar actually on its monitor.
$ErrorActionPreference = 'Stop'
$widget = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$glass = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window\blur_backdrop.rs')

if ($widget -notmatch '(?s)fn desktop_taskbar_owner_for_rect\(window: RECT\).*?find_taskbars\(\).*?desktop_taskbar_overlap_index\(window, &bounds\).*?MonitorFromWindow\(bar\.hwnd, MONITOR_DEFAULTTONEAREST\)') {
    throw 'Desktop owner must come from actual rectangle overlap or current monitor, never the embedded taskbar setting.'
}
$ownerMatch = [regex]::Match($widget, '(?s)fn sync_desktop_taskbar_owner\(hwnd: HWND\)\s*\{(?<body>.*?)\n\}')
if (-not $ownerMatch.Success -or $ownerMatch.Groups['body'].Value -notmatch 'desktop_taskbar_owner_for_rect\(window_rect\)' -or
    $ownerMatch.Groups['body'].Value -notmatch 'GetWindow\(hwnd, GW_OWNER\)' -or
    $ownerMatch.Groups['body'].Value -notmatch 'set_popup_owner\(hwnd, Some\(owner\)\)') {
    throw 'Desktop foreground must verify and synchronize the owner of its current monitor.'
}
if ($ownerMatch.Groups['body'].Value -match 's\.taskbar_hwnd') {
    throw 'Desktop foreground must not use the configured embedded taskbar as owner.'
}
if (([regex]::Matches($widget, 'sync_desktop_taskbar_owner\(hwnd\);')).Count -lt 3) {
    throw 'Desktop ownership must sync on initial show, reuse and drag release.'
}
if ($glass -notmatch '(?s)let desktop_owner = if desktop \{.*?GetWindow\(hwnd, GW_OWNER\).*?taskbar_parent\.or\(desktop_owner\)\.unwrap_or_default\(\)') {
    throw 'New desktop glass must inherit the foreground popup actual owner.'
}
if ($glass -notmatch '(?s)let desktop_owner = unsafe \{ GetWindow\(foreground_hwnd, GW_OWNER\)\.ok\(\) \}.*?GetWindow\(backdrop_hwnd, GW_OWNER\).*?set_popup_owner\(backdrop_hwnd, Some\(owner\)\)') {
    throw 'Desktop glass must follow the actual owner when crossing monitors.'
}
if ($glass -notmatch '(?s)fn move_frosted_pair\(.*?SWP_NOZORDER \| SWP_NOACTIVATE') {
    throw 'Drag movement must preserve the composition pair Z-order to prevent flicker.'
}
if ($widget -notmatch 'desktop_popup_uses_overlapping_secondary_taskbar_not_configured_primary') {
    throw 'Missing field-geometry regression for overlapping secondary taskbar.'
}
Write-Host 'PASS: desktop Z-order owner uses local taskbar, cross-monitor release and frosted pairs.'
