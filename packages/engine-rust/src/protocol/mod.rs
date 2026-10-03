//! The engine<->client wire protocol: versions, the handshake vocabulary, event names, errors and the
//! command schemas. The source of truth for the shapes is `packages/protocol`.

pub mod errors;
pub mod schema;

pub use errors::{EngineError, ErrorCode, Result};

pub const PROTOCOL_VERSION: &str = "1.0.0";

/// The session opener, intercepted by the transport before the command registry sees it.
pub const HELLO_ACTION: &str = "hello";

/// Transport control actions; never commands.
pub const RESERVED_ACTIONS: [&str; 2] = ["subscribe", "unsubscribe"];

pub const CLIENT_NAMES: [&str; 5] = ["electron-shell", "web-ui", "desktop-ui", "cli", "bifurc-extension"];

pub mod capability {
    pub const BLOB: &str = "blob";
    pub const STDIO: &str = "stdio";
    pub const SOCKET: &str = "socket";
    pub const WS: &str = "ws";
    pub const EVENTS_REPLAY: &str = "events.replay";
    pub const TLS_INSTALL: &str = "tls.install";
}

/// Every event that travels on the wire. A bus event is named without the `event.` prefix.
pub const EVENT_NAMES: [&str; 16] = [
    "event.sync.status",
    "event.sync.entityStatus",
    "event.log.entry",
    "event.log.chunk",
    "event.server.error",
    "event.entity.changed",
    "event.process.output",
    "event.settings.changed",
    "event.runner.progress",
    "event.runner.finished",
    "event.ws.status",
    "event.ws.message",
    "event.grpc.stream.status",
    "event.grpc.stream.message",
    "event.engine.stopping",
    "event.ui.activate",
];

pub fn is_wire_event(wire: &str) -> bool {
    EVENT_NAMES.contains(&wire)
}

/// The bus name of a wire event (`event.log.entry` -> `log.entry`).
pub fn bus_name_for(wire: &str) -> Option<&str> {
    if is_wire_event(wire) { wire.strip_prefix("event.") } else { None }
}

/// The wire name of a bus event, if that event is on the wire.
pub fn wire_name_for(bus: &str) -> Option<&'static str> {
    EVENT_NAMES.iter().copied().find(|wire| wire.strip_prefix("event.") == Some(bus))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionCompatibility {
    Exact,
    Compatible,
    Incompatible,
}

fn parse_version(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.').map(|part| {
        let digits: String = part.trim().chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u64>().unwrap_or(0)
    });
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

pub fn check_version_compatibility(client: &str, engine: &str) -> VersionCompatibility {
    let (client, engine) = (parse_version(client), parse_version(engine));
    if client.0 != engine.0 {
        VersionCompatibility::Incompatible
    } else if client.1 != engine.1 {
        VersionCompatibility::Compatible
    } else {
        VersionCompatibility::Exact
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_map_both_ways() {
        assert_eq!(bus_name_for("event.log.entry"), Some("log.entry"));
        assert_eq!(bus_name_for("event.nope"), None);
        assert_eq!(wire_name_for("ws.message"), Some("event.ws.message"));
        assert_eq!(wire_name_for("process.statusChange"), None);
    }

    #[test]
    fn major_mismatch_is_incompatible() {
        assert_eq!(check_version_compatibility("1.0.0", "1.0.0"), VersionCompatibility::Exact);
        assert_eq!(check_version_compatibility("1.2.0", "1.0.0"), VersionCompatibility::Compatible);
        assert_eq!(check_version_compatibility("2.0.0", "1.0.0"), VersionCompatibility::Incompatible);
    }
}
