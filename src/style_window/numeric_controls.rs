//! Create the four RGBA numeric EDIT children without changing their order.
//! Parent HWND and font cleanup remain with the settings-window caller.
use super::*;

pub(super) unsafe fn create_rgba_numeric_edits(hwnd: HWND, font: HFONT, dpi: u32) -> Option<[SendHwnd; 4]> {
    let s = |v: i32| ((v as i64 * dpi as i64 + 48) / 96) as i32;
    let edit_class = native_interop::wide_str("EDIT");
    let empty = native_interop::wide_str("");
    let mut numeric_edits_raw = [HWND::default(); 4];
    for (index, id) in [ID_EDIT_R, ID_EDIT_G, ID_EDIT_B, ID_EDIT_A]
        .iter()
        .copied()
        .enumerate()
    {
        let edit = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR::from_raw(edit_class.as_ptr()),
            PCWSTR::from_raw(empty.as_ptr()),
            WINDOW_STYLE(
                WS_CHILD.0
                    | ES_NUMBER as u32
                    | ES_CENTER as u32
                    | ES_AUTOHSCROLL as u32,
            ),
            0,
            0,
            s(54),
            s(22),
            hwnd,
            HMENU(id as usize as *mut _),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ) {
            Ok(edit) => edit,
            Err(_) => return None,
        };
        let _ = SendMessageW(
            edit,
            WM_SETFONT,
            WPARAM(font.0 as usize),
            LPARAM(1),
        );
        let _ = SendMessageW(
            edit,
            EM_SETLIMITTEXT_MSG,
            WPARAM(3),
            LPARAM(0),
        );
        let _ = SetWindowSubclass(
            edit,
            Some(settings_edit_subclass_proc),
            SETTINGS_EDIT_SUBCLASS_ID,
            hwnd.0 as usize,
        );
        numeric_edits_raw[index] = edit;
    }
    Some(numeric_edits_raw.map(SendHwnd::from_hwnd))
}
