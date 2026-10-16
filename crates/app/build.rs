//! Gives the executable's main thread a larger stack on Windows.
//!
//! GPUI lays out and paints the element tree by recursion on the main thread, and Windows gives that thread
//! 1 MB where Linux and macOS give it 8 MB. A deep enough tree (an editor section inside the docks inside
//! the shell) runs out in an unoptimized build. The size is a link-time property of the executable.
//!
//! The tray helper (`bifurc-tray`) is its own workspace binary, built by `cargo xtask package` and
//! `cargo dev:ui` (`cargo xtask dev-ui`), not here.

use std::env;
#[cfg(windows)]
use std::path::Path;

const MAIN_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os == "windows" {
        if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-bins=/STACK:{MAIN_THREAD_STACK_BYTES}");
        } else {
            println!("cargo:rustc-link-arg-bins=-Wl,--stack,{MAIN_THREAD_STACK_BYTES}");
        }
    }
    #[cfg(windows)]
    if let Err(problem) = embed_icon() {
        println!("cargo:warning=the app icon was not embedded, so the taskbar shows a blank one: {problem}");
    }
}

/// Compiles the app icon into the executable as icon resource 1.
///
/// GPUI gives every window the icon it loads from resource 1 of its own module, and Explorer and the
/// taskbar show an executable's first icon — with no resource there is nothing for either to show.
#[cfg(windows)]
fn embed_icon() -> Result<(), String> {
    let icon = Path::new(&env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?).join("../../build/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon_with_id(&icon.to_string_lossy(), "1");
    resource.set("ProductName", "Bifurc");
    resource.set("FileDescription", "Bifurc");
    resource.compile().map_err(|error| error.to_string())
}
