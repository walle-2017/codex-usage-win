$ErrorActionPreference = 'Stop'

$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$style = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$composition = Get-Content -Raw (Join-Path $PSScriptRoot '..\native\composition_blur.cpp')
$styleWindow = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window.rs')
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
if ($styleWindow -notmatch '"排版"' -or
    $styleWindow -notmatch '"主题"' -or
    $windowProduction -notmatch '样式设置\\t⚙' -or
    $windowProduction -notmatch 'Style settings\\t⚙') {
    throw 'Unified panel Theme/Layout labels and the right-aligned gear Style Settings menu entry must be present.'
}
if ($windowProduction -notmatch 'IDM_STYLE_SETTINGS' -or
    $styleWindow -notmatch 'StyleWindowSnapshot' -or
    $styleWindow -notmatch 'WM_STYLE_COLOR_PREVIEW' -or
    $styleWindow -notmatch 'WM_STYLE_BLUR_PREVIEW' -or
    $styleWindow -notmatch 'Section::Panel' -or
    $styleWindow -notmatch 'Section::Text' -or
    $styleWindow -notmatch 'Section::Progress' -or
    $styleWindow -notmatch 'Section::Interaction') {
    throw 'Unified style settings panel contract is missing.'
}
if ($windowProduction -match 'AppendMenuW\([\s\S]{0,180}theme_menu' -or
    $windowProduction -match 'AppendMenuW\([\s\S]{0,180}layout_menu') {
    throw 'Theme and Layout must not be duplicated in the taskbar context menu.'
}
if ($styleWindow -match 'paint_preview\(') {
    throw 'The settings-panel preview card must be removed; the taskbar widget is the live preview.'
}
if ($styleWindow -match 'msctls_trackbar32' -or $styleWindow -match 'WM_HSCROLL') {
    throw 'Unified style panel must use self-drawn sliders instead of white native trackbars.'
}
if ($styleWindow -match '"RGBA"' -or $styleWindow -match '"强度调节"' -or $styleWindow -match '"Intensity"') {
    throw 'Redundant RGBA/intensity editor headings must stay removed.'
}
if ($styleWindow -match 'R/G/B 调整颜色' -or
    $styleWindow -match 'R/G/B adjust color' -or
    $styleWindow -match '0% 关闭磨砂' -or
    $styleWindow -match '0% turns blur off') {
    throw 'Adjustment help text must not be shown in the style panel.'
}
if ($styleWindow -notmatch 'HitTarget' -or
    $styleWindow -notmatch 'hovered:\s*Option<HitTarget>' -or
    $styleWindow -notmatch 'pressed:\s*Option<HitTarget>' -or
    $styleWindow -notmatch 'button_background\(' -or
    $styleWindow -notmatch 'TrackMouseEvent') {
    throw 'All style-panel buttons must expose hover and pressed feedback.'
}
if ($styleWindow -match 'WS_SYSMENU' -or
    $styleWindow -notmatch 'WM_CLOSE\s*=>\s*LRESULT\(0\)') {
    throw 'Style panel must not expose a title-bar close button; only the lower Close button may close it.'
}
if ($styleWindow -notmatch 'ID_EDIT_R' -or
    $styleWindow -notmatch 'ES_NUMBER' -or
    $styleWindow -notmatch 'EN_CHANGE_CODE' -or
    $styleWindow -notmatch 'sync_numeric_edits\(' -or
    $styleWindow -notmatch 'update_color_from_numeric_edit\(') {
    throw 'RGBA values must use linked numeric input controls.'
}
if ($styleWindow -notmatch 'ID_EDIT_BLUR' -or
    $styleWindow -notmatch 'sync_blur_edit\(' -or
    $styleWindow -notmatch 'update_blur_from_numeric_edit\(' -or
    $styleWindow -notmatch 'focused_blur_edit' -or
    $styleWindow -notmatch 'blur_edit_frame_rect\(') {
    throw 'Blur intensity must use a linked 0-100 numeric input beside the slider.'
}
if ($styleWindow -match 'WS_BORDER' -or
    $styleWindow -notmatch 'numeric_edit_frame_rect\(' -or
    $styleWindow -notmatch 'focused_numeric_edit' -or
    $styleWindow -notmatch 'paint_numeric_edit_frames\(') {
    throw 'RGBA numeric inputs must use the custom focus-aware borderless style.'
}
if ($styleWindow -match 'WINDOW_HEIGHT_BLUR' -or
    $styleWindow -match 'resize_for_editor\(' -or
    $styleWindow -notmatch '"当前强度"' -or
    $styleWindow -match '"关闭 0%"' -or
    $styleWindow -match '"最强 100%"' -or
    $styleWindow -notmatch 'EditorSelection::Blur => rect\(hwnd, 174, 326, 786, 382\)') {
    throw 'Style window height must stay fixed while only the blur editor block shrinks to one row.'
}
if ($styleWindow -notmatch 'DwmSetWindowAttribute' -or
    $styleWindow -notmatch 'DWMWA_CAPTION_COLOR' -or
    $styleWindow -notmatch 'DWMWA_TEXT_COLOR' -or
    $styleWindow -notmatch 'FIXED_CAPTION_COLORREF:\s*u32\s*=\s*0x00524843' -or
    $styleWindow -notmatch 'FIXED_CAPTION_TEXT_COLORREF:\s*u32\s*=\s*0x00FFFFFF' -or
    $styleWindow -notmatch 'apply_fixed_titlebar\(' -or
    $styleWindow -match 'DWMWA_USE_IMMERSIVE_DARK_MODE' -or
    $styleWindow -match 'DWMWA_TRANSITIONS_FORCEDISABLED') {
    throw 'Style window title bar must use one fixed neutral caption color in every theme.'
}
if ($styleWindow -notmatch 'WM_SETCURSOR' -or
    $styleWindow -notmatch 'IDC_HAND' -or
    $styleWindow -notmatch 'IDC_SIZEWE' -or
    $styleWindow -notmatch 'IDC_IBEAM' -or
    $styleWindow -notmatch 'cursor_hwnd' -or
    $styleWindow -notmatch 'slider_kind_at\(' -or
    $styleWindow -notmatch 'hit_target_at\(') {
    throw 'Style panel buttons, sliders, and numeric inputs must expose semantic mouse cursors.'
}
if ($windowProduction -notmatch 'WM_SETCURSOR' -or
    $windowProduction -notmatch 'small_taskbar_mode' -or
    $windowProduction -notmatch 'IDC_HAND') {
    throw 'Small-taskbar click-to-toggle mode must expose a hand cursor outside the drag handle.'
}
foreach ($hex in @('#E9EEF4FF', '#DCE5EFFF', '#CBD7E4FF', '#C1CCD8FF', '#EEF3F8FF')) {
    if ($styleWindow -notmatch [regex]::Escape($hex)) {
        throw "Expected higher-contrast light style-panel color is missing: $hex"
    }
}
if ($styleWindow -notmatch 'WS_CLIPCHILDREN' -or
    $styleWindow -notmatch 'CreateCompatibleDC' -or
    $styleWindow -notmatch 'CreateCompatibleBitmap' -or
    $styleWindow -notmatch 'BitBlt') {
    throw 'Style panel must use clipped child controls and double-buffered painting to reduce flicker.'
}
if ($styleWindow -notmatch 'WM_APP \+ 120' -or
    $styleWindow -notmatch 'WM_APP \+ 123' -or
    $styleWindow -notmatch 'draw_slider\(') {
    throw 'Style panel messages must use a collision-free WM_APP range and self-drawn sliders.'
}
if ($styleProduction -notmatch 'pub\s+dark:\s+ThemeStyle' -or $styleProduction -notmatch 'pub\s+light:\s+ThemeStyle') {
    throw 'Dark and light theme styles must be stored separately.'
}
if ($styleProduction -match '(?i)rounded') {
    throw 'v1.0.5 style settings must not expose rounded panel/progress options.'
}
if ($styleProduction -notmatch 'panel_frosted_strength:\s*u8' -or
    $styleProduction -notmatch 'FROSTED_STRENGTH_MAX:\s*u8\s*=\s*100') {
    throw 'Per-theme 0-100 frosted intensity setting is missing.'
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
if ($windowProduction -notmatch 'apply_style_color\(' -or
    $windowProduction -notmatch 'apply_frosted_strength\(') {
    throw 'Color and frosted-intensity editors must apply live previews.'
}
if ($windowProduction -notmatch 'render_layered\(\);[\s\S]{0,200}TB_ENDTRACK_CODE') {
    Write-Warning 'Live-preview implementation shape changed; inspect manually if this warning appears.'
}
if ($native -notmatch 'create_composition_blur' -or
    $native -notmatch 'set_composition_blur_amount' -or
    $native -notmatch 'destroy_composition_blur') {
    throw 'Rust must use the native Windows Composition Gaussian-blur helper.'
}
if ($composition -notmatch 'CreateDesktopWindowTarget' -or
    $composition -notmatch 'CreateBackdropBrush' -or
    $composition -notmatch 'CLSID_D2D1GaussianBlur' -or
    $composition -notmatch 'Blur\.BlurAmount' -or
    $composition -notmatch 'IGraphicsEffectD2D1Interop') {
    throw 'Native helper must use DesktopWindowTarget + BackdropBrush + real GaussianBlurEffect.'
}
if ($composition -notmatch 'codex_composition_blur_set_bounds' -or
    $composition -notmatch 'CreateInsetClip' -or
    $composition -notmatch 'root\.Size\(size\)' -or
    $composition -notmatch 'blur_visual\.Size\(size\)' -or
    $composition -notmatch 'tint_visual\.Size\(size\)') {
    throw 'Composition backdrop must use explicit size and hard clipping to prevent stale-DPI blur tails.'
}
if ($native -notmatch 'set_composition_blur_bounds' -or
    $windowProduction -notmatch 'sync_composition_blur_bounds') {
    throw 'Rust must synchronize Composition visual bounds with the backdrop HWND.'
}
if ($windowProduction -notmatch 's\.taskbar_hwnd\.unwrap_or_else\(\|\| s\.hwnd\.to_hwnd\(\)\)') {
    throw 'DPI refresh must prefer the selected taskbar during cross-monitor popup moves.'
}
if ($windowProduction -notmatch 'FROSTED_MAX_BLUR_PX:\s*f32\s*=\s*20\.0' -or
    $windowProduction -notmatch 'blur_amount_for_strength' -or
    $windowProduction -notmatch 'FROSTED_MAX_BLUR_PX\s*\*\s*f32::from\(strength' -or
    $windowProduction -notmatch 'frosted_strength\s*>\s*0') {
    throw 'Frosted intensity must map 0-100 linearly to a real Gaussian blur amount.'
}
if ($native -match 'SetWindowCompositionAttribute' -or
    $native -match 'ACCENT_ENABLE_ACRYLICBLURBEHIND' -or
    $native -match 'set_native_acrylic') {
    throw 'Legacy WCA Acrylic backend must not remain in the production blur path.'
}
if ($styleProduction -notmatch 'skip_serializing' -or
    $styleProduction -notmatch 'LEGACY_FROSTED_STRENGTH_SENTINEL' -or
    $styleProduction -notmatch 'panel_blur_radius') {
    throw 'Legacy boolean frosted settings must migrate without being written back.'
}
if ($windowProduction -notmatch 'BLUR_BACKDROP_HWND' -or
    $windowProduction -notmatch 'BLUR_BACKDROP_CONTEXT' -or
    $windowProduction -notmatch 'ensure_blur_backdrop' -or
    $windowProduction -notmatch 'sync_blur_backdrop_zorder' -or
    $windowProduction -notmatch 'WS_EX_NOREDIRECTIONBITMAP') {
    throw 'Frosted glass must use a separate no-redirection Composition backdrop window.'
}
if ($windowProduction -match 'set_layered_style\(hwnd, false\)') {
    throw 'The foreground widget must remain layered while frosted glass is active.'
}
if ($native -notmatch 'detach_from_taskbar_as_popup' -or
    $windowProduction -notmatch 'activate_blur_popup' -or
    $windowProduction -notmatch 'restore_layered_taskbar_mode') {
    throw 'Frosted foreground must detach as a layered popup and safely restore taskbar embedding.'
}
if ($windowProduction -notmatch 'MIN_INTERACTIVE_ALPHA:\s*u8\s*=\s*1' -or
    $windowProduction -notmatch 'else if background\.a == 0' -or
    $windowProduction -notmatch 'surface_style\.panel_background\s*=\s*Color::rgba' -or
    $windowProduction -notmatch 'UpdateLayeredWindow') {
    throw 'Transparent layered panels must keep a minimally nonzero alpha so blank areas remain interactive with or without Composition blur.'
}
if ($windowProduction -notmatch 'select_taskbar_for_popup' -or
    $windowProduction -notmatch 'taskbar_rect\.left \+ drag_left' -or
    $windowProduction -notmatch 'sync_blur_backdrop_zorder\(hwnd\)') {
    throw 'Frosted popup dragging must keep the Composition backdrop aligned and preserve taskbar selection.'
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
$bindOwnerBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn\s+bind_popup_windows_to_taskbar_owner\s*\(.*?\n\}'
).Value
$backdropOwnerIndex = $bindOwnerBlock.IndexOf('set_popup_owner(backdrop_hwnd')
$foregroundOwnerIndex = $bindOwnerBlock.IndexOf('set_popup_owner(foreground_hwnd')
if ($backdropOwnerIndex -lt 0 -or
    $foregroundOwnerIndex -lt 0 -or
    $backdropOwnerIndex -ge $foregroundOwnerIndex) {
    throw 'Taskbar owner rebinding must promote the blur backdrop before the foreground.'
}
$popupTaskbarSwitchBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn\s+select_taskbar_for_popup\s*\(.*?\n\}'
).Value
if ($popupTaskbarSwitchBlock -notmatch 'composition_blur_active' -or
    $popupTaskbarSwitchBlock -notmatch 'if\s+blur_active\s*\{[\s\S]*?sync_blur_backdrop_zorder\(foreground_hwnd\)') {
    throw 'Cross-taskbar popup switches must reassert blur/foreground z-order once after owner rebinding.'
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
if ($windowProduction -notmatch 'move_window_without_repaint' -or
    $windowProduction -notmatch 'SWP_NOZORDER\s*\|\s*SWP_NOACTIVATE') {
    throw 'Drag motion must preserve existing layered pixels instead of requesting redundant repaint work.'
}
if ($windowProduction -notmatch 'BLUR_BACKDROP_PARAMS' -or
    $windowProduction -notmatch '\*cached == Some\(params\)') {
    throw 'Composition blur/tint parameters must be cached to avoid redundant effect updates.'
}
$mouseMoveBlock = [regex]::Match(
    $windowProduction,
    '(?s)WM_MOUSEMOVE\s*=>\s*\{.*?WM_CANCELMODE\s*=>'
).Value
if ($mouseMoveBlock -notmatch 'let\s+Some\(\(hovered_taskbar_index, hovered_taskbar\)\)\s*=\s*taskbar_at_point\(pt\)\s+else\s*\{\s*return\s+LRESULT\(0\);') {
    throw 'Cross-monitor dragging must freeze while the pointer is outside every taskbar.'
}
$mouseUpBlock = [regex]::Match(
    $windowProduction,
    '(?s)WM_LBUTTONUP\s*=>\s*\{.*?updater::WM_APP_STARTUP_UPDATE_RESULT'
).Value
if ($mouseUpBlock -notmatch 'drag released outside taskbars; restored last valid taskbar position') {
    throw 'Releasing a drag over desktop must restore a valid taskbar position.'
}
if ($mouseMoveBlock -match 'sync_blur_backdrop_zorder') {
    throw 'Drag frames must not reorder Composition backdrop/foreground windows.'
}
if ($windowProduction -match 'capture_taskbar_background' -or
    $windowProduction -match 'box_blur_bitmap' -or
    $windowProduction -match 'tint_frosted_panel_bitmap') {
    throw 'Taskbar screenshot/software blur must not be used for frosted glass.'
}
if ($styleWindow -notmatch '"磨砂强度"' -or
    $styleWindow -notmatch '"Frosted intensity"') {
    throw 'Unified frosted-intensity UI labels are missing.'
}
if ($windowProduction -notmatch 'reset_active\(s\.is_dark\)') {
    throw 'Reset Style must only reset the active theme.'
}
if ($windowProduction -notmatch 's\.styles\.active\(s\.is_dark\)') {
    throw 'Style menu/rendering must resolve the currently active theme.'
}

Write-Host 'PASS: v1.0.5 theme/style customization contract is satisfied.'


# Fixed-caption and layered-text regression contract
if ($styleWindow -match 'DWMWA_TRANSITIONS_FORCEDISABLED' -or
    $styleWindow -match 'DWMWA_USE_IMMERSIVE_DARK_MODE') {
    throw 'The fixed neutral style-window caption must not switch or animate with the active theme.'
}
if ($windowProduction -notmatch 'text_quality_for_layered_surface' -or
    $windowProduction -notmatch 'NONANTIALIASED_QUALITY' -or
    $windowProduction -notmatch 'widget_text_quality\(\)') {
    throw 'Transparent/frosted taskbar text must avoid ClearType background-fringe artifacts.'
}
