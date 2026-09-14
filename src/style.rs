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
