from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly 1 match, found {count}")
    return text.replace(old, new, 1)


def verify_regression_present() -> None:
    style = Path("src/style_window.rs").read_text(encoding="utf-8")
    window = Path("src/window.rs").read_text(encoding="utf-8")
    missing = []
    if "DWMWA_TRANSITIONS_FORCEDISABLED" not in style:
        missing.append("DWM title-bar transitions are still enabled")
    if "UpdateWindow(hwnd)" not in style:
        missing.append("style-panel repaint is still deferred")
    if "fn widget_text_quality()" not in window:
        missing.append("transparent widget text has no alpha-safe quality selector")
    if "NONANTIALIASED_QUALITY" not in window:
        missing.append("transparent widget text still relies on background-blended antialiasing")
    if not missing:
        raise SystemExit("Regression probe unexpectedly passed before the fix")
    print("RED confirmed: " + "; ".join(missing))


def patch_style_window() -> None:
    path = Path("src/style_window.rs")
    text = path.read_text(encoding="utf-8")
    text = replace_once(
        text,
        "use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};",
        "use windows::Win32::Graphics::Dwm::{\n    DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED, DWMWA_USE_IMMERSIVE_DARK_MODE,\n};",
        "DWM import",
    )

    old_apply = """fn apply_titlebar_theme(hwnd: HWND, is_dark: bool) {
    let enabled = BOOL::from(is_dark);
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &enabled as *const BOOL as *const std::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}
"""
    new_apply = """fn apply_titlebar_theme(hwnd: HWND, is_dark: bool) {
    let transitions_disabled = BOOL::from(true);
    let enabled = BOOL::from(is_dark);
    unsafe {
        // DWM otherwise animates the non-client title bar after the client area
        // has already repainted, which makes theme changes visibly two-stage.
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &transitions_disabled as *const BOOL as *const std::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &enabled as *const BOOL as *const std::ffi::c_void,
            std::mem::size_of::<BOOL>() as u32,
        );
    }
}
"""
    text = replace_once(text, old_apply, new_apply, "apply_titlebar_theme")

    old_sync = """    apply_titlebar_theme(hwnd, is_dark);
    layout_numeric_edits(hwnd);
    sync_numeric_edits();
    sync_blur_edit();
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
    }
}
"""
    new_sync = """    apply_titlebar_theme(hwnd, is_dark);
    layout_numeric_edits(hwnd);
    sync_numeric_edits();
    sync_blur_edit();
    unsafe {
        let _ = InvalidateRect(hwnd, None, false);
        // Commit the client-area theme in this UI turn so it lands together
        // with the now non-animated DWM title-bar update.
        let _ = UpdateWindow(hwnd);
    }
}
"""
    text = replace_once(text, old_sync, new_sync, "style sync tail")
    path.write_text(text, encoding="utf-8")


def patch_widget_text_rendering() -> None:
    path = Path("src/window.rs")
    text = path.read_text(encoding="utf-8")

    anchor = """fn sc(px: i32) -> i32 {
    let dpi = CURRENT_DPI.load(Ordering::Relaxed);
    (px as f64 * dpi as f64 / 96.0).round() as i32
}
"""
    helpers = anchor + """
fn text_quality_for_layered_surface(panel_alpha: u8, composition_blur_active: bool) -> u32 {
    if composition_blur_active || panel_alpha < u8::MAX {
        // ClearType/antialiased GDI glyph edges are pre-blended against the
        // panel RGB. The layered-window finalizer later promotes changed pixels
        // to opaque foreground, turning those edge blends into visible halos.
        NONANTIALIASED_QUALITY.0 as u32
    } else {
        CLEARTYPE_QUALITY.0 as u32
    }
}

fn widget_text_quality() -> u32 {
    let state = lock_state();
    let Some(s) = state.as_ref() else {
        return CLEARTYPE_QUALITY.0 as u32;
    };
    let panel_alpha = s
        .styles
        .active(s.is_dark)
        .color(StyleColorTarget::PanelBackground)
        .a;
    text_quality_for_layered_surface(panel_alpha, s.composition_blur_active)
}
"""
    text = replace_once(text, anchor, helpers, "text-quality helper anchor")

    marker = "/// Paint widget foreground."
    split_at = text.index(marker)
    prefix = text[:split_at]
    rendering = text[split_at:]
    count = rendering.count("CLEARTYPE_QUALITY.0 as u32")
    if count != 3:
        raise SystemExit(f"expected 3 widget ClearType rendering sites, found {count}")
    rendering = rendering.replace("CLEARTYPE_QUALITY.0 as u32", "widget_text_quality()")
    text = prefix + rendering

    tests_anchor = """    #[test]
    fn frosted_strength_maps_linearly_to_gaussian_radius() {
"""
    new_tests = """    #[test]
    fn transparent_layered_text_avoids_cleartype_background_fringe() {
        assert_eq!(
            text_quality_for_layered_surface(255, false),
            CLEARTYPE_QUALITY.0 as u32
        );
        assert_eq!(
            text_quality_for_layered_surface(254, false),
            NONANTIALIASED_QUALITY.0 as u32
        );
        assert_eq!(
            text_quality_for_layered_surface(255, true),
            NONANTIALIASED_QUALITY.0 as u32
        );
    }

""" + tests_anchor
    text = replace_once(text, tests_anchor, new_tests, "layered text regression test anchor")
    path.write_text(text, encoding="utf-8")


def patch_contract() -> None:
    path = Path("scripts/assert-style-customization.ps1")
    text = path.read_text(encoding="utf-8")
    marker = "# Theme-sync and layered-text regression contract"
    if marker in text:
        return
    text += r"""

# Theme-sync and layered-text regression contract
if ($styleWindow -notmatch 'DWMWA_TRANSITIONS_FORCEDISABLED' -or
    $styleWindow -notmatch 'UpdateWindow\(hwnd\)') {
    throw 'Style-panel client and title-bar theme changes must commit without a delayed DWM transition.'
}
if ($windowProduction -notmatch 'text_quality_for_layered_surface' -or
    $windowProduction -notmatch 'NONANTIALIASED_QUALITY' -or
    $windowProduction -notmatch 'widget_text_quality\(\)') {
    throw 'Transparent/frosted taskbar text must avoid ClearType background-fringe artifacts.'
}
"""
    path.write_text(text, encoding="utf-8")


if __name__ == "__main__":
    verify_regression_present()
    patch_style_window()
    patch_widget_text_rendering()
    patch_contract()
    print("Patch applied")
