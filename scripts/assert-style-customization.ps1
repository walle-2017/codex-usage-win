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
    $windowProduction -notmatch '"样式\.\.\."' -or
    $windowProduction -notmatch '"Style\.\.\."') {
    throw 'Unified panel Theme/Layout labels and the native Style submenu entry must be present.'
}
if ($styleProduction -notmatch 'enum\s+ThemePreset' -or
    $styleProduction -notmatch 'Classic' -or
    $styleProduction -notmatch 'Ocean' -or
    $styleProduction -notmatch 'Forest' -or
    $styleProduction -notmatch 'apply_preset\(' -or
    $styleProduction -notmatch 'matches_preset\(' -or
    $styleWindow -notmatch '"石墨"' -or
    $styleWindow -notmatch '"深海"' -or
    $styleWindow -notmatch '"松影"' -or
    $styleWindow -notmatch '"晨霜"' -or
    $styleWindow -notmatch '"雾蓝"' -or
    $styleWindow -notmatch '"暖砂"' -or
    $styleWindow -notmatch '"Graphite"' -or
    $styleWindow -notmatch '"Deep Sea"' -or
    $styleWindow -notmatch '"Pine Shade"' -or
    $styleWindow -notmatch '"Morning Frost"' -or
    $styleWindow -notmatch '"Mist Blue"' -or
    $styleWindow -notmatch '"Warm Sand"' -or
    $styleWindow -notmatch 'Section::Preset' -or
    $styleWindow -notmatch 'paint_preset_gallery\(' -or
    $styleWindow -notmatch 'preset_card_rect\(' -or
    $styleWindow -notmatch 'WM_STYLE_PRESET_CHANGE' -or
    $windowProduction -notmatch 'WM_STYLE_PRESET_CHANGE' -or
    $windowProduction -notmatch 'ThemePreset::from_index') {
    throw 'Dark and light presets must have distinct user-facing names and live in the dedicated Presets section.'
}
if ($styleWindow -match 'fn\s+preset_rect\(') {
    throw 'The old top-row preset strip must stay removed; presets belong in sidebar preview cards.'
}
foreach ($hex in @(
    '#0F1B24FF', '#294252FF', '#3FB7E9FF',
    '#14211DFF', '#2B4038FF', '#56C596FF',
    '#F3F7FAFF', '#CFDCE4FF', '#49616FFF', '#1F3440FF', '#5E7480FF', '#B53F3FFF', '#2F8FB8FF',
    '#F8F4ECFF', '#DED4C4FF', '#665A48FF', '#2E2922FF', '#756A59FF', '#B4463EFF', '#3C8F8AFF'
)) {
    if ($styleProduction -notmatch [regex]::Escape($hex)) {
        throw "Expected coordinated theme-preset color is missing: $hex"
    }
}
if ($style -notmatch 'preset_text_colors_keep_readable_contrast' -or
    $style -notmatch 'redesigned_light_presets_keep_error_text_readable' -or
    $style -notmatch 'ratio\s*>=\s*4\.5') {
    throw 'Theme presets must keep an automated readable text-contrast quality gate.'
}
if ($windowProduction -notmatch 'IDM_STYLE_SETTINGS' -or
    $styleWindow -notmatch 'StyleWindowSnapshot' -or
    $styleWindow -notmatch 'WM_STYLE_COLOR_PREVIEW' -or
    $styleWindow -notmatch 'WM_STYLE_BLUR_PREVIEW' -or
    $styleWindow -notmatch 'Section::Preset' -or
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
    $styleWindow -match 'WS_CHILD\.0\s*\|\s*WS_VISIBLE\.0\s*\|\s*ES_NUMBER' -or
    $styleWindow -notmatch 'EN_CHANGE_CODE' -or
    $styleWindow -notmatch 'sync_numeric_edits\(' -or
    $styleWindow -notmatch 'update_color_from_numeric_edit\(') {
    throw 'RGBA values must use linked numeric inputs that are created hidden and shown only on editor pages.'
}
if ($styleWindow -notmatch 'if\s+s\.section\s*==\s*Section::Preset\s*\{\s*return;\s*\}' -or
    $styleWindow -notmatch 'preset_page_can_open_paint_and_destroy_without_editor_reentry') {
    throw 'Preset-page startup must avoid hidden EDIT synchronization and keep a real Win32 open/paint/destroy smoke test.'
}
if ($styleWindow -notmatch 'ID_EDIT_HEX_BASE' -or
    $styleWindow -notmatch 'HEX_EDIT_COUNT:\s*usize\s*=\s*11' -or
    $styleWindow -notmatch 'sync_hex_edits\(' -or
    $styleWindow -notmatch 'update_color_from_hex_edit\(' -or
    $styleWindow -notmatch 'parse_hex_input\(' -or
    $styleWindow -notmatch 'focused_hex_edit' -or
    $styleWindow -notmatch 'invalid_hex_edits' -or
    $styleWindow -notmatch 'select_editor\(EditorSelection::Color\(target\)\)') {
    throw 'Every color row must expose an editable Hex field that selects and synchronizes the matching RGBA editor.'
}
if ($styleWindow -notmatch 'sync_numeric_edits\(\);\s*sync_hex_edits\(\);' -or
    $styleWindow -notmatch 'set_edit_text_string\(' -or
    $styleWindow -notmatch 'Color::try_from_hex') {
    throw 'Hex and RGBA color editors must synchronize in both directions.'
}
if ($styleWindow -notmatch 'ID_EDIT_BLUR' -or
    $styleWindow -notmatch 'sync_blur_edit\(' -or
    $styleWindow -notmatch 'update_blur_from_numeric_edit\(' -or
    $styleWindow -notmatch 'focused_blur_edit' -or
    $styleWindow -notmatch 'blur_edit_frame_rect\(' -or
    $styleWindow -notmatch 's\.section\s*==\s*Section::Panel') {
    throw 'Blur intensity must use one inline 0-100 slider and numeric input in the Panel row.'
}
if ($styleWindow -notmatch 'editor_layout_snapshot\(' -or
    $styleWindow -notmatch 'release_editor_focus_before_layout\(' -or
    $styleWindow -notmatch 'hiding_focused_color' -or
    $styleWindow -notmatch 'hiding_focused_blur' -or
    $styleWindow -notmatch 'SetFocus\(hwnd\)' -or
    $styleWindow -notmatch 'ShowWindow/SetFocus can synchronously send') {
    throw 'Style editor switching must release STATE before focus/visibility Win32 calls to avoid EDIT focus deadlocks.'
}
if ($styleWindow -match 'WS_BORDER' -or
    $styleWindow -notmatch 'numeric_edit_frame_rect\(' -or
    $styleWindow -notmatch 'focused_numeric_edit' -or
    $styleWindow -notmatch 'paint_numeric_edit_frames\(') {
    throw 'RGBA numeric inputs must use the custom focus-aware borderless style.'
}
if ($styleWindow -match 'WINDOW_HEIGHT_BLUR' -or
    $styleWindow -match 'resize_for_editor\(' -or
    $styleWindow -match '"当前强度"' -or
    $styleWindow -match '"Current"' -or
    $styleWindow -notmatch 'blur_slider_track_rect\(hwnd\)' -or
    $styleWindow -notmatch 'rect\(hwnd, 390, 224, 650, 228\)' -or
    $styleWindow -notmatch 'matches!\(editor, EditorSelection::Color\(_\)\)\s*&&\s*section\s*!=\s*Section::Preset') {
    throw 'Blur must stay inline in its row and must never render a lower secondary editor.'
}
if ($styleWindow -notmatch 'select_editor\(EditorSelection::Blur\)' -or
    $styleWindow -notmatch 'blur selection must clear the lower RGBA editor' -or
    $styleWindow -notmatch 'hex_input_accepts_rgb_rgba_and_transient_partial_values') {
    throw 'Clicking blur must clear lower RGBA, while Hex focus must restore the matching RGBA editor in the Win32 smoke test.'
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
if ($windowProduction -match 'MF_OWNERDRAW' -or
    $windowProduction -match 'WM_MEASUREITEM' -or
    $windowProduction -match 'draw_style_settings_menu_item\(') {
    throw 'Style must remain a plain native menu item.'
}
if ($windowProduction -notmatch 'AppendMenuW\(\s*settings_menu,[\s\S]{0,180}IDM_STYLE_SETTINGS' -or
    $windowProduction -match 'AppendMenuW\(\s*menu,[\s\S]{0,180}IDM_STYLE_SETTINGS') {
    throw 'Style must live only inside the Settings submenu.'
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
    $styleWindow -notmatch 'WM_APP \+ 126' -or
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
