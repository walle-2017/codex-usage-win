from pathlib import Path

path = Path('scripts/assert-style-customization.ps1')
text = path.read_text(encoding='utf-8-sig')
old = """# Theme-sync and layered-text regression contract
if ($styleWindow -notmatch 'DWMWA_TRANSITIONS_FORCEDISABLED' -or
    $styleWindow -notmatch 'UpdateWindow\\(hwnd\\)') {
    throw 'Style-panel client and title-bar theme changes must commit without a delayed DWM transition.'
}
if ($windowProduction -notmatch 'text_quality_for_layered_surface' -or
    $windowProduction -notmatch 'NONANTIALIASED_QUALITY' -or
    $windowProduction -notmatch 'widget_text_quality\\(\\)') {
    throw 'Transparent/frosted taskbar text must avoid ClearType background-fringe artifacts.'
}
"""
new = """# Fixed-caption and layered-text regression contract
if ($styleWindow -match 'DWMWA_TRANSITIONS_FORCEDISABLED' -or
    $styleWindow -match 'DWMWA_USE_IMMERSIVE_DARK_MODE') {
    throw 'The fixed neutral style-window caption must not switch or animate with the active theme.'
}
if ($windowProduction -notmatch 'text_quality_for_layered_surface' -or
    $windowProduction -notmatch 'NONANTIALIASED_QUALITY' -or
    $windowProduction -notmatch 'widget_text_quality\\(\\)') {
    throw 'Transparent/frosted taskbar text must avoid ClearType background-fringe artifacts.'
}
"""
if old not in text:
    raise SystemExit('legacy theme-sync regression contract anchor not found')
path.write_text(text.replace(old, new), encoding='utf-8')
