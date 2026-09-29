fn main() {
    println!("cargo:rerun-if-env-changed=STATIC_VCRUNTIME");
    let mut attributes = tauri_build::Attributes::new();
    // Older launchers may still inject this deprecated variable. Preserve its
    // existing semantics through the supported API before Tauri reads the env.
    if let Some(value) = std::env::var_os("STATIC_VCRUNTIME") {
        attributes = attributes.windows_attributes(
            tauri_build::WindowsAttributes::new().static_vc_runtime(value != "false"),
        );
        // This build script is single-threaded; only its own process is changed.
        std::env::remove_var("STATIC_VCRUNTIME");
    }
    tauri_build::try_build(attributes).expect("failed to build Tauri application");
}
