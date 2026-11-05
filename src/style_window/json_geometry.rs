//! JSON editor geometry shared by rendering, child layout and hit testing.
//! Keep the original dimensions and Win32 client-area measurements.

use super::*;

pub(super) fn json_edit_rect(hwnd: HWND) -> RECT {
    let mut client = RECT::default();
    unsafe { let _ = GetClientRect(hwnd, &mut client); }
    RECT {
        left: scale(hwnd, 200),
        top: scale(hwnd, 108),
        right: client.right - scale(hwnd, 24),
        bottom: client.bottom - scale(hwnd, 140),
    }
}

pub(super) fn json_edit_text_rect(hwnd: HWND) -> RECT {
    let outer = json_edit_rect(hwnd);
    RECT {
        left: outer.left + scale(hwnd, 2),
        top: outer.top + scale(hwnd, 2),
        right: outer.right - scale(hwnd, 16),
        bottom: outer.bottom - scale(hwnd, 2),
    }
}

pub(super) fn json_save_mask_rect(hwnd: HWND) -> RECT {
    let outer = json_edit_rect(hwnd);
    RECT {
        left: outer.left + scale(hwnd, 2),
        top: outer.top + scale(hwnd, 2),
        right: outer.right - scale(hwnd, 2),
        bottom: outer.bottom - scale(hwnd, 2),
    }
}

pub(super) fn json_action_rect(hwnd: HWND, action: JsonAction) -> RECT {
    let edit = json_edit_rect(hwnd);
    let button_width = scale(hwnd, 108);
    let button_gap = scale(hwnd, 12);
    match action {
        JsonAction::Reload => rect(hwnd, 200, 58, 306, 94),
        JsonAction::Format => rect(hwnd, 318, 58, 424, 94),
        JsonAction::Export => RECT {
            left: edit.right - button_width,
            top: scale(hwnd, 58),
            right: edit.right,
            bottom: scale(hwnd, 94),
        },
        JsonAction::Import => RECT {
            left: edit.right - button_width * 2 - button_gap,
            top: scale(hwnd, 58),
            right: edit.right - button_width - button_gap,
            bottom: scale(hwnd, 94),
        },
        JsonAction::Apply => {
            let mut client = RECT::default();
            unsafe { let _ = GetClientRect(hwnd, &mut client); }
            let right = edit.right;
            let width = button_width;
            let height = scale(hwnd, 36);
            let bottom = client.bottom - scale(hwnd, 24);
            RECT {
                left: right - width,
                top: bottom - height,
                right,
                bottom,
            }
        }
    }
}

pub(super) fn json_error_shake_offset(hwnd: HWND, step: u8) -> i32 {
    if step == 0 || JSON_ERROR_SHAKE_STEPS <= 1 {
        return 0;
    }
    let progress = f32::from(step.saturating_sub(1))
        / f32::from(JSON_ERROR_SHAKE_STEPS.saturating_sub(1));
    let amplitude = scale(hwnd, 4) as f32 * (1.0 - progress);
    let phase = progress * std::f32::consts::PI * 4.0;
    (amplitude * phase.sin()).round() as i32
}

pub(super) fn json_editor_should_be_visible(section: Section, discard_pending: bool) -> bool {
    section == Section::Json && !discard_pending
}
