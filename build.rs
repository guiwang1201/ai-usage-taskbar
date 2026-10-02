#[cfg(windows)]
mod build_resources;

fn main() {
    // Layered child windows need the Windows 8+ compatibility declaration, and the
    // widget must be per-monitor DPI aware to line up with the taskbar.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        {
            use embed_manifest::manifest::DpiAwareness;
            let manifest = embed_manifest::new_manifest("IUsage").dpi_awareness(DpiAwareness::PerMonitorV2).to_string();
            let icon = std::fs::read("assets/iusage.ico").expect("unable to read application icon");
            let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
            let object = build_resources::write(&icon, manifest.as_bytes(), &arch);
            let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("iusage-resources.o");
            std::fs::write(&path, object).expect("unable to write application resources");
            println!("cargo:rustc-link-arg-bins={}", path.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_resources.rs");
    println!("cargo:rerun-if-changed=assets/iusage.ico");
}
