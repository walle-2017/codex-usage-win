from pathlib import Path

path = Path('scripts/assert-style-customization.ps1')
text = path.read_text(encoding='utf-8-sig')
original = text

text = text.replace(
    "$windowProduction -notmatch '样式设置\\\\t⚙' -or",
    "$windowProduction -notmatch '样式设置\\.\\.\\.\\\\t⚙' -or",
)
text = text.replace(
    "$windowProduction -notmatch 'Style settings\\\\t⚙') {",
    "$windowProduction -notmatch 'Style settings\\.\\.\\.\\\\t⚙') {",
)
text = text.replace(
    "    $styleWindow -notmatch 'IDC_SIZEWE' -or\n    $styleWindow -notmatch 'slider_kind_at\\(' -or",
    "    $styleWindow -notmatch 'IDC_SIZEWE' -or\n    $styleWindow -notmatch 'IDC_IBEAM' -or\n    $styleWindow -notmatch 'is_numeric_edit_control\\(' -or\n    $styleWindow -notmatch 'slider_kind_at\\(' -or",
)

if text == original:
    raise SystemExit('RED contract patch made no changes')
if "样式设置\\.\\.\\.\\\\t⚙" not in text:
    raise SystemExit('Chinese menu contract was not patched')
if "IDC_IBEAM" not in text or "is_numeric_edit_control" not in text:
    raise SystemExit('I-Beam contract was not patched')

path.write_text(text, encoding='utf-8')
