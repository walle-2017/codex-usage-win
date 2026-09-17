from pathlib import Path

path = Path('scripts/assert-style-customization.ps1')
text = path.read_text(encoding='utf-8-sig')

text = text.replace(
    "$styleWindow -notmatch '\"主题\"' -or\n    $windowProduction -notmatch '\"样式设置\\.\\.\\.\"') {\n    throw 'Unified panel Theme/Layout labels and the Style Settings menu entry must be present.'\n}",
    "$styleWindow -notmatch '\"主题\"' -or\n    $windowProduction -notmatch '样式设置\\\\t⚙' -or\n    $windowProduction -notmatch 'Style settings\\\\t⚙') {\n    throw 'Unified panel Theme/Layout labels and the right-aligned gear Style Settings menu entry must be present.'\n}"
)

old_title = """if ($styleWindow -notmatch 'DwmSetWindowAttribute' -or
    $styleWindow -notmatch 'DWMWA_USE_IMMERSIVE_DARK_MODE' -or
    $styleWindow -notmatch 'apply_titlebar_theme\\(') {
    throw 'Style window title bar must follow the active dark/light theme.'
}
"""
new_title = """if ($styleWindow -notmatch 'DwmSetWindowAttribute' -or
    $styleWindow -notmatch 'DWMWA_CAPTION_COLOR' -or
    $styleWindow -notmatch 'DWMWA_TEXT_COLOR' -or
    $styleWindow -notmatch 'FIXED_CAPTION_COLORREF:\\s*u32\\s*=\\s*0x00524843' -or
    $styleWindow -notmatch 'FIXED_CAPTION_TEXT_COLORREF:\\s*u32\\s*=\\s*0x00FFFFFF' -or
    $styleWindow -notmatch 'apply_fixed_titlebar\\(' -or
    $styleWindow -match 'DWMWA_USE_IMMERSIVE_DARK_MODE' -or
    $styleWindow -match 'DWMWA_TRANSITIONS_FORCEDISABLED') {
    throw 'Style window title bar must use one fixed neutral caption color in every theme.'
}
if ($styleWindow -notmatch 'WM_SETCURSOR' -or
    $styleWindow -notmatch 'IDC_HAND' -or
    $styleWindow -notmatch 'IDC_SIZEWE' -or
    $styleWindow -notmatch 'slider_kind_at\\(' -or
    $styleWindow -notmatch 'hit_target_at\\(') {
    throw 'Style panel clickable controls and sliders must expose semantic mouse cursors.'
}
if ($windowProduction -notmatch 'WM_SETCURSOR' -or
    $windowProduction -notmatch 'small_taskbar_mode' -or
    $windowProduction -notmatch 'IDC_HAND') {
    throw 'Small-taskbar click-to-toggle mode must expose a hand cursor outside the drag handle.'
}
"""
if old_title not in text:
    raise SystemExit('titlebar contract anchor not found')
text = text.replace(old_title, new_title)

path.write_text(text, encoding='utf-8')
