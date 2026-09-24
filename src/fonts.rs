use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Gdi::{
    AddFontMemResourceEx, CreateCompatibleDC, DeleteDC, EnumFontFamiliesW, LOGFONTW, TEXTMETRICW,
};

use crate::localization::LanguageId;

pub const INTER_FACE: &str = "Inter Variable";
pub const NOTO_SANS_SC_FACE: &str = "Noto Sans SC";
pub const JETBRAINS_MONO_FACE: &str = "JetBrains Mono";
pub const SEGOE_UI_FACE: &str = "Segoe UI";
pub const MICROSOFT_YAHEI_UI_FACE: &str = "Microsoft YaHei UI";
const SANS_SERIF_FALLBACK_FACE: &str = "Arial";

static INITIALIZED: OnceLock<bool> = OnceLock::new();
static TASKBAR_WIDGET_FACE: OnceLock<&'static str> = OnceLock::new();

static INTER_FONT: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
static NOTO_SANS_SC_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-UI.ttf");
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

pub fn init() -> bool {
    *INITIALIZED.get_or_init(|| {
        register_font(INTER_FONT)
            && register_font(NOTO_SANS_SC_FONT)
            && register_font(JETBRAINS_MONO_FONT)
    })
}

pub fn ui_face(language: LanguageId) -> &'static str {
    if !init() {
        return "Segoe UI";
    }
    match language {
        LanguageId::SimplifiedChinese => NOTO_SANS_SC_FACE,
        _ => INTER_FACE,
    }
}

pub fn taskbar_face() -> &'static str {
    if init() {
        INTER_FACE
    } else {
        "Segoe UI"
    }
}

pub fn taskbar_widget_face() -> &'static str {
    TASKBAR_WIDGET_FACE.get_or_init(|| {
        [SEGOE_UI_FACE, MICROSOFT_YAHEI_UI_FACE]
            .into_iter()
            .find(|face| font_available(face))
            .unwrap_or(SANS_SERIF_FALLBACK_FACE)
    })
}

pub fn mono_face() -> &'static str {
    if init() {
        JETBRAINS_MONO_FACE
    } else {
        "Consolas"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_font_assets_remain_small() {
        let total = INTER_FONT.len() + NOTO_SANS_SC_FONT.len() + JETBRAINS_MONO_FONT.len();
        assert!(
            total <= 3 * 1024 * 1024,
            "embedded fonts unexpectedly exceed 3 MiB: {total}"
        );
        assert!(NOTO_SANS_SC_FONT.len() < 1024 * 1024);
    }
}
