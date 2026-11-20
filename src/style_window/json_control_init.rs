//! JSON opaque pane is created before RichEdit: preserve sibling Z ordering.
//! Caller owns window and font cleanup on failure.
use super::*;

pub(super) unsafe fn create_json_editor_children(hwnd: HWND, json_font: HFONT, dpi: u32, is_dark: bool) -> Option<(HWND, HWND)> {
    let s = |v: i32| ((v as i64 * dpi as i64 + 48) / 96) as i32;
    let empty = native_interop::wide_str("");
    // Create the opaque JSON content pane before RichEdit. As a sibling
    // child, it stays BELOW RichEdit in Z order but ABOVE all stale parent
    // page pixels. Painting the entire pane prevents mixed-page frames.
    let json_page_class = native_interop::wide_str(JSON_PAGE_SURFACE_CLASS);
    let page_wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(json_page_surface_wnd_proc),
        hInstance: GetModuleHandleW(PCWSTR::null()).unwrap().into(),
        hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
        hbrBackground: HBRUSH(std::ptr::null_mut()),
        lpszClassName: PCWSTR::from_raw(json_page_class.as_ptr()),
        ..Default::default()
    };
    let _ = RegisterClassExW(&page_wc);
    let json_page_surface = match CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(json_page_class.as_ptr()),
        PCWSTR::from_raw(empty.as_ptr()),
        WS_CHILD | WS_CLIPSIBLINGS,
        0, 0, 1, 1,
        hwnd,
        HMENU::default(),
        GetModuleHandleW(PCWSTR::null()).unwrap(),
        None,
    ) {
        Ok(page) => page,
        Err(_) => {
            return None;
        }
    };

    let msftedit = native_interop::wide_str("Msftedit.dll");
    if LoadLibraryW(PCWSTR::from_raw(msftedit.as_ptr())).is_err() {
            return None;
    }
    let rich_edit_class = native_interop::wide_str(RICH_EDIT_CLASS);
    let json_edit = match CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR::from_raw(rich_edit_class.as_ptr()),
        PCWSTR::from_raw(empty.as_ptr()),
        WINDOW_STYLE(
            WS_CHILD.0
                | WS_CLIPSIBLINGS.0
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_AUTOHSCROLL as u32
                | ES_WANTRETURN as u32
                | ES_NOHIDESEL as u32,
        ),
        0,
        0,
        s(700),
        s(480),
        hwnd,
        HMENU(ID_EDIT_JSON as usize as *mut _),
        GetModuleHandleW(PCWSTR::null()).unwrap(),
        None,
    ) {
        Ok(value) => value,
        Err(_) => {
            return None;
        }
    };
    let _ = SendMessageW(json_edit, WM_SETFONT, WPARAM(json_font.0 as usize), LPARAM(1));
    let _ = SetWindowSubclass(
        json_edit,
        Some(json_edit_subclass_proc),
        JSON_EDIT_SUBCLASS_ID,
        0,
    );
    let _ = SendMessageW(json_edit, EM_SETLIMITTEXT_MSG, WPARAM(JSON_EDIT_LIMIT), LPARAM(0));
    let _ = SendMessageW(
        json_edit,
        EM_SETEVENTMASK_MSG,
        WPARAM(0),
        LPARAM(ENM_CHANGE_MASK),
    );
    let json_background = if is_dark {
        Color::from_hex("#1E1E1EFF")
    } else {
        Color::from_hex("#FFFFFFFF")
    };
    let _ = SendMessageW(
        json_edit,
        EM_SETBKGNDCOLOR_MSG,
        WPARAM(0),
        LPARAM(json_background.to_colorref() as isize),
    );
    Some((json_page_surface, json_edit))
}
