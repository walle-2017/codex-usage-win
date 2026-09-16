use serde::{Deserialize, Deserializer, Serialize};

use crate::localization::LanguageId;
use crate::models::UsageSection;
use crate::native_interop;
use crate::poller::{self, UsageWindowKind};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppearancePreset {
    #[default]
    Compact,
    Minimal,
}

impl<'de> Deserialize<'de> for AppearancePreset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "compact" | "default" => Ok(Self::Compact),
            "minimal" => Ok(Self::Minimal),
            _ => Err(serde::de::Error::unknown_variant(
                &value,
                &["compact", "minimal"],
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleMetrics {
    pub widget_height: i32,
    pub panel_radius: i32,
    pub outer_padding: i32,
    pub label_bar_gap: i32,
    pub bar_percent_gap: i32,
    pub percent_width: i32,
    pub percent_reset_gap: i32,
    pub reset_width: i32,
    pub bar_width: i32,
    pub bar_value_width: i32,
    pub bar_value_gap: i32,
    pub bar_height: i32,
    pub label_width: i32,
    pub label_right_margin: i32,
    pub bar_right_margin: i32,
    pub text_width: i32,
    pub secondary_width: i32,
    pub row_gap: i32,
    pub font_height: i32,
    pub value_font_height: i32,
    pub secondary_font_height: i32,
    pub divider_right_margin: i32,
    pub model_right_margin: i32,
    pub hide_reset_time: bool,
}

impl AppearancePreset {
    pub fn metrics(self) -> StyleMetrics {
        match self {
            Self::Compact => StyleMetrics {
                widget_height: 42,
                panel_radius: 0,
                outer_padding: 6,
                label_bar_gap: 6,
                bar_percent_gap: 4,
                percent_width: 36,
                percent_reset_gap: 3,
                reset_width: 34,
                bar_width: 82,
                bar_value_width: 34,
                bar_value_gap: 2,
                bar_height: 8,
                label_width: 18,
                label_right_margin: 6,
                bar_right_margin: 6,
                text_width: 34,
                secondary_width: 34,
                row_gap: 7,
                font_height: -11,
                value_font_height: -12,
                secondary_font_height: -11,
                divider_right_margin: 7,
                model_right_margin: 4,
                hide_reset_time: false,
            },
            Self::Minimal => StyleMetrics {
                widget_height: 40,
                panel_radius: 0,
                outer_padding: 6,
                label_bar_gap: 6,
                bar_percent_gap: 4,
                percent_width: 36,
                percent_reset_gap: 3,
                reset_width: 0,
                bar_width: 62,
                bar_value_width: 34,
                bar_value_gap: 2,
                bar_height: 7,
                label_width: 18,
                label_right_margin: 5,
                bar_right_margin: 5,
                text_width: 0,
                secondary_width: 0,
                row_gap: 7,
                font_height: -11,
                value_font_height: -12,
                secondary_font_height: -11,
                divider_right_margin: 6,
                model_right_margin: 3,
                hide_reset_time: true,
            },
        }
    }

}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskbarValueText {
    pub primary: String,
    pub secondary: Option<String>,
}

pub fn taskbar_value_text(
    preset: AppearancePreset,
    _language: LanguageId,
    section: &UsageSection,
    window: UsageWindowKind,
) -> TaskbarValueText {
    let percentage = poller::remaining_percentage(section.percentage);

    let primary = format!("{percentage:.0}%");
    let secondary = if preset.metrics().hide_reset_time {
        None
    } else {
        section
            .resets_at
            .and_then(native_interop::system_time_to_local)
            .map(|reset| match window {
                UsageWindowKind::Session => format!("{:02}:{:02}", reset.wHour, reset.wMinute),
                UsageWindowKind::Weekly => format!("{:02}/{:02}", reset.wMonth, reset.wDay),
            })
    };

    TaskbarValueText { primary, secondary }
}

pub fn taskbar_line(
    preset: AppearancePreset,
    language: LanguageId,
    section: &UsageSection,
    window: UsageWindowKind,
) -> String {
    let value = taskbar_value_text(preset, language, section, window);
    match value.secondary {
        Some(reset) => format!("{}  {}", value.primary, reset),
        None => value.primary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    fn section_with_local_reset(used: f64, unix_seconds: u64) -> UsageSection {
        UsageSection {
            percentage: used,
            resets_at: Some(UNIX_EPOCH + Duration::from_secs(unix_seconds)),
        }
    }

    #[test]
    fn compact_is_the_default_preset() {
        assert_eq!(AppearancePreset::default(), AppearancePreset::Compact);
    }

    #[test]
    fn legacy_default_setting_migrates_to_compact() {
        let preset: AppearancePreset = serde_json::from_str("\"default\"").unwrap();
        assert_eq!(preset, AppearancePreset::Compact);
        assert_eq!(serde_json::to_string(&preset).unwrap(), "\"compact\"");
    }

    #[test]
    fn minimal_has_the_smallest_layout() {
        let compact = AppearancePreset::Compact.metrics();
        let minimal = AppearancePreset::Minimal.metrics();

        assert!(minimal.bar_width < compact.bar_width);
        assert_eq!(minimal.bar_value_width, compact.bar_value_width);
        assert_eq!(minimal.bar_value_gap, compact.bar_value_gap);
        assert_eq!(minimal.text_width, 0);
        assert!(minimal.hide_reset_time);
        assert!(!compact.hide_reset_time);
    }

    #[test]
    fn percentage_value_slot_has_room_for_three_digits() {
        let compact = AppearancePreset::Compact.metrics();
        let minimal = AppearancePreset::Minimal.metrics();

        assert!(compact.bar_value_width >= 34);
        assert!(minimal.bar_value_width >= 34);
    }

    #[test]
    fn percentage_value_gap_is_subtle_but_visible() {
        let compact = AppearancePreset::Compact.metrics();
        let minimal = AppearancePreset::Minimal.metrics();

        assert_eq!(compact.bar_value_gap, 2);
        assert_eq!(minimal.bar_value_gap, 2);
    }

    #[test]
    fn taskbar_text_separates_percentage_from_reset_hint() {
        let section = section_with_local_reset(81.0, 1_789_000_000);
        let compact = taskbar_value_text(
            AppearancePreset::Compact,
            LanguageId::SimplifiedChinese,
            &section,
            UsageWindowKind::Session,
        );
        assert_eq!(compact.primary, "19%");
        assert!(compact.secondary.is_some());

        let minimal = taskbar_value_text(
            AppearancePreset::Minimal,
            LanguageId::SimplifiedChinese,
            &section,
            UsageWindowKind::Session,
        );
        assert_eq!(minimal.primary, "19%");
        assert_eq!(minimal.secondary, None);
    }

    #[test]
    fn remaining_quota_is_language_independent() {
        let section = section_with_local_reset(8.0, 1_789_000_000);
        for language in [
            LanguageId::SimplifiedChinese,
            LanguageId::English,
            LanguageId::Japanese,
        ] {
            let value = taskbar_value_text(
                AppearancePreset::Compact,
                language,
                &section,
                UsageWindowKind::Session,
            );
            assert_eq!(value.primary, "92%");
        }
    }

    #[test]
    fn taskbar_line_never_uses_reset_icon() {
        let section = section_with_local_reset(81.0, 1_789_000_000);
        let compact = taskbar_line(
            AppearancePreset::Compact,
            LanguageId::SimplifiedChinese,
            &section,
            UsageWindowKind::Session,
        );
        let minimal = taskbar_line(
            AppearancePreset::Minimal,
            LanguageId::SimplifiedChinese,
            &section,
            UsageWindowKind::Session,
        );

        assert!(compact.starts_with("19%  "));
        assert!(!compact.contains('↻'));
        assert_eq!(minimal, "19%");
    }
}
