use serde::{Deserialize, Serialize};

use crate::localization::{LanguageId, LanguageId as L};
use crate::native_interop::Color;
use crate::style::ThemeStyle;

pub const EDITABLE_SETTINGS_SCHEMA_VERSION: u32 = 1;

fn localized<'a>(zh: bool, zh_text: &'a str, en_text: &'a str) -> &'a str {
    if zh { zh_text } else { en_text }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableSettings {
    pub schema_version: u32,
    pub general: EditableGeneral,
    pub appearance: EditableAppearance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableGeneral {
    pub refresh_interval: String,
    pub show_usage: EditableUsage,
    pub quota_alert_percent: u8,
    pub start_with_windows: bool,
    pub language: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableUsage {
    pub session_5h: bool,
    pub weekly: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableAppearance {
    pub theme: String,
    pub layout: String,
    pub dark: EditableThemeStyle,
    pub light: EditableThemeStyle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditableThemeStyle {
    pub panel_background: String,
    pub panel_border: String,
    pub frosted_strength: u8,
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

impl EditableThemeStyle {
    pub fn from_theme_style(style: &ThemeStyle) -> Self {
        Self {
            panel_background: style.panel_background.clone(),
            panel_border: style.panel_border.clone(),
            frosted_strength: style.panel_frosted_strength,
            quota_type: style.quota_type.clone(),
            remaining: style.remaining.clone(),
            reset_time: style.reset_time.clone(),
            error: style.error.clone(),
            progress_high: style.progress_high.clone(),
            progress_medium: style.progress_medium.clone(),
            progress_low: style.progress_low.clone(),
            progress_consumed: style.progress_consumed.clone(),
            drag_handle: style.drag_handle.clone(),
        }
    }

    pub fn validate_and_normalize(&self, path: &str) -> Result<Self, String> {
        if self.frosted_strength > 100 {
            return Err(format!("{path}.frosted_strength: allowed range is 0-100"));
        }
        Ok(Self {
            panel_background: normalize_hex(&self.panel_background, &format!("{path}.panel_background"))?,
            panel_border: normalize_hex(&self.panel_border, &format!("{path}.panel_border"))?,
            frosted_strength: self.frosted_strength,
            quota_type: normalize_hex(&self.quota_type, &format!("{path}.quota_type"))?,
            remaining: normalize_hex(&self.remaining, &format!("{path}.remaining"))?,
            reset_time: normalize_hex(&self.reset_time, &format!("{path}.reset_time"))?,
            error: normalize_hex(&self.error, &format!("{path}.error"))?,
            progress_high: normalize_hex(&self.progress_high, &format!("{path}.progress_high"))?,
            progress_medium: normalize_hex(&self.progress_medium, &format!("{path}.progress_medium"))?,
            progress_low: normalize_hex(&self.progress_low, &format!("{path}.progress_low"))?,
            progress_consumed: normalize_hex(
                &self.progress_consumed,
                &format!("{path}.progress_consumed"),
            )?,
            drag_handle: normalize_hex(&self.drag_handle, &format!("{path}.drag_handle"))?,
        })
    }

    pub fn to_theme_style(&self) -> ThemeStyle {
        ThemeStyle {
            panel_background: self.panel_background.clone(),
            panel_border: self.panel_border.clone(),
            panel_blur_radius: 0,
            panel_frosted_strength: self.frosted_strength,
            quota_type: self.quota_type.clone(),
            remaining: self.remaining.clone(),
            reset_time: self.reset_time.clone(),
            error: self.error.clone(),
            progress_high: self.progress_high.clone(),
            progress_medium: self.progress_medium.clone(),
            progress_low: self.progress_low.clone(),
            progress_consumed: self.progress_consumed.clone(),
            drag_handle: self.drag_handle.clone(),
        }
    }
}

impl EditableSettings {
    pub fn validate_and_normalize(&self) -> Result<Self, String> {
        if self.schema_version != EDITABLE_SETTINGS_SCHEMA_VERSION {
            return Err(format!(
                "schema_version: expected {}, got {}",
                EDITABLE_SETTINGS_SCHEMA_VERSION, self.schema_version
            ));
        }

        if !matches!(
            self.general.refresh_interval.as_str(),
            "1m" | "5m" | "15m" | "1h"
        ) {
            return Err(
                "general.refresh_interval: allowed values are 1m, 5m, 15m, 1h".to_string(),
            );
        }
        if !self.general.show_usage.session_5h && !self.general.show_usage.weekly {
            return Err(
                "general.show_usage: session_5h and weekly cannot both be false".to_string(),
            );
        }
        if !matches!(self.general.quota_alert_percent, 0 | 10 | 20 | 30) {
            return Err(
                "general.quota_alert_percent: allowed values are 0, 10, 20, 30".to_string(),
            );
        }
        if self.general.language != "system"
            && !LanguageId::ALL
                .iter()
                .any(|language| language.code() == self.general.language)
        {
            return Err(format!(
                "general.language: unsupported language code '{}'",
                self.general.language
            ));
        }
        if !matches!(self.appearance.theme.as_str(), "system" | "dark" | "light") {
            return Err(
                "appearance.theme: allowed values are system, dark, light".to_string(),
            );
        }
        if !matches!(self.appearance.layout.as_str(), "default" | "minimal") {
            return Err(
                "appearance.layout: allowed values are default, minimal".to_string(),
            );
        }

        let mut normalized = self.clone();
        normalized.appearance.dark = self.appearance.dark.validate_and_normalize("appearance.dark")?;
        normalized.appearance.light =
            self.appearance.light.validate_and_normalize("appearance.light")?;
        Ok(normalized)
    }

    pub fn to_pretty_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|error| error.to_string())
    }

    pub fn to_jsonc(&self, language: LanguageId) -> String {
        let zh = language == L::SimplifiedChinese;
        let q = |value: &str| serde_json::to_string(value).unwrap_or_else(|_| "\"?\"".into());
        let g = &self.general;
        let a = &self.appearance;
        let dark = &a.dark;
        let light = &a.light;

        format!(
r#"{{
  // {general_group}
  "general": {{
    // {refresh_desc}
    // {refresh_options}
    "refresh_interval": {refresh},

    // {usage_group_desc}
    "show_usage": {{
      // {session_desc}
      // {bool_options}; {usage_rule}
      "session_5h": {session},

      // {weekly_desc}
      // {bool_options}; {usage_rule}
      "weekly": {weekly}
    }},

    // {alert_desc}
    // {alert_options}
    "quota_alert_percent": {alert},

    // {startup_desc}
    // {bool_options}
    "start_with_windows": {startup},

    // {language_desc}
    // {language_options}
    "language": {language}
  }},

  // {appearance_group}
  "appearance": {{
    // {theme_desc}
    // {theme_options}
    "theme": {theme},

    // {layout_desc}
    // {layout_options}
    "layout": {layout},

    // {dark_desc}
    "dark": {{
{dark_json}
    }},

    // {light_desc}
    "light": {{
{light_json}
    }}
  }},

  // {schema_desc}
  // {schema_options}
  "schema_version": {schema}
}}
"#,
            general_group = localized(zh, "常规设置", "General settings"),
            refresh_desc = localized(zh, "自动刷新额度数据的时间间隔", "Automatic usage refresh interval"),
            refresh_options = localized(zh, "可选：1m | 5m | 15m | 1h", "Options: 1m | 5m | 15m | 1h"),
            refresh = q(&g.refresh_interval),
            usage_group_desc = localized(zh, "任务栏组件中显示的额度类型", "Quota types shown in the taskbar widget"),
            session_desc = localized(zh, "是否显示 5 小时额度", "Show the 5-hour quota"),
            weekly_desc = localized(zh, "是否显示每周额度", "Show the weekly quota"),
            bool_options = localized(zh, "可选：true | false", "Options: true | false"),
            usage_rule = localized(zh, "5 小时额度和每周额度至少开启一个", "at least one quota must stay enabled"),
            session = g.show_usage.session_5h,
            weekly = g.show_usage.weekly,
            alert_desc = localized(zh, "剩余额度达到阈值时发送提醒；0 表示关闭", "Notify when remaining quota reaches the threshold; 0 disables alerts"),
            alert_options = localized(zh, "可选：0 | 10 | 20 | 30", "Options: 0 | 10 | 20 | 30"),
            alert = g.quota_alert_percent,
            startup_desc = localized(zh, "是否随 Windows 启动", "Start with Windows"),
            startup = g.start_with_windows,
            language_desc = localized(zh, "界面语言", "UI language"),
            language_options = localized(zh, 
                "可选：system | en | nl | es | fr | de | ja | ko | zh-CN | zh-TW | ru | pt-BR",
                "Options: system | en | nl | es | fr | de | ja | ko | zh-CN | zh-TW | ru | pt-BR",
            ),
            language = q(&g.language),
            appearance_group = localized(zh, "外观设置", "Appearance settings"),
            theme_desc = localized(zh, "主题模式", "Theme mode"),
            theme_options = localized(zh, "可选：system | dark | light", "Options: system | dark | light"),
            theme = q(&a.theme),
            layout_desc = localized(zh, "组件排版", "Widget layout"),
            layout_options = localized(zh, "可选：default | minimal", "Options: default | minimal"),
            layout = q(&a.layout),
            dark_desc = localized(zh, "深色主题可编辑样式", "Editable dark-theme style"),
            light_desc = localized(zh, "浅色主题可编辑样式", "Editable light-theme style"),
            dark_json = theme_jsonc(dark, zh, 6),
            light_json = theme_jsonc(light, zh, 6),
            schema_desc = localized(zh, "公开配置结构版本", "Public configuration schema version"),
            schema_options = localized(zh, "固定值：1", "Fixed value: 1"),
            schema = self.schema_version,
        )
    }
}

fn push_color_jsonc(
    lines: &mut Vec<String>,
    pad: &str,
    zh: bool,
    comment: &str,
    key: &str,
    value: &str,
    comma: bool,
) {
    let quoted = serde_json::to_string(value).unwrap_or_else(|_| "\"?\"".into());
    lines.push(format!("{pad}// {comment}"));
    lines.push(format!(
        "{pad}// {}",
        localized(
            zh,
            "格式：#RRGGBB 或 #RRGGBBAA",
            "Format: #RRGGBB or #RRGGBBAA"
        )
    ));
    lines.push(format!(
        "{pad}\"{key}\": {quoted}{}",
        if comma { "," } else { "" }
    ));
    lines.push(String::new());
}

fn theme_jsonc(style: &EditableThemeStyle, zh: bool, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut lines = Vec::new();

    push_color_jsonc(
        &mut lines,
        &pad,
        zh,
        localized(zh, "面板背景颜色", "Panel background color"),
        "panel_background",
        &style.panel_background,
        true,
    );
    push_color_jsonc(
        &mut lines,
        &pad,
        zh,
        localized(zh, "面板边框颜色", "Panel border color"),
        "panel_border",
        &style.panel_border,
        true,
    );
    lines.push(format!(
        "{pad}// {}",
        localized(zh, "磨砂强度", "Frosted intensity")
    ));
    lines.push(format!(
        "{pad}// {}",
        localized(zh, "范围：0–100", "Range: 0–100")
    ));
    lines.push(format!(
        "{pad}\"frosted_strength\": {},",
        style.frosted_strength
    ));
    lines.push(String::new());

    for (comment_zh, comment_en, key, value, comma) in [
        ("额度类型文字颜色", "Quota-type text color", "quota_type", style.quota_type.as_str(), true),
        ("剩余额度文字颜色", "Remaining-quota text color", "remaining", style.remaining.as_str(), true),
        ("重置时间文字颜色", "Reset-time text color", "reset_time", style.reset_time.as_str(), true),
        ("异常状态文字颜色", "Error-state text color", "error", style.error.as_str(), true),
        ("充足额度颜色", "High-quota color", "progress_high", style.progress_high.as_str(), true),
        ("中等额度颜色", "Medium-quota color", "progress_medium", style.progress_medium.as_str(), true),
        ("低额度颜色", "Low-quota color", "progress_low", style.progress_low.as_str(), true),
        ("已消耗部分颜色", "Consumed-progress color", "progress_consumed", style.progress_consumed.as_str(), true),
        ("拖拽点颜色", "Drag-handle color", "drag_handle", style.drag_handle.as_str(), false),
    ] {
        push_color_jsonc(
            &mut lines,
            &pad,
            zh,
            localized(zh, comment_zh, comment_en),
            key,
            value,
            comma,
        );
    }

    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn normalize_hex(value: &str, path: &str) -> Result<String, String> {
    let trimmed = value.trim();
    let valid_len = matches!(trimmed.len(), 7 | 9);
    if !trimmed.starts_with('#')
        || !valid_len
        || !trimmed[1..].chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(format!("{path}: expected #RRGGBB or #RRGGBBAA"));
    }
    Color::try_from_hex(trimmed)
        .map(|color| color.to_hex_rgba())
        .ok_or_else(|| format!("{path}: invalid color"))
}

pub fn parse_jsonc(text: &str) -> Result<EditableSettings, String> {
    let stripped = strip_jsonc_comments(text)?;
    let parsed: EditableSettings = serde_json::from_str(&stripped).map_err(|error| {
        format!(
            "line {}, column {}: {}",
            error.line(),
            error.column(),
            error
        )
    })?;
    parsed.validate_and_normalize()
}

pub fn strip_jsonc_comments(text: &str) -> Result<String, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut i = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut line_comment = false;
    let mut block_comment = false;

    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();

        if line_comment {
            if ch == '\n' {
                line_comment = false;
                output.push('\n');
            } else {
                output.push(' ');
            }
            i += 1;
            continue;
        }
        if block_comment {
            if ch == '*' && next == Some('/') {
                output.push(' ');
                output.push(' ');
                i += 2;
                block_comment = false;
            } else {
                output.push(if ch == '\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        if in_string {
            output.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            i += 1;
        } else if ch == '/' && next == Some('/') {
            output.push(' ');
            output.push(' ');
            i += 2;
            line_comment = true;
        } else if ch == '/' && next == Some('*') {
            output.push(' ');
            output.push(' ');
            i += 2;
            block_comment = true;
        } else {
            output.push(ch);
            i += 1;
        }
    }

    if block_comment {
        return Err("unterminated block comment".to_string());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EditableSettings {
        EditableSettings {
            schema_version: 1,
            general: EditableGeneral {
                refresh_interval: "15m".into(),
                show_usage: EditableUsage {
                    session_5h: true,
                    weekly: true,
                },
                quota_alert_percent: 20,
                start_with_windows: true,
                language: "zh-CN".into(),
            },
            appearance: EditableAppearance {
                theme: "system".into(),
                layout: "default".into(),
                dark: EditableThemeStyle::from_theme_style(&ThemeStyle::dark_default()),
                light: EditableThemeStyle::from_theme_style(&ThemeStyle::light_default()),
            },
        }
    }

    #[test]
    fn jsonc_comments_round_trip_without_touching_urls_inside_strings() {
        let mut value = sample();
        value.appearance.dark.panel_background = "#123456".into();
        let text = value.to_jsonc(LanguageId::SimplifiedChinese);
        let parsed = parse_jsonc(&text).unwrap();
        assert_eq!(parsed.appearance.dark.panel_background, "#123456FF");

        let stripped = strip_jsonc_comments(r#"{ "value": "https://example.com//x" // note
}"#)
        .unwrap();
        assert!(stripped.contains("https://example.com//x"));
    }

    #[test]
    fn unknown_properties_are_rejected() {
        let text = sample().to_pretty_json().unwrap().replace(
            "\"schema_version\": 1",
            "\"unknown_setting\": true,\n  \"schema_version\": 1",
        );
        let error = parse_jsonc(&text).unwrap_err();
        assert!(error.contains("unknown field"));
    }

    #[test]
    fn invalid_business_rules_are_rejected_transactionally() {
        let mut value = sample();
        value.general.show_usage.session_5h = false;
        value.general.show_usage.weekly = false;
        assert!(value.validate_and_normalize().is_err());

        value = sample();
        value.general.quota_alert_percent = 15;
        assert!(value.validate_and_normalize().is_err());
    }

    #[test]
    fn generated_jsonc_contains_comments_for_options() {
        let text = sample().to_jsonc(LanguageId::SimplifiedChinese);
        assert!(text.contains("// 可选：1m | 5m | 15m | 1h"));
        assert!(text.contains("// 格式：#RRGGBB 或 #RRGGBBAA"));
        assert!(text.contains("// 范围：0–100"));
    }
}
