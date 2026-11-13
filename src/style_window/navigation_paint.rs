//! Settings sidebar painting, section icons and localized navigation labels.
//!
//! Preserve the original item ordering, palette, icon inset, active indicator,
//! and appearance subpage titles. Hit testing stays in the parent module.

use super::*;

pub(super) fn navigation_text_inset(_section: Section) -> i32 {
    44
}

pub(super) fn navigation_icon(section: Section) -> SettingsIcon {
    match section {
        Section::General => SettingsIcon::General,
        Section::Preset => SettingsIcon::Preset,
        Section::Panel => SettingsIcon::Panel,
        Section::Tooltip => SettingsIcon::Tooltip,
        Section::Text => SettingsIcon::Text,
        Section::Progress => SettingsIcon::Progress,
        Section::Interaction => SettingsIcon::Interaction,
        Section::Json => SettingsIcon::Json,
    }
}


#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint_navigation(
    hdc: HDC,
    hwnd: HWND,
    snapshot: &StyleWindowSnapshot,
    section: Section,
    hovered: Option<HitTarget>,
    pressed: Option<HitTarget>,
    background: Color,
    hover: Color,
    pressed_color: Color,
    accent: Color,
    primary: Color,
    secondary: Color,
) {
    for item in [
        Section::General,
        Section::Preset,
        Section::Panel,
        Section::Tooltip,
        Section::Text,
        Section::Progress,
        Section::Interaction,
        Section::Json,
    ] {
        let selected = item == section;
        let r = section_rect(hwnd, item);
        let target = HitTarget::Section(item);
        let section_bg = button_background(
            target,
            selected,
            hovered,
            pressed,
            ButtonPalette {
                normal: background,
                hover,
                pressed: pressed_color,
                selected: hover,
                selected_hover: hover,
                selected_pressed: pressed_color,
            },
        );
        fill_rounded_rect(hdc, r, section_bg, scale(hwnd, 7));
        if selected {
            fill_rounded_rect(
                hdc,
                RECT {
                    right: r.left + scale(hwnd, 3),
                    ..r
                },
                accent,
                scale(hwnd, 2),
            );
        }
        let icon_color = if selected { accent } else { secondary };
        let icon_size = scale(hwnd, 20);
        let icon_rect = RECT {
            left: r.left + scale(hwnd, 14),
            top: r.top + ((r.bottom - r.top) - icon_size) / 2,
            right: r.left + scale(hwnd, 14) + icon_size,
            bottom: r.top + ((r.bottom - r.top) - icon_size) / 2 + icon_size,
        };
        draw_settings_icon(
            hdc,
            hwnd,
            navigation_icon(item),
            icon_rect,
            icon_color,
            section_bg,
        );

        let _ = SetTextColor(
            hdc,
            COLORREF(if selected { primary } else { secondary }.to_colorref()),
        );
        draw_text(
            hdc,
            section_label(item, snapshot.language),
            RECT {
                left: r.left + scale(hwnd, navigation_text_inset(item)),
                ..r
            },
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
    }
}

pub(super) fn appearance_page_title(section: Section, language: LanguageId) -> String {
    let root = if language == LanguageId::SimplifiedChinese {
        "外观"
    } else {
        "Appearance"
    };
    if matches!(
        section,
        Section::Preset | Section::Panel | Section::Tooltip | Section::Text
            | Section::Progress | Section::Interaction
    ) {
        format!("{root} · {}", section_label(section, language))
    } else {
        root.to_string()
    }
}

pub(super) fn section_label(section: Section, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match section {
        Section::General => {
            if zh { "常规" } else { "General" }
        }
        Section::Preset => {
            if zh {
                "预设"
            } else {
                "Presets"
            }
        }
        Section::Panel => {
            if zh {
                "面板"
            } else {
                "Panel"
            }
        }
        Section::Tooltip => {
            if zh { "浮框" } else { "Tooltip" }
        }
        Section::Text => {
            if zh {
                "文字"
            } else {
                "Text"
            }
        }
        Section::Progress => {
            if zh {
                "进度条"
            } else {
                "Progress"
            }
        }
        Section::Interaction => {
            if zh {
                "交互"
            } else {
                "Interaction"
            }
        }
        Section::Json => {
            if zh { "JSON 配置" } else { "JSON config" }
        }
    }
}

