//! The engine's whole command surface, registered in one call. A command the protocol declares and no
//! module registers answers `UNKNOWN_COMMAND`; `tests::every_protocol_command_is_registered` keeps the two
//! lists equal.

use crate::kernel::CommandRegistry;

/// Register every engine command except `engine.shutdown` and `engine.companions`, which belong to the process (`runtime::host`).
pub fn register_all_engine_commands(registry: &mut CommandRegistry) {
    crate::workspace::blob::register(registry);
    crate::importexport::register(registry);
    crate::fileops::register(registry);
    crate::certificates::register(registry);
    crate::proxy::register(registry);
    crate::workspace::entities::register(registry);
    crate::workspace::response_history::register(registry);
    crate::rest::register(registry);
    crate::scripting::register(registry);
    crate::services::register(registry);
    crate::env::register(registry);
    crate::runner::register(registry);
    crate::flows::register(registry);
    crate::graphql::register(registry);
    crate::soap::register(registry);
    crate::grpc::register(registry);
    crate::sse::register(registry);
    crate::socketio::register(registry);
    crate::mcp::register(registry);
    crate::gitops::register(registry);
    crate::websocket::register(registry);
    crate::capture::register(registry);
    crate::vault::register(registry);
    crate::cookies::register(registry);
    crate::updates::register(registry);
}

/// The protocol commands no module has registered.
pub fn unregistered_commands(registry: &CommandRegistry) -> Vec<&'static str> {
    crate::protocol::schema::command_names()
        .into_iter()
        .filter(|name| *name != "engine.shutdown" && *name != "engine.companions" && !registry.is_registered(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_protocol_command_is_registered() {
        let mut registry = CommandRegistry::new();
        register_all_engine_commands(&mut registry);
        let missing = unregistered_commands(&registry);
        assert!(missing.is_empty(), "{} protocol commands have no handler: {missing:?}", missing.len());
    }
}
