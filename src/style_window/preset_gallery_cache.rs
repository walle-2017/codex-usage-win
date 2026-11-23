//! Cached preset-gallery snapshots for taskbar and desktop layouts.
//! Keep GDI rasterization, deferred warmup and blit ordering identical.

use super::*;

pub(super) fn preset_gallery_cache_ready(
    hwnd: HWND, width: i32, height: i32, snapshot: &StyleWindowSnapshot,
) -> bool {
    let key = PresetGalleryKey::new(hwnd, width, height, snapshot);
    PRESET_GALLERY_FRAMES.with(|cell| {
        cell.borrow().as_ref().is_some_and(|frames| frames.key == key)
    })
}

pub(super) unsafe fn build_preset_gallery_cache(
    screen: HDC, hwnd: HWND, width: i32, height: i32,
    snapshot: &StyleWindowSnapshot, font: isize,
) -> bool {
    let key = PresetGalleryKey::new(hwnd, width, height, snapshot);
    PRESET_GALLERY_FRAMES.with(|cell| {
        let mut cache = cell.borrow_mut();
        if cache.as_ref().is_some_and(|frames| frames.key == key) {
            return true;
        }
        let Some(taskbar) = SettingsPaintBuffer::create(screen, width, height) else {
            return false;
        };
        let Some(desktop) = SettingsPaintBuffer::create(screen, width, height) else {
            return false;
        };
        let background = if snapshot.is_dark {
            Color::from_hex("#1F2125FF")
        } else {
            Color::from_hex("#E9EEF4FF")
        };
        let card = if snapshot.is_dark {
            Color::from_hex("#292C31FF")
        } else { Color::from_hex("#FFFFFFFF") };
        let hover = if snapshot.is_dark {
            Color::from_hex("#343840FF")
        } else { Color::from_hex("#DCE5EFFF") };
        let pressed = if snapshot.is_dark {
            Color::from_hex("#414751FF")
        } else { Color::from_hex("#CBD7E4FF") };
        let border = if snapshot.is_dark {
            Color::from_hex("#454A52FF")
        } else { Color::from_hex("#C1CCD8FF") };
        let primary = if snapshot.is_dark {
            Color::from_hex("#F2F3F5FF")
        } else { Color::from_hex("#202124FF") };
        let palette = || PresetGalleryPalette {
            card, card_hover: hover, card_pressed: pressed,
            border, accent: Color::from_hex("#4C8DFFFF"), primary,
        };
        for (frame, position) in [
            (&taskbar, DisplayPosition::Taskbar),
            (&desktop, DisplayPosition::Desktop),
        ] {
            let mut view = snapshot.clone();
            view.display_position = position;
            let region = preset_gallery_region(hwnd, width, height);
            fill(frame.dc, region, background);
            let previous = SelectObject(frame.dc, HGDIOBJ(font as *mut _));
            let _ = SetBkMode(frame.dc, TRANSPARENT);
            paint_preset_gallery(frame.dc, hwnd, &view, None, None, palette());
            SelectObject(frame.dc, previous);
        }
        *cache = Some(PresetGalleryFrames { key, taskbar, desktop });
        true
    })
}

pub(super) fn request_preset_gallery_warmup(hwnd: HWND) {
    let already_requested = SETTINGS_GALLERY_WARM_REQUESTED.with(|cell| {
        cell.replace(true)
    });
    if !already_requested
        && unsafe {
            PostMessageW(hwnd, WM_SETTINGS_PREWARM_GALLERY, WPARAM(0), LPARAM(0))
        }.is_err()
    {
        SETTINGS_GALLERY_WARM_REQUESTED.with(|cell| cell.set(false));
    }
}

pub(super) unsafe fn copy_cached_preset_gallery(
    target: HDC, hwnd: HWND, width: i32, height: i32,
    snapshot: &StyleWindowSnapshot, dirty: RECT,
) -> bool {
    let key = PresetGalleryKey::new(hwnd, width, height, snapshot);
    PRESET_GALLERY_FRAMES.with(|cell| {
        let cache = cell.borrow();
        let Some(frames) = cache.as_ref().filter(|frames| frames.key == key) else {
            return false;
        };
        let source = if snapshot.display_position == DisplayPosition::Desktop {
            &frames.desktop
        } else { &frames.taskbar };
        BitBlt(target, dirty.left, dirty.top,
            dirty.right - dirty.left, dirty.bottom - dirty.top,
            source.dc, dirty.left, dirty.top, SRCCOPY).is_ok()
    })
}
