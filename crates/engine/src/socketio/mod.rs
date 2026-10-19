//! The Socket.IO request type: the engine opens the outbound connection to the user's server, joins a
//! namespace, emits events, keeps the message log and reports status and messages as `socketio.status` /
//! `socketio.message` events.

mod commands;
mod packet;
mod registry;

use crate::kernel::CommandRegistry;

pub use registry::{DEFAULT_PATH, MAX_SOCKETIO_CONNECTIONS, MAX_SOCKETIO_MESSAGES};

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
}
