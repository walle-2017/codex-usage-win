//! Reset-time tooltip text measurement and DPI-aware width caching.
//! The actual DirectWrite ink width determines the visible popup bounds.

use super::*;

// Mousemove repaints the tooltip often; do not re-rasterize the same text
// solely to measure its width on every cursor event.
static MINIMAL_TOOLTIP_WIDTH_CACHE: Mutex<Option<(String, u32, i32)>> = Mutex::new(None);

// Keep the reset-time tooltip content-sized. Its measured glyph bounds must
// come from the same DirectWrite mask used for the actual layered rendering,
// not the slightly different GDI font metrics. The final width needs only
// normal padding and a small rasterization safety allowance.
pub(super) const TOOLTIP_TEXT_INSET: i32 = 12;
const TOOLTIP_TEXT_MEASUREMENT_SLACK: i32 = 3;

pub(super) fn tooltip_width_with_render_slack(measured_text_px: i32, logical: impl Fn(i32) -> i32) -> i32 {
    (measured_text_px.max(0)
        + logical(2 * TOOLTIP_TEXT_INSET + TOOLTIP_TEXT_MEASUREMENT_SLACK))
        .max(logical(48))
}

// Extract the last painted column, including antialiased edge pixels. Using
// the rightmost pixel rather than the total nonzero pixel count also keeps
// space between digits and glyph overhangs from being cropped.
pub(super) fn tooltip_mask_content_width(mask: &[u8], row_width: usize) -> Option<i32> {
    if row_width == 0 || !mask.len().is_multiple_of(row_width) {
        return None;
    }
    mask.chunks_exact(row_width)
        .filter_map(|row| row.iter().rposition(|&coverage| coverage != 0))
        .max()
        .and_then(|x| i32::try_from(x + 1).ok())
}

unsafe fn measure_minimal_tooltip_width(text: &str) -> i32 {
    let hdc = GetDC(HWND::default());
    if hdc.0.is_null() {
        // Failed measurement must not fall back to a tiny fixed-width box.
        return tooltip_width_with_render_slack(
            sc((text.chars().count().min(64) as i32) * 12),
            sc,
        );
    }
    let font_name = native_interop::wide_str(fonts::face(fonts::FontRole::Ui, None));
    let font = CreateFontW(
        -sc(12),
        0,
        0,
        0,
        FW_NORMAL.0 as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_TT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        ANTIALIASED_QUALITY.0 as u32,
        (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
        PCWSTR::from_raw(font_name.as_ptr()),
    );
    let old_font = SelectObject(hdc, font);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide, &mut size);
    SelectObject(hdc, old_font);
    let _ = DeleteObject(font);
    let _ = ReleaseDC(HWND::default(), hdc);

    // Probe the real DirectWrite glyph mask with generous temporary room,
    // but do not carry that probe width into the final popup size. GDI only
    // provides a fallback and an approximate probe bound.
    let probe_width = size.cx.max(0)
        .saturating_add(sc(64))
        .max(sc((text.chars().count().min(1024) as i32) * 20))
        .clamp(sc(64), sc(4096));
    let content_width = native_interop::directwrite_text_mask(
        text,
        fonts::face(fonts::FontRole::Ui, None),
        12.0 * CURRENT_DPI.load(Ordering::Relaxed) as f32 / 96.0,
        FW_NORMAL.0 as i32,
        probe_width,
        sc(28),
    ).and_then(|mask| {
        tooltip_mask_content_width(&mask, probe_width as usize)
    });
    tooltip_width_with_render_slack(content_width.unwrap_or(size.cx), sc)
}

pub(super) fn cached_minimal_tooltip_width(text: &str) -> i32 {
    let dpi = CURRENT_DPI.load(Ordering::Relaxed);
    {
        let cache = MINIMAL_TOOLTIP_WIDTH_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((last_text, last_dpi, width)) = cache.as_ref() {
            if last_text == text && *last_dpi == dpi {
                return *width;
            }
        }
    }
    let width = unsafe { measure_minimal_tooltip_width(text) };
    *MINIMAL_TOOLTIP_WIDTH_CACHE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((text.to_owned(), dpi, width));
    width
}

