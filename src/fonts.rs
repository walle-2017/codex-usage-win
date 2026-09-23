use std::ffi::c_void;
use std::sync::OnceLock;

use windows::Win32::Graphics::Gdi::AddFontMemResourceEx;

use crate::localization::LanguageId;

pub const INTER_FACE: &str = "Inter Variable";
pub const NOTO_SANS_SC_FACE: &str = "Noto Sans SC";
pub const JETBRAINS_MONO_FACE: &str = "JetBrains Mono";

static INITIALIZED: OnceLock<bool> = OnceLock::new();

static INTER_FONT: &[u8] = include_bytes!("../assets/fonts/InterVariable.ttf");
static NOTO_SANS_SC_FONT: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-UI.ttf");
static JETBRAINS_MONO_FONT: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono.ttf");

fn register_font(bytes: &'static [u8]) -> bool {
    unsafe {
        let mut count = 0u32;
        let handle = AddFontMemResourceEx(
            bytes.as_ptr().cast::<c_void>(),
            bytes.len() as u32,
            None,
            &mut count,
        );
        !handle.0.is_null() && count > 0
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
