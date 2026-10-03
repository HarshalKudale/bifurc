//! The WebSocket request type: the engine opens the outbound connection to the user's server, keeps the
//! message log and reports status and messages as `ws.status` / `ws.message` events.

mod commands;
mod registry;

use crate::kernel::CommandRegistry;

pub use registry::{MAX_WS_CONNECTIONS, MAX_WS_MESSAGES};

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
}
