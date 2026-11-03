//! Preset-gallery cards, names and sample component thumbnails.
//!
//! Keep selected/hover/pressed palette selection, horizontal/vertical desktop
//! previews, color swatches and the existing single-card outline. The preview
//! content rectangle is a positioning guide, not a second painted frame.

use super::*;

pub(super) fn preset_label(preset: ThemePreset, is_dark: bool, language: LanguageId) -> &'static str {
    match (language == LanguageId::SimplifiedChinese, is_dark, preset) {
        (true, true, ThemePreset::Classic) => "石墨",
        (true, true, ThemePreset::Ocean) => "深海",
        (true, true, ThemePreset::Forest) => "松影",
        (false, true, ThemePreset::Classic) => "Graphite",
        (false, true, ThemePreset::Ocean) => "Deep Sea",
        (false, true, ThemePreset::Forest) => "Pine Shade",
        (true, false, ThemePreset::Classic) => "云瓷",
        (true, false, ThemePreset::Ocean) => "晴湾",
        (true, false, ThemePreset::Forest) => "麦光",
        (false, false, ThemePreset::Classic) => "Cloud Porcelain",
        (false, false, ThemePreset::Ocean) => "Clear Bay",
        (false, false, ThemePreset::Forest) => "Wheat Glow",
    }
}

pub(super) unsafe fn paint_preset_gallery(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    palette: PresetGalleryPalette,
) {
    let PresetGalleryPalette {
        card,
        card_hover,
        card_pressed,
        border,
        accent,
        primary,
    } = palette;
    // Preset is part of the page title ("Appearance · Presets").
    // The gallery begins just below the shared appearance controls.
    let mut matched = false;
    for preset in ThemePreset::ALL {
        let r = preset_card_rect_for_orientation(
            hwnd, preset, preview_vertical_for_snapshot(snapshot),
            snapshot.appearance_preset == AppearancePreset::Minimal,
        );
        let target = HitTarget::Preset(preset);
        let selected = snapshot
            .active_style
            .matches_preset(snapshot.is_dark, preset);
        matched |= selected;
        let surface = if pressed == Some(target) {
            card_pressed
        } else if hovered == Some(target) {
            card_hover
        } else {
            card
        };
        let style = ThemeStyle::preset(snapshot.is_dark, preset);
        paint_style_preview_card(
            hdc,
            hwnd,
            r,
            &style,
            preset_icon(preset, snapshot.is_dark),
            preset_label(preset, snapshot.is_dark, snapshot.language),
            selected,
            surface,
            border,
            accent,
            primary,
            preview_vertical_for_snapshot(snapshot),
            snapshot.appearance_preset == AppearancePreset::Minimal,
            snapshot.display_position == DisplayPosition::Desktop,
        );
    }

    if !matched {
        let zh = snapshot.language == LanguageId::SimplifiedChinese;
        paint_style_preview_card(
            hdc,
            hwnd,
            custom_preset_card_rect_for_orientation(
                hwnd, preview_vertical_for_snapshot(snapshot),
                snapshot.appearance_preset == AppearancePreset::Minimal,
            ),
            &snapshot.active_style,
            SettingsIcon::PresetCustom,
            if zh { "自定义" } else { "Custom" },
            true,
            card,
            border,
            accent,
            primary,
            preview_vertical_for_snapshot(snapshot),
            snapshot.appearance_preset == AppearancePreset::Minimal,
            snapshot.display_position == DisplayPosition::Desktop,
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_style_preview_card(
    hdc: HDC,
    hwnd: HWND,
    r: RECT,
    style: &ThemeStyle,
    icon: SettingsIcon,
    label: &str,
    selected: bool,
    surface: Color,
    border: Color,
    accent: Color,
    primary: Color,
    vertical: bool,
    minimal: bool,
    desktop: bool,
) {
    fill_rounded_rect(hdc, r, surface, scale(hwnd, SETTINGS_CARD_RADIUS));
    draw_rounded_outline_rect(
        hdc,
        r,
        if selected { accent } else { border },
        scale(hwnd, SETTINGS_CARD_RADIUS),
        if selected { 2 } else { 1 },
    );
    let title_icon_size = scale(hwnd, 20);
    let title_icon_rect = RECT {
        left: r.left + scale(hwnd, 12),
        top: r.top + scale(hwnd, 10),
        right: r.left + scale(hwnd, 12) + title_icon_size,
        bottom: r.top + scale(hwnd, 10) + title_icon_size,
    };
    draw_settings_icon(hdc, hwnd, icon, title_icon_rect, primary, surface);

    let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
    draw_text(
        hdc,
        label,
        RECT {
            left: r.left + scale(hwnd, 42),
            top: r.top + scale(hwnd, 8),
            right: r.right - scale(hwnd, 12),
            bottom: r.top + scale(hwnd, 38),
        },
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let preview = RECT {
        left: r.left + scale(hwnd, 12),
        top: r.top + scale(hwnd, if vertical { 50 } else { 46 }),
        right: r.right - scale(hwnd, 12),
        bottom: if vertical { r.bottom - scale(hwnd, 53) } else { r.top + scale(hwnd, 152) },
    };
    // The preview rectangle is only a positioning region. The card itself
    // provides the backdrop; drawing a second frame here would surround the
    // grip, panel and tooltip with an unwanted inset outline.

    if vertical {
        paint_vertical_style_preview(hdc, hwnd, preview, style, minimal, desktop);
    } else {
        paint_horizontal_style_preview(hdc, hwnd, preview, style, minimal, desktop);
    }
    for (index, target) in [
        StyleColorTarget::ProgressHigh,
        StyleColorTarget::ProgressMedium,
        StyleColorTarget::ProgressLow,
        StyleColorTarget::ProgressConsumed,
    ]
    .iter()
    .copied()
    .enumerate()
    {
        let (left, top, width) = if vertical {
            let swatch_width = 26;
            let swatch_gap = 10;
            let total_width = 4 * swatch_width + 3 * swatch_gap;
            (
                (r.left + r.right - scale(hwnd, total_width)) / 2
                    + scale(hwnd, index as i32 * (swatch_width + swatch_gap)),
                r.bottom - scale(hwnd, 34),
                swatch_width,
            )
        } else {
            // Preserve the original horizontal preset-card color strips.
            (r.left + scale(hwnd, 14 + index as i32 * 48),
             r.top + scale(hwnd, 158), 34)
        };
        fill_rounded_rect(
            hdc,
            RECT {
                left, top,
                right: left + scale(hwnd, width),
                bottom: top + scale(hwnd, 14),
            },
            style.color(target),
            scale(hwnd, 4),
        );
    }

    if selected {
        fill_rounded_rect(
            hdc,
            RECT {
                left: r.right - scale(hwnd, 24),
                top: r.top + scale(hwnd, 12),
                right: r.right - scale(hwnd, 12),
                bottom: r.top + scale(hwnd, 24),
            },
            accent,
            scale(hwnd, 3),
        );
    }
}

