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
    import_pending_apply: bool,
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
        let disabled_export = action == JsonAction::Export
            && !json_action_enabled(action, dirty, import_pending_apply);
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
            if disabled_export { card_pressed } else { action_background },
            if disabled_export { secondary } else { primary },
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
        None if import_pending_apply => Some((
            if zh { "● 已导入，待应用" } else { "● Imported, pending Apply" },
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
    let apply_enabled = json_action_enabled(apply, dirty, import_pending_apply);
    let apply_background = if apply_enabled {
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
        apply_enabled,
        apply_background,
        if apply_enabled {
            Color::from_hex("#FFFFFFFF")
        } else {
            secondary
        },
        json_action_icon(apply),
        if zh { "应用" } else { "Apply" },
    );
}

// Unsaved JSON confirmation overlay. Its hit-test rectangles and action
// routing remain owned by the parent settings window.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_discard_dialog(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    background: Color,
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    primary: Color,
    secondary: Color,
) {
    let zh = snapshot.language == LanguageId::SimplifiedChinese;
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let veil = if snapshot.is_dark {
        Color::from_hex("#17191DFF")
    } else {
        Color::from_hex("#DCE3EBFF")
    };
    fill(hdc, client, veil);

    let dialog = discard_dialog_rect(hwnd);
    fill_rounded_rect(hdc, dialog, card, scale(hwnd, 12));
    draw_rounded_outline_rect(
        hdc,
        dialog,
        if snapshot.is_dark {
            Color::from_hex("#4B5360FF")
        } else {
            Color::from_hex("#B8C3CFFF")
        },
        scale(hwnd, 12),
        1,
    );

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        if zh { "未保存的更改" } else { "Unsaved changes" },
        RECT {
            left: dialog.left + scale(hwnd, 24),
            top: dialog.top + scale(hwnd, 20),
            right: dialog.right - scale(hwnd, 24),
            bottom: dialog.top + scale(hwnd, 52),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if zh {
            "当前 JSON 配置尚未保存。继续操作将丢失这些更改。"
        } else {
            "The current JSON configuration has not been saved. Continuing will discard these changes."
        },
        RECT {
            left: dialog.left + scale(hwnd, 24),
            top: dialog.top + scale(hwnd, 58),
            right: dialog.right - scale(hwnd, 24),
            bottom: dialog.top + scale(hwnd, 112),
        },
        DT_LEFT | DT_VCENTER | DT_WORDBREAK,
    );

    let discard_normal = if snapshot.is_dark {
        Color::from_hex("#8F5054FF")
    } else {
        Color::from_hex("#B65F63FF")
    };
    let discard_hover = if snapshot.is_dark {
        Color::from_hex("#A25A5EFF")
    } else {
        Color::from_hex("#C56A6EFF")
    };
    let discard_pressed = if snapshot.is_dark {
        Color::from_hex("#7E464AFF")
    } else {
        Color::from_hex("#A95357FF")
    };
    draw_segment(
        hdc,
        discard_dialog_button_rect(hwnd, true),
        false,
        button_background(
            HitTarget::DiscardChanges,
            false,
            hovered,
            pressed,
            ButtonPalette {
                normal: discard_normal,
                hover: discard_hover,
                pressed: discard_pressed,
                selected: discard_normal,
                selected_hover: discard_hover,
                selected_pressed: discard_pressed,
            },
        ),
        Color::from_hex("#FFF8F8FF"),
        if zh { "丢弃更改" } else { "Discard changes" },
    );
    draw_segment(
        hdc,
        discard_dialog_button_rect(hwnd, false),
        false,
        button_background(
            HitTarget::KeepEditing,
            false,
            hovered,
            pressed,
            ButtonPalette {
                normal: card_hover,
                hover: card_pressed,
                pressed: card_pressed,
                selected: card_hover,
                selected_hover: card_pressed,
                selected_pressed: card_pressed,
            },
        ),
        primary,
        if zh { "返回编辑" } else { "Return to editing" },
    );
    let _ = background;
}

// Render an entire scrollbar frame offscreen, then publish it in one blit.
// This is also directly exercised by the Windows UI test; HWND invalidation
// can be coalesced with a whole-page repaint, hiding the narrow paint path.
pub(super) unsafe fn paint_json_scrollbar_buffered(
    screen: HDC, owner: HWND, is_dark: bool, origin_x: i32, track: RECT,
) -> bool {
    JSON_SCROLLBAR_PAINT_BUFFER.with(|cell| {
        let width = (track.right - track.left).max(1);
        let height = (track.bottom - track.top).max(1);
        let mut slot = cell.borrow_mut();
        if slot.as_ref().is_none_or(|buffer| {
            buffer.width != width || buffer.height != height
        }) {
            *slot = SettingsPaintBuffer::create(screen, width, height);
        }
        let Some(buffer) = slot.as_ref() else { return false; };
        let _ = SetViewportOrgEx(buffer.dc, -track.left, -track.top, None);
        fill(buffer.dc, track, json_editor_background(is_dark));
        paint_json_scrollbar_decoration(buffer.dc, owner, is_dark);
        let _ = SetViewportOrgEx(buffer.dc, 0, 0, None);
        let _ = BitBlt(
            screen, track.left - origin_x, track.top,
            width, height, buffer.dc, 0, 0, SRCCOPY,
        );
        true
    })
}

pub(super) unsafe fn paint_json_page_surface(page: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let screen = BeginPaint(page, &mut ps);
    // A scrollbar-only invalidation is far narrower than the JSON pane.
    // Paint the track directly on the child HDC instead of allocating a
    // fullscreen bitmap and redrawing the toolbar and status on every scroll.
    let owner_state = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        state.as_ref().map(|s| (s.hwnd.to_hwnd(), s.snapshot.is_dark))
    };
    if let Some((owner, is_dark)) = owner_state {
        let origin_x = scale(owner, 180);
        let track = json_scrollbar_track_rect(owner);
        let dirty = ps.rcPaint;
        let scrollbar_only = dirty.left >= track.left - origin_x
            && dirty.right <= track.right - origin_x
            && dirty.top >= track.top && dirty.bottom <= track.bottom
            && dirty.left < dirty.right && dirty.top < dirty.bottom;
        if scrollbar_only {
            // Do not erase the visible track before drawing its moving thumb.
            // Without WS_EX_COMPOSITED the direct-to-window erase/draw steps
            // become separate visible frames, making the scrollbar blink
            // during continuous dragging. Compose the complete track in a
            // tiny retained bitmap and publish it with one clipped BitBlt.
            let buffered = paint_json_scrollbar_buffered(
                screen, owner, is_dark, origin_x, track,
            );
            if !buffered {
                // Low-memory fallback: retain the existing paint behavior
                // instead of leaving a missing scrollbar.
                let _ = SetViewportOrgEx(screen, -origin_x, 0, None);
                fill(screen, track, json_editor_background(is_dark));
                paint_json_scrollbar_decoration(screen, owner, is_dark);
                let _ = SetViewportOrgEx(screen, 0, 0, None);
            }
            let _ = EndPaint(page, &ps);
            return;
        }
    }
    let mut client = RECT::default();
    let _ = GetClientRect(page, &mut client);
    let width = (client.right - client.left).max(1);
    let height = (client.bottom - client.top).max(1);
    let dc = CreateCompatibleDC(screen);
    if dc.0.is_null() {
        let _ = EndPaint(page, &ps);
        return;
    }
    let bitmap = CreateCompatibleBitmap(screen, width, height);
    if bitmap.0.is_null() {
        let _ = DeleteDC(dc);
        let _ = EndPaint(page, &ps);
        return;
    }
    let previous_bitmap = SelectObject(dc, HGDIOBJ(bitmap.0));
    let state_snapshot = {
        let guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().map(|s| (
            s.hwnd.to_hwnd(), s.snapshot.clone(),
            s.json_status.clone(), s.json_status_path.clone(),
            s.json_save_feedback, s.json_error_shake_step, s.json_dirty,
            s.hovered, s.pressed, s.font,
        ))
    };
    if let Some((
        owner, snapshot, status, status_path, feedback, shake_step,
        dirty, hovered, pressed, font,
    )) = state_snapshot {
        let left = scale(owner, 180);
        // Reuse JSON painting geometry, which is expressed in the parent's
        // client coordinates. Only the viewport origin changes here.
        let _ = SetViewportOrgEx(dc, -left, 0, None);
        let background = if snapshot.is_dark {
            Color::from_hex("#1F2125FF")
        } else {
            Color::from_hex("#E9EEF4FF")
        };
        fill(dc, RECT { left, top: 0, right: left + width, bottom: height }, background);
        let old_font = SelectObject(dc, HGDIOBJ(font as *mut _));
        let _ = SetBkMode(dc, TRANSPARENT);
        let primary = if snapshot.is_dark {
            Color::from_hex("#F2F3F5FF")
        } else {
            Color::from_hex("#202124FF")
        };
        let secondary = if snapshot.is_dark {
            Color::from_hex("#A8ADB5FF")
        } else {
            Color::from_hex("#666B73FF")
        };
        let card = if snapshot.is_dark {
            Color::from_hex("#292C31FF")
        } else {
            Color::from_hex("#FFFFFFFF")
        };
        let card_hover = if snapshot.is_dark {
            Color::from_hex("#343840FF")
        } else {
            Color::from_hex("#DCE5EFFF")
        };
        let card_pressed = if snapshot.is_dark {
            Color::from_hex("#414751FF")
        } else {
            Color::from_hex("#CBD7E4FF")
        };
        paint_json_page(
            dc, owner, &snapshot, &status, status_path.as_deref(),
            feedback, shake_step, dirty, hovered, pressed,
            card, card_hover, card_pressed,
            Color::from_hex("#4C8DFFFF"),
            Color::from_hex("#629CFFFF"),
            Color::from_hex("#3678E6FF"),
            primary, secondary,
        );
        SelectObject(dc, old_font);
        let _ = SetViewportOrgEx(dc, 0, 0, None);
    }
    let _ = BitBlt(screen, 0, 0, width, height, dc, 0, 0, SRCCOPY);
    SelectObject(dc, previous_bitmap);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(dc);
    let _ = EndPaint(page, &ps);
}

