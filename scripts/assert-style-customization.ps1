$ErrorActionPreference = 'Stop'

$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$style = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$windowProduction = ($window -split '#\[cfg\(test\)\]', 2)[0]
$styleProduction = ($style -split '#\[cfg\(test\)\]', 2)[0]

foreach ($mode in @('System', 'Dark', 'Light')) {
    if ($styleProduction -notmatch ("\b" + $mode + "\b")) {
        throw "Theme mode $mode is missing."
    }
}
if ($windowProduction -notmatch 'IDM_THEME_SYSTEM' -or
    $windowProduction -notmatch 'IDM_THEME_DARK' -or
    $windowProduction -notmatch 'IDM_THEME_LIGHT') {
    throw 'Top-level theme menu commands are missing.'
}
if ($windowProduction -notmatch '"排版"' -or $windowProduction -notmatch '"样式"' -or $windowProduction -notmatch '"主题"') {
    throw 'Chinese top-level Theme/Layout/Style labels must be present.'
}
if ($styleProduction -notmatch 'pub\s+dark:\s+ThemeStyle' -or $styleProduction -notmatch 'pub\s+light:\s+ThemeStyle') {
    throw 'Dark and light theme styles must be stored separately.'
}
if ($styleProduction -match '(?i)rounded') {
    throw 'v1.0.5 style settings must not expose rounded panel/progress options.'
}
if ($styleProduction -notmatch 'panel_blur_radius:\s*u8') {
    throw 'Per-theme panel blur setting is missing.'
}
if ($native -notmatch 'pub\s+a:\s+u8' -or $native -notmatch 'to_hex_rgba') {
    throw 'Native Color must support an alpha channel and RGBA serialization.'
}
foreach ($hex in @(
    '#242A31FF', '#343B43FF', '#A0A0A0FF', '#FFFFFFFF', '#92979DFF',
    '#EEF1F4FF', '#D4D9DFFF', '#404040FF', '#202020FF', '#666666FF',
    '#55A8F2FF', '#E6B84AFF', '#D95C5CFF', '#363A3FFF', '#AAAAAAFF'
)) {
    if ($styleProduction -notmatch [regex]::Escape($hex)) {
        throw "Expected RGBA default is missing: $hex"
    }
}
if ($windowProduction -notmatch 'TB_ENDTRACK_CODE') {
    throw 'Continuous editors must save when trackbar interaction ends.'
}
if ($windowProduction -notmatch 'apply_style_color\(' -or $windowProduction -notmatch 'apply_style_blur\(') {
    throw 'Color and blur editors must apply live previews.'
}
if ($windowProduction -notmatch 'render_layered\(\);[\s\S]{0,200}TB_ENDTRACK_CODE') {
    Write-Warning 'Live-preview implementation shape changed; inspect manually if this warning appears.'
}
if ($native -notmatch 'set_native_acrylic' -or
    $native -notmatch 'ACCENT_ENABLE_ACRYLICBLURBEHIND' -or
    $native -notmatch 'SetWindowCompositionAttribute') {
    throw 'Frosted glass must use native DWM acrylic composition.'
}
if ($windowProduction -notmatch 'ACRYLIC_BACKDROP_HWND' -or
    $windowProduction -notmatch 'ensure_acrylic_backdrop' -or
    $windowProduction -notmatch 'sync_acrylic_backdrop_zorder') {
    throw 'Frosted glass must use a separate Acrylic backdrop window.'
}
if ($windowProduction -match 'set_layered_style\(hwnd, false\)') {
    throw 'The foreground widget must remain layered while frosted glass is active.'
}
if ($native -notmatch 'detach_from_taskbar_as_popup' -or
    $windowProduction -notmatch 'activate_acrylic_popup' -or
    $windowProduction -notmatch 'restore_layered_taskbar_mode') {
    throw 'Frosted foreground must detach as a layered popup and safely restore taskbar embedding.'
}
if ($windowProduction -notmatch 'FROSTED_HIT_TEST_ALPHA:\s*u8\s*=\s*1' -or
    $windowProduction -notmatch 'surface_style\.panel_background\s*=\s*Color::rgba' -or
    $windowProduction -notmatch 'UpdateLayeredWindow') {
    throw 'Frosted foreground must keep a minimally nonzero layered alpha so blank panel areas remain interactive.'
}
if ($windowProduction -notmatch 'select_taskbar_for_popup' -or
    $windowProduction -notmatch 'taskbar_rect\.left \+ drag_left' -or
    $windowProduction -notmatch 'sync_acrylic_backdrop_zorder\(hwnd\)') {
    throw 'Frosted popup dragging must keep the Acrylic backdrop aligned and preserve taskbar selection.'
}
if ($windowProduction -notmatch 'frosted_popup_session' -or
    $windowProduction -notmatch 'keeping foreground in stable layered popup mode') {
    throw 'After frosted mode is entered, foreground must remain a stable layered popup for the process lifetime.'
}
$restoreBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn\s+restore_layered_taskbar_mode\s*\(.*?\n\}'
).Value
if ($restoreBlock -notmatch 'if\s+frosted_popup_session\s*\{[\s\S]*?return;') {
    throw 'Stable frosted-popup shutdown path must return before any Explorer reparent fallback.'
}
if ($windowProduction -notmatch 'refresh_widget_after_style_editor_close') {
    throw 'Closing style editors must re-render and restore foreground z-order.'
}
if ($native -notmatch 'set_popup_owner' -or
    $native -notmatch 'GWLP_HWNDPARENT' -or
    $windowProduction -notmatch 'bind_popup_windows_to_taskbar_owner') {
    throw 'Stable frosted popups must be owned by the selected taskbar so Explorer cannot cover them.'
}
if ($windowProduction -notmatch 'STYLE_PREVIEW_FRAME_MS:\s*u64\s*=\s*16' -or
    $windowProduction -notmatch 'render_style_preview\(' -or
    $windowProduction -notmatch 'LAST_STYLE_PREVIEW_RENDER') {
    throw 'Live style preview must be frame-limited instead of repainting for every raw trackbar event.'
}
if ($windowProduction -notmatch 'DRAG_FRAME_MS:\s*u64\s*=\s*8' -or
    $windowProduction -notmatch 'drag_frame_due\(' -or
    $windowProduction -notmatch 'move_frosted_pair') {
    throw 'High-frequency drag updates must be throttled and move the frosted pair without z-order churn.'
}
if ($windowProduction -notmatch 'ACRYLIC_BACKDROP_COLOR' -or
    $windowProduction -notmatch '\*cached == Some\(color\)') {
    throw 'Acrylic composition color must be cached to avoid redundant DWM reconfiguration.'
}
$mouseMoveBlock = [regex]::Match(
    $windowProduction,
    '(?s)WM_MOUSEMOVE\s*=>\s*\{.*?WM_CANCELMODE\s*=>'
).Value
if ($mouseMoveBlock -match 'sync_acrylic_backdrop_zorder') {
    throw 'Drag frames must not reorder Acrylic/foreground windows.'
}
if ($windowProduction -match 'capture_taskbar_background' -or
    $windowProduction -match 'box_blur_bitmap' -or
    $windowProduction -match 'tint_frosted_panel_bitmap') {
    throw 'Taskbar screenshot/software blur must not be used for frosted glass.'
}
if ($windowProduction -notmatch '"磨砂玻璃"' -or $windowProduction -notmatch '"Frosted glass"') {
    throw 'Frosted-glass UI labels are missing.'
}
if ($windowProduction -notmatch 'reset_active\(s\.is_dark\)') {
    throw 'Reset Style must only reset the active theme.'
}
if ($windowProduction -notmatch 's\.styles\.active\(s\.is_dark\)') {
    throw 'Style menu/rendering must resolve the currently active theme.'
}

Write-Host 'PASS: v1.0.5 theme/style customization contract is satisfied.'
