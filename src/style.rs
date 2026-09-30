use serde::{Deserialize, Serialize};

use crate::appearance::AppearancePreset;
use crate::native_interop::Color;

pub const FROSTED_STRENGTH_MAX: u8 = 100;
pub const TOOLTIP_HEIGHT_LOGICAL: i32 = 28;

pub fn panel_corner_radius_max(preset: AppearancePreset) -> u8 {
    (preset.metrics().widget_height.max(0) / 2)
        .min(i32::from(u8::MAX)) as u8
}

pub fn tooltip_corner_radius_max() -> u8 {
    (TOOLTIP_HEIGHT_LOGICAL / 2) as u8
}

pub fn progress_corner_radius_max(preset: AppearancePreset) -> u8 {
    (preset.metrics().bar_height.max(0) / 2)
        .min(i32::from(u8::MAX)) as u8
}
const LEGACY_ROUNDED_RADIUS: u8 = 8;
const LEGACY_FROSTED_STRENGTH_SENTINEL: u8 = u8::MAX;

#[derive(Deserialize)]
#[serde(untagged)]
enum CornerRadiusValue {
    Radius(u8),
    LegacyRounded(bool),
}

fn deserialize_corner_radius<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match CornerRadiusValue::deserialize(deserializer)? {
        CornerRadiusValue::Radius(value) => value,
        CornerRadiusValue::LegacyRounded(true) => LEGACY_ROUNDED_RADIUS,
        CornerRadiusValue::LegacyRounded(false) => 0,
    })
}

fn missing_frosted_strength() -> u8 {
    LEGACY_FROSTED_STRENGTH_SENTINEL
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemePreset {
    Classic,
    Ocean,
    Forest,
}

impl ThemePreset {
    pub const ALL: [Self; 3] = [Self::Classic, Self::Ocean, Self::Forest];

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Classic),
            1 => Some(Self::Ocean),
            2 => Some(Self::Forest),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleColorTarget {
    PanelBackground,
    PanelBorder,
    TooltipBackground,
    TooltipBorder,
    QuotaType,
    Remaining,
    ResetTime,
    Error,
    ProgressHigh,
    ProgressMedium,
    ProgressLow,
    ProgressConsumed,
    DragHandle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeStyle {
    pub panel_background: String,
    pub panel_border: String,
    /// Legacy v1.0.5 test-build field. Read for migration only and omit on save.
    #[serde(default, skip_serializing)]
    pub panel_blur_radius: u8,
    /// Visual Acrylic intensity: 0 = off, 1..=100 = increasingly frosted.
    #[serde(default = "missing_frosted_strength")]
    pub panel_frosted_strength: u8,
    #[serde(default, alias = "panel_rounded", deserialize_with = "deserialize_corner_radius")]
    pub panel_corner_radius: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip_background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip_border: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip_frosted_strength: Option<u8>,
    #[serde(default, alias = "tooltip_rounded", deserialize_with = "deserialize_corner_radius")]
    pub tooltip_corner_radius: u8,
    pub quota_type: String,
    pub remaining: String,
    pub reset_time: String,
    pub error: String,
    pub progress_high: String,
    pub progress_medium: String,
    pub progress_low: String,
    pub progress_consumed: String,
    #[serde(default, alias = "progress_rounded", deserialize_with = "deserialize_corner_radius")]
    pub progress_corner_radius: u8,
    pub drag_handle: String,
}

impl Default for ThemeStyle {
    fn default() -> Self {
        Self::dark_default()
    }
}

impl ThemeStyle {
    pub fn dark_default() -> Self {
        Self {
            panel_background: "#242A31FF".into(),
            panel_border: "#343B43FF".into(),
            panel_blur_radius: 0,
            panel_frosted_strength: 0,
            panel_corner_radius: 2,
            tooltip_background: Some("#242A31FF".into()),
            tooltip_border: Some("#343B43FF".into()),
            tooltip_frosted_strength: Some(0),
            tooltip_corner_radius: 2,
            quota_type: "#A0A0A0FF".into(),
            remaining: "#FFFFFFFF".into(),
            reset_time: "#92979DFF".into(),
            error: "#D95C5CFF".into(),
            progress_high: "#55A8F2FF".into(),
            progress_medium: "#E6B84AFF".into(),
            progress_low: "#D95C5CFF".into(),
            progress_consumed: "#363A3FFF".into(),
            progress_corner_radius: 1,
            drag_handle: "#69727CFF".into(),
        }
    }

    pub fn light_default() -> Self {
        // Cloud Porcelain: neutral cool-white surfaces with crisp slate text.
        Self {
            panel_background: "#F6F8FAFF".into(),
            panel_border: "#C7D0D9FF".into(),
            panel_blur_radius: 0,
            panel_frosted_strength: 0,
            panel_corner_radius: 2,
            tooltip_background: Some("#F6F8FAFF".into()),
            tooltip_border: Some("#C7D0D9FF".into()),
            tooltip_frosted_strength: Some(0),
            tooltip_corner_radius: 2,
            quota_type: "#46515DFF".into(),
            remaining: "#17212BFF".into(),
            reset_time: "#596777FF".into(),
            error: "#B33F49FF".into(),
            progress_high: "#4A8FD8FF".into(),
            progress_medium: "#A87521FF".into(),
            progress_low: "#C34F59FF".into(),
            progress_consumed: "#D6DDE4FF".into(),
            progress_corner_radius: 1,
            drag_handle: "#7D8997FF".into(),
        }
    }

    pub fn preset(is_dark: bool, preset: ThemePreset) -> Self {
        match (is_dark, preset) {
            (true, ThemePreset::Classic) => Self::dark_default(),
            (false, ThemePreset::Classic) => Self::light_default(),
            (true, ThemePreset::Ocean) => Self {
                panel_background: "#0F1B24FF".into(),
                panel_border: "#294252FF".into(),
                panel_blur_radius: 0,
                panel_frosted_strength: 0,
                panel_corner_radius: 2,
                tooltip_background: Some("#0F1B24FF".into()),
                tooltip_border: Some("#294252FF".into()),
                tooltip_frosted_strength: Some(0),
                tooltip_corner_radius: 2,
                quota_type: "#8CA7B8FF".into(),
                remaining: "#EAF7FFFF".into(),
                reset_time: "#7192A8FF".into(),
                error: "#FF7474FF".into(),
                progress_high: "#3FB7E9FF".into(),
                progress_medium: "#E3B65BFF".into(),
                progress_low: "#F06A6AFF".into(),
                progress_consumed: "#263B49FF".into(),
                progress_corner_radius: 1,
                drag_handle: "#648397FF".into(),
            },
            (false, ThemePreset::Ocean) => Self {
                // Clear Bay: visibly blue-tinted surfaces with marine cyan accents.
                panel_background: "#DCEFF5FF".into(),
                panel_border: "#AFCFD8FF".into(),
                panel_blur_radius: 0,
                panel_frosted_strength: 0,
                panel_corner_radius: 2,
                tooltip_background: Some("#DCEFF5FF".into()),
                tooltip_border: Some("#AFCFD8FF".into()),
                tooltip_frosted_strength: Some(0),
                tooltip_corner_radius: 2,
                quota_type: "#335965FF".into(),
                remaining: "#12343DFF".into(),
                reset_time: "#426976FF".into(),
                error: "#AD4146FF".into(),
                progress_high: "#1597B7FF".into(),
                progress_medium: "#A6751FFF".into(),
                progress_low: "#C14A54FF".into(),
                progress_consumed: "#BDD8DEFF".into(),
                progress_corner_radius: 1,
                drag_handle: "#5F8995FF".into(),
            },
            (true, ThemePreset::Forest) => Self {
                panel_background: "#14211DFF".into(),
                panel_border: "#2B4038FF".into(),
                panel_blur_radius: 0,
                panel_frosted_strength: 0,
                panel_corner_radius: 2,
                tooltip_background: Some("#14211DFF".into()),
                tooltip_border: Some("#2B4038FF".into()),
                tooltip_frosted_strength: Some(0),
                tooltip_corner_radius: 2,
                quota_type: "#9AB3A8FF".into(),
                remaining: "#F0FAF5FF".into(),
                reset_time: "#7F9C8FFF".into(),
                error: "#F5746BFF".into(),
                progress_high: "#56C596FF".into(),
                progress_medium: "#D9B45BFF".into(),
                progress_low: "#E96B5DFF".into(),
                progress_consumed: "#2B3E37FF".into(),
                progress_corner_radius: 1,
                drag_handle: "#6A887BFF".into(),
            },
            (false, ThemePreset::Forest) => Self {
                // Wheat Glow: warm ivory surfaces with restrained sage and amber accents.
                panel_background: "#FFF1DCFF".into(),
                panel_border: "#DDBF8FFF".into(),
                panel_blur_radius: 0,
                panel_frosted_strength: 0,
                panel_corner_radius: 2,
                tooltip_background: Some("#FFF1DCFF".into()),
                tooltip_border: Some("#DDBF8FFF".into()),
                tooltip_frosted_strength: Some(0),
                tooltip_corner_radius: 2,
                quota_type: "#685339FF".into(),
                remaining: "#332514FF".into(),
                reset_time: "#795F3DFF".into(),
                error: "#B24039FF".into(),
                progress_high: "#4E8A6EFF".into(),
                progress_medium: "#B97917FF".into(),
                progress_low: "#C64C43FF".into(),
                progress_consumed: "#DEC9A7FF".into(),
                progress_corner_radius: 1,
                drag_handle: "#9B7B55FF".into(),
            },
        }
    }

    pub fn apply_preset(&mut self, is_dark: bool, preset: ThemePreset) {
        *self = Self::preset(is_dark, preset);
    }

    pub fn matches_preset(&self, is_dark: bool, preset: ThemePreset) -> bool {
        self == &Self::preset(is_dark, preset)
    }

    pub fn clamp_corner_radii(&mut self, preset: AppearancePreset) {
        self.panel_corner_radius =
            self.panel_corner_radius.min(panel_corner_radius_max(preset));
        self.tooltip_corner_radius =
            self.tooltip_corner_radius.min(tooltip_corner_radius_max());
        self.progress_corner_radius =
            self.progress_corner_radius.min(progress_corner_radius_max(preset));
    }

    pub fn normalize(&mut self, fallback: &Self) {
        self.panel_background = normalize_color(&self.panel_background, &fallback.panel_background);
        self.panel_border = normalize_color(&self.panel_border, &fallback.panel_border);
        if let Some(value) = self.tooltip_background.as_mut() {
            *value = normalize_color(value, &self.panel_background);
        }
        if let Some(value) = self.tooltip_border.as_mut() {
            *value = normalize_color(value, &self.panel_border);
        }
        if let Some(value) = self.tooltip_frosted_strength.as_mut() {
            *value = (*value).min(FROSTED_STRENGTH_MAX);
        }
        self.quota_type = normalize_color(&self.quota_type, &fallback.quota_type);
        self.remaining = normalize_color(&self.remaining, &fallback.remaining);
        self.reset_time = normalize_color(&self.reset_time, &fallback.reset_time);
        self.error = normalize_color(&self.error, &fallback.error);
        self.progress_high = normalize_color(&self.progress_high, &fallback.progress_high);
        self.progress_medium = normalize_color(&self.progress_medium, &fallback.progress_medium);
        self.progress_low = normalize_color(&self.progress_low, &fallback.progress_low);
        self.progress_consumed =
            normalize_color(&self.progress_consumed, &fallback.progress_consumed);
        self.drag_handle = normalize_color(&self.drag_handle, &fallback.drag_handle);

        // Earlier v1.0.5 test builds stored frosted glass as a boolean-like
        // panel_blur_radius (0/1). Preserve an enabled setting by migrating it
        // to 100% the first time the new linear-strength schema is loaded.
        if self.panel_frosted_strength == LEGACY_FROSTED_STRENGTH_SENTINEL {
            self.panel_frosted_strength = if self.panel_blur_radius > 0 {
                FROSTED_STRENGTH_MAX
            } else {
                0
            };
        } else {
            self.panel_frosted_strength =
                self.panel_frosted_strength.min(FROSTED_STRENGTH_MAX);
        }
        self.panel_blur_radius = 0;

        if self.tooltip_background.is_some()
            || self.tooltip_border.is_some()
            || self.tooltip_frosted_strength.is_some()
        {
            self.ensure_tooltip_override();
            if let Some(value) = self.tooltip_background.as_mut() {
                *value = normalize_color(value, &self.panel_background);
            }
            if let Some(value) = self.tooltip_border.as_mut() {
                *value = normalize_color(value, &self.panel_border);
            }
            if let Some(value) = self.tooltip_frosted_strength.as_mut() {
                *value = (*value).min(FROSTED_STRENGTH_MAX);
            }
        }
    }

    pub fn color(&self, target: StyleColorTarget) -> Color {
        let value = match target {
            StyleColorTarget::PanelBackground => &self.panel_background,
            StyleColorTarget::PanelBorder => &self.panel_border,
            StyleColorTarget::TooltipBackground => self.tooltip_background.as_ref().unwrap_or(&self.panel_background),
            StyleColorTarget::TooltipBorder => self.tooltip_border.as_ref().unwrap_or(&self.panel_border),
            StyleColorTarget::QuotaType => &self.quota_type,
            StyleColorTarget::Remaining => &self.remaining,
            StyleColorTarget::ResetTime => &self.reset_time,
            StyleColorTarget::Error => &self.error,
            StyleColorTarget::ProgressHigh => &self.progress_high,
            StyleColorTarget::ProgressMedium => &self.progress_medium,
            StyleColorTarget::ProgressLow => &self.progress_low,
            StyleColorTarget::ProgressConsumed => &self.progress_consumed,
            StyleColorTarget::DragHandle => &self.drag_handle,
        };
        Color::from_hex(value)
    }

    pub fn set_color(&mut self, target: StyleColorTarget, color: Color) {
        *match target {
            StyleColorTarget::PanelBackground => &mut self.panel_background,
            StyleColorTarget::PanelBorder => &mut self.panel_border,
            StyleColorTarget::TooltipBackground => {
                self.ensure_tooltip_override();
                self.tooltip_background = Some(color.to_hex_rgba());
                return;
            }
            StyleColorTarget::TooltipBorder => {
                self.ensure_tooltip_override();
                self.tooltip_border = Some(color.to_hex_rgba());
                return;
            }
            StyleColorTarget::QuotaType => &mut self.quota_type,
            StyleColorTarget::Remaining => &mut self.remaining,
            StyleColorTarget::ResetTime => &mut self.reset_time,
            StyleColorTarget::Error => &mut self.error,
            StyleColorTarget::ProgressHigh => &mut self.progress_high,
            StyleColorTarget::ProgressMedium => &mut self.progress_medium,
            StyleColorTarget::ProgressLow => &mut self.progress_low,
            StyleColorTarget::ProgressConsumed => &mut self.progress_consumed,
            StyleColorTarget::DragHandle => &mut self.drag_handle,
        } = color.to_hex_rgba();
    }

    pub fn tooltip_frosted_strength(&self) -> u8 {
        self.tooltip_frosted_strength.unwrap_or(self.panel_frosted_strength)
    }

    fn ensure_tooltip_override(&mut self) {
        if self.tooltip_background.is_none()
            && self.tooltip_border.is_none()
            && self.tooltip_frosted_strength.is_none()
        {
            self.tooltip_background = Some(self.panel_background.clone());
            self.tooltip_border = Some(self.panel_border.clone());
            self.tooltip_frosted_strength = Some(self.panel_frosted_strength);
        }
    }

    pub fn set_tooltip_frosted_strength(&mut self, strength: u8) {
        self.ensure_tooltip_override();
        self.tooltip_frosted_strength = Some(strength.min(FROSTED_STRENGTH_MAX));
    }

}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StyleSettings {
    pub dark: ThemeStyle,
    pub light: ThemeStyle,
}

impl Default for StyleSettings {
    fn default() -> Self {
        Self {
            dark: ThemeStyle::dark_default(),
            light: ThemeStyle::light_default(),
        }
    }
}

impl StyleSettings {
    pub fn normalize(&mut self) {
        self.dark.normalize(&ThemeStyle::dark_default());
        self.light.normalize(&ThemeStyle::light_default());
    }

    pub fn clamp_corner_radii(&mut self, preset: AppearancePreset) {
        self.dark.clamp_corner_radii(preset);
        self.light.clamp_corner_radii(preset);
    }

    pub fn active(&self, is_dark: bool) -> &ThemeStyle {
        if is_dark {
            &self.dark
        } else {
            &self.light
        }
    }

    pub fn active_mut(&mut self, is_dark: bool) -> &mut ThemeStyle {
        if is_dark {
            &mut self.dark
        } else {
            &mut self.light
        }
    }

    pub fn reset_active(&mut self, is_dark: bool) {
        if is_dark {
            self.dark = ThemeStyle::dark_default();
        } else {
            self.light = ThemeStyle::light_default();
        }
    }
}

fn normalize_color(value: &str, fallback: &str) -> String {
    Color::try_from_hex(value)
        .map(Color::to_hex_rgba)
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_dark_and_light_separate() {
        let styles = StyleSettings::default();
        assert_eq!(styles.dark.panel_background, "#242A31FF");
        assert_eq!(styles.light.panel_background, "#F6F8FAFF");
    }

    #[test]
    fn component_corner_radius_limits_follow_component_dimensions() {
        assert_eq!(panel_corner_radius_max(AppearancePreset::Default), 21);
        assert_eq!(panel_corner_radius_max(AppearancePreset::Minimal), 20);
        assert_eq!(tooltip_corner_radius_max(), 14);
        assert_eq!(progress_corner_radius_max(AppearancePreset::Default), 4);
        assert_eq!(progress_corner_radius_max(AppearancePreset::Minimal), 3);
    }

    #[test]
    fn built_in_presets_use_requested_corner_radii() {
        for is_dark in [true, false] {
            for preset in ThemePreset::ALL {
                let style = ThemeStyle::preset(is_dark, preset);
                assert_eq!(style.panel_corner_radius, 2, "{preset:?} panel radius");
                assert_eq!(style.tooltip_corner_radius, 2, "{preset:?} tooltip radius");
                assert_eq!(style.progress_corner_radius, 1, "{preset:?} progress radius");
            }
        }
    }

    #[test]
    fn presets_restore_complete_panel_and_tooltip_style() {
        let dark_ocean = ThemeStyle::preset(true, ThemePreset::Ocean);
        let light_ocean = ThemeStyle::preset(false, ThemePreset::Ocean);
        assert_ne!(dark_ocean.panel_background, light_ocean.panel_background);
        assert!(dark_ocean.tooltip_background.is_some());
        assert!(dark_ocean.tooltip_border.is_some());
        assert!(dark_ocean.tooltip_frosted_strength.is_some());

        let mut style = ThemeStyle::dark_default();
        style.panel_frosted_strength = 37;
        style.set_color(
            StyleColorTarget::TooltipBackground,
            Color::from_hex("#01020304"),
        );
        style.set_tooltip_frosted_strength(61);
        style.apply_preset(true, ThemePreset::Forest);

        assert_eq!(style, ThemeStyle::preset(true, ThemePreset::Forest));
        assert_eq!(style.panel_frosted_strength, 0);
        assert_eq!(style.tooltip_frosted_strength(), 0);
        assert!(style.matches_preset(true, ThemePreset::Forest));

        style.remaining = "#FFFFFFFF".into();
        assert!(!style.matches_preset(true, ThemePreset::Forest));
    }

    fn relative_luminance(color: Color) -> f64 {
        fn channel(value: u8) -> f64 {
            let value = f64::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    fn contrast_ratio(a: Color, b: Color) -> f64 {
        let (lighter, darker) = {
            let a = relative_luminance(a);
            let b = relative_luminance(b);
            if a >= b { (a, b) } else { (b, a) }
        };
        (lighter + 0.05) / (darker + 0.05)
    }

    #[test]
    fn preset_text_colors_keep_readable_contrast() {
        for is_dark in [true, false] {
            for preset in ThemePreset::ALL {
                let style = ThemeStyle::preset(is_dark, preset);
                let background = style.color(StyleColorTarget::PanelBackground);
                for target in [
                    StyleColorTarget::QuotaType,
                    StyleColorTarget::Remaining,
                    StyleColorTarget::ResetTime,
                ] {
                    let ratio = contrast_ratio(background, style.color(target));
                    assert!(
                        ratio >= 4.5,
                        "{preset:?} {target:?} contrast {ratio:.2} is below 4.5"
                    );
                }
            }
        }
    }

    #[test]
    fn redesigned_light_presets_keep_error_text_readable() {
        for preset in ThemePreset::ALL {
            let style = ThemeStyle::preset(false, preset);
            let background = style.color(StyleColorTarget::PanelBackground);
            let ratio = contrast_ratio(background, style.color(StyleColorTarget::Error));
            assert!(
                ratio >= 4.5,
                "{preset:?} error contrast {ratio:.2} is below 4.5"
            );
        }
    }

    #[test]
    fn redesigned_light_presets_are_visually_distinct() {
        let backgrounds = ThemePreset::ALL.map(|preset| {
            ThemeStyle::preset(false, preset).color(StyleColorTarget::PanelBackground)
        });
        for left in 0..backgrounds.len() {
            for right in (left + 1)..backgrounds.len() {
                let a = backgrounds[left];
                let b = backgrounds[right];
                let channel_distance = u16::from(a.r.abs_diff(b.r))
                    + u16::from(a.g.abs_diff(b.g))
                    + u16::from(a.b.abs_diff(b.b));
                assert!(
                    channel_distance >= 32,
                    "light preset backgrounds {left} and {right} are too similar: {channel_distance}"
                );
            }
        }
    }

    #[test]
    fn invalid_colors_and_strength_are_normalized() {
        let mut styles = StyleSettings::default();
        styles.dark.panel_background = "bad".into();
        styles.dark.panel_frosted_strength = 180;
        styles.normalize();
        assert_eq!(styles.dark.panel_background, "#242A31FF");
        assert_eq!(styles.dark.panel_frosted_strength, FROSTED_STRENGTH_MAX);
    }

    #[test]
    fn legacy_boolean_frosted_setting_migrates_to_full_strength() {
        let json = r##"{
            "dark": {
                "panel_blur_radius": 1
            },
            "light": {
                "panel_blur_radius": 0
            }
        }"##;
        let mut styles: StyleSettings = serde_json::from_str(json).unwrap();
        styles.normalize();
        assert_eq!(styles.dark.panel_frosted_strength, 100);
        assert_eq!(styles.light.panel_frosted_strength, 0);

        let saved = serde_json::to_string(&styles).unwrap();
        assert!(saved.contains("panel_frosted_strength"));
        assert!(!saved.contains("panel_blur_radius"));
    }

    #[test]
    fn color_target_round_trips_rgba() {
        let mut style = ThemeStyle::dark_default();
        let color = Color::from_hex("#12345678");
        style.set_color(StyleColorTarget::ProgressHigh, color);
        assert_eq!(style.progress_high, "#12345678");
        assert_eq!(style.color(StyleColorTarget::ProgressHigh), color);
    }
}
