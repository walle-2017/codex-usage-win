//! Hex editing child controls for the settings panel.
//! The caller retains ownership of the parent window and GDI font cleanup.
use super::*;

pub(super) unsafe fn create_hex_edits(hwnd: HWND, font: HFONT, dpi: u32) -> Option<[SendHwnd; HEX_EDIT_COUNT]> {
    let s = |v: i32| ((v as i64 * dpi as i64 + 48) / 96) as i32;
    let edit_class = native_interop::wide_str("EDIT");
    let empty = native_interop::wide_str("");
    let mut hex_edits_raw = [HWND::default(); HEX_EDIT_COUNT];
    for (index, slot) in hex_edits_raw.iter_mut().enumerate() {
        let edit = match CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(edit_class.as_ptr()),
        PCWSTR::from_raw(empty.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | ES_CENTER as u32 | ES_AUTOHSCROLL as u32),
        0,
        0,
        s(112),
        s(22),
        hwnd,
        HMENU((ID_EDIT_HEX_BASE + index as u16) as usize as *mut _),
        GetModuleHandleW(PCWSTR::null()).unwrap(),
        None,
        ) {
        Ok(edit) => edit,
        Err(_) => return None,
        };
        let _ = SendMessageW(edit, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        let _ = SendMessageW(edit, EM_SETLIMITTEXT_MSG, WPARAM(9), LPARAM(0));
        *slot = edit;
    }
    Some(hex_edits_raw.map(SendHwnd::from_hwnd))
}
