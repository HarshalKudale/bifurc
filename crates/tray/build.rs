//! Compiles the app icon into `bifurc-tray.exe` on Windows, for Task Manager and Explorer. The icon in the
//! notification area is a PNG the tray loads at run time, which Windows does not use for the process.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if let Err(problem) = embed_icon() {
        println!("cargo:warning=the tray icon was not embedded, so Task Manager shows a blank one: {problem}");
    }
}

/// Compiles the app icon into the executable as icon resource 1.
#[cfg(windows)]
fn embed_icon() -> Result<(), String> {
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon_with_id(&icon.to_string_lossy(), "1");
    resource.set("ProductName", "Bifurc");
    resource.set("FileDescription", "Bifurc Tray");
    resource.compile().map_err(|error| error.to_string())
}
