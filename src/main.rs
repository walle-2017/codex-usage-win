#![windows_subsystem = "windows"]

mod appearance;
mod diagnose;
mod fonts;
mod localization;
mod models;
mod native_interop;
mod poller;
mod popup_menu;
mod settings_model;
mod system_proxy;
mod style;
mod style_window;
mod theme;
mod tray_icon;
mod updater;
mod window;

fn main() {
    let diagnose_enabled = std::env::args().skip(1).any(|arg| arg == "--diagnose");
    if diagnose_enabled {
        let executable = std::env::current_exe()
            .map(|value| value.display().to_string())
            .unwrap_or_else(|error| format!("unavailable:{error}"));
        if let Ok(path) = diagnose::init() {
            diagnose::log(format!(
                "version={} executable={} log_path={}",
                env!("CARGO_PKG_VERSION"),
                executable,
                path.display()
            ));
        }
    }
    system_proxy::apply_windows_system_proxy_env();
    let bundled_fonts = fonts::init();
    diagnose::log(format!("bundled_fonts_registered={bundled_fonts}"));
    diagnose::log("entering window::run");
    window::run();
}
