//! JSON RichEdit scroll interception, opaque page paint and save-mask barrier.
//! Creation/order and callback behavior are unchanged; this module owns event handling only.
use super::*;

pub(super) unsafe extern "system" fn json_edit_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    _ref_data: usize,
) -> LRESULT {
    let saving = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state
            .as_ref()
            .is_some_and(|s| s.json_save_feedback == Some(JsonSaveFeedback::Saving))
    };
    if saving && matches!(msg, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
        return LRESULT(0);
    }

    if msg == WM_MOUSEWHEEL {
        let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        if delta != 0 {
            let notches = ((delta.abs() + JSON_WHEEL_DELTA - 1) / JSON_WHEEL_DELTA).max(1);
            let lines = if delta > 0 {
                -JSON_WHEEL_LINES_PER_NOTCH * notches
            } else {
                JSON_WHEEL_LINES_PER_NOTCH * notches
            };
            let panel = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.hwnd.to_hwnd())
            };
            if let Some(panel) = panel {
                scroll_json_editor_lines(hwnd, panel, lines);
            } else {
                let _ = SendMessageW(hwnd, EM_LINESCROLL_MSG, WPARAM(0), LPARAM(lines as isize));
            }
            return LRESULT(0);
        }
    }

    if msg == WM_NCDESTROY {
        let _ = RemoveWindowSubclass(hwnd, Some(json_edit_subclass_proc), JSON_EDIT_SUBCLASS_ID);
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

// JSON is rendered in a full opaque child content pane rather than directly
// over the previous appearance page. The editor is a sibling *above* this pane,
// so its background cannot accidentally reveal old preset/gallery pixels.
pub(super) unsafe extern "system" fn json_page_surface_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        // Clicking JSON toolbar controls must still reach the parent hit-test.
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEWHEEL => {
            // If Windows delivers a wheel message directly to the opaque
            // JSON pane, use the same parent routing as wheel messages that
            // hit the empty scrollbar strip.
            let owner = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.hwnd.to_hwnd())
            };
            if let Some(owner) = owner {
                SendMessageW(owner, msg, wparam, lparam)
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_PAINT => {
            paint_json_page_surface(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub(super) unsafe extern "system" fn json_save_mask_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1),
        // The save mask is a real interaction barrier for the JSON editor.
        // Keep it hit-testable so mouse clicks cannot pass through to the
        // RichEdit or the parent's custom scrollbar.
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_MOUSEWHEEL
        | WM_MOUSEHWHEEL
        | WM_LBUTTONDOWN
        | WM_LBUTTONUP
        | WM_LBUTTONDBLCLK
        | WM_RBUTTONDOWN
        | WM_RBUTTONUP
        | WM_RBUTTONDBLCLK
        | WM_MBUTTONDOWN
        | WM_MBUTTONUP
        | WM_MBUTTONDBLCLK
        | WM_XBUTTONDOWN
        | WM_XBUTTONUP
        | WM_XBUTTONDBLCLK => LRESULT(0),
        WM_SETCURSOR => {
            SetCursor(LoadCursorW(None, IDC_ARROW).unwrap_or_default());
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let is_dark = {
                let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                state.as_ref().map(|s| s.snapshot.is_dark).unwrap_or(false)
            };
            fill(
                hdc,
                client,
                if is_dark {
                    Color::from_hex("#0F141AFF")
                } else {
                    Color::from_hex("#AEB8C4FF")
                },
            );
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub(super) fn ensure_json_save_mask(owner: HWND) -> Option<HWND> {
    let existing = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.json_save_mask.to_hwnd())
    }?;
    if !existing.0.is_null() {
        return Some(existing);
    }

    unsafe {
        let class_name = native_interop::wide_str(JSON_SAVE_MASK_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(json_save_mask_wnd_proc),
            hInstance: GetModuleHandleW(PCWSTR::null()).unwrap().into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            lpszClassName: PCWSTR::from_raw(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);

        let empty = native_interop::wide_str("");
        let mask = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            PCWSTR::from_raw(class_name.as_ptr()),
            PCWSTR::from_raw(empty.as_ptr()),
            WS_POPUP,
            0,
            0,
            1,
            1,
            owner,
            HMENU::default(),
            GetModuleHandleW(PCWSTR::null()).unwrap(),
            None,
        ).ok()?;

        let _ = SetLayeredWindowAttributes(
            mask,
            COLORREF(0),
            JSON_SAVE_MASK_ALPHA,
            LWA_ALPHA,
        );

        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = state.as_mut() {
            s.json_save_mask = SendHwnd::from_hwnd(mask);
        }
        Some(mask)
    }
}
