fn main() {
    // Embed Info.plist (NSBluetoothAlwaysUsageDescription) into the raw binary too, for when the
    // unbundled executable is its own TCC-responsible process. The bundled .app gets it anyway.
    // Note: under `tauri dev` macOS checks the *launching* app (your terminal) instead, and aborts
    // the process if that app has no Bluetooth usage description.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Info.plist");
        println!("cargo:rerun-if-changed={}", plist.display());
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}", plist.display());
    }
    tauri_build::build()
}
