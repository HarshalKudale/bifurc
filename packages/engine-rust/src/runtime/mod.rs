//! The engine as a process: its arguments, how clients find it, the listeners it runs and its lifetime.

pub mod args;
pub mod desktop;
pub mod discovery;
pub mod entity_status;
pub mod host;
pub mod instances;
pub mod ports;

use std::sync::atomic::{AtomicBool, Ordering};

static NOTICES_ON_STDERR: AtomicBool = AtomicBool::new(false);

/// Send notices to stderr: with the stdio transport, stdout carries frames.
pub fn route_notices_to_stderr() {
    NOTICES_ON_STDERR.store(true, Ordering::Relaxed);
}

/// Tell whoever started the engine something they may need to act on.
pub fn notice(message: &str) {
    if NOTICES_ON_STDERR.load(Ordering::Relaxed) {
        eprintln!("[host] {message}");
    } else {
        println!("[host] {message}");
    }
}
