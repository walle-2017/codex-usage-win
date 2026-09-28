use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CreateCompatibleDC, DeleteDC, EnumFontFamiliesW, LOGFONTW, TEXTMETRICW,
};

use crate::localization::LanguageId;

pub const JETBRAINS_MONO_FACE: &str = "JetBrains Mono";
pub const SEGOE_UI_FACE: &str = "Segoe UI";
pub const MICROSOFT_YAHEI_UI_FACE: &str = "Microsoft YaHei UI";
const SANS_SERIF_FALLBACK_FACE: &str = "Arial";
const MONO_FALLBACK_FACE: &str = "Consolas";

static MONO_INITIALIZED: OnceLock<bool> = OnceLock::new();
static SYSTEM_UI_FACE: OnceLock<&'static str> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontRole {
    /// General application UI such as menus, settings panels and compact tooltips.
    Ui,
    /// Small taskbar widget text. Uses the same Windows-native family as Ui,
    /// while its renderer remains the dedicated DirectWrite path.
    Taskbar,
    /// Monospaced editor/code text such as the JSON editor.
    Mono,
}

static JETBRAINS_MONO_FONT: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono.ttf");

fn register_font(bytes: &'static [u8]) -> bool {
    unsafe {
        let count = 0u32;
        let handle = AddFontMemResourceEx(
            bytes.as_ptr().cast::<c_void>(),
            bytes.len() as u32,
            None,
            &count,
        );
        !handle.0.is_null() && count > 0
    }
}

unsafe extern "system" fn mark_font_found(
    _log_font: *const LOGFONTW,
    _text_metric: *const TEXTMETRICW,
    _font_type: u32,
    lparam: LPARAM,
) -> i32 {
    let found = lparam.0 as *mut bool;
    if !found.is_null() {
        *found = true;
    }
    0
}

fn font_available(face: &str) -> bool {
    unsafe {
        let hdc = CreateCompatibleDC(None);
        if hdc.0.is_null() {
            return false;
        }

        let mut found = false;
        let wide: Vec<u16> = face.encode_utf16().chain(std::iter::once(0)).collect();
        let _ = EnumFontFamiliesW(
            hdc,
            PCWSTR::from_raw(wide.as_ptr()),
            Some(mark_font_found),
            LPARAM((&mut found as *mut bool) as isize),
        );
        let _ = DeleteDC(hdc);
        found
    }
}

fn system_ui_face() -> &'static str {
    SYSTEM_UI_FACE.get_or_init(|| {
        [SEGOE_UI_FACE, MICROSOFT_YAHEI_UI_FACE]
            .into_iter()
            .find(|face| font_available(face))
            .unwrap_or(SANS_SERIF_FALLBACK_FACE)
    })
}

pub fn init() -> bool {
    *MONO_INITIALIZED.get_or_init(|| register_font(JETBRAINS_MONO_FONT))
}

fn mono_role_face() -> &'static str {
    if init() {
        JETBRAINS_MONO_FACE
    } else {
        MONO_FALLBACK_FACE
    }
}

/// Resolve the font family for a semantic UI role.
///
/// Ui and Taskbar deliberately share one Windows-native family stack. The
/// language parameter is retained at call sites so future script-specific
/// fallback can be added without changing the public role API.
pub fn face(role: FontRole, _language: Option<LanguageId>) -> &'static str {
    match role {
        FontRole::Ui | FontRole::Taskbar => system_ui_face(),
        FontRole::Mono => mono_role_face(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_and_taskbar_roles_share_the_same_native_family() {
        for language in [
            None,
            Some(LanguageId::English),
            Some(LanguageId::SimplifiedChinese),
        ] {
            assert_eq!(
                face(FontRole::Ui, language),
                face(FontRole::Taskbar, language)
            );
        }
    }

    #[test]
    fn embedded_mono_font_remains_small() {
        assert!(
            JETBRAINS_MONO_FONT.len() < 1024 * 1024,
            "embedded mono font unexpectedly exceeds 1 MiB"
        );
    }
}
