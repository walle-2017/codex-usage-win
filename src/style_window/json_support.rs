//! Pure JSONC editor helpers: line endings, syntax-token classification,
//! color palettes, and localized configuration validation messages.
//!
//! This module intentionally has no window handles or state-lock ownership;
//! painting, RichEdit, file I/O, and focus live in dedicated sibling modules.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonTokenKind {
    Comment,
    Key,
    String,
    Number,
    Keyword,
}

pub(super) fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn to_windows_newlines(text: &str) -> String {
    normalize_to_lf(text).replace('\n', "\r\n")
}


pub(super) fn json_token_color(kind: JsonTokenKind, is_dark: bool) -> Color {
    match (is_dark, kind) {
        (true, JsonTokenKind::Comment) => Color::from_hex("#777777FF"),
        (true, JsonTokenKind::Key) => Color::from_hex("#9CDCFEFF"),
        (true, JsonTokenKind::String) => Color::from_hex("#CE9178FF"),
        (true, JsonTokenKind::Number) => Color::from_hex("#B5CEA8FF"),
        (true, JsonTokenKind::Keyword) => Color::from_hex("#C586C0FF"),
        (false, JsonTokenKind::Comment) => Color::from_hex("#999999FF"),
        (false, JsonTokenKind::Key) => Color::from_hex("#0451A5FF"),
        (false, JsonTokenKind::String) => Color::from_hex("#A31515FF"),
        (false, JsonTokenKind::Number) => Color::from_hex("#098658FF"),
        (false, JsonTokenKind::Keyword) => Color::from_hex("#AF00DBFF"),
    }
}

pub(super) fn json_default_text_color(is_dark: bool) -> Color {
    if is_dark {
        Color::from_hex("#D4D4D4FF")
    } else {
        Color::from_hex("#202020FF")
    }
}

pub(super) fn json_editor_background(is_dark: bool) -> Color {
    if is_dark {
        Color::from_hex("#1E1E1EFF")
    } else {
        Color::from_hex("#FFFFFFFF")
    }
}

pub(super) fn jsonc_highlight_ranges(text: &str) -> Vec<(i32, i32, JsonTokenKind)> {
    let chars: Vec<char> = text.chars().collect();
    let mut utf16_offsets = Vec::with_capacity(chars.len() + 1);
    let mut offset = 0i32;
    utf16_offsets.push(offset);
    for ch in &chars {
        offset += ch.len_utf16() as i32;
        utf16_offsets.push(offset);
    }

    let mut ranges = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            let start = i;
            i += 2;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            ranges.push((
                utf16_offsets[start],
                utf16_offsets[i],
                JsonTokenKind::Comment,
            ));
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < chars.len() {
                i += 2;
            } else {
                i = chars.len();
            }
            ranges.push((
                utf16_offsets[start],
                utf16_offsets[i],
                JsonTokenKind::Comment,
            ));
            continue;
        }
        if chars[i] == '"' {
            let start = i;
            i += 1;
            let mut escaped = false;
            while i < chars.len() {
                let ch = chars[i];
                i += 1;
                if escaped {
                    escaped = false;
                    continue;
                }
                if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    break;
                }
            }
            let mut lookahead = i;
            while lookahead < chars.len() && chars[lookahead].is_whitespace() {
                lookahead += 1;
            }
            let kind = if chars.get(lookahead) == Some(&':') {
                JsonTokenKind::Key
            } else {
                JsonTokenKind::String
            };
            ranges.push((utf16_offsets[start], utf16_offsets[i], kind));
            continue;
        }
        if chars[i].is_ascii_digit() || chars[i] == '-' {
            let start = i;
            i += 1;
            while i < chars.len()
                && matches!(
                    chars[i],
                    '0'..='9' | '.' | 'e' | 'E' | '+' | '-'
                )
            {
                i += 1;
            }
            ranges.push((
                utf16_offsets[start],
                utf16_offsets[i],
                JsonTokenKind::Number,
            ));
            continue;
        }

        let remaining: String = chars[i..].iter().take(5).collect();
        let keyword_len = if remaining.starts_with("false") {
            Some(5)
        } else if remaining.starts_with("true") || remaining.starts_with("null") {
            Some(4)
        } else {
            None
        };
        if let Some(len) = keyword_len {
            let end = (i + len).min(chars.len());
            ranges.push((
                utf16_offsets[i],
                utf16_offsets[end],
                JsonTokenKind::Keyword,
            ));
            i = end;
            continue;
        }
        i += 1;
    }
    ranges
}


pub(super) fn json_error_location(error: &str) -> Option<(usize, usize)> {
    let rest = error.strip_prefix("line ")?;
    let (line, rest) = rest.split_once(", column ")?;
    let (column, _) = rest.split_once(':')?;
    Some((line.trim().parse().ok()?, column.trim().parse().ok()?))
}

#[derive(Clone, Copy)]
pub(super) enum JsonErrorPhrase {
    Heading,
    ControlCharacter,
    UnknownSetting,
    MissingSetting,
    DuplicateSetting,
    InvalidColor,
    FrostedRange,
    CornerRange,
    UsageDisplay,
    QuotaAlert,
    RefreshInterval,
    SchemaVersion,
    UnsupportedLanguage,
    Theme,
    Layout,
    InvalidType,
    UnterminatedComment,
    GenericSyntax,
}

pub(super) fn json_error_phrase(language: LanguageId, phrase: JsonErrorPhrase) -> &'static str {
    use JsonErrorPhrase::*;
    match language {
        LanguageId::English => match phrase {
            Heading => "Configuration has errors",
            ControlCharacter => "A string contains an invalid control character. Check quotes, line breaks, and escapes.",
            UnknownSetting => "Unknown setting. Check the property name.",
            MissingSetting => "A required setting is missing.",
            DuplicateSetting => "A setting is defined more than once.",
            InvalidColor => "Invalid color format. Use #RRGGBB or #RRGGBBAA.",
            FrostedRange => "Value is out of range. Allowed range: 0–100.",
            CornerRange => "Value is out of range. Allowed range: 0–24.",
            UsageDisplay => "`session_5h` and `weekly` cannot both be false.",
            QuotaAlert => "Invalid value. Allowed values: 0, 10, 20, 30.",
            RefreshInterval => "Invalid value. Allowed values: 1m, 5m, 15m, 1h.",
            SchemaVersion => "Unsupported value.",
            UnsupportedLanguage => "Unsupported value.",
            Theme => "Invalid value. Allowed values: system, dark, light.",
            Layout => "Invalid value. Allowed values: default, minimal.",
            InvalidType => "A configuration value has the wrong type.",
            UnterminatedComment => "A block comment is not closed.",
            GenericSyntax => "Invalid JSON syntax or configuration value near this position.",
        },
        LanguageId::SimplifiedChinese => match phrase {
            Heading => "配置存在错误",
            ControlCharacter => "字符串中包含非法控制字符。请检查引号、换行或转义字符。",
            UnknownSetting => "存在未知配置项。请检查属性名是否拼写正确。",
            MissingSetting => "缺少必填配置项。",
            DuplicateSetting => "同一配置项被重复定义。",
            InvalidColor => "颜色格式无效。请使用 #RRGGBB 或 #RRGGBBAA。",
            FrostedRange => "值超出范围。允许范围：0–100。",
            CornerRange => "值超出范围。允许范围：0–24。",
            UsageDisplay => "`session_5h` 和 `weekly` 不能同时为 false。",
            QuotaAlert => "值无效。允许：0、10、20、30。",
            RefreshInterval => "值无效。允许：1m、5m、15m、1h。",
            SchemaVersion => "值不受支持。",
            UnsupportedLanguage => "值不受支持。",
            Theme => "值无效。允许：system、dark、light。",
            Layout => "值无效。允许：default、minimal。",
            InvalidType => "配置值类型不正确。",
            UnterminatedComment => "块注释未正确结束。",
            GenericSyntax => "此位置附近的 JSON 语法或配置值无效。",
        },
    }
}

pub(super) fn json_error_location_text(
    language: LanguageId, line: usize, column: usize, detail: &str,
) -> String {
    match language {
        LanguageId::English => format!("Line {line}, column {column}: {detail}"),
        LanguageId::SimplifiedChinese => format!("第 {line} 行，第 {column} 列：{detail}"),
    }
}

pub(super) fn json_error_status(language: LanguageId, detail: &str) -> String {
    let heading = json_error_phrase(language, JsonErrorPhrase::Heading);
    match language {
        LanguageId::SimplifiedChinese => format!("× {heading}：{detail}"),
        _ => format!("× {heading}: {detail}"),
    }
}

pub(super) fn json_error_key(error: &str) -> Option<String> {
    // Business validation errors use the exact dotted JSON path before the
    // first colon, e.g. "general.refresh_interval: allowed values ...".
    // Keep that path verbatim in localized UI so users can find the setting.
    let without_location = if let Some((_, detail)) = error.split_once(": ") {
        if error.starts_with("line ") {
            detail
        } else {
            error
        }
    } else {
        error
    };

    if let Some((candidate, _)) = without_location.split_once(':') {
        let candidate = candidate.trim();
        if !candidate.is_empty()
            && candidate
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
        {
            return candidate
                .rsplit('.')
                .next()
                .map(str::to_string);
        }
    }

    // serde field errors quote the actual JSON key with backticks.
    for marker in ["unknown field `", "missing field `", "duplicate field `"] {
        if let Some(rest) = error.split_once(marker).map(|(_, rest)| rest) {
            if let Some((key, _)) = rest.split_once('`') {
                if !key.is_empty() {
                    return Some(key.to_string());
                }
            }
        }
    }
    None
}

pub(super) fn localized_corner_range_detail(error: &str, language: LanguageId) -> Option<String> {
    let marker = "corner_radius: allowed range is 0-";
    let tail = error.split(marker).nth(1)?;
    let max: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if max.is_empty() {
        return None;
    }
    Some(if language == LanguageId::SimplifiedChinese {
        format!("值超出范围。允许范围：0–{max}。")
    } else {
        format!("Value is out of range. Allowed range: 0–{max}.")
    })
}

pub(super) fn friendly_json_error(error: &str, language: LanguageId) -> String {
    let location = json_error_location(error);
    let full = error;
    let phrase = if full.contains("control character") {
        JsonErrorPhrase::ControlCharacter
    } else if full.contains("unknown field") {
        JsonErrorPhrase::UnknownSetting
    } else if full.contains("missing field") {
        JsonErrorPhrase::MissingSetting
    } else if full.contains("duplicate field") {
        JsonErrorPhrase::DuplicateSetting
    } else if full.contains("expected #RRGGBB or #RRGGBBAA")
        || full.contains("invalid color")
    {
        JsonErrorPhrase::InvalidColor
    } else if full.contains("frosted_strength: allowed range is 0-100") {
        JsonErrorPhrase::FrostedRange
    } else if full.contains("corner_radius: allowed range is") {
        JsonErrorPhrase::CornerRange
    } else if full.contains("session_5h and weekly cannot both be false") {
        JsonErrorPhrase::UsageDisplay
    } else if full.contains("quota_alert_percent: allowed values") {
        JsonErrorPhrase::QuotaAlert
    } else if full.contains("refresh_interval: allowed values") {
        JsonErrorPhrase::RefreshInterval
    } else if full.contains("schema_version: expected") {
        JsonErrorPhrase::SchemaVersion
    } else if full.contains("general.language: unsupported language code") {
        JsonErrorPhrase::UnsupportedLanguage
    } else if full.contains("appearance.theme: allowed values") {
        JsonErrorPhrase::Theme
    } else if full.contains("appearance.layout: allowed values") {
        JsonErrorPhrase::Layout
    } else if full.contains("invalid type") {
        JsonErrorPhrase::InvalidType
    } else if full.contains("unterminated block comment") {
        JsonErrorPhrase::UnterminatedComment
    } else {
        JsonErrorPhrase::GenericSyntax
    };

    let localized = if matches!(phrase, JsonErrorPhrase::CornerRange) {
        localized_corner_range_detail(error, language)
            .unwrap_or_else(|| json_error_phrase(language, phrase).to_string())
    } else {
        json_error_phrase(language, phrase).to_string()
    };
    let detail = if let Some(key) = json_error_key(error) {
        match language {
            LanguageId::SimplifiedChinese => format!("`{key}`：{localized}"),
            _ => format!("`{key}`: {localized}"),
        }
    } else {
        localized
    };

    if let Some((line, column)) = location {
        json_error_location_text(language, line, column, &detail)
    } else {
        detail
    }
}

pub(super) fn json_action_error(action_zh: &str, action_en: &str, error: &str, language: LanguageId) -> String {
    let detail = friendly_json_error(error, language);
    match language {
        LanguageId::SimplifiedChinese => format!("× {action_zh}：{detail}"),
        LanguageId::English => format!("× {action_en}: {detail}"),
    }
}

pub(super) fn json_action_running_text(action: JsonAction, language: LanguageId) -> &'static str {
    let zh = language == LanguageId::SimplifiedChinese;
    match action {
        JsonAction::Reload => if zh { "◌ 载入中" } else { "◌ Reloading" },
        JsonAction::Format => if zh { "◌ 格式化中" } else { "◌ Formatting" },
        JsonAction::Import => if zh { "◌ 导入中" } else { "◌ Importing" },
        JsonAction::Export => if zh { "◌ 导出中" } else { "◌ Exporting" },
        JsonAction::Apply => if zh { "◌ 应用中" } else { "◌ Applying" },
    }
}

/// Once RichEdit text changes, previous successful operations no longer
/// describe the current buffer. Keep errors for the validation debounce.
pub(super) fn json_success_status_is_stale_on_edit(status: &str) -> bool {
    status.starts_with('✓')
}

/// Imported files are unedited but may differ from the applied runtime settings.
pub(super) fn json_has_pending_changes(dirty: bool, import_pending_apply: bool) -> bool {
    dirty || import_pending_apply
}

pub(super) fn json_action_enabled(
    action: JsonAction, dirty: bool, import_pending_apply: bool,
) -> bool {
    match action {
        JsonAction::Apply => json_has_pending_changes(dirty, import_pending_apply),
        JsonAction::Export => !json_has_pending_changes(dirty, import_pending_apply),
        _ => true,
    }
}

pub(super) fn json_action_needs_confirmation(
    action: JsonAction, dirty: bool, import_pending_apply: bool,
) -> bool {
    json_has_pending_changes(dirty, import_pending_apply)
        && matches!(action, JsonAction::Reload | JsonAction::Import)
}

pub(super) fn export_default_filename() -> String {
    let time = unsafe { GetLocalTime() };
    format!(
        "codex-usage-win-config-{:04}{:02}{:02}-{:02}{:02}{:02}.json",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute, time.wSecond
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_user_edit_clears_stale_success_statuses_but_not_errors() {
        for status in [
            "✓ 已重新载入", "✓ 已格式化", "✓ 成功导入",
            "✓ 成功导出", "✓ Reloaded", "✓ Formatted",
            "✓ Imported successfully", "✓ Exported successfully",
        ] {
            assert!(json_success_status_is_stale_on_edit(status), "{status}");
        }
        for status in [
            "", "× JSON 格式错误", "× Invalid JSON", "◌ 导入中",
            "◌ Importing", "● 有未保存更改",
        ] {
            assert!(!json_success_status_is_stale_on_edit(status), "{status}");
        }
    }

    #[test]
    fn json_actions_guard_export_and_confirm_destructive_reload_or_import() {
        for action in [JsonAction::Reload, JsonAction::Import] {
            assert!(!json_action_needs_confirmation(action, false, false));
            assert!(json_action_needs_confirmation(action, true, false));
            assert!(json_action_needs_confirmation(action, false, true));
        }
        for (dirty, imported, export_allowed, apply_allowed) in [
            (false, false, true, false),
            (true, false, false, true),
            (false, true, false, true),
            (true, true, false, true),
        ] {
            assert_eq!(json_action_enabled(JsonAction::Export, dirty, imported), export_allowed);
            assert_eq!(json_action_enabled(JsonAction::Apply, dirty, imported), apply_allowed);
            assert!(!json_action_needs_confirmation(JsonAction::Export, dirty, imported));
        }
    }

    #[test]
    fn newline_normalization_preserves_json_content() {
        assert_eq!(normalize_to_lf("{\r\n  \"key\": 1\r}\n"), "{\n  \"key\": 1\n}\n");
        assert_eq!(to_windows_newlines("{\r\n  \"key\": 1\r}\n"), "{\r\n  \"key\": 1\r\n}\r\n");
    }

    #[test]
    fn jsonc_highlighting_uses_utf16_offsets_for_non_bmp_keys() {
        let ranges = jsonc_highlight_ranges("{\"🧪\":12}\n// note");
        assert!(ranges.contains(&(1, 5, JsonTokenKind::Key)));
        assert!(ranges.contains(&(6, 8, JsonTokenKind::Number)));
        assert!(ranges.iter().any(|(_, _, kind)| *kind == JsonTokenKind::Comment));
    }

    #[test]
    fn localized_json_errors_keep_keys_and_precise_locations() {
        let error = "line 2, column 4: unknown field `missing_setting`";
        let zh = friendly_json_error(error, LanguageId::SimplifiedChinese);
        let en = friendly_json_error(error, LanguageId::English);
        assert!(zh.contains("第 2 行，第 4 列"));
        assert!(en.contains("Line 2, column 4"));
        assert!(zh.contains("`missing_setting`"));
        assert!(en.contains("`missing_setting`"));
    }
}
