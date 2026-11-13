//! WM_COMMAND edit-control notifications for the settings window.
//!
//! The existing synchronous EDIT notification and state-lock sequencing is
//! retained here, including Hex focus normalization and RichEdit validation.
//! Other window messages continue to be dispatched by style_window.rs.

use super::*;

pub(super) fn decode_settings_command(wparam: WPARAM) -> (u16, u16) {
    ((wparam.0 & 0xFFFF) as u16, ((wparam.0 >> 16) & 0xFFFF) as u16)
}

pub(super) unsafe fn handle_settings_command(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
            let (control_id, notification) = decode_settings_command(wparam);

            if control_id == ID_EDIT_JSON && notification == EN_CHANGE_CODE {
                invalidate_json_scroll_layout_cache();
                schedule_json_validation(hwnd);
                return LRESULT(0);
            }
            if let Some(target) = target_from_hex_control_id(control_id) {
                match notification {
                    EN_CHANGE_CODE => {
                        update_color_from_hex_edit(target);
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_hex_edit = Some(target);
                            }
                        }
                        select_editor(EditorSelection::Color(target));
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        normalize_hex_edit(target);
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                if s.focused_hex_edit == Some(target) {
                                    s.focused_hex_edit = None;
                                }
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            let channel = match control_id {
                ID_EDIT_R => Some(0),
                ID_EDIT_G => Some(1),
                ID_EDIT_B => Some(2),
                ID_EDIT_A => Some(3),
                _ => None,
            };
            if let Some(channel) = channel {
                match notification {
                    EN_CHANGE_CODE => {
                        update_color_from_numeric_edit(channel);
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_numeric_edit = Some(channel);
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                if s.focused_numeric_edit == Some(channel) {
                                    s.focused_numeric_edit = None;
                                }
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            if control_id == ID_EDIT_CORNER {
                match notification {
                    EN_CHANGE_CODE => {
                        update_corner_from_numeric_edit();
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_corner_edit = true;
                            }
                        }
                        select_editor(EditorSelection::CornerRadius);
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_corner_edit = false;
                            }
                        }
                        sync_corner_edit();
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            if control_id == ID_EDIT_BLUR {
                match notification {
                    EN_CHANGE_CODE => {
                        update_blur_from_numeric_edit();
                        return LRESULT(0);
                    }
                    EN_SETFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_blur_edit = true;
                            }
                        }
                        let editor = {
                            let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            state
                                .as_ref()
                                .map(|s| {
                                    if s.section == Section::Tooltip {
                                        EditorSelection::TooltipBlur
                                    } else {
                                        EditorSelection::Blur
                                    }
                                })
                                .unwrap_or(EditorSelection::Blur)
                        };
                        select_editor(editor);
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    EN_KILLFOCUS_CODE => {
                        {
                            let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(s) = state.as_mut() {
                                s.focused_blur_edit = false;
                            }
                        }
                        let _ = InvalidateRect(hwnd, None, false);
                        return LRESULT(0);
                    }
                    _ => {}
                }
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
}
