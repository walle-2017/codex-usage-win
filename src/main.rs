#![windows_subsystem = "windows"]

mod appearance;
mod diagnose;
mod localization;
mod models;
mod native_interop;
mod poller;
mod system_proxy;
mod style;
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
    diagnose::log("entering window::run");
    window::run();
}
