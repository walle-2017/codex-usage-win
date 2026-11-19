//! Numeric auxiliary edits for blur strength and corner radius.
//! Each control is created, font-initialized, limited and subclassed in order.
//! The caller retains parent HWND and font failure cleanup.
use super::*;

pub(super) unsafe fn create_auxiliary_numeric_edit(
    hwnd: HWND,
    font: HFONT,
    dpi: u32,
    id: u16,
    max_digits: usize,
) -> Option<HWND> {
    let s = |v: i32| ((v as i64 * dpi as i64 + 48) / 96) as i32;
    let edit_class = native_interop::wide_str("EDIT");
    let empty = native_interop::wide_str("");
    let edit = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(edit_class.as_ptr()),
        PCWSTR::from_raw(empty.as_ptr()),
        WINDOW_STYLE(
            WS_CHILD.0 | ES_NUMBER as u32 | ES_CENTER as u32 | ES_AUTOHSCROLL as u32,
        ),
        0,
        0,
        s(54),
        s(22),
        hwnd,
        HMENU(id as usize as *mut _),
        GetModuleHandleW(PCWSTR::null()).unwrap(),
        None,
    ).ok()?;
    let _ = SendMessageW(edit, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    let _ = SendMessageW(edit, EM_SETLIMITTEXT_MSG, WPARAM(max_digits), LPARAM(0));
    let _ = SetWindowSubclass(
        edit,
        Some(settings_edit_subclass_proc),
        SETTINGS_EDIT_SUBCLASS_ID,
        hwnd.0 as usize,
    );
    Some(edit)
}
