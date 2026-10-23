//! SVG-only settings glyphs. Source assets are embedded into the EXE; no file lookup
//! or SVG parsing happens on repeated paints for a cached size/color.
//! premultiplied BGRA pixels are passed to native AlphaBlend so both D2D-capable
//! and GDI-only Windows configurations display the same vector artwork.
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

use resvg::{tiny_skia, usvg};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::HDC;

use crate::native_interop::{self, Color};

const ICONS: [&str; 31] = [
    include_str!("../assets/settings-icons/general.svg"),
    include_str!("../assets/settings-icons/preset.svg"),
    include_str!("../assets/settings-icons/panel.svg"),
    include_str!("../assets/settings-icons/tooltip.svg"),
    include_str!("../assets/settings-icons/text.svg"),
    include_str!("../assets/settings-icons/progress.svg"),
    include_str!("../assets/settings-icons/interaction.svg"),
    include_str!("../assets/settings-icons/json.svg"),
    include_str!("../assets/settings-icons/theme-system.svg"),
    include_str!("../assets/settings-icons/theme-dark.svg"),
    include_str!("../assets/settings-icons/theme-light.svg"),
    include_str!("../assets/settings-icons/layout-default.svg"),
    include_str!("../assets/settings-icons/layout-minimal.svg"),
    include_str!("../assets/settings-icons/preset-classic.svg"),
    include_str!("../assets/settings-icons/preset-ocean.svg"),
    include_str!("../assets/settings-icons/preset-forest.svg"),
    include_str!("../assets/settings-icons/preset-custom.svg"),
    include_str!("../assets/settings-icons/preset-cloud.svg"),
    include_str!("../assets/settings-icons/preset-bay.svg"),
    include_str!("../assets/settings-icons/preset-wheat.svg"),
    include_str!("../assets/settings-icons/json-reload.svg"),
    include_str!("../assets/settings-icons/json-format.svg"),
    include_str!("../assets/settings-icons/json-import.svg"),
    include_str!("../assets/settings-icons/json-export.svg"),
    include_str!("../assets/settings-icons/json-apply.svg"),
    include_str!("../assets/settings-icons/direction-horizontal.svg"),
    include_str!("../assets/settings-icons/direction-vertical.svg"),
    include_str!("../assets/settings-icons/toggle-on.svg"),
    include_str!("../assets/settings-icons/toggle-off.svg"),
    include_str!("../assets/settings-icons/position-taskbar.svg"),
    include_str!("../assets/settings-icons/position-desktop.svg"),
];
const CACHE_CAPACITY: usize = 192;
type Key = (usize, u32, u32, u8, u8, u8, u8);

#[derive(Default)]
struct RasterCache {
    pixels: HashMap<Key, Vec<u8>>,
    fifo: VecDeque<Key>,
}
thread_local! {
    static CACHE: RefCell<RasterCache> = RefCell::new(RasterCache::default());
}

/// Rasterize the embedded SVG at the exact *physical* pixel dimensions.
/// tiny-skia uses premultiplied RGBA; Win32 AlphaBlend expects premultiplied BGRA.
fn rasterize(source: &str, width: u32, height: u32, color: Color) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width > 512 || height > 512 {
        return None;
    }
    let colored = source.replace(
        "#010203",
        &format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b),
    );
    let tree = usvg::Tree::from_str(&colored, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    let transform = tiny_skia::Transform::from_scale(width as f32 / 24.0, height as f32 / 24.0);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut bgra = Vec::with_capacity(width as usize * height as usize * 4);
    for pixel in pixmap.data().chunks_exact(4) {
        let premultiply_alpha = |component: u8| -> u8 {
            ((component as u16 * color.a as u16 + 127) / 255) as u8
        };
        bgra.extend_from_slice(&[
            premultiply_alpha(pixel[2]),
            premultiply_alpha(pixel[1]),
            premultiply_alpha(pixel[0]),
            premultiply_alpha(pixel[3]),
        ]);
    }
    Some(bgra)
}

pub fn draw(hdc: HDC, rect: RECT, icon_index: usize, color: Color) -> bool {
    let Some(source) = ICONS.get(icon_index) else { return false; };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if hdc.0.is_null() || width <= 0 || height <= 0 || width > 512 || height > 512 {
        return false;
    }
    let key: Key = (icon_index, width as u32, height as u32, color.r, color.g, color.b, color.a);
    CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if !cache.pixels.contains_key(&key) {
            let Some(bgra) = rasterize(source, width as u32, height as u32, color) else {
                return false;
            };
            if cache.fifo.len() >= CACHE_CAPACITY {
                if let Some(old) = cache.fifo.pop_front() {
                    cache.pixels.remove(&old);
                }
            }
            cache.fifo.push_back(key);
            cache.pixels.insert(key, bgra);
        }
        cache.pixels.get(&key).is_some_and(|pixels| {
            native_interop::blend_svg_icon(hdc, rect, pixels)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_31_svg_assets_parse_and_render_at_small_and_high_dpi() {
        assert_eq!(ICONS.len(), 31);
        for (id, source) in ICONS.iter().enumerate() {
            assert!(source.contains("viewBox=\"0 0 24 24\""), "bad viewBox: {id}");
            assert!(source.contains("#010203"), "missing theme foreground: {id}");
            for size in [18, 20, 22, 27, 36, 44] {
                let pixels = rasterize(source, size, size, Color::rgba(255, 255, 255, 255))
                    .unwrap_or_else(|| panic!("failed SVG render for icon {id} at {size}px"));
                assert_eq!(pixels.len(), size as usize * size as usize * 4);
                assert!(pixels.chunks_exact(4).any(|p| p[3] > 0), "empty icon: {id}");
            }
        }
    }

    #[test]
    fn transparency_and_theme_tint_are_respected() {
        let fg = Color::rgba(64, 128, 192, 128);
        let pixels = rasterize(ICONS[29], 20, 20, fg).unwrap();
        assert!(pixels.chunks_exact(4).any(|p| p[3] == 0));
        assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
        assert!(pixels.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]));
        assert!(pixels.chunks_exact(4).any(|p| p[3] >= 100 && p[3] <= 128));
    }
}
