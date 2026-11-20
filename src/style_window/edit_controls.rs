//! Native EDIT and RichEdit text messaging helpers.
//! Preserve original Win32 message order and edit control semantics.

use super::*;

pub(super) fn set_edit_text_string(edit: HWND, value: &str) {
    let text = native_interop::wide_str(value);
    unsafe {
        let _ = SendMessageW(
            edit,
            WM_SETTEXT,
            WPARAM(0),
            LPARAM(text.as_ptr() as isize),
        );
    }
}

pub(super) fn set_edit_text(edit: HWND, value: u8) {
    let text = native_interop::wide_str(&value.to_string());
    unsafe {
        let _ = SendMessageW(
            edit,
            WM_SETTEXT,
            WPARAM(0),
            LPARAM(text.as_ptr() as isize),
        );
    }
}

pub(super) fn read_edit_value(edit: HWND) -> Option<u16> {
    let mut buffer = [0u16; 4];
    let len = unsafe {
        SendMessageW(
            edit,
            WM_GETTEXT,
            WPARAM(buffer.len()),
            LPARAM(buffer.as_mut_ptr() as isize),
        )
        .0 as usize
    };
    if len == 0 {
        return None;
    }
    String::from_utf16_lossy(&buffer[..len]).parse::<u16>().ok()
}

pub(super) fn read_edit_text(edit: HWND) -> String {
    let mut buffer = [0u16; 16];
    let len = unsafe {
        SendMessageW(
            edit,
            WM_GETTEXT,
            WPARAM(buffer.len()),
            LPARAM(buffer.as_mut_ptr() as isize),
        )
        .0 as usize
    };
    String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
}

pub(super) unsafe fn rich_set_selection(edit: HWND, start: i32, end: i32) {
    let mut range = RichCharRange {
        cp_min: start,
        cp_max: end,
    };
    let _ = SendMessageW(
        edit,
        EM_EXSETSEL_MSG,
        WPARAM(0),
        LPARAM((&mut range as *mut RichCharRange) as isize),
    );
}

pub(super) unsafe fn rich_set_selected_color(edit: HWND, color: Color) {
    let mut format = RichCharFormatW {
        cb_size: std::mem::size_of::<RichCharFormatW>() as u32,
        dw_mask: CFM_COLOR_MASK,
        cr_text_color: COLORREF(color.to_colorref()),
        ..Default::default()
    };
    let _ = SendMessageW(
        edit,
        EM_SETCHARFORMAT_MSG,
        WPARAM(SCF_SELECTION_FLAG),
        LPARAM((&mut format as *mut RichCharFormatW) as isize),
    );
}
