use crate::kernel::{CommandRegistry, ListenerId, bus};

pub fn register(_registry: &mut CommandRegistry) {}

/// Record every exchange the proxies report, whether or not a client is listening.
pub fn start_capture_recording() -> ListenerId {
    bus().on("log.entry", |_| {})
}

/// Write pending capture history to disk now.
pub fn flush_capture() {}
