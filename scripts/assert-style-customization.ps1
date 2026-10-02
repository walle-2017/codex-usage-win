$ErrorActionPreference = 'Stop'

$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$style = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style.rs')
$native = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\native_interop.rs')
$composition = Get-Content -Raw (Join-Path $PSScriptRoot '..\native\composition_blur.cpp')
$controlPrimitives = Get-Content -Raw (Join-Path $PSScriptRoot '..\native\control_primitives.cpp')
$styleWindow = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\style_window.rs')
$settingsModel = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\settings_model.rs')
$popupMenu = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\popup_menu.rs')
$fonts = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\fonts.rs')
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
    $windowProduction -notmatch '"设置\.\.\."' -or
    $windowProduction -notmatch '"Settings\.\.\."') {
    throw 'Unified settings must retain appearance Theme/Layout controls and expose one top-level Settings entry.'
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
    $styleWindow -notmatch '"云瓷"' -or
    $styleWindow -notmatch '"晴湾"' -or
    $styleWindow -notmatch '"麦光"' -or
    $styleWindow -notmatch '"Graphite"' -or
    $styleWindow -notmatch '"Deep Sea"' -or
    $styleWindow -notmatch '"Pine Shade"' -or
    $styleWindow -notmatch '"Cloud Porcelain"' -or
    $styleWindow -notmatch '"Clear Bay"' -or
    $styleWindow -notmatch '"Wheat Glow"' -or
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
    '#F6F8FAFF', '#C7D0D9FF', '#46515DFF', '#17212BFF', '#596777FF', '#B33F49FF', '#4A8FD8FF',
    '#DCEFF5FF', '#AFCFD8FF', '#335965FF', '#12343DFF', '#426976FF', '#AD4146FF', '#1597B7FF',
    '#FFF1DCFF', '#DDBF8FFF', '#685339FF', '#332514FF', '#795F3DFF', '#B24039FF', '#4E8A6EFF'
)) {
    if ($styleProduction -notmatch [regex]::Escape($hex)) {
        throw "Expected coordinated theme-preset color is missing: $hex"
    }
}
if ($style -notmatch 'preset_text_colors_keep_readable_contrast' -or
    $style -notmatch 'redesigned_light_presets_keep_error_text_readable' -or
    $style -notmatch 'redesigned_light_presets_are_visually_distinct' -or
    $style -notmatch 'ratio\s*>=\s*4\.5') {
    throw 'Theme presets must keep an automated readable text-contrast quality gate.'
}
if ($windowProduction -notmatch 'IDM_STYLE_SETTINGS' -or
    $styleWindow -notmatch 'StyleWindowSnapshot' -or
    $styleWindow -notmatch 'WM_STYLE_COLOR_PREVIEW' -or
    $styleWindow -notmatch 'WM_STYLE_BLUR_PREVIEW' -or
    $styleWindow -notmatch 'Section::General' -or
    $styleWindow -notmatch 'Section::Preset' -or
    $styleWindow -notmatch 'Section::Panel' -or
    $styleWindow -notmatch 'Section::Tooltip' -or
    $styleWindow -notmatch 'Section::Text' -or
    $styleWindow -notmatch 'Section::Progress' -or
    $styleWindow -notmatch 'Section::Interaction' -or
    $styleWindow -notmatch 'Section::Json') {
    throw 'Unified General/Appearance/Advanced settings center contract is missing.'
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
if ($styleWindow -notmatch 'CreateWindowExW\(\s*WS_EX_APPWINDOW' -or
    $styleWindow -match 'WS_MAXIMIZEBOX' -or
    $styleWindow -notmatch 'WS_CAPTION\s*\|\s*WS_SYSMENU\s*\|\s*WS_MINIMIZEBOX' -or
    $styleWindow -notmatch 'WINDOW_HEIGHT,\s*HWND::default\(\)' -or
    $styleWindow -notmatch 'WM_CLOSE\s*=>\s*\{' -or
    $styleWindow -notmatch 'send_parent\(WM_STYLE_SAVE, 0, 0\)' -or
    $styleWindow -notmatch 'DestroyWindow\(hwnd\)') {
    throw 'Settings must be an ownerless taskbar window with minimize support and a standard caption close button.'
}
if ($styleWindow -notmatch 'json_save_mask' -or
    $styleWindow -notmatch 'WS_EX_LAYERED\s*\|\s*WS_EX_TOOLWINDOW\s*\|\s*WS_EX_NOACTIVATE' -or
    $styleWindow -notmatch 'WS_POPUP' -or
    $styleWindow -notmatch 'JSON_SAVE_MASK_ALPHA') {
    throw 'JSON save feedback mask must use an owned no-activate layered popup without changing the main settings window style.'
}
if ($styleWindow -notmatch 'ID_EDIT_R' -or
    $styleWindow -notmatch 'ES_NUMBER' -or
    $styleWindow -match 'WS_CHILD\.0\s*\|\s*WS_VISIBLE\.0\s*\|\s*ES_NUMBER' -or
    $styleWindow -notmatch 'EN_CHANGE_CODE' -or
    $styleWindow -notmatch 'sync_numeric_edits\(' -or
    $styleWindow -notmatch 'update_color_from_numeric_edit\(') {
    throw 'RGBA values must use linked numeric inputs that are created hidden and shown only on editor pages.'
}
if ($styleWindow -notmatch 'Section::General\s*\|\s*Section::Preset\s*\|\s*Section::Json' -or
    $styleWindow -notmatch 'preset_page_can_open_paint_and_destroy_without_editor_reentry') {
    throw 'Non-color settings pages must avoid hidden EDIT synchronization and keep the Win32 open/paint/destroy smoke test.'
}
if ($styleWindow -notmatch 'ID_EDIT_HEX_BASE' -or
    $styleWindow -notmatch 'HEX_EDIT_COUNT:\s*usize\s*=\s*13' -or
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
    $styleWindow -notmatch 'Section::Panel\s*\|\s*Section::Tooltip' -or
    $styleWindow -notmatch 'WM_STYLE_TOOLTIP_BLUR_PREVIEW') {
    throw 'Panel and Tooltip frosted intensity must use the shared inline 0-100 slider and numeric input.'
}
if ($styleWindow -notmatch 'editor_layout_snapshot\(' -or
    $styleWindow -notmatch 'release_editor_focus_before_layout\(' -or
    $styleWindow -notmatch 'hiding_focused_color' -or
    $styleWindow -notmatch 'hiding_focused_blur' -or
    $styleWindow -notmatch 'hiding_focused_corner' -or
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
    $styleWindow -notmatch 'blur_slider_track_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'blur_edit_frame_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'inline_numeric_frame_rect\(' -or
    $styleWindow -notmatch 'blur_suffix_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'matches!\(editor, EditorSelection::Color\(_\)\)') {
    throw 'Blur must stay inline in its actual row, share the numeric frame geometry, keep its percent suffix internal, and never render a lower secondary editor.'
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
if ($windowProduction -notmatch 'popup_menu::show' -or
    $windowProduction -notmatch 'PopupItem::submenu' -or
    $windowProduction -match 'TrackPopupMenu' -or
    $windowProduction -match 'CreatePopupMenu') {
    throw 'Tray menu must use the standalone custom popup window rather than native HMENU/TrackPopupMenu.'
}
if ($popupMenu -notmatch 'WS_POPUP' -or
    $popupMenu -notmatch 'CS_DROPSHADOW' -or
    $popupMenu -notmatch 'DWMWA_WINDOW_CORNER_PREFERENCE' -or
    $popupMenu -notmatch 'ITEM_RADIUS' -or
    $popupMenu -notmatch 'PopupAction::Submenu' -or
    $popupMenu -notmatch 'menu_width\(' -or
    $popupMenu -notmatch 'DT_CALCRECT' -or
    $popupMenu -notmatch 'MENU_MIN_WIDTH' -or
    $popupMenu -match 'ROOT_WIDTH' -or
    $popupMenu -match 'SUBMENU_WIDTH') {
    throw 'Custom popup menu must provide native popup styling and content-adaptive root/submenu width.'
}
if ($windowProduction -notmatch 'PopupItem::command\(settings_text, IDM_STYLE_SETTINGS\)' -or
    $windowProduction -match 'settings_menu') {
    throw 'Unified Settings must remain one top-level custom-popup entry.'
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
    $styleWindow -notmatch 'WM_APP \+ 126' -or
    $styleWindow -notmatch 'WM_APP \+ 132' -or
    $styleWindow -notmatch 'draw_slider\(') {
    throw 'Unified settings messages must use a collision-free WM_APP range and appearance must keep self-drawn sliders.'
}
if ($styleProduction -notmatch 'pub\s+dark:\s+ThemeStyle' -or $styleProduction -notmatch 'pub\s+light:\s+ThemeStyle') {
    throw 'Dark and light theme styles must be stored separately.'
}
if ($styleProduction -notmatch 'panel_corner_radius_max\(' -or
    $styleProduction -notmatch 'tooltip_corner_radius_max\(' -or
    $styleProduction -notmatch 'progress_corner_radius_max\(' -or
    $styleProduction -notmatch 'metrics\(\)\.widget_height' -or
    $styleProduction -notmatch 'metrics\(\)\.bar_height' -or
    $styleWindow -notmatch 'ID_EDIT_CORNER' -or
    $styleWindow -notmatch 'EditorSelection::CornerRadius' -or
    $styleWindow -notmatch 'corner_radius_max_for_section\(' -or
    $styleWindow -notmatch 'raw_value\.min\(u16::from\(radius_max\)\)' -or
    $styleWindow -notmatch 'WM_STYLE_CORNER_PREVIEW' -or
    $windowProduction -notmatch 'clamp_corner_radii') {
    throw 'Panel, tooltip, and progress corner-radius limits must adapt to their actual component dimensions.'
}
if ($styleProduction -notmatch 'panel_frosted_strength:\s*u8' -or
    $styleProduction -notmatch 'FROSTED_STRENGTH_MAX:\s*u8\s*=\s*100') {
    throw 'Per-theme 0-100 frosted intensity setting is missing.'
}
if ($styleProduction -notmatch 'tooltip_background:\s*Option<String>' -or
    $styleProduction -notmatch 'tooltip_border:\s*Option<String>' -or
    $styleProduction -notmatch 'tooltip_frosted_strength:\s*Option<u8>' -or
    $styleProduction -notmatch 'StyleColorTarget::TooltipBackground' -or
    $styleProduction -notmatch 'StyleColorTarget::TooltipBorder' -or
    $styleProduction -notmatch 'unwrap_or\(&self\.panel_background\)' -or
    $styleProduction -notmatch 'unwrap_or\(&self\.panel_border\)' -or
    $styleWindow -notmatch '"浮框"' -or
    $styleWindow -notmatch 'Section::Tooltip' -or
    $styleWindow -notmatch 'EditorSelection::TooltipBlur') {
    throw 'Tooltip style must expose independent background, border, and frosted controls.'
}
if ($styleWindow -match '恢复继承面板样式' -or
    $styleWindow -match 'Inherit panel style' -or
    $styleWindow -match '恢复当前主题默认' -or
    $styleWindow -match 'Reset current theme' -or
    $styleWindow -match 'WM_STYLE_TOOLTIP_RESET' -or
    $styleWindow -match 'WM_STYLE_RESET_CURRENT') {
    throw 'Appearance pages must not expose reset buttons; presets are the single restore path.'
}
if ($styleProduction -notmatch 'tooltip_background:\s*Some\(' -or
    $styleProduction -notmatch 'tooltip_border:\s*Some\(' -or
    $styleProduction -notmatch 'tooltip_frosted_strength:\s*Some\(0\)' -or
    $styleProduction -notmatch '\*self\s*=\s*Self::preset\(is_dark, preset\)' -or
    $styleWindow -notmatch 'StyleColorTarget::TooltipBackground' -or
    $styleWindow -notmatch 'StyleColorTarget::TooltipBorder') {
    throw 'Built-in appearance presets must include and preview tooltip styling and restore the complete appearance.'
}
if ($native -notmatch 'pub\s+a:\s+u8' -or $native -notmatch 'to_hex_rgba') {
    throw 'Native Color must support an alpha channel and RGBA serialization.'
}
foreach ($hex in @(
    '#242A31FF', '#343B43FF', '#A0A0A0FF', '#FFFFFFFF', '#92979DFF',
    '#F6F8FAFF', '#C7D0D9FF', '#46515DFF', '#17212BFF', '#596777FF',
    '#55A8F2FF', '#E6B84AFF', '#D95C5CFF', '#363A3FFF', '#D6DDE4FF'
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
    $composition -notmatch 'CreateRoundedRectangleGeometry' -or
    $composition -notmatch 'CreateGeometricClip' -or
    $composition -notmatch 'clip_geometry\.CornerRadius' -or
    $composition -notmatch 'root\.Size\(size\)' -or
    $composition -notmatch 'blur_visual\.Size\(size\)' -or
    $composition -notmatch 'tint_visual\.Size\(size\)') {
    throw 'Composition backdrop must use explicit size and rounded hard clipping to match the panel and prevent stale-DPI blur tails.'
}
if ($native -notmatch 'codex_draw_settings_icon') {
    throw 'Rust native interop must expose the Direct2D settings-icon renderer.'
}
if ($controlPrimitives -notmatch 'codex_draw_settings_icon' -or
    $controlPrimitives -notmatch 'D2D1_CAP_STYLE_ROUND' -or
    $controlPrimitives -notmatch 'D2D1_LINE_JOIN_ROUND' -or
    $controlPrimitives -notmatch 'Painter palette|painter palette' -or
    $controlPrimitives -notmatch 'crescent moon' -or
    $controlPrimitives -notmatch 'Default layout - full two-row usage panel' -or
    $controlPrimitives -notmatch 'Minimal layout - one compact usage row' -or
    $controlPrimitives -notmatch 'graphite cube' -or
    $controlPrimitives -notmatch 'smooth waves' -or
    $controlPrimitives -notmatch 'pine tree' -or
    $controlPrimitives -notmatch 'Cloud Porcelain - cloud' -or
    $controlPrimitives -notmatch 'Clear Bay - sun over calm bay' -or
    $controlPrimitives -notmatch 'Wheat Glow - wheat ear' -or
    $controlPrimitives -notmatch 'JSON Reload - circular arrow' -or
    $controlPrimitives -notmatch 'JSON Format - braces with formatted lines' -or
    $controlPrimitives -notmatch 'JSON Import - arrow into tray' -or
    $controlPrimitives -notmatch 'JSON Export - arrow out of tray' -or
    $controlPrimitives -notmatch 'JSON Apply - check in circle' -or
    $controlPrimitives -notmatch 'paintbrush') {
    throw 'Settings icons must use rounded Direct2D vector paths with distinct layout, light-preset, and JSON-action glyphs.'
}

if ($native -notmatch 'set_composition_blur_bounds' -or
    $native -notmatch 'clip_inset' -or
    $native -notmatch 'clip_radius' -or
    $windowProduction -notmatch 'sync_composition_blur_bounds' -or
    $windowProduction -notmatch 'outer_inset \+ PANEL_BORDER_WIDTH_PX' -or
    $windowProduction -notmatch 'outer_radius - PANEL_BORDER_WIDTH_PX') {
    throw 'Rust must synchronize exact inner-fill geometry with the Composition backdrop.'
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
    '(?s)fn\s+select_taskbar_for_popup_window\s*\(.*?\n\}'
).Value
if ($popupTaskbarSwitchBlock -notmatch 'composition_blur_active' -or
    $popupTaskbarSwitchBlock -notmatch 'if\s+blur_active\s*\{[\s\S]*?sync_blur_backdrop_zorder\(foreground_hwnd\)') {
    throw 'Cross-taskbar popup switches must reassert blur/foreground z-order once after owner rebinding.'
}
if ($native -notmatch 'monitor_device_name\(' -or
    $native -notmatch 'MONITORINFOEXW' -or
    $native -notmatch 'monitor_device:\s*Option<String>' -or
    $windowProduction -notmatch 'taskbar_left_offset:\s*i32' -or
    $windowProduction -notmatch 'taskbar_monitor:\s*Option<String>' -or
    $windowProduction -notmatch 'legacy_tray_offset:\s*Option<i32>') {
    throw 'Taskbar placement must use a stable monitor identity and a left-edge offset, while retaining one-time legacy offset migration.'
}
$positionBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn\s+position_at_taskbar\s*\(\)\s*\{.*?\n\}'
).Value
if ($positionBlock -notmatch 'actual_left_offset\s*=\s*desired_left_offset\.clamp' -or
    $positionBlock -notmatch 'Never write this transient clamp back' -or
    $positionBlock -match 'taskbar_left_offset\s*=\s*actual_left_offset') {
    throw 'Transient TrayNotifyWnd width changes must clamp only the current frame and must never persist positional drift.'
}
if ($windowProduction -notmatch 'current_taskbar_hwnd\s*!=\s*Some\(hovered_taskbar\.hwnd\)' -or
    $windowProduction -notmatch 'attach_to_taskbar_window\(' -or
    $windowProduction -notmatch 'select_taskbar_for_popup_window\(' -or
    $windowProduction -match 'attach_to_taskbar\(hwnd,\s*hovered_taskbar_index\)') {
    throw 'Cross-monitor dragging must bind the exact taskbar HWND already hit-tested instead of re-resolving an array index.'
}
$watchdogBlock = [regex]::Match(
    $windowProduction,
    '(?s)fn\s+spawn_taskbar_watchdog\s*\(\)\s*\{.*?\n\}'
).Value
if ($watchdogBlock -notmatch 'taskbar_monitor' -or
    $watchdogBlock -notmatch 'waiting instead of switching monitors') {
    throw 'Taskbar watchdog recovery must wait for the same monitor instead of silently moving the widget to another taskbar.'
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


$settingsModel = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\settings_model.rs')
if ($styleWindow -notmatch 'WINDOW_WIDTH:\s*i32\s*=\s*980' -or
    $styleWindow -notmatch 'WS_THICKFRAME' -or
    $styleWindow -notmatch '"常规"' -or
    $styleWindow -notmatch '"JSON 配置"' -or
    $styleWindow -notmatch 'language_button_rect' -or
    $styleWindow -notmatch 'rect\(hwnd,\s*760,\s*414,\s*920,\s*450\)' -or
    $styleWindow -notmatch 'language_popup_open' -or
    $styleWindow -notmatch 'LanguageOption' -or
    $styleWindow -notmatch 'ID_EDIT_JSON') {
    throw 'Unified settings window must provide resizable General, Appearance, JSON pages and a self-drawn language selector.'
}
foreach ($message in @(
    'WM_SETTINGS_REFRESH_CHANGE',
    'WM_SETTINGS_USAGE_CHANGE',
    'WM_SETTINGS_ALERT_CHANGE',
    'WM_SETTINGS_STARTUP_CHANGE',
    'WM_SETTINGS_LANGUAGE_CHANGE',
    'WM_SETTINGS_JSON_APPLY'
)) {
    if ($styleWindow -notmatch $message -or $windowProduction -notmatch $message) {
        throw "Unified settings message is not connected end-to-end: $message"
    }
}
if ($styleWindow -match '"当前自定义"' -or
    $styleWindow -match '"Current custom"' -or
    $styleWindow -notmatch 'paint_style_preview_card\(' -or
    $styleWindow -notmatch 'if\s+!matched' -or
    $styleWindow -notmatch 'rect\(hwnd, 200, 420, 424, 604\)') {
    throw 'Preset page must show the custom preview below the preset row without a redundant Current custom heading.'
}
if ($settingsModel -notmatch 'serde\(deny_unknown_fields\)' -or
    $settingsModel -notmatch 'strip_jsonc_comments' -or
    $settingsModel -notmatch 'parse_jsonc' -or
    $settingsModel -notmatch 'show_usage.*cannot both be false' -or
    $settingsModel -notmatch 'expected #RRGGBB or #RRGGBBAA' -or
    $settingsModel -notmatch 'to_jsonc') {
    throw 'Advanced JSONC settings must use a strict public schema, safe comment parsing, and business validation.'
}
if ($styleWindow -match '编辑公开设置；注释仅用于说明' -or
    $styleWindow -match 'comments are documentation only and are omitted when applying or exporting' -or
    $styleWindow -notmatch 'top: scale\(hwnd, 108\)' -or
    $settingsModel -notmatch '注释仅用于说明，不属于配置数据；应用、保存和导出时不会写入配置') {
    throw 'JSON documentation notice must live inside generated JSONC, not as a separate page subtitle.'
}
if ($styleWindow -notmatch 'read_large_edit_text_raw' -or
    $styleWindow -notmatch 'EM_GETSCROLLPOS_MSG' -or
    $styleWindow -notmatch 'EM_SETSCROLLPOS_MSG' -or
    $styleWindow -notmatch 'previous_event_mask' -or
    $styleWindow -notmatch 'syntax_highlight_json_editor\(edit, &normalized, is_dark\)' -or
    $styleWindow -notmatch 'normalize_to_lf\(&read_large_edit_text_raw\(edit\)\)' -or
    $styleWindow -notmatch 'InvalidateRect\(edit, None, true\)' -or
    $styleWindow -notmatch 'UpdateWindow\(edit\)') {
    throw 'JSON RichEdit live highlighting must use RichEdit logical newline offsets, preserve selection/scroll state, suppress formatting notifications, and fully repaint after edits.'
}
if ($styleWindow -notmatch '● 有未保存更改' -or
    $styleWindow -notmatch '● Unsaved changes' -or
    $styleWindow -notmatch 'settings != &applied_settings' -or
    $styleWindow -notmatch 'action == JsonAction::Apply && !json_dirty' -or
    $styleWindow -notmatch 'let apply_background = if dirty') {
    throw 'JSON editor must derive dirty state from parsed settings, show it on a separate line, and disable Apply while clean.'
}
if ($styleWindow -notmatch '✓ 已重新载入' -or
    $styleWindow -notmatch '✓ 已格式化' -or
    $styleWindow -notmatch '✓ 成功导入' -or
    $styleWindow -notmatch '✓ 成功导出' -or
    $styleWindow -notmatch 'JsonSaveFeedback::Saving' -or
    $styleWindow -notmatch 'JsonSaveFeedback::Saved' -or
    $styleWindow -notmatch '◌ 保存中' -or
    $styleWindow -notmatch '✓ 保存成功' -or
    $styleWindow -notmatch 'JsonStatusPath' -or
    $styleWindow -notmatch 'ShellExecuteW' -or
    $styleWindow -notmatch 'Comment\) => Color::from_hex\("#777777FF"\)' -or
    $styleWindow -notmatch 'Comment\) => Color::from_hex\("#999999FF"\)' -or
    $styleWindow -notmatch 'if zh \{ "应用" \} else \{ "Apply" \}') {
    throw 'JSON action feedback, second-line saving/saved state, clickable file paths, Apply label, and subdued comment colors must remain stable.'
}
if ($styleWindow -notmatch 'JSON_ACTION_DELAY_MS: u32 = 200' -or
    $styleWindow -notmatch '◌ 载入中' -or
    $styleWindow -notmatch '◌ 格式化中' -or
    $styleWindow -notmatch '◌ 导入中' -or
    $styleWindow -notmatch '◌ 导出中' -or
    $styleWindow -notmatch '◌ 保存中' -or
    $styleWindow -notmatch 'SetTimer\(hwnd, JSON_ACTION_TIMER_ID, JSON_ACTION_DELAY_MS' -or
    $styleWindow -notmatch 'pending_json_action' -or
    $styleWindow -notmatch 'json_apply_pending' -or
    $styleWindow -notmatch 'json_save_feedback = None' -or
    $styleWindow -notmatch 'GetTextExtentPoint32W' -or
    $styleWindow -notmatch 'path_left \+ path_width') {
    throw 'JSON actions including Apply must expose a 200ms running state; editing after save must restore unsaved state; file-link underline/hit area must match the rendered path width.'
}
if ($styleWindow -notmatch 'codex-usage-win-config-\{:\\04\}' -and
    $styleWindow -notmatch 'codex-usage-win-config-') {
    throw 'JSON export must provide a timestamped default filename.'
}
if ($styleWindow -notmatch 'GetOpenFileNameW' -or
    $styleWindow -notmatch 'GetSaveFileNameW' -or
    $styleWindow -notmatch 'RICHEDIT50W' -or
    $styleWindow -notmatch 'Msftedit\.dll' -or
    $styleWindow -notmatch 'to_windows_newlines' -or
    $styleWindow -notmatch 'syntax_highlight_json_editor' -or
    $styleWindow -notmatch 'friendly_json_error' -or
    $styleWindow -notmatch 'locate_json_error' -or
    $styleWindow -notmatch 'reload_json_editor_from_snapshot' -or
    $styleWindow -notmatch 'format_json_editor' -or
    $styleWindow -notmatch 'apply_json_editor') {
    throw 'JSON settings page must use RichEdit JSONC formatting/highlighting, friendly error navigation, reload, import/export, and validated apply.'
}
if ($windowProduction -notmatch 'theme::is_dark_mode\(\)' -or
    $windowProduction -notmatch 'PopupItem::submenu\(version_label' -or
    $popupMenu -notmatch '#1F1F1FFF' -or
    $popupMenu -notmatch '#FAFAFAFF' -or
    $popupMenu -notmatch 'draw_text' -and $popupMenu -notmatch 'DrawTextW' -or
    $popupMenu -match 'AppendMenuW') {
    throw 'Custom tray popup must follow Windows Dark/Light, draw its own single submenu arrow, and avoid native menu arrows.'
}
if ($fonts -notmatch 'Segoe UI' -or
    $fonts -notmatch 'Microsoft YaHei UI' -or
    $fonts -notmatch 'JetBrains Mono' -or
    $fonts -notmatch 'FontRole::Ui \| FontRole::Taskbar' -or
    $fonts -match 'Inter Variable' -or
    $fonts -match 'Noto Sans SC') {
    throw 'UI and taskbar must share the Windows-native font stack, with only JetBrains Mono embedded for monospaced text.'
}
if ($styleWindow -notmatch 'FontRole::Ui' -or
    $styleWindow -notmatch 'FontRole::Mono' -or
    $fonts -notmatch 'pub enum FontRole') {
    throw 'Settings and JSON editors must use the centralized semantic UI/mono font roles.'
}
if ($styleWindow -notmatch 'SWP_SHOWWINDOW' -or
    $styleWindow -notmatch 'SWP_HIDEWINDOW' -or
    $styleWindow -notmatch 'IsWindowVisible\(json_edit\)') {
    throw 'JSON editor visibility must follow the active settings section atomically and be covered by a UI regression test.'
}
if ($styleWindow -notmatch 'fill_rounded_rect' -or
    $styleWindow -notmatch 'draw_rounded_outline_rect' -or
    $styleWindow -notmatch 'draw_switch' -or
    $native -notmatch 'draw_antialiased_rounded_rect' -or
    $controlPrimitives -notmatch 'D2D1_ANTIALIAS_MODE_PER_PRIMITIVE' -or
    $controlPrimitives -notmatch 'FillRoundedRectangle' -or
    $controlPrimitives -notmatch 'DrawRoundedRectangle') {
    throw 'Settings controls must keep the shared Direct2D-antialiased Fluent-lite control primitives.'
}
$paintNavigationBlock = [regex]::Match(
    $styleWindow,
    '(?s)unsafe\s+fn\s+paint_navigation\s*\(.*?\n\}'
).Value
if ($paintNavigationBlock -notmatch 'Section::Tooltip' -or
    $paintNavigationBlock -notmatch 'navigation_icon\(item\)' -or
    $paintNavigationBlock -notmatch 'draw_settings_icon\(') {
    throw 'Settings sidebar must paint the Tooltip entry and its matching navigation icon.'
}
if ($styleWindow -notmatch 'enum\s+SettingsIcon' -or
    $styleWindow -notmatch '#\[repr\(i32\)\]' -or
    $styleWindow -notmatch 'SettingsIcon::General' -or
    $styleWindow -notmatch 'SettingsIcon::Preset' -or
    $styleWindow -notmatch 'SettingsIcon::Panel' -or
    $styleWindow -notmatch 'SettingsIcon::Tooltip' -or
    $styleWindow -notmatch 'SettingsIcon::Text' -or
    $styleWindow -notmatch 'SettingsIcon::Progress' -or
    $styleWindow -notmatch 'SettingsIcon::Interaction' -or
    $styleWindow -notmatch 'SettingsIcon::Json' -or
    $styleWindow -notmatch 'SettingsIcon::ThemeSystem' -or
    $styleWindow -notmatch 'SettingsIcon::ThemeDark' -or
    $styleWindow -notmatch 'SettingsIcon::ThemeLight' -or
    $styleWindow -notmatch 'SettingsIcon::LayoutDefault' -or
    $styleWindow -notmatch 'SettingsIcon::LayoutMinimal' -or
    $styleWindow -notmatch 'SettingsIcon::PresetClassic' -or
    $styleWindow -notmatch 'SettingsIcon::PresetOcean' -or
    $styleWindow -notmatch 'SettingsIcon::PresetForest' -or
    $styleWindow -notmatch 'SettingsIcon::PresetCustom' -or
    $styleWindow -notmatch 'SettingsIcon::PresetCloud' -or
    $styleWindow -notmatch 'SettingsIcon::PresetBay' -or
    $styleWindow -notmatch 'SettingsIcon::PresetWheat' -or
    $styleWindow -notmatch 'SettingsIcon::JsonReload' -or
    $styleWindow -notmatch 'SettingsIcon::JsonFormat' -or
    $styleWindow -notmatch 'SettingsIcon::JsonImport' -or
    $styleWindow -notmatch 'SettingsIcon::JsonExport' -or
    $styleWindow -notmatch 'SettingsIcon::JsonApply' -or
    $styleWindow -notmatch 'draw_segment_with_icon\(' -or
    $styleWindow -notmatch 'draw_antialiased_settings_icon\(' -or
    $styleWindow -notmatch 'theme_icon\(mode\)' -or
    $styleWindow -notmatch 'layout_icon\(preset\)' -or
    $styleWindow -notmatch 'preset_icon\(preset,\s*snapshot\.is_dark\)' -or
    $styleWindow -notmatch 'json_action_icon\(action\)' -or
    $styleWindow -notmatch 'json_action_icon\(apply\)') {
    throw 'Settings icon vocabulary must use the precise anti-aliased path renderer for navigation, Theme/Layout buttons, preset cards, and Custom.'
}
if ($styleWindow -match 'snapshot\.language\.strings\(\)\.settings' -or
    $styleWindow -notmatch 'navigation_text_inset' -or
    $styleWindow -notmatch 'Section::General => \(28, 68\)' -or
    $styleWindow -notmatch 'Section::Tooltip => \(220, 260\)' -or
    $styleWindow -notmatch 'Section::Json => \(456, 496\)') {
    throw 'Settings sidebar must remain a compact grouped navigation without the redundant Settings heading.'
}
$sliderKindBlock = [regex]::Match(
    $styleWindow,
    '(?s)fn\s+slider_kind_at\s*\(.*?\n\}'
).Value
if ($sliderKindBlock -notmatch 'Section::Panel\s*\|\s*Section::Tooltip\s*\|\s*Section::Text\s*\|\s*Section::Progress\s*\|\s*Section::Interaction' -or
    $sliderKindBlock -notmatch 'return None;' -or
    $styleWindow -notmatch 'Section::General\s*\|\s*Section::Preset\s*\|\s*Section::Json\s*=>\s*&\[\]' -or
    $styleWindow -notmatch 's\.focused_numeric_edit\s*=\s*None' -or
    $styleWindow -notmatch 's\.focused_blur_edit\s*=\s*false' -or
    $styleWindow -notmatch 's\.focused_hex_edit\s*=\s*None' -or
    $styleWindow -notmatch 's\.pressed\s*=\s*None' -or
    $styleWindow -notmatch 's\.hovered\s*=\s*None') {
    throw 'Editorless settings pages must not inherit color-slider hit testing or stale editor interaction state.'
}
if ($styleWindow -notmatch 'if section == Section::Json' -or
    $styleWindow -notmatch 'reload_json_editor_from_snapshot\(\)' -or
    $styleWindow -notmatch 'refresh_json_editor_theme\(\)') {
    throw 'JSON editor must only refresh on the JSON section and remain untouched while preset/theme pages synchronize.'
}
$presetHitBlock = [regex]::Match(
    $styleWindow,
    '(?s)if\s+section\s*==\s*Section::Preset\s*\{.*?HitTarget::Preset\(preset\).*?\}'
).Value
if ([string]::IsNullOrWhiteSpace($presetHitBlock) -or
    $styleWindow -notmatch 'preset_card_rect\(hwnd, preset\)' -or
    $styleWindow -notmatch 'ThemePreset::ALL') {
    throw 'Preset page must keep dedicated hit targets for all three preset cards.'
}
if ($styleWindow -match 'Manage refresh, display, alerts, and application behavior' -or
    $styleWindow -notmatch 'settings_page_title_rect' -or
    $styleWindow -notmatch 'settings_card_rect' -or
    $styleWindow -notmatch 'settings_card_label_rect' -or
    $styleWindow -notmatch 'settings_choice_rect' -or
    $styleWindow -notmatch 'if zh \{ "常规" \} else \{ "General" \}' -or
    $styleWindow -notmatch 'settings_choice_rect\(hwnd, 68, 102, index\)' -or
    $styleWindow -notmatch 'settings_choice_rect\(hwnd, 268, 302, index\)' -or
    $styleWindow -notmatch 'settings_card_rect\(hwnd, 328, 464\)') {
    throw 'Settings pages must keep the shared right-pane layout baseline, restored General title, and matched refresh/alert padding.'
}
if ($styleWindow -notmatch 'SETTINGS_EDITOR_HEIGHT: i32 = 164' -or
    $styleWindow -notmatch 'SETTINGS_EDITOR_CHANNEL_GAP: i32 = 30' -or
    $styleWindow -notmatch 'client\.bottom - scale\(hwnd, 24\)' -or
    $styleWindow -notmatch 'editor\.bottom = editor\.bottom\.min\(safe_bottom\)') {
    throw 'Appearance editors must keep compact shared channel spacing and a bottom safety margin.'
}
if ($styleWindow -notmatch 'WS_CAPTION\s*\|\s*WS_SYSMENU' -or
    $styleWindow -notmatch 'WM_CLOSE\s*=>\s*\{' -or
    $styleWindow -notmatch 'send_parent\(WM_STYLE_SAVE, 0, 0\)' -or
    $styleWindow -notmatch 'DestroyWindow\(hwnd\)') {
    throw 'Settings must use the native titlebar close button and save before closing.'
}
if ($styleWindow -notmatch 'let edit = json_edit_rect\(hwnd\)' -or
    $styleWindow -notmatch 'let button_width = scale\(hwnd, 108\)' -or
    $styleWindow -notmatch 'JsonAction::Export\s*=>\s*RECT\s*\{[\s\S]{0,180}right:\s*edit\.right' -or
    $styleWindow -notmatch 'JsonAction::Import\s*=>\s*RECT\s*\{[\s\S]{0,220}right:\s*edit\.right - button_width - button_gap' -or
    $styleWindow -notmatch 'JsonAction::Apply\s*=>[\s\S]{0,260}let right = edit\.right') {
    throw 'JSON Import, Export, and Apply must share the JSON editor right edge and compact button width.'
}
if ($styleWindow -notmatch 'blur_row_index\(' -or
    $styleWindow -notmatch 'inline_numeric_frame_rect\(' -or
    $styleWindow -notmatch 'blur_edit_frame_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'blur_suffix_rect\(hwnd, section\)' -or
    $styleWindow -notmatch 'right: frame\.right - scale\(hwnd, 28\)') {
    throw 'Frosted-strength input must follow its actual settings row and keep percent as an internal fixed suffix.'
}
if ($styleWindow -match 'draw_outline_rect\(hdc, language_button') {
    throw 'Language selector must use one rounded outline rather than stacking a square outline over it.'
}


if ($styleWindow -notmatch 'load_embedded_app_icons\(' -or
    $styleWindow -notmatch 'hIcon:\s*large_icon' -or
    $styleWindow -notmatch 'hIconSm:\s*small_icon' -or
    $styleWindow -notmatch 'WM_SETICON' -or
    $styleWindow -notmatch 'ICON_BIG' -or
    $styleWindow -notmatch 'ICON_SMALL') {
    throw 'Settings window must bind the embedded executable icon to its standard caption.'
}
if ($styleWindow -match 'WS_VSCROLL' -or
    $styleWindow -match 'ShowScrollBar' -or
    $styleWindow -notmatch 'EM_GETLINECOUNT_MSG' -or
    $styleWindow -notmatch 'EM_GETFIRSTVISIBLELINE_MSG' -or
    $styleWindow -notmatch 'EM_LINESCROLL_MSG' -or
    $styleWindow -notmatch 'json_scroll_line_metrics\(' -or
    $styleWindow -notmatch 'json_scroll_thumb_rect\(' -or
    $styleWindow -notmatch 'first_visible\.clamp\(' -or
    $styleWindow -notmatch 'json_scroll_hovered' -or
    $styleWindow -notmatch '#555B64FF' -or
    $styleWindow -notmatch '#777E88FF' -or
    $styleWindow -match 'DarkMode_Explorer' -or
    $styleWindow -match 'SetWindowTheme\(') {
    throw 'JSON editor must have no native scrollbar and its single custom thumb must follow the RichEdit visible line.'
}
if ($styleWindow -notmatch 'PendingDiscardAction' -or
    $styleWindow -notmatch 'paint_discard_dialog\(' -or
    $styleWindow -notmatch 'HitTarget::DiscardChanges' -or
    $styleWindow -notmatch 'HitTarget::KeepEditing' -or
    $styleWindow -notmatch 'request_discard_confirmation\(hwnd, PendingDiscardAction::SwitchSection\(section\)\)' -or
    $styleWindow -notmatch 'request_discard_confirmation\(hwnd, PendingDiscardAction::Close\)' -or
    $styleWindow -notmatch 'if zh \{ "未保存的更改" \} else \{ "Unsaved changes" \}' -or
    $styleWindow -notmatch '当前 JSON 配置尚未保存。继续操作将丢失这些更改。' -or
    $styleWindow -notmatch 'if zh \{ "丢弃更改" \} else \{ "Discard changes" \}' -or
    $styleWindow -notmatch 'if zh \{ "返回编辑" \} else \{ "Return to editing" \}' -or
    $styleWindow -notmatch '#B65F63FF' -or
    $styleWindow -match '#C93C49FF' -or
    $styleWindow -match 'MessageBoxW\(' -or
    $styleWindow -match 'MB_YESNO') {
    throw 'Unsaved JSON changes must use the revised warning copy, a muted red Discard changes action, and Return to editing.'
}
if ($styleWindow -notmatch 'redraw_settings_window\(' -or
    $styleWindow -notmatch 'RDW_INVALIDATE\s*\|\s*RDW_ERASE\s*\|\s*RDW_ALLCHILDREN\s*\|\s*RDW_UPDATENOW' -or
    $styleWindow -notmatch 'fill_rounded_rect\(hdc, r, section_bg' -or
    $styleWindow -notmatch 'fill_rounded_rect\([\s\S]{0,260}r\.right - scale\(hwnd, 202\)' -or
    $styleWindow -notmatch 'fn language_popup_rect\(' -or
    $styleWindow -notmatch 'fill_rounded_rect\(hdc, popup_rect, card' -or
    $styleWindow -notmatch 'draw_rounded_outline_rect\([\s\S]{0,100}popup_rect') {
    throw 'Settings page switching must fully redraw and settings UI blocks must use rounded navigation, swatches, and an opaque rounded language popup.'
}
if ($styleWindow -notmatch 'i32::from\(style\.panel_corner_radius\)' -or
    $styleWindow -notmatch 'i32::from\(style\.tooltip_corner_radius\)' -or
    $styleWindow -notmatch 'i32::from\(style\.progress_corner_radius\)' -or
    $styleWindow -notmatch 'if panel_radius > 0' -or
    $styleWindow -notmatch 'if tooltip_radius > 0' -or
    $styleWindow -notmatch 'if progress_radius > 0' -or
    $styleWindow -notmatch 'fill\(hdc, preview, style\.color\(StyleColorTarget::PanelBackground\)\)') {
    throw 'Preset component previews must render the exact configured numeric component radii, including 0 as square.'
}
if ($settingsModel -notmatch 'panel_corner_radius:\s*u8' -or
    $settingsModel -notmatch 'tooltip_corner_radius:\s*u8' -or
    $settingsModel -notmatch 'progress_corner_radius:\s*u8' -or
    $settingsModel -notmatch 'panel_radius_max' -or
    $settingsModel -notmatch 'tooltip_radius_max' -or
    $settingsModel -notmatch 'progress_radius_max' -or
    $settingsModel -notmatch 'panel_corner_radius_max\(' -or
    $settingsModel -notmatch 'tooltip_corner_radius_max\(' -or
    $settingsModel -notmatch 'progress_corner_radius_max\(') {
    throw 'Numeric corner radii must round-trip through JSON/JSONC and validate against component-specific bounds.'
}

if ($styleWindow -notmatch "if s\.json_status\.starts_with\('×'\)" -or
    $styleWindow -notmatch 's\.json_status\.clear\(\)' -or
    $styleWindow -notmatch 's\.json_status_path = None') {
    throw 'JSON syntax/configuration errors must clear immediately after the edited JSON becomes valid.'
}

$drawSliderBlock = [regex]::Match(
    $styleWindow,
    '(?s)unsafe\s+fn\s+draw_slider\s*\(.*?\n\}'
).Value
if ($drawSliderBlock -match 'Ellipse\(' -or
    $drawSliderBlock -notmatch 'fill_rounded_rect\(hdc, track' -or
    $drawSliderBlock -notmatch 'fill_rounded_rect\(hdc, thumb, accent, radius\)') {
    throw 'Settings sliders must use anti-aliased rounded primitives instead of GDI Ellipse thumbs.'
}
if ($styleWindow -notmatch 'SETTINGS_EDIT_SUBCLASS_ID' -or
    $styleWindow -notmatch 'handle_settings_mouse_wheel\(' -or
    $styleWindow -notmatch 'adjust_numeric_by_wheel\(' -or
    $styleWindow -notmatch 'adjust_slider_by_wheel\(' -or
    $styleWindow -notmatch 'JSON_ERROR_SHAKE_STEPS:\s*u8\s*=\s*10' -or
    $styleWindow -notmatch 'scale\(hwnd, 4\)' -or
    $styleWindow -notmatch '"5H"' -or
    $styleWindow -notmatch '"7D"' -or
    $styleWindow -notmatch 'LanguageId::SELECTABLE\.len\(\)' -or
    $styleWindow -notmatch 'Some\(LanguageId::SimplifiedChinese\).*"中文"' -or
    $styleWindow -notmatch 'Some\(LanguageId::English\).*"English"') {
    throw 'Settings must support wheel adjustment, compact JSON-error shake, 5H/7D labels, and Chinese/English-only language selection.'
}

Write-Host 'PASS: v1.0.5 theme/style customization contract is satisfied.'


# Fixed-caption and layered-text regression contract
if ($styleWindow -match 'DWMWA_TRANSITIONS_FORCEDISABLED' -or
    $styleWindow -match 'DWMWA_USE_IMMERSIVE_DARK_MODE') {
    throw 'The fixed neutral style-window caption must not switch or animate with the active theme.'
}
if ($windowProduction -notmatch 'render_taskbar_text' -or
    $windowProduction -notmatch 'draw_directwrite_text' -or
    $windowProduction -notmatch 'directwrite_text_mask' -or
    $windowProduction -notmatch 'composite_premultiplied_text' -or
    $windowProduction -match 'text_quality_for_layered_surface' -or
    $windowProduction -match 'widget_text_quality\(\)' -or
    $windowProduction -match 'repair_taskbar_text' -or
    $windowProduction -match 'draw_layered_antialiased_text') {
    throw 'Taskbar text must use the final DirectWrite coverage path without obsolete rasterization experiments.'
}
