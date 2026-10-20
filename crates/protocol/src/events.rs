//! The event model: wire names, the sequenced envelope and the resync signal.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Every event that travels on the wire. A bus event is named without the `event.` prefix.
pub const EVENT_NAMES: [&str; 44] = [
    "event.request.stream",
    "event.flow.progress",
    "event.flow.finished",
    "event.flow.loadtest.progress",
    "event.flow.loadtest.finished",
    "event.sync.status",
    "event.sync.entityStatus",
    "event.history.changed",
    "event.policy.changed",
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
    "event.sse.status",
    "event.sse.message",
    "event.socketio.status",
    "event.socketio.message",
    "event.mcp.status",
    "event.mcp.message",
    "event.engine.stopping",
    "event.ui.activate",
    "event.vault.changed",
    "event.cookies.changed",
    "event.account.changed",
    "event.companions.changed",
    "event.update.changed",
    "event.update.progress",
    "event.approval.requested",
    "event.approval.decided",
    "event.faults.changed",
    "event.cassette.changed",
    "event.mail.changed",
    "event.devservice.changed",
    "event.devservice.output",
    "event.webhook.changed",
    "event.claims.changed",
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

/// Every event carries a monotonic `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub event: String,
    pub seq: u64,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResyncReason {
    BufferOverrun,
    UnknownSeq,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResyncRequiredEvent {
    pub event: String,
    pub reason: ResyncReason,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_names_map_both_ways() {
        assert_eq!(bus_name_for("event.log.entry"), Some("log.entry"));
        assert_eq!(bus_name_for("event.nope"), None);
        assert_eq!(wire_name_for("ws.message"), Some("event.ws.message"));
        assert_eq!(wire_name_for("process.statusChange"), None);
    }

    #[test]
    fn envelope_round_trips() {
        let frame = json!({ "event": "event.log.entry", "seq": 3, "payload": { "a": 1 } });
        let parsed: EventEnvelope = serde_json::from_value(frame.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), frame);
        assert!(serde_json::from_value::<EventEnvelope>(json!({ "event": "e", "seq": -1, "payload": null })).is_err());
    }

    #[test]
    fn resync_reasons_are_snake_case() {
        let event = ResyncRequiredEvent { event: "event.x".into(), reason: ResyncReason::BufferOverrun };
        assert_eq!(serde_json::to_value(event).unwrap()["reason"], "buffer_overrun");
    }
}
