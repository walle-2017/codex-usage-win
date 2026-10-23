# Settings SVG icon contract

All 31 settings icons are SVG resources in `assets/settings-icons/`. Their
index order matches `SettingsIcon` in `src/style_window.rs`, bound through
`src/svg_icons.rs`. No disk files are required at runtime.

- Canvas: `24 x 24`, `viewBox="0 0 24 24"`.
- Theme tint placeholder: `#010203`; every visible shape must use the
  placeholder, including fills, except transparent cutouts. The loader
  substitutes it with the current control foreground color.
- Use simple, bold silhouettes and round caps/joins. Avoid subpixel features
  that disappear at the 18 px / 100% DPI setting.
- Physical-pixel rasterization uses `resvg` + premultiplied BGRA and
  native `AlphaBlend`. The cache is bounded to 192 variants.
- Cache key includes index, physical size and complete RGBA foreground.
  Hover, selection, theme and submenu animation get correct per-frame colors.
- Run `cargo test`, `cargo clippy -- -D warnings`, and
  `cargo build --release` on Windows for every change.

This package intentionally does not keep the former Rust GDI and C++ Direct2D
icon geometry fallbacks: both paths now render the same SVG source.
