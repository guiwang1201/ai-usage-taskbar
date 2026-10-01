fn main() {
    // Layered child windows need the Windows 8+ compatibility declaration, and the
    // widget must be per-monitor DPI aware to line up with the taskbar.
    #[cfg(windows)]
    {
        use embed_manifest::manifest::DpiAwareness;
        use embed_manifest::{embed_manifest, new_manifest};
        embed_manifest(new_manifest("IUsage").dpi_awareness(DpiAwareness::PerMonitorV2))
            .expect("unable to embed manifest");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
