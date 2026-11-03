//! Appearance settings page and desktop inset submenu painting.
//!
//! Preserve the original Theme/Layout/Position grouping, submenu reveal and
//! clipping behavior, draw order and all GDI/DC resource management. The parent
//! module continues to own state, message routing and frame scheduling.

use super::*;

pub(super) fn submenu_mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let c = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color::rgba(c(a.r,b.r),c(a.g,b.g),c(a.b,b.b),c(a.a,b.a))
}
pub(super) fn submenu_content_alpha(progress: f32) -> f32 {
    // Fade complete controls in as a group, rather than revealing clipped
    // fragments one after another. Reverse playback naturally fades them out.
    let p = ((progress - 0.08) / 0.82).clamp(0.0, 1.0);
    p * p * (3.0 - 2.0 * p)
}

/// The inset surface is independent of the selected display position.
/// Only the opacity of the controls changes while switching positions.
pub(super) unsafe fn paint_submenu_well(
    hdc: HDC, hwnd: HWND, _card: Color, dark: bool,
) -> Color {
    let surface = if dark {
        Color::from_hex("#22272DFF")
    } else {
        Color::from_hex("#DFE7EFFF")
    };
    let rim = if dark {
        Color::from_hex("#15191EFF")
    } else {
        Color::from_hex("#BFCCD9FF")
    };
    let bounds = scaled_rect(hwnd, desktop_submenu_layout().well);
    let radius = scale(hwnd, 11);
    if !native_interop::draw_antialiased_rounded_rect(
        hdc, bounds, radius as f32, Some(surface),
        Some((rim, scale(hwnd, 1).max(1) as f32)),
    ) {
        fill_rounded_rect(hdc, bounds, surface, radius);
        draw_rounded_outline_rect(hdc, bounds, rim, radius, 1);
    }
    surface
}

/// Connected single-choice group with optional submenu fade. All groups
/// share general_segment_rect so the painted boundaries and hit areas agree.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn draw_appearance_choice_group(
    hdc: HDC, hwnd: HWND, bounds: RECT,
    choices: &[(HitTarget, bool, SettingsIcon, &str)],
    hovered: Option<HitTarget>, pressed: Option<HitTarget>,
    palette: ButtonPalette, primary: Color, fade: Option<(Color, f32)>,
) {
    if choices.is_empty() { return; }
    let alpha = fade.map_or(1.0, |(_, a)| a.clamp(0.0, 1.0));
    if alpha <= 0.005 { return; }
    // The painted controls stay aligned with their hit rectangles.
    let tint = |color: Color| match fade {
        Some((inset, _)) => submenu_mix(inset, color, alpha),
        None => color,
    };
    let radius = scale(hwnd, 8);
    fill_rounded_rect(hdc, bounds, tint(palette.normal), radius);
    for (index, &(target, selected, icon, label)) in choices.iter().enumerate() {
        let option = general_segment_rect(bounds, index, choices.len());
        let desired = button_background(target, selected, hovered, pressed, palette);
        let background = tint(desired);
        if selected || hovered == Some(target) || pressed == Some(target) {
            fill_rounded_rect(hdc, option, background, radius);
            if index > 0 {
                fill(hdc, RECT { right: option.left + radius, ..option }, background);
            }
            if index + 1 < choices.len() {
                fill(hdc, RECT { left: option.right - radius, ..option }, background);
            }
        }
        let foreground = tint(if selected {
            Color::from_hex("#FFFFFFFF")
        } else { primary });
        let height = (option.bottom - option.top).max(1);
        let glyph_size = scale(hwnd, DESKTOP_SUBMENU_ICON_SIZE)
            .min((height - scale(hwnd, 8)).max(1));
        let gap = scale(hwnd, 4);
        let label_width = text_width_px(hwnd, label).max(0);
        let group_width = glyph_size + gap + label_width;
        let group_left = option.left + ((option.right - option.left - group_width) / 2)
            .max(scale(hwnd, 3));
        let glyph_top = option.top + (height - glyph_size) / 2;
        draw_settings_icon(
            hdc, hwnd, icon,
            RECT {
                left: group_left, top: glyph_top,
                right: group_left + glyph_size, bottom: glyph_top + glyph_size,
            },
            foreground, background,
        );
        let _ = SetTextColor(hdc, COLORREF(foreground.to_colorref()));
        draw_text(hdc, label, RECT {
            left: group_left + glyph_size + gap, top: option.top,
            right: option.right - scale(hwnd, 3), bottom: option.bottom,
        }, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    }
    let border = tint(subtle_control_border(palette.normal));
    for index in 1..choices.len() {
        let x = general_segment_rect(bounds, index, choices.len()).left;
        fill(hdc, RECT {
            left: x, right: x + scale(hwnd, 1),
            top: bounds.top + scale(hwnd, 5),
            bottom: bounds.bottom - scale(hwnd, 5),
        }, border);
    }
    draw_rounded_outline_rect(hdc, bounds, border, radius, 1);
}

/// The desktop collapse toggle shares the General page switch geometry and
/// colors; its Composition-style submenu reveal tints both track and knob.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_submenu_collapse_switch(
    hdc: HDC, hit: RECT, enabled: bool,
    hovered: Option<HitTarget>, pressed: Option<HitTarget>,
    alpha: f32, inset: Color, palette: ButtonPalette,
) {
    if alpha <= 0.005 { return; }
    let height = hit.bottom - hit.top;
    let track_height = (height * 11 / 16).max(12);
    let track_width = (track_height * 20 / 11).min(hit.right - hit.left);
    let track = RECT {
        left: hit.right - track_width,
        right: hit.right,
        top: hit.top + (height - track_height) / 2,
        bottom: hit.top + (height - track_height) / 2 + track_height,
    };
    let target = HitTarget::DesktopAutoCollapse(!enabled);
    let bg = submenu_mix(inset, button_background(target, enabled, hovered, pressed, palette), alpha);
    fill_rounded_rect(hdc, track, bg, track_height / 2);
    draw_rounded_outline_rect(
        hdc, track, submenu_mix(inset, subtle_control_border(bg), alpha),
        track_height / 2, 1,
    );
    let knob = (track_height - 4).max(8);
    let left = if enabled { track.right - knob - 2 } else { track.left + 2 };
    let top = track.top + (track_height - knob) / 2;
    fill_rounded_rect(
        hdc, RECT { left, top, right: left + knob, bottom: top + knob },
        submenu_mix(inset, Color::from_hex("#FFFFFFFF"), alpha), knob / 2,
    );
}
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_submenu_content(
    hdc: HDC, hwnd: HWND, snapshot: &StyleWindowSnapshot,
    progress: f32, inset: Color,
    hovered: Option<HitTarget>, pressed: Option<HitTarget>,
    palette: ButtonPalette, primary: Color, secondary: Color,
) {
    // The controls remain legible but disabled in taskbar mode; their tint
    // transitions smoothly without moving the well or click targets.
    let opacity = 0.45 + 0.55 * submenu_content_alpha(progress);
    let l = desktop_submenu_layout();
    let saved = SaveDC(hdc);
    let well = scaled_rect(hwnd, l.well);
    let _ = IntersectClipRect(hdc, well.left, well.top, well.right, well.bottom);
    let zh = snapshot.language == LanguageId::SimplifiedChinese;
    let title_color = if snapshot.is_dark {
        Color::from_hex("#98A3AEFF")
    } else {
        Color::from_hex("#7D8996FF")
    };
    let _ = SetTextColor(hdc, COLORREF(title_color.to_colorref()));
    draw_text(
        hdc, if zh { "桌面模式" } else { "Desktop mode" },
        scaled_rect(hwnd, l.title), DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    let _ = SetTextColor(hdc, COLORREF(submenu_mix(inset, secondary, opacity).to_colorref()));
    draw_text(
        hdc, if zh { "方向" } else { "Direction" },
        scaled_rect(hwnd, l.direction_label), DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc, if zh { "收缩" } else { "Collapse" },
        scaled_rect(hwnd, l.collapse_label), DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_appearance_choice_group(
        hdc, hwnd,
        scaled_rect(hwnd, RECT { right: l.vertical.right, ..l.horizontal }),
        &[
            (HitTarget::DesktopDirection(DesktopDirection::Horizontal),
                snapshot.desktop_direction == DesktopDirection::Horizontal,
                SettingsIcon::DirectionHorizontal, if zh { "横向" } else { "Horz" }),
            (HitTarget::DesktopDirection(DesktopDirection::Vertical),
                snapshot.desktop_direction == DesktopDirection::Vertical,
                SettingsIcon::DirectionVertical, if zh { "纵向" } else { "Vert" }),
        ],
        hovered, pressed, palette, primary, Some((inset, opacity)),
    );
    paint_submenu_collapse_switch(
        hdc, scaled_rect(hwnd, l.collapse_switch),
        snapshot.desktop_auto_collapse, hovered, pressed,
        opacity, inset, palette,
    );
    let _ = RestoreDC(hdc, saved);
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_appearance_page(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    section: Section,
    editor: EditorSelection,
    desktop_submenu_progress: f32,
    submenu_only: bool,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    focused_numeric_edit: Option<usize>,
    focused_blur_edit: bool,
    focused_corner_edit: bool,
    focused_hex_edit: Option<StyleColorTarget>,
    invalid_hex_edits: &[bool; HEX_EDIT_COUNT],
    card: Color,
    card_hover: Color,
    card_pressed: Color,
    track_background: Color,
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
        &appearance_page_title(section, snapshot.language),
        settings_page_title_rect(hwnd),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    fill_rounded_rect(
        hdc,
        settings_card_rect(hwnd, 58, 206),
        card,
        scale(hwnd, SETTINGS_CARD_RADIUS),
    );
    let submenu_inset = paint_submenu_well(hdc, hwnd, card, snapshot.is_dark);
    let _ = SetTextColor(hdc, COLORREF(secondary.to_colorref()));
    draw_text(
        hdc,
        if zh { "主题" } else { "Theme" },
        rect(hwnd, 218, 70, 300, 104),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if zh { "排版" } else { "Layout" },
        rect(hwnd, 218, 116, 300, 150),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );
    draw_text(
        hdc,
        if zh { "位置" } else { "Position" },
        rect(hwnd, 218, 162, 300, 196),
        DT_LEFT | DT_VCENTER | DT_SINGLELINE,
    );

    let choice_palette = ButtonPalette {
        normal: card_hover,
        hover: card_pressed,
        pressed: card_pressed,
        selected: accent,
        selected_hover: accent_hover,
        selected_pressed: accent_pressed,
    };
    draw_appearance_choice_group(
        hdc, hwnd, appearance_group_rect(hwnd, 70, 3),
        &[
            (HitTarget::Theme(ThemeMode::System), snapshot.theme_mode == ThemeMode::System,
                theme_icon(ThemeMode::System), if zh { "系统" } else { "System" }),
            (HitTarget::Theme(ThemeMode::Dark), snapshot.theme_mode == ThemeMode::Dark,
                theme_icon(ThemeMode::Dark), if zh { "深色" } else { "Dark" }),
            (HitTarget::Theme(ThemeMode::Light), snapshot.theme_mode == ThemeMode::Light,
                theme_icon(ThemeMode::Light), if zh { "浅色" } else { "Light" }),
        ],
        hovered, pressed, choice_palette, primary, None,
    );
    draw_appearance_choice_group(
        hdc, hwnd, appearance_group_rect(hwnd, 116, 2),
        &[
            (HitTarget::Layout(AppearancePreset::Default),
                snapshot.appearance_preset == AppearancePreset::Default,
                layout_icon(AppearancePreset::Default), if zh { "默认" } else { "Default" }),
            (HitTarget::Layout(AppearancePreset::Minimal),
                snapshot.appearance_preset == AppearancePreset::Minimal,
                layout_icon(AppearancePreset::Minimal), if zh { "极简" } else { "Minimal" }),
        ],
        hovered, pressed, choice_palette, primary, None,
    );
    draw_appearance_choice_group(
        hdc, hwnd, appearance_group_rect(hwnd, 162, 2),
        &[
            (HitTarget::Position(DisplayPosition::Taskbar),
                snapshot.display_position == DisplayPosition::Taskbar,
                SettingsIcon::PositionTaskbar, if zh { "任务栏" } else { "Taskbar" }),
            (HitTarget::Position(DisplayPosition::Desktop),
                snapshot.display_position == DisplayPosition::Desktop,
                SettingsIcon::PositionDesktop, if zh { "桌面" } else { "Desktop" }),
        ],
        hovered, pressed, choice_palette, primary, None,
    );

    paint_submenu_content(
        hdc, hwnd, snapshot, desktop_submenu_progress,
        submenu_inset, hovered, pressed,
        ButtonPalette {
            normal: card_hover, hover: card_pressed, pressed: card_pressed,
            selected: accent, selected_hover: accent_hover, selected_pressed: accent_pressed,
        }, primary, secondary,
    );

    if submenu_only { return; }

    if section == Section::Preset {
        paint_preset_gallery(
            hdc,
            hwnd,
            snapshot,
            hovered,
            pressed,
            PresetGalleryPalette {
                card,
                card_hover,
                card_pressed,
                border: track_background,
                accent,
                primary,
            },
        );
        return;
    }

    // The current section name belongs in the page heading. Drawing it at
    // y=180..212 would overlap the shared appearance card (y=58..206).
    for (index, row) in rows(section).iter().copied().enumerate() {
        let r = row_rect(hwnd, index);
        let selected = row == editor;
        let target = HitTarget::Row(row);
        fill_rounded_rect(
            hdc,
            r,
            button_background(
                target,
                selected,
                hovered,
                pressed,
                ButtonPalette {
                    normal: card,
                    hover: card_hover,
                    pressed: card_pressed,
                    selected: card_hover,
                    selected_hover: card_hover,
                    selected_pressed: card_pressed,
                },
            ),
            scale(hwnd, 8),
        );
        let _ = SetTextColor(hdc, COLORREF(primary.to_colorref()));
        draw_text(
            hdc,
            row_label(row, snapshot.language),
            RECT {
                left: r.left + scale(hwnd, 18),
                right: r.left + scale(hwnd, 280),
                ..r
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        match row {
            EditorSelection::CornerRadius => {}
            EditorSelection::Color(target) => {
                let color = snapshot.active_style.color(target);
                fill_rounded_rect(
                    hdc,
                    RECT {
                        left: r.right - scale(hwnd, 202),
                        top: r.top + scale(hwnd, 10),
                        right: r.right - scale(hwnd, 172),
                        bottom: r.bottom - scale(hwnd, 10),
                    },
                    color,
                    scale(hwnd, 4),
                );
            }
            EditorSelection::Blur | EditorSelection::TooltipBlur => {
                let value = if section == Section::Tooltip {
                    snapshot.active_style.tooltip_frosted_strength()
                } else {
                    snapshot.active_style.panel_frosted_strength
                };
                draw_slider(
                    hdc,
                    hwnd,
                    blur_slider_track_rect(hwnd, section),
                    value,
                    FROSTED_STRENGTH_MAX,
                    track_background,
                    accent,
                );
            }
        }
    }

    if matches!(section, Section::Panel | Section::Tooltip) {
        paint_frosted_alpha_hint(hdc, hwnd, snapshot.language, secondary);
    }

    paint_hex_edit_frames(
        hdc,
        hwnd,
        section,
        snapshot.is_dark,
        focused_hex_edit,
        invalid_hex_edits,
        EditFramePalette {
            border: track_background,
            accent,
        },
    );

    if matches!(section, Section::Panel | Section::Tooltip | Section::Progress) {
        paint_corner_edit_frame(
            hdc,
            hwnd,
            snapshot.is_dark,
            focused_corner_edit,
            track_background,
            accent,
            secondary,
        );
    }

    if matches!(section, Section::Panel | Section::Tooltip) {
        paint_blur_edit_frame(
            hdc,
            hwnd,
            section,
            snapshot.is_dark,
            focused_blur_edit,
            track_background,
            accent,
            secondary,
        );
    }

    if matches!(editor, EditorSelection::Color(_)) {
        fill_rounded_rect(
            hdc,
            editor_box_rect(hwnd, section),
            card,
            scale(hwnd, SETTINGS_CARD_RADIUS),
        );
        paint_numeric_edit_frames(
            hdc,
            hwnd,
            section,
            snapshot.is_dark,
            focused_numeric_edit,
            track_background,
            accent,
        );
        paint_editor(
            hdc,
            hwnd,
            section,
            snapshot,
            editor,
            EditorPalette {
                secondary,
                track_background,
                accent,
            },
        );
    }
}

