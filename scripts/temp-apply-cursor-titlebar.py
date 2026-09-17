from pathlib import Path
import re

BRANCH_FILES = [
    Path('src/style_window.rs'),
    Path('src/window.rs'),
    Path('scripts/assert-style-customization.ps1'),
]

# ---- Permanent contract first ----
contract = BRANCH_FILES[2]
text = contract.read_text(encoding='utf-8-sig')
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
    raise SystemExit('permanent titlebar contract anchor not found')
text = text.replace(old_title, new_title)
contract.write_text(text, encoding='utf-8')

# ---- Style settings window ----
path = BRANCH_FILES[0]
text = path.read_text(encoding='utf-8-sig')
text = text.replace(
    "use windows::Win32::Graphics::Dwm::{\n    DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_IMMERSIVE_DARK_MODE,\n};",
    "use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR};"
)

anchor = "const WM_MOUSELEAVE_MSG: u32 = 0x02A3;\n"
if anchor not in text:
    raise SystemExit('style window constant anchor not found')
text = text.replace(
    anchor,
    anchor + "const FIXED_CAPTION_COLORREF: u32 = 0x00524843; // #434852 in COLORREF byte order\n"
             "const FIXED_CAPTION_TEXT_COLORREF: u32 = 0x00FFFFFF;\n"
)

text = text.replace("        apply_titlebar_theme(hwnd, snapshot.is_dark);", "        apply_fixed_titlebar(hwnd);")
text = text.replace("    let is_dark = snapshot.is_dark;\n", "")
text = text.replace("    apply_titlebar_theme(hwnd, is_dark);\n", "")

old_fn = re.compile(
    r"fn apply_titlebar_theme\(hwnd: HWND, is_dark: bool\) \{.*?\n\}\n\npub fn decode_color_target",
    re.S,
)
new_fn = """fn apply_fixed_titlebar(hwnd: HWND) {
    let caption_color = COLORREF(FIXED_CAPTION_COLORREF);
    let text_color = COLORREF(FIXED_CAPTION_TEXT_COLORREF);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption_color as *const COLORREF as *const std::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text_color as *const COLORREF as *const std::ffi::c_void,
            std::mem::size_of::<COLORREF>() as u32,
        );
    }
}

pub fn decode_color_target"""
text, count = old_fn.subn(new_fn, text, count=1)
if count != 1:
    raise SystemExit(f'expected one apply_titlebar_theme function, replaced {count}')

# Semantic cursor: hand for clickable custom controls, horizontal-resize for sliders.
mouse_anchor = "        WM_LBUTTONDOWN => {\n"
if mouse_anchor not in text:
    raise SystemExit('style window mouse anchor not found')
mouse_block = """        WM_SETCURSOR => {
            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = ScreenToClient(hwnd, &mut point);

            let cursor_id = if slider_kind_at(hwnd, point.x, point.y).is_some() {
                IDC_SIZEWE
            } else if hit_target_at(hwnd, point.x, point.y).is_some() {
                IDC_HAND
            } else {
                IDC_ARROW
            };
            let cursor = LoadCursorW(None, cursor_id).unwrap_or_default();
            SetCursor(cursor);
            LRESULT(1)
        }
"""
text = text.replace(mouse_anchor, mouse_block + mouse_anchor, 1)
path.write_text(text, encoding='utf-8')

# ---- Main taskbar window ----
path = BRANCH_FILES[1]
text = path.read_text(encoding='utf-8-sig')
old_menu = """        let style_settings_label =
            native_interop::wide_str(if language == LanguageId::SimplifiedChinese {
                \"样式设置...\"
            } else {
                \"Style settings...\"
            });
"""
new_menu = """        let style_settings_label =
            native_interop::wide_str(if language == LanguageId::SimplifiedChinese {
                \"样式设置\\t⚙\"
            } else {
                \"Style settings\\t⚙\"
            });
"""
if old_menu not in text:
    raise SystemExit('style settings menu label anchor not found')
text = text.replace(old_menu, new_menu, 1)

# In the existing WM_SETCURSOR block, drag handle keeps SIZEALL. The remaining
# small-taskbar surface is click-to-toggle, so advertise it with IDC_HAND.
pattern = re.compile(
    r"(\s*if cursor_is_on_drag_handle\(hwnd\) \{.*?return LRESULT\(1\);\s*\})",
    re.S,
)
match = pattern.search(text)
if not match:
    raise SystemExit('taskbar cursor drag-handle block not found')
insert = match.group(1) + """
            let small_taskbar_mode = {
                let state = lock_state();
                state
                    .as_ref()
                    .map(|s| s.small_taskbar_mode)
                    .unwrap_or(false)
            };
            if small_taskbar_mode {
                let cursor = LoadCursorW(HINSTANCE::default(), IDC_HAND).unwrap_or_default();
                SetCursor(cursor);
                return LRESULT(1);
            }
"""
text = text[:match.start()] + insert + text[match.end():]
path.write_text(text, encoding='utf-8')
