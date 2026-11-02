//! JSON settings page and custom scrollbar decoration painting.
//!
//! Original draw order, GDI resource ownership, theme colors, editor status,
//! validation feedback and scroll-thumb styling are preserved unchanged.
//! The parent window procedure still controls when each painter is called.

use super::*;

// Shared between the full JSON page paint and scrollbar-only child paints.
pub(super) unsafe fn paint_json_scrollbar_decoration(hdc: HDC, hwnd: HWND, is_dark: bool) {
    let scroll_hovered = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| s.json_scroll_hovered).unwrap_or(false)
    };
    if let Some(thumb) = json_scroll_thumb_rect(hwnd) {
        if scroll_hovered {
            let track = json_scrollbar_track_rect(hwnd);
            fill_rounded_rect(
                hdc,
                track,
                if is_dark {
                    // Composite equivalent of a very low-opacity light gray over
                    // the JSON editor background; GDI brushes do not alpha blend.
                    Color::from_hex("#303033FF")
                } else {
                    Color::from_hex("#F1F2F4FF")
                },
                ((track.right - track.left) / 2).max(1),
            );
        }
        fill_rounded_rect(
            hdc,
            thumb,
            if scroll_hovered {
                if is_dark {
                    Color::from_hex("#777E88FF")
                } else {
                    Color::from_hex("#666D76FF")
                }
            } else if is_dark {
                Color::from_hex("#555B64FF")
            } else {
                Color::from_hex("#747B84FF")
            },
            ((thumb.right - thumb.left) / 2).max(1),
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_json_page(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    status: &str,
    status_path: Option<&std::path::Path>,
    save_feedback: Option<JsonSaveFeedback>,
    error_shake_step: u8,
    dirty: bool,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    accent: Color,
    accent_hover: Color,
    accent_pressed: Color,
    primary: Color,
    secondary: Color,
) {
    let zh = snapshot.language == LanguageId::SimplifiedChinese;
    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "JSON 配置" } else { "JSON configuration" },
        settings_page_title_rect(hwnd),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    for (action, zh_label, en_label) in [
        (JsonAction::Reload, "重新载入", "Reload"),
        (JsonAction::Format, "格式化", "Format"),
        (JsonAction::Import, "导入", "Import"),
        (JsonAction::Export, "导出", "Export"),
    ] {
        let target = HitTarget::Json(action);
        let action_background = button_background(
            target,
            false,
            hovered,
            pressed,
            ButtonPalette {
                normal: card,
                hover: card_hover,
                pressed: card_pressed,
                selected: card,
                selected_hover: card_hover,
                selected_pressed: card_pressed,
            },
        );
        draw_segment_with_icon(
            hdc,
            hwnd,
            json_action_rect(hwnd, action),
            false,
            action_background,
            primary,
            json_action_icon(action),
            if zh { zh_label } else { en_label },
        );
    }

    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let edit_rect = json_edit_rect(hwnd);
    let editor_background = json_editor_background(snapshot.is_dark);
    fill_rounded_rect(hdc, edit_rect, editor_background, scale(hwnd, 7));
    draw_rounded_outline_rect(
        hdc,
        edit_rect,
        if snapshot.is_dark {
            Color::from_hex("#59606AFF")
        } else {
            Color::from_hex("#AEB9C5FF")
        },
        scale(hwnd, 7),
        1,
    );
    paint_json_scrollbar_decoration(hdc, hwnd, snapshot.is_dark);

    let running = status.starts_with('◌');
    let success = !status.starts_with('×') && !running;
    let status_color = if running {
        Color::from_hex("#8FA8C7FF")
    } else if success {
        Color::from_hex("#55B879FF")
    } else {
        Color::from_hex("#D95C5CFF")
    };
    let _ = SetTextColor(hdc, COLORREF(status_color.to_colorref()));

    let status_top = edit_rect.bottom + scale(hwnd, 8);
    let prefix_width = if zh { scale(hwnd, 96) } else { scale(hwnd, 150) };

    // Invalid Apply uses a short horizontal shake whose amplitude decays
    // smoothly to zero. Font size stays stable to avoid layout jitter.
    let error_shake_x = if !success {
        json_error_shake_offset(hwnd, error_shake_step)
    } else {
        0
    };

    draw_text(
        hdc,
        status,
        RECT {
            left: scale(hwnd, 200) + error_shake_x,
            top: status_top,
            right: if status_path.is_some() {
                scale(hwnd, 200) + prefix_width + error_shake_x
            } else {
                client.right - scale(hwnd, 24) + error_shake_x
            },
            bottom: status_top + scale(hwnd, 26),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    if let Some(path) = status_path {
        let path_text = path.to_string_lossy();
        let path_left = scale(hwnd, 200) + prefix_width;
        let path_width = text_width_px(hwnd, &path_text).max(0);
        let path_rect = RECT {
            left: path_left,
            top: status_top,
            right: (path_left + path_width).min(client.right - scale(hwnd, 24)),
            bottom: status_top + scale(hwnd, 26),
        };
        let _ = SetTextColor(hdc, COLORREF(status_color.to_colorref()));
        draw_text(hdc, &path_text, path_rect, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS);
        // Underline the clickable path area.
        let y = path_rect.bottom - scale(hwnd, 4);
        let pen = CreatePen(PS_SOLID, 1, COLORREF(status_color.to_colorref()));
        let old_pen = SelectObject(hdc, pen);
        let _ = MoveToEx(hdc, path_rect.left, y, None);
        let _ = LineTo(hdc, path_rect.right, y);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(pen);
    }

    let second_line = match save_feedback {
        Some(JsonSaveFeedback::Saving) => Some((
            if zh { "◌ 保存中" } else { "◌ Saving" },
            Color::from_hex("#8FA8C7FF"),
        )),
        Some(JsonSaveFeedback::Saved) => Some((
            if zh { "✓ 保存成功" } else { "✓ Saved" },
            Color::from_hex("#55B879FF"),
        )),
        None if dirty => Some((
            if zh { "● 有未保存更改" } else { "● Unsaved changes" },
            Color::from_hex("#D69E2EFF"),
        )),
        None => None,
    };
    if let Some((text, color)) = second_line {
        let _ = SetTextColor(hdc, COLORREF(color.to_colorref()));
        draw_text(
            hdc,
            text,
            RECT {
                left: scale(hwnd, 200),
                top: status_top + scale(hwnd, 28),
                right: scale(hwnd, 630),
                bottom: status_top + scale(hwnd, 54),
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }

    let apply = JsonAction::Apply;
    let apply_background = if dirty {
        button_background(
            HitTarget::Json(apply),
            true,
            hovered,
            pressed,
            ButtonPalette {
                normal: accent,
                hover: accent_hover,
                pressed: accent_pressed,
                selected: accent,
                selected_hover: accent_hover,
                selected_pressed: accent_pressed,
            },
        )
    } else {
        card_pressed
    };
    draw_segment_with_icon(
        hdc,
        hwnd,
        json_action_rect(hwnd, apply),
        dirty,
        apply_background,
        if dirty {
            Color::from_hex("#FFFFFFFF")
        } else {
            secondary
        },
        json_action_icon(apply),
        if zh { "应用" } else { "Apply" },
    );
}


