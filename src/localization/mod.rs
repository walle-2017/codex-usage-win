mod english;
mod simplified_chinese;

use windows::core::PWSTR;
use windows::Win32::Globalization::{
    GetUserDefaultLocaleName, GetUserDefaultUILanguage, GetUserPreferredUILanguages,
    LCIDToLocaleName, LOCALE_ALLOW_NEUTRAL_NAMES, MAX_LOCALE_NAME, MUI_LANGUAGE_NAME,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanguageId {
    English,
    SimplifiedChinese,
}

impl LanguageId {
    pub const SELECTABLE: [LanguageId; 2] = [LanguageId::SimplifiedChinese, LanguageId::English];

    pub fn ui_supported(self) -> Self { self }

    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::SimplifiedChinese => "zh-CN",
        }
    }

    pub fn strings(self) -> Strings {
        match self {
            Self::English => english::STRINGS,
            Self::SimplifiedChinese => simplified_chinese::STRINGS,
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim().replace('_', "-").to_ascii_lowercase().split('-').next()? {
            "en" => Some(Self::English),
            "zh" => Some(Self::SimplifiedChinese),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LanguageId;

    #[test]
    fn only_two_languages_are_selectable_and_loaded() {
        assert_eq!(LanguageId::SELECTABLE, [LanguageId::SimplifiedChinese, LanguageId::English]);
        for locale in ["zh-CN", "zh_SG", "zh-Hans", "zh-TW", "zh-HK", "zh-Hant"] {
            assert_eq!(LanguageId::from_code(locale), Some(LanguageId::SimplifiedChinese));
        }
        assert_eq!(LanguageId::from_code("en-US"), Some(LanguageId::English));
        assert_eq!(LanguageId::from_code("ja-JP"), None);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Strings {
    pub window_title: &'static str,
    pub refresh: &'static str,
    pub update_frequency: &'static str,
    pub one_minute: &'static str,
    pub five_minutes: &'static str,
    pub fifteen_minutes: &'static str,
    pub one_hour: &'static str,
    pub codex_model: &'static str,
    pub start_with_windows: &'static str,
    pub reset_position: &'static str,
    pub language: &'static str,
    pub system_default: &'static str,
    pub exit: &'static str,
    pub session_window: &'static str,
    pub weekly_window: &'static str,
    pub now: &'static str,
    pub day_suffix: &'static str,
    pub hour_suffix: &'static str,
    pub minute_suffix: &'static str,
    pub second_suffix: &'static str,
    pub codex_token_expired_title: &'static str,
    pub codex_token_expired_body: &'static str,
    pub update_title: &'static str,
    pub update_checking: &'static str,
    pub update_downloading: &'static str,
    pub update_success: &'static str,
    pub update_current: &'static str,
    pub update_available: &'static str,
    pub update_check_failed: &'static str,
    pub update_download_failed: &'static str,
    pub update_checksum_failed: &'static str,
    pub update_target_not_writable: &'static str,
    pub update_helper_failed: &'static str,
}

pub fn resolve_language(language_override: Option<LanguageId>) -> LanguageId {
    language_override
        .map(LanguageId::ui_supported)
        .unwrap_or_else(detect_system_language)
}

pub fn detect_system_language() -> LanguageId {
    preferred_ui_languages()
        .into_iter()
        .find_map(|locale| LanguageId::from_code(&locale))
        .or_else(default_ui_locale)
        .or_else(default_locale_name)
        .unwrap_or(LanguageId::English)
        .ui_supported()
}

fn preferred_ui_languages() -> Vec<String> {
    unsafe {
        let mut num_languages = 0u32;
        let mut buffer_len = 0u32;
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut num_languages,
            PWSTR::null(),
            &mut buffer_len,
        )
        .is_err()
            || buffer_len == 0
        {
            return Vec::new();
        }

        let mut buffer = vec![0u16; buffer_len as usize];
        if GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut num_languages,
            PWSTR(buffer.as_mut_ptr()),
            &mut buffer_len,
        )
        .is_err()
        {
            return Vec::new();
        }

        buffer
            .split(|unit| *unit == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16_lossy)
            .collect()
    }
}

fn default_ui_locale() -> Option<LanguageId> {
    unsafe {
        let lang_id = GetUserDefaultUILanguage();
        let mut buffer = [0u16; MAX_LOCALE_NAME as usize];
        let len = LCIDToLocaleName(
            lang_id as u32,
            Some(&mut buffer),
            LOCALE_ALLOW_NEUTRAL_NAMES,
        );
        if len <= 1 {
            return None;
        }
        let locale = String::from_utf16_lossy(&buffer[..(len as usize - 1)]);
        LanguageId::from_code(&locale)
    }
}

fn default_locale_name() -> Option<LanguageId> {
    unsafe {
        let mut buffer = [0u16; MAX_LOCALE_NAME as usize];
        let len = GetUserDefaultLocaleName(&mut buffer);
        if len <= 1 {
            return None;
        }
        let locale = String::from_utf16_lossy(&buffer[..(len as usize - 1)]);
        LanguageId::from_code(&locale)
    }
}
