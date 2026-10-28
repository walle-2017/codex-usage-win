//! SVG-only settings glyphs. Source assets are embedded into the EXE; no file lookup
//! or SVG parsing happens when only the animated icon tint changes.
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
    include_str!("../assets/settings-icons/preset-graphite.svg"),
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
// Cache the antialiased coverage mask by icon and physical dimensions only.
// Animating foreground colors must never trigger SVG parsing or rasterization.
type Key = (usize, u32, u32);

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

// Tint the cached white SVG raster in linear byte arithmetic. The alpha
// channel carries the coverage and SVG-local opacity. All output channels
// remain premultiplied as required by AlphaBlend.
fn tint_cached_icon(white_bgra: &[u8], color: Color) -> Vec<u8> {
    let mut output = Vec::with_capacity(white_bgra.len());
    for pixel in white_bgra.chunks_exact(4) {
        let alpha = (u16::from(pixel[3]) * u16::from(color.a) + 127) / 255;
        output.push(((alpha * u16::from(color.b) + 127) / 255) as u8);
        output.push(((alpha * u16::from(color.g) + 127) / 255) as u8);
        output.push(((alpha * u16::from(color.r) + 127) / 255) as u8);
        output.push(alpha as u8);
    }
    output
}

pub fn draw(hdc: HDC, rect: RECT, icon_index: usize, color: Color) -> bool {
    let Some(source) = ICONS.get(icon_index) else { return false; };
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if hdc.0.is_null() || width <= 0 || height <= 0 || width > 512 || height > 512 {
        return false;
    }
    let key: Key = (icon_index, width as u32, height as u32);
    CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        if !cache.pixels.contains_key(&key) {
            let Some(white) = rasterize(
                source, width as u32, height as u32, Color::rgba(255, 255, 255, 255)
            ) else {
                return false;
            };
            if cache.fifo.len() >= CACHE_CAPACITY {
                if let Some(old) = cache.fifo.pop_front() {
                    cache.pixels.remove(&old);
                }
            }
            cache.fifo.push_back(key);
            cache.pixels.insert(key, white);
        }
        cache.pixels.get(&key).is_some_and(|white| {
            let tinted = tint_cached_icon(white, color);
            native_interop::blend_svg_icon(hdc, rect, &tinted)
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
    fn cached_icon_tint_preserves_coverage_and_premultiplied_alpha() {
        let source = include_str!("../assets/settings-icons/direction-horizontal.svg");
        let white = rasterize(source, 20, 20, Color::rgba(255, 255, 255, 255)).unwrap();
        let colored = tint_cached_icon(&white, Color::rgba(88, 160, 230, 128));
        assert_eq!(colored.len(), white.len());
        for (w, c) in white.chunks_exact(4).zip(colored.chunks_exact(4)) {
            assert_eq!(u16::from(c[3]), (u16::from(w[3]) * 128 + 127) / 255);
            assert!(c[0] <= c[3] && c[1] <= c[3] && c[2] <= c[3]);
        }
        assert!(colored.chunks_exact(4).any(|c| c[3] > 0));
    }

    #[test]
    fn cached_color_raster_key_ignores_animated_tint() {
        let light: Key = (25, 18, 18);
        let dark: Key = (25, 18, 18);
        assert_eq!(light, dark, "animation color is not part of the SVG cache key");
    }

    #[test]
    fn graphite_preset_uses_named_svg_asset() {
        assert_eq!(ICONS[13], include_str!("../assets/settings-icons/preset-graphite.svg"));
    }

    #[test]
    fn preset_cloud_stays_inside_viewbox_at_button_and_dpi_sizes() {
        // Guard against truncated cloud curls at the SVG's right boundary.
        let cloud = include_str!("../assets/settings-icons/preset-cloud.svg");
        for size in [18u32, 20, 24, 36, 48, 96] {
            let pixels = rasterize(cloud, size, size, Color::rgba(255, 255, 255, 255))
                .expect("cloud SVG should render at every control size");
            let stride = size as usize;
            let alpha = |x: usize, y: usize| pixels[(y * stride + x) * 4 + 3];
            assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] > 0),
                "cloud should be visible at {size}px");
            for position in 0..stride {
                assert_eq!(alpha(position, 0), 0, "cloud clipped at top ({size}px)");
                assert_eq!(alpha(position, stride - 1), 0,
                    "cloud clipped at bottom ({size}px)");
                assert_eq!(alpha(0, position), 0, "cloud clipped at left ({size}px)");
                assert_eq!(alpha(stride - 1, position), 0,
                    "cloud clipped at right ({size}px)");
            }
        }
    }

    #[test]
    fn theme_system_gear_is_centered_inside_monitor_without_edge_clipping() {
        let source = include_str!("../assets/settings-icons/theme-system.svg");
        assert!(source.contains("fill-rule=\"evenodd\""));
        for size in [18u32, 20, 22, 24, 36, 48, 96] {
            let pixels = rasterize(source, size, size, Color::rgba(255, 255, 255, 255))
                .expect("system theme icon must rasterize");
            let stride = size as usize;
            let alpha = |x: usize, y: usize| pixels[(y * stride + x) * 4 + 3];
            for offset in 0..stride {
                assert_eq!(alpha(offset, 0), 0, "top clipped at {size}px");
                assert_eq!(alpha(offset, stride - 1), 0, "bottom clipped at {size}px");
                assert_eq!(alpha(0, offset), 0, "left clipped at {size}px");
                assert_eq!(alpha(stride - 1, offset), 0, "right clipped at {size}px");
            }
        }
        let pixels = rasterize(source, 96, 96, Color::rgba(255, 255, 255, 255))
            .expect("system theme icon must render at high DPI");
        let alpha = |x: usize, y: usize| pixels[(y * 96 + x) * 4 + 3];
        assert_eq!(alpha(48, 40), 0, "center of gear must remain transparent");
        assert!(alpha(58, 40) > 200, "gear must be centered and visible in monitor");
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
