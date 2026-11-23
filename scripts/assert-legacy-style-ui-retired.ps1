$ErrorActionPreference = 'Stop'
$window = Get-Content -Raw (Join-Path $PSScriptRoot '..\src\window.rs')
$legacy = @(
    'fn open_color_editor(',
    'fn open_blur_editor(',
    'fn register_color_editor_class(',
    'fn register_blur_editor_class(',
    'fn close_style_editors(',
    'COLOR_EDITOR_STATE',
    'BLUR_EDITOR_STATE',
    'IDM_STYLE_PANEL_BACKGROUND',
    'IDM_STYLE_PANEL_BLUR',
    'IDM_STYLE_RESET_CURRENT',
    'TBM_GETPOS_MSG',
    'STYLE_PREVIEW_FRAME_MS',
    'IDM_START_WITH_WINDOWS',
    'IDM_SHOW_SESSION_WINDOW',
    'IDM_SHOW_WEEKLY_WINDOW',
    'IDM_FREQ_1MIN',
    'IDM_ALERT_OFF',
    'IDM_LANG_SYSTEM'
)
foreach ($symbol in $legacy) {
    if ($window.Contains($symbol)) { throw "Unreachable legacy native style editor returned: $symbol" }
}
if ($window -notmatch 'PopupItem::command\(settings_text, IDM_STYLE_SETTINGS\)' -or
    $window -notmatch 'style_window::open_or_focus\(' -or
    $window -notmatch 'style_window::WM_STYLE_COLOR_PREVIEW' -or
    $window -notmatch 'style_window::WM_STYLE_BLUR_PREVIEW' -or
    $window -notmatch 'quick_appearance_submenus\(') {
    throw 'Unified Settings, style preview messages, or quick appearance actions are missing.'
}
Write-Output 'PASS: unreachable legacy native style dialogs are retired; unified Settings remains.'
