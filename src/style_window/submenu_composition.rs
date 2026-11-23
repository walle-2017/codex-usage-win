//! Cached endpoint painting for the desktop-position submenu animation.
//! Preserve GDI backing frames, opacity blending and terminal repaint ordering.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_submenu_endpoint(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    section: Section,
    editor: EditorSelection,
    font: isize,
    card: Color, hover: Color, pressed_color: Color, track: Color,
    accent: Color, accent_hover: Color, accent_pressed: Color,
    primary: Color, secondary: Color,
) {
    let underlay = if snapshot.is_dark {
        Color::from_hex("#1F2125FF")
    } else {
        Color::from_hex("#E9EEF4FF")
    };
    fill(hdc, settings_card_rect(hwnd, 58, 206), underlay);
    let prior_font = SelectObject(hdc, HGDIOBJ(font as *mut _));
    let _ = SetBkMode(hdc, TRANSPARENT);
    // Hover and pressed effects are intentionally excluded from the cached
    // animation; normal painting immediately restores live hover feedback.
    paint_appearance_page(
        hdc, hwnd, snapshot, section, editor,
        if snapshot.display_position == DisplayPosition::Desktop { 1.0 } else { 0.0 },
        true, None, None, None, false, false, None, &[false; HEX_EDIT_COUNT],
        card, hover, pressed_color, track, accent, accent_hover,
        accent_pressed, primary, secondary,
    );
    SelectObject(hdc, prior_font);
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn composite_cached_submenu_card(
    screen: HDC, dest: HDC, hwnd: HWND, width: i32, height: i32,
    snapshot: &StyleWindowSnapshot, section: Section, editor: EditorSelection,
    font: isize, card: Color, hover: Color, pressed_color: Color,
    track: Color, accent: Color, accent_hover: Color, accent_pressed: Color,
    primary: Color, secondary: Color, progress: f32,
) -> bool {
    let key = SubmenuCardKey::new(hwnd, width, height, snapshot);
    SUBMENU_CARD_FRAMES.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.as_ref().is_none_or(|frames| frames.key != key) {
            let Some(closed) = SettingsPaintBuffer::create(screen, width, height) else {
                return false;
            };
            let Some(open) = SettingsPaintBuffer::create(screen, width, height) else {
                return false;
            };
            let mut closed_snapshot = snapshot.clone();
            closed_snapshot.display_position = DisplayPosition::Taskbar;
            let mut open_snapshot = snapshot.clone();
            open_snapshot.display_position = DisplayPosition::Desktop;
            paint_submenu_endpoint(closed.dc, hwnd, &closed_snapshot, section, editor,
                font, card, hover, pressed_color, track, accent, accent_hover,
                accent_pressed, primary, secondary);
            paint_submenu_endpoint(open.dc, hwnd, &open_snapshot, section, editor,
                font, card, hover, pressed_color, track, accent, accent_hover,
                accent_pressed, primary, secondary);
            *cache = Some(SubmenuCardFrames { key, closed, open });
        }
        let Some(frames) = cache.as_ref() else { return false };
        let card_bounds = settings_card_rect(hwnd, 58, 206);
        let w = card_bounds.right - card_bounds.left;
        let h = card_bounds.bottom - card_bounds.top;
        let copied = BitBlt(dest, card_bounds.left, card_bounds.top, w, h,
            frames.closed.dc, card_bounds.left, card_bounds.top, SRCCOPY);
        if copied.is_err() { return false; }
        let opacity = (progress.clamp(0.0, 1.0) * 255.0).round() as u8;
        opacity == 0 || native_interop::alpha_blend_cached_card(
            dest, frames.open.dc, card_bounds, opacity)
    })
}

pub(super) fn finish_submenu_terminal_paint(hwnd: HWND) {
    let needs_gallery = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = state.as_mut() else { return };
        s.desktop_submenu.final_frame_pending = false;
        s.section == Section::Preset && s.preset_gallery_refresh_pending
    };
    if needs_gallery {
        unsafe {
            let _ = PostMessageW(
                hwnd, WM_SETTINGS_PRESET_GALLERY_REFRESH, WPARAM(0), LPARAM(0),
            );
        }
    }
}

// A position-only sync can have its card rect coalesced with a prior
// invalid region by Win32. The rest of the retained backing bitmap is still
// correct for this section; avoid a full page redraw solely because rcPaint
// extends beyond the animated card. Never reuse uninitialized or other-page
// pixels, or a genuine gallery/other-settings refresh.
pub(super) fn can_reuse_coalesced_position_paint(
    section: Section,
    painted_section: Option<Section>,
    cause: SettingsPaintCause,
    submenu_only: bool,
    animating: bool,
    gallery_refresh: bool,
) -> bool {
    !submenu_only
        && animating
        && !gallery_refresh
        && is_appearance_section(section)
        && painted_section == Some(section)
        && cause == SettingsPaintCause::PositionSync
}
