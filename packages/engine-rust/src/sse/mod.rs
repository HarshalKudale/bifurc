//! The SSE request type: the engine opens the outbound `text/event-stream` connection to the user's server,
//! keeps the event log and reports status and events as `sse.status` / `sse.message` events.

mod commands;
mod parser;
mod registry;

use crate::kernel::CommandRegistry;

pub use registry::{MAX_SSE_MESSAGES, MAX_SSE_STREAMS};

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
}
