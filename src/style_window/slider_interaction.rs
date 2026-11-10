//! Native numeric wheel, appearance slider wheel and dragging interactions.
//! Preserve focused EDIT repaint, preview/save notification order and 60 FPS presentation.

use super::*;

pub(super) fn adjust_numeric_by_wheel(target: WheelNumericTarget, direction: i32) {
    if direction == 0 {
        return;
    }
    let (edit, current, max) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        match target {
            WheelNumericTarget::Color(index) => {
                let EditorSelection::Color(color_target) = s.editor else {
                    return;
                };
                let color = s.snapshot.active_style.color(color_target);
                let values = [color.r, color.g, color.b, color.a];
                let Some(current) = values.get(index).copied() else {
                    return;
                };
                (s.numeric_edits[index].to_hwnd(), current, u8::MAX)
            }
            WheelNumericTarget::Blur => {
                let current = match s.section {
                    Section::Panel => s.snapshot.active_style.panel_frosted_strength,
                    Section::Tooltip => s.snapshot.active_style.tooltip_frosted_strength(),
                    _ => return,
                };
                (s.blur_edit.to_hwnd(), current, FROSTED_STRENGTH_MAX)
            }
            WheelNumericTarget::Corner => {
                let Some(max) =
                    corner_radius_max_for_section(s.snapshot.appearance_preset, s.section)
                else {
                    return;
                };
                let current = match s.section {
                    Section::Panel => s.snapshot.active_style.panel_corner_radius,
                    Section::Tooltip => s.snapshot.active_style.tooltip_corner_radius,
                    Section::Progress => s.snapshot.active_style.progress_corner_radius,
                    _ => return,
                };
                (s.corner_edit.to_hwnd(), current, max)
            }
        }
    };

    let next = (i32::from(current) + direction.signum())
        .clamp(0, i32::from(max)) as u8;
    if next != current {
        // WM_SETTEXT synchronously triggers EN_CHANGE. Blur's handler already
        // updates state, redraws EDIT and queues a slider frame. Other numeric
        // controls keep their existing synchronization and repaint behavior.
        set_edit_text(edit, next);
        match target {
            WheelNumericTarget::Color(_) => sync_numeric_edits(),
            WheelNumericTarget::Blur => return,
            WheelNumericTarget::Corner => sync_corner_edit(),
        }
        unsafe {
            let _ = RedrawWindow(
                edit, None, HRGN::default(),
                RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW,
            );
        }
    }
}

pub(super) fn adjust_slider_by_wheel(hwnd: HWND, kind: SliderKind, direction: i32) {
    if direction == 0 {
        return;
    }
    let (current, max, track) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            return;
        };
        match kind {
            SliderKind::Blur => {
                let current = match s.section {
                    Section::Panel => s.snapshot.active_style.panel_frosted_strength,
                    Section::Tooltip => s.snapshot.active_style.tooltip_frosted_strength(),
                    _ => return,
                };
                (
                    current,
                    FROSTED_STRENGTH_MAX,
                    blur_slider_track_rect(hwnd, s.section),
                )
            }
            SliderKind::Red | SliderKind::Green | SliderKind::Blue | SliderKind::Alpha => {
                let EditorSelection::Color(target) = s.editor else {
                    return;
                };
                let color = s.snapshot.active_style.color(target);
                let index = match kind {
                    SliderKind::Red => 0,
                    SliderKind::Green => 1,
                    SliderKind::Blue => 2,
                    SliderKind::Alpha => 3,
                    SliderKind::Blur => unreachable!(),
                };
                (
                    [color.r, color.g, color.b, color.a][index],
                    u8::MAX,
                    color_slider_track_rect(hwnd, s.section, index),
                )
            }
        }
    };

    let next = (i32::from(current) + direction.signum())
        .clamp(0, i32::from(max)) as u8;
    if next == current {
        return;
    }
    let width = (track.right - track.left).max(1);
    let denominator = i32::from(max.max(1));
    let x = track.left + (i32::from(next) * width + denominator / 2) / denominator;
    update_slider(hwnd, kind, x);
    send_parent(WM_STYLE_SAVE, 0, 0);
}

pub(super) fn update_slider(hwnd: HWND, kind: SliderKind, x: i32) {
    let mut color_update: Option<(StyleColorTarget, Color)> = None;
    let mut blur_update: Option<(Section, u8)> = None;

    {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else {
            return;
        };

        match (s.editor, kind) {
            (EditorSelection::Color(target), SliderKind::Red)
            | (EditorSelection::Color(target), SliderKind::Green)
            | (EditorSelection::Color(target), SliderKind::Blue)
            | (EditorSelection::Color(target), SliderKind::Alpha) => {
                let index = match kind {
                    SliderKind::Red => 0,
                    SliderKind::Green => 1,
                    SliderKind::Blue => 2,
                    SliderKind::Alpha => 3,
                    SliderKind::Blur => return,
                };
                let value = slider_value_from_x(
                    color_slider_track_rect(hwnd, s.section, index),
                    x,
                    u8::MAX,
                );
                let current = s.snapshot.active_style.color(target);
                let color = match kind {
                    SliderKind::Red => Color::rgba(value, current.g, current.b, current.a),
                    SliderKind::Green => Color::rgba(current.r, value, current.b, current.a),
                    SliderKind::Blue => Color::rgba(current.r, current.g, value, current.a),
                    SliderKind::Alpha => Color::rgba(current.r, current.g, current.b, value),
                    SliderKind::Blur => current,
                };
                s.snapshot.active_style.set_color(target, color);
                sync_active_style_into_editable(s);
                color_update = Some((target, color));
            }
            (_, SliderKind::Blur) if matches!(s.section, Section::Panel | Section::Tooltip) => {
                let value = slider_value_from_x(
                    blur_slider_track_rect(hwnd, s.section),
                    x,
                    FROSTED_STRENGTH_MAX,
                );
                if s.section == Section::Tooltip {
                    s.snapshot.active_style.set_tooltip_frosted_strength(value);
                } else {
                    s.snapshot.active_style.panel_frosted_strength = value;
                }
                sync_active_style_into_editable(s);
                blur_update = Some((s.section, value));
            }
            _ => {}
        }
    }

    if let Some((target, color)) = color_update {
        sync_numeric_edits();
        sync_hex_edits();
        send_parent(
            WM_STYLE_COLOR_PREVIEW,
            encode_color_target(target),
            pack_color(color),
        );
    }
    if let Some((section, value)) = blur_update {
        sync_blur_edit();
        send_parent(
            if section == Section::Tooltip { WM_STYLE_TOOLTIP_BLUR_PREVIEW } else { WM_STYLE_BLUR_PREVIEW },
            value as usize,
            0,
        );
    }

    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
    // A drag can keep the message queue nonempty, starving generated WM_PAINT.
    // Cap active drag presentation at ~60 fps, then flush the final value on
    // WM_LBUTTONUP via invalidate_appearance_card_or_window.
    queue_settings_interactive_frame(hwnd, true);
}
