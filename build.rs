use winres::{VersionInfo, WindowsResource};

fn main() {
    let version = env!("CARGO_PKG_VERSION");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cc::Build::new()
            .cpp(true)
            .file("native/composition_blur.cpp")
            .flag_if_supported("/std:c++20")
            .flag_if_supported("/EHsc")
            .compile("codex_composition_blur");

        println!("cargo:rerun-if-changed=native/composition_blur.cpp");
        println!("cargo:rustc-link-lib=windowsapp");
        println!("cargo:rustc-link-lib=CoreMessaging");
        println!("cargo:rustc-link-lib=runtimeobject");
        println!("cargo:rustc-link-lib=d2d1");
    }

    // Embed the icon and richer PE version metadata into the executable.
    let mut res = WindowsResource::new();
    let numeric_version = pack_version(version);

    res.set_icon("src/icons/icon.ico")
        .set("FileVersion", version)
        .set("ProductVersion", version)
        .set_version_info(VersionInfo::FILEVERSION, numeric_version)
        .set_version_info(VersionInfo::PRODUCTVERSION, numeric_version);

    res.compile().expect("Failed to compile Windows resources");
}

fn pack_version(version: &str) -> u64 {
    let core = version.split('-').next().unwrap_or(version);
    let mut parts = core.split('.').map(|part| part.parse::<u64>().unwrap_or(0));

    let major = parts.next().unwrap_or(0).min(u16::MAX as u64);
    let minor = parts.next().unwrap_or(0).min(u16::MAX as u64);
    let patch = parts.next().unwrap_or(0).min(u16::MAX as u64);

    (major << 48) | (minor << 32) | (patch << 16)
}
