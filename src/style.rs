use serde::{Deserialize, Serialize};

use crate::native_interop::Color;

pub const FROSTED_STRENGTH_MAX: u8 = 100;
const LEGACY_FROSTED_STRENGTH_SENTINEL: u8 = u8::MAX;

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
    pub quota_type: String,
    pub remaining: String,
    pub reset_time: String,
    pub error: String,
    pub progress_high: String,
    pub progress_medium: String,
    pub progress_low: String,
    pub progress_consumed: String,
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
            quota_type: "#A0A0A0FF".into(),
            remaining: "#FFFFFFFF".into(),
            reset_time: "#92979DFF".into(),
            error: "#D95C5CFF".into(),
            progress_high: "#55A8F2FF".into(),
            progress_medium: "#E6B84AFF".into(),
            progress_low: "#D95C5CFF".into(),
            progress_consumed: "#363A3FFF".into(),
            drag_handle: "#69727CFF".into(),
        }
    }

    pub fn light_default() -> Self {
        Self {
            panel_background: "#EEF1F4FF".into(),
            panel_border: "#D4D9DFFF".into(),
            panel_blur_radius: 0,
            panel_frosted_strength: 0,
            quota_type: "#404040FF".into(),
            remaining: "#202020FF".into(),
            reset_time: "#666666FF".into(),
            error: "#D95C5CFF".into(),
            progress_high: "#55A8F2FF".into(),
            progress_medium: "#E6B84AFF".into(),
            progress_low: "#D95C5CFF".into(),
            progress_consumed: "#AAAAAAFF".into(),
            drag_handle: "#8A929AFF".into(),
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
                quota_type: "#8CA7B8FF".into(),
                remaining: "#EAF7FFFF".into(),
                reset_time: "#7192A8FF".into(),
                error: "#FF7474FF".into(),
                progress_high: "#3FB7E9FF".into(),
                progress_medium: "#E3B65BFF".into(),
                progress_low: "#F06A6AFF".into(),
                progress_consumed: "#263B49FF".into(),
                drag_handle: "#648397FF".into(),
            },
            (false, ThemePreset::Ocean) => Self {
                panel_background: "#EEF8FCFF".into(),
                panel_border: "#C7E1ECFF".into(),
                panel_blur_radius: 0,
                panel_frosted_strength: 0,
                quota_type: "#477080FF".into(),
                remaining: "#16333FFF".into(),
                reset_time: "#58737FFF".into(),
                error: "#C94D4DFF".into(),
                progress_high: "#188BC0FF".into(),
                progress_medium: "#AD7922FF".into(),
                progress_low: "#CD5151FF".into(),
                progress_consumed: "#BEDAE5FF".into(),
                drag_handle: "#6A8C99FF".into(),
            },
            (true, ThemePreset::Forest) => Self {
                panel_background: "#14211DFF".into(),
                panel_border: "#2B4038FF".into(),
                panel_blur_radius: 0,
                panel_frosted_strength: 0,
                quota_type: "#9AB3A8FF".into(),
                remaining: "#F0FAF5FF".into(),
                reset_time: "#7F9C8FFF".into(),
                error: "#F5746BFF".into(),
                progress_high: "#56C596FF".into(),
                progress_medium: "#D9B45BFF".into(),
                progress_low: "#E96B5DFF".into(),
                progress_consumed: "#2B3E37FF".into(),
                drag_handle: "#6A887BFF".into(),
            },
            (false, ThemePreset::Forest) => Self {
                panel_background: "#F1F7F3FF".into(),
                panel_border: "#CDDED3FF".into(),
                panel_blur_radius: 0,
                panel_frosted_strength: 0,
                quota_type: "#52705FFF".into(),
                remaining: "#1C3025FF".into(),
                reset_time: "#5C7366FF".into(),
                error: "#BF4A42FF".into(),
                progress_high: "#2E9369FF".into(),
                progress_medium: "#A97921FF".into(),
                progress_low: "#C55348FF".into(),
                progress_consumed: "#C6D8CDFF".into(),
                drag_handle: "#71897BFF".into(),
            },
        }
    }

    pub fn apply_preset(&mut self, is_dark: bool, preset: ThemePreset) {
        let frosted_strength = self.panel_frosted_strength;
        let mut replacement = Self::preset(is_dark, preset);
        replacement.panel_frosted_strength = frosted_strength;
        *self = replacement;
    }

    pub fn matches_preset(&self, is_dark: bool, preset: ThemePreset) -> bool {
        let mut expected = Self::preset(is_dark, preset);
        expected.panel_frosted_strength = self.panel_frosted_strength;
        self == &expected
    }

    pub fn normalize(&mut self, fallback: &Self) {
        self.panel_background = normalize_color(&self.panel_background, &fallback.panel_background);
        self.panel_border = normalize_color(&self.panel_border, &fallback.panel_border);
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
    }

    pub fn color(&self, target: StyleColorTarget) -> Color {
        let value = match target {
            StyleColorTarget::PanelBackground => &self.panel_background,
            StyleColorTarget::PanelBorder => &self.panel_border,
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
        assert_eq!(styles.light.panel_background, "#EEF1F4FF");
    }

    #[test]
    fn presets_are_theme_specific_and_preserve_blur_when_applied() {
        let dark_ocean = ThemeStyle::preset(true, ThemePreset::Ocean);
        let light_ocean = ThemeStyle::preset(false, ThemePreset::Ocean);
        assert_ne!(dark_ocean.panel_background, light_ocean.panel_background);

        let mut style = ThemeStyle::dark_default();
        style.panel_frosted_strength = 37;
        style.apply_preset(true, ThemePreset::Forest);
        assert_eq!(style.panel_frosted_strength, 37);
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
