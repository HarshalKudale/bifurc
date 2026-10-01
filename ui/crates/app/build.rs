//! Gives the executable's main thread a larger stack on Windows.
//!
//! GPUI lays out and paints the element tree by recursion on the main thread, and Windows gives that thread
//! 1 MB where Linux and macOS give it 8 MB. A deep enough tree (an editor section inside the docks inside
//! the shell) runs out in an unoptimized build. The size is a link-time property of the executable.

const MAIN_THREAD_STACK_BYTES: usize = 8 * 1024 * 1024;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:{MAIN_THREAD_STACK_BYTES}");
    } else {
        println!("cargo:rustc-link-arg-bins=-Wl,--stack,{MAIN_THREAD_STACK_BYTES}");
    }
}
