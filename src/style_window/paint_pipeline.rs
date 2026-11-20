//! Buffered settings-window paint orchestration with retained gallery and card frames.
//! Backbuffer ownership remains in the parent, with BeginPaint/EndPaint unchanged.
use super::*;

pub(super) unsafe fn paint(hwnd: HWND) {
    let paint_cause = SETTINGS_PAINT_CAUSE.with(|cell| cell.replace(SettingsPaintCause::Unknown));
    let gallery_refresh = SETTINGS_GALLERY_REFRESH.with(|cell| cell.replace(false));
    let mut ps = PAINTSTRUCT::default();
    let screen_hdc = BeginPaint(hwnd, &mut ps);

    let mut paint_client = RECT::default();
    let _ = GetClientRect(hwnd, &mut paint_client);
    let width = (paint_client.right - paint_client.left).max(1);
    let height = (paint_client.bottom - paint_client.top).max(1);

    SETTINGS_PAINT_BUFFER.with(|cell| {
        let mut cached = cell.borrow_mut();
        if cached.as_ref().is_none_or(|backbuffer| {
            backbuffer.width != width || backbuffer.height != height
        }) {
            *cached = SettingsPaintBuffer::create(screen_hdc, width, height);
        }
        let Some(backbuffer) = cached.as_ref() else {
            let _ = EndPaint(hwnd, &ps);
            return;
        };
        let hdc = backbuffer.dc;

    let (
        snapshot,
        section,
        editor,
        hovered,
        pressed,
        focused_numeric_edit,
        focused_blur_edit,
        focused_corner_edit,
        focused_hex_edit,
        invalid_hex_edits,
        json_status,
        json_status_path,
        json_save_feedback,
        json_error_shake_step,
        json_dirty,
        json_import_pending_apply,
        discard_pending,
        font,
        desktop_submenu_progress,
        desktop_submenu_active,
        desktop_submenu_final_frame_pending,
    ) = {
        let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_ref() else {
            let _ = EndPaint(hwnd, &ps);
            return;
        };
        (
            s.snapshot.clone(),
            s.section,
            s.editor,
            s.hovered,
            s.pressed,
            s.focused_numeric_edit,
            s.focused_blur_edit,
            s.focused_corner_edit,
            s.focused_hex_edit,
            s.invalid_hex_edits,
            s.json_status.clone(),
            s.json_status_path.clone(),
            s.json_save_feedback,
            s.json_error_shake_step,
            s.json_dirty,
            s.json_import_pending_apply,
            s.pending_discard_action.is_some(),
            s.font,
            s.desktop_submenu.progress,
            s.desktop_submenu.active(),
            s.desktop_submenu.final_frame_pending,
        )
    };
    // WM_TIMER dirties exactly the shared top card. Keep all drawing outside
    // that card intact in the cached buffer, skipping expensive preset previews.
    let dirty = ps.rcPaint;
    let card_bounds = settings_card_rect(hwnd, 58, 206);
    let submenu_only = is_appearance_section(section)
        && dirty.left >= card_bounds.left && dirty.top >= card_bounds.top
        && dirty.right <= card_bounds.right && dirty.bottom <= card_bounds.bottom
        && dirty.left < dirty.right && dirty.top < dirty.bottom;

    let dark = snapshot.is_dark;
    let background = if dark {
        Color::from_hex("#1F2125FF")
    } else {
        Color::from_hex("#E9EEF4FF")
    };
    let sidebar = if dark {
        Color::from_hex("#191B1FFF")
    } else {
        Color::from_hex("#F5F7FAFF")
    };
    let card = if dark {
        Color::from_hex("#292C31FF")
    } else {
        Color::from_hex("#FFFFFFFF")
    };
    let card_hover = if dark {
        Color::from_hex("#343840FF")
    } else {
        Color::from_hex("#DCE5EFFF")
    };
    let card_pressed = if dark {
        Color::from_hex("#414751FF")
    } else {
        Color::from_hex("#CBD7E4FF")
    };
    let track_background = if dark {
        Color::from_hex("#454A52FF")
    } else {
        Color::from_hex("#C1CCD8FF")
    };
    let accent = Color::from_hex("#4C8DFFFF");
    let accent_hover = Color::from_hex("#629CFFFF");
    let accent_pressed = Color::from_hex("#3678E6FF");
    let primary = if dark {
        Color::from_hex("#F2F3F5FF")
    } else {
        Color::from_hex("#202124FF")
    };
    let secondary = if dark {
        Color::from_hex("#A8ADB5FF")
    } else {
        Color::from_hex("#666B73FF")
    };

    // Deferred Preset-gallery refresh must not trigger a full-page paint.
    // Both position variants were rasterized while the settings page was idle.
    let gallery_bounds = preset_gallery_region(hwnd, width, height);
    let gallery_only = section == Section::Preset
        && dirty.left >= gallery_bounds.left && dirty.top >= gallery_bounds.top
        && dirty.right <= gallery_bounds.right && dirty.bottom <= gallery_bounds.bottom
        && dirty.left < dirty.right && dirty.top < dirty.bottom;
    let preview_interacting = matches!(hovered, Some(HitTarget::Preset(_)))
        || matches!(pressed, Some(HitTarget::Preset(_)));
    if gallery_refresh && gallery_only && !desktop_submenu_active
        && !desktop_submenu_final_frame_pending && !discard_pending
        && !preview_interacting
        && copy_cached_preset_gallery(hdc, hwnd, width, height, &snapshot, dirty)
    {
        let _ = BitBlt(screen_hdc, dirty.left, dirty.top,
            dirty.right - dirty.left, dirty.bottom - dirty.top,
            hdc, dirty.left, dirty.top, SRCCOPY);
        let _ = EndPaint(hwnd, &ps);
        return;
    }

    // Keep the non-card pixels from the already painted backbuffer if Win32
    // merges a pure position-sync invalidation with a larger pending region.
    // The lower Preset gallery has its own deferred refresh.
    let coalesced_position = can_reuse_coalesced_position_paint(
        section, backbuffer.painted_section.get(), paint_cause, submenu_only,
        desktop_submenu_active, gallery_refresh,
    );
    // Animation normally touches only the shared card; both the exact card
    // invalidation and a safe coalesced position-only invalidation use blits.
    if (submenu_only || coalesced_position)
        && (desktop_submenu_active || desktop_submenu_final_frame_pending)
        && !discard_pending && composite_cached_submenu_card(
            screen_hdc, hdc, hwnd, width, height, &snapshot, section, editor,
            font, card, card_hover, card_pressed, track_background,
            accent, accent_hover, accent_pressed, primary, secondary,
            desktop_submenu_progress,
        )
    {
        let rect = ps.rcPaint;
        let _ = BitBlt(screen_hdc, rect.left, rect.top,
            rect.right - rect.left, rect.bottom - rect.top,
            hdc, rect.left, rect.top, SRCCOPY);
        let _ = EndPaint(hwnd, &ps);
        if desktop_submenu_final_frame_pending {
            finish_submenu_terminal_paint(hwnd);
        }
        return;
    }

    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let old_font = SelectObject(hdc, HGDIOBJ(font as *mut _));
    let _ = SetBkMode(hdc, TRANSPARENT);
    if !submenu_only {
    fill(hdc, client, background);
    fill(
        hdc,
        RECT {
            left: 0,
            top: 0,
            right: scale(hwnd, 180),
            bottom: client.bottom,
        },
        sidebar,
    );
    fill(
        hdc,
        RECT {
            left: scale(hwnd, 179),
            top: 0,
            right: scale(hwnd, 180),
            bottom: client.bottom,
        },
        track_background,
    );

    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "外观"
        } else {
            "Appearance"
        },
        rect(hwnd, 20, 92, 166, 120),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if snapshot.language == LanguageId::SimplifiedChinese {
            "高级"
        } else {
            "Advanced"
        },
        rect(hwnd, 20, 416, 166, 444),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    paint_navigation(
        hdc,
        hwnd,
        &snapshot,
        section,
        hovered,
        pressed,
        background,
        card_hover,
        card_pressed,
        accent,
        primary,
        secondary,
    );
    }

    match section {
        Section::General => paint_general_page(
            hdc,
            hwnd,
            &snapshot,
            hovered,
            pressed,
            card,
            card_hover,
            card_pressed,
            accent,
            accent_hover,
            accent_pressed,
            primary,
            secondary,
        ),
        Section::Json => paint_json_page(
            hdc,
            hwnd,
            &snapshot,
            &json_status,
            json_status_path.as_deref(),
            json_save_feedback,
            json_error_shake_step,
            json_dirty,
            json_import_pending_apply,
            hovered,
            pressed,
            card,
            card_hover,
            card_pressed,
            accent,
            accent_hover,
            accent_pressed,
            primary,
            secondary,
        ),
        _ => paint_appearance_page(
            hdc,
            hwnd,
            &snapshot,
            section,
            editor,
            desktop_submenu_progress,
            submenu_only,
            hovered,
            pressed,
            focused_numeric_edit,
            focused_blur_edit,
            focused_corner_edit,
            focused_hex_edit,
            &invalid_hex_edits,
            card,
            card_hover,
            card_pressed,
            track_background,
            accent,
            accent_hover,
            accent_pressed,
            primary,
            secondary,
        ),
    }

    if discard_pending {
        paint_discard_dialog(
            hdc,
            hwnd,
            &snapshot,
            hovered,
            pressed,
            background,
            card,
            card_hover,
            card_pressed,
            primary,
            secondary,
        );
    }

    SelectObject(hdc, old_font);
    // Only transfer the invalid rectangle, rather than the full window.
    // Full-frame backing pixels remain available for subsequent repaints.
    let dirty = ps.rcPaint;
    if dirty.right > dirty.left && dirty.bottom > dirty.top {
        let _ = BitBlt(screen_hdc, dirty.left, dirty.top,
            dirty.right - dirty.left, dirty.bottom - dirty.top,
            hdc, dirty.left, dirty.top, SRCCOPY);
    }
    // Prepare both endpoint images while the page is idle. The next click
    // can then start the animation without blocking on SVG/Direct2D rendering.
    // This runs after presenting the normal frame, so cache preparation only
    // touches the offscreen DC and cannot overwrite what the user sees.
    if !submenu_only && !desktop_submenu_active && !discard_pending &&
        !matches!(section, Section::General | Section::Json)
    {
        let cache_key = SubmenuCardKey::new(hwnd, width, height, &snapshot);
        let needs_warmup = SUBMENU_CARD_FRAMES.with(|cell| {
            cell.borrow().as_ref().is_none_or(|f| f.key != cache_key)
        });
        if needs_warmup {
            let _ = composite_cached_submenu_card(
                screen_hdc, hdc, hwnd, width, height, &snapshot, section, editor,
                font, card, card_hover, card_pressed, track_background,
                accent, accent_hover, accent_pressed, primary, secondary,
                desktop_submenu_progress,
            );
        }
    }
    // Prepare gallery variants via a deferred idle message. No costly
    // gallery rasterization is performed from the 60Hz animation paint path.
    if section == Section::Preset && !desktop_submenu_active
        && !desktop_submenu_final_frame_pending && !discard_pending
        && !preset_gallery_cache_ready(hwnd, width, height, &snapshot)
    {
        request_preset_gallery_warmup(hwnd);
    }
    // Only a full draw establishes valid retained pixels for this page;
    // coalesced partial paint reuse requires this exact painted section.
    if !submenu_only {
        backbuffer.painted_section.set(Some(section));
    }
    let _ = EndPaint(hwnd, &ps);
    if desktop_submenu_final_frame_pending {
        finish_submenu_terminal_paint(hwnd);
    }
    });
}
