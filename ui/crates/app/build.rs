//! Gives the executable's main thread a larger stack on Windows, and builds the engine's tray helper beside it.
//!
//! GPUI lays out and paints the element tree by recursion on the main thread, and Windows gives that thread
//! 1 MB where Linux and macOS give it 8 MB. A deep enough tree (an editor section inside the docks inside
//! the shell) runs out in an unoptimized build. The size is a link-time property of the executable.
//!
//! The tray helper (`bifurc-tray`, `../tray`) is a separate executable the engine runs, and the engine this
//! app starts is told to use the one beside the app. `cargo run -p bifurc-ui` builds this package alone, so
//! without this step there would be no helper there, the engine would have no icon, and nothing would be
//! left to reach it by once the window closed. So building the app builds the helper, whatever the command.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const MAIN_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:{MAIN_THREAD_STACK_BYTES}");
    } else {
        println!("cargo:rustc-link-arg-bins=-Wl,--stack,{MAIN_THREAD_STACK_BYTES}");
    }
    if let Err(problem) = build_tray_helper() {
        println!("cargo:warning=the tray helper was not built, so the engine will have no tray icon: {problem}");
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
    let icon = Path::new(&env::var("CARGO_MANIFEST_DIR").map_err(|e| e.to_string())?).join("../../../build/icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon_with_id(&icon.to_string_lossy(), "1");
    resource.set("ProductName", "Bifurc");
    resource.set("FileDescription", "Bifurc");
    resource.compile().map_err(|error| error.to_string())
}

/// Builds `bifurc-tray` with a cargo of its own and copies it next to where `bifurc-ui` is about to land.
///
/// It gets its own target directory: the cargo running this script holds the lock on the outer one. Always a
/// release build — the helper is a megabyte and has nothing to debug.
fn build_tray_helper() -> Result<(), String> {
    let var = |name: &str| env::var(name).map_err(|_| format!("{name} is not set"));
    let tray = Path::new(&var("CARGO_MANIFEST_DIR")?).join("..").join("tray");
    println!("cargo:rerun-if-changed={}", tray.join("src").display());
    println!("cargo:rerun-if-changed={}", tray.join("Cargo.toml").display());
    println!("cargo:rerun-if-changed={}", tray.join("../../../build/tray-icon.png").display());

    // OUT_DIR is `<target>/<profile>/build/<package>-<hash>/out`.
    let out_dir = PathBuf::from(var("OUT_DIR")?);
    let profile_dir = out_dir.ancestors().nth(3).ok_or("OUT_DIR has an unexpected shape")?.to_path_buf();
    let helper_target = profile_dir.join("tray-helper");
    let target = var("TARGET")?;

    let status = Command::new(var("CARGO")?)
        .args(["build", "--release", "--target", &target, "--manifest-path"])
        .arg(tray.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&helper_target)
        // What this cargo set for the app's own compilation is not the helper's.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_TARGET_DIR")
        .status()
        .map_err(|error| format!("could not run cargo: {error}"))?;
    if !status.success() {
        return Err(format!("cargo build of the helper failed ({status})"));
    }

    let name = "bifurc-tray.exe";
    let built = helper_target.join(&target).join("release").join(name);
    let beside_app = profile_dir.join(name);
    match fs::copy(&built, &beside_app) {
        Ok(_) => Ok(()),
        // A running engine holds the helper open. The one already there keeps working.
        Err(_) if beside_app.is_file() => Ok(()),
        Err(error) => Err(format!("could not copy {} to {}: {error}", built.display(), beside_app.display())),
    }
}
