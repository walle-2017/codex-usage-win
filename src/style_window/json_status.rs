//! JSON status text, clickable export paths and validation-error feedback.
//! Preserve the original shared state locking and error-shake timer ordering.

use super::*;

pub(super) fn set_json_status(status: String) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_status = status;
        s.json_status_path = None;
        s.hwnd.to_hwnd()
    };
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn set_json_status_with_path(status: String, path: PathBuf) {
    let hwnd = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_status = status;
        s.json_status_path = Some(path);
        s.hwnd.to_hwnd()
    };
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub(super) fn open_json_status_path() {
    let path = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().and_then(|s| s.json_status_path.clone())
    };
    let Some(path) = path else {
        return;
    };
    let operation = native_interop::wide_str("open");
    let path_wide = native_interop::wide_str(&path.to_string_lossy());
    unsafe {
        let _ = ShellExecuteW(
            HWND::default(),
            PCWSTR::from_raw(operation.as_ptr()),
            PCWSTR::from_raw(path_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub(super) fn start_json_error_shake(hwnd: HWND) {
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_error_shake_step = 1;
    }
    unsafe {
        let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        let _ = SetTimer(
            hwnd,
            JSON_ERROR_SHAKE_TIMER_ID,
            JSON_ERROR_SHAKE_INTERVAL_MS,
            None,
        );
        let _ = InvalidateRect(hwnd, None, false);
        let _ = UpdateWindow(hwnd);
    }
}

pub(super) fn advance_json_error_shake(hwnd: HWND) {
    let finished = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        if s.json_error_shake_step == 0 {
            true
        } else if s.json_error_shake_step >= JSON_ERROR_SHAKE_STEPS {
            s.json_error_shake_step = 0;
            true
        } else {
            s.json_error_shake_step += 1;
            false
        }
    };
    unsafe {
        if finished {
            let _ = KillTimer(hwnd, JSON_ERROR_SHAKE_TIMER_ID);
        }
        let _ = InvalidateRect(hwnd, None, false);
        let _ = UpdateWindow(hwnd);
    }
}

pub(super) fn show_json_apply_validation_error(
    hwnd: HWND,
    edit: HWND,
    language: LanguageId,
    error: &str,
) {
    let detail = friendly_json_error(error, language);
    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };
        s.json_save_feedback = None;
        s.json_status_path = None;
        s.json_status = json_error_status(language, &detail);
    }
    locate_json_error(edit, error);
    start_json_error_shake(hwnd);
}
