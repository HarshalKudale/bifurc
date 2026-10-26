//! The engine's event bus. Every module emits here; the transports read from it (through the event log) and
//! a few modules listen to each other through it.
//!
//! Bus names carry no `event.` prefix (`log.entry`); the wire adds it. `process.statusChange` is a bus event
//! with no wire name.

use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

use parking_lot::Mutex;
use serde_json::Value;

pub const ENGINE_EVENT_NAMES: [&str; 46] = [
    "request.stream",
    "flow.progress",
    "flow.finished",
    "flow.loadtest.progress",
    "flow.loadtest.finished",
    "sync.status",
    "sync.entityStatus",
    "history.changed",
    "policy.changed",
    "log.entry",
    "log.chunk",
    "log.message",
    "server.error",
    "entity.changed",
    "process.output",
    "process.statusChange",
    "settings.changed",
    "runner.progress",
    "runner.finished",
    "ws.status",
    "ws.message",
    "grpc.stream.status",
    "grpc.stream.message",
    "sse.status",
    "sse.message",
    "socketio.status",
    "socketio.message",
    "mcp.status",
    "mcp.message",
    "engine.stopping",
    "ui.activate",
    "vault.changed",
    "cookies.changed",
    "account.changed",
    "companions.changed",
    "update.changed",
    "update.progress",
    "approval.requested",
    "approval.decided",
    "faults.changed",
    "cassette.changed",
    "mail.changed",
    "devservice.changed",
    "devservice.output",
    "webhook.changed",
    "claims.changed",
];

type Listener = Arc<dyn Fn(&str, &Value) + Send + Sync>;

struct Entry {
    id: u64,
    /// `None` listens to every event.
    event: Option<String>,
    listener: Listener,
}

#[derive(Default)]
pub struct EngineEventBus {
    listeners: Mutex<Vec<Entry>>,
    next_id: AtomicU64,
}

/// What [`EngineEventBus::on`] returns; pass it to [`EngineEventBus::off`] to stop listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenerId(u64);

impl EngineEventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deliver `payload` to every listener of `event`, synchronously, in registration order.
    pub fn emit(&self, event: &str, payload: Value) {
        let targets: Vec<Listener> = self
            .listeners
            .lock()
            .iter()
            .filter(|entry| entry.event.as_deref().is_none_or(|name| name == event))
            .map(|entry| entry.listener.clone())
            .collect();
        for listener in targets {
            listener(event, &payload);
        }
    }

    pub fn on(&self, event: &str, listener: impl Fn(&Value) + Send + Sync + 'static) -> ListenerId {
        self.add(Some(event.to_string()), Arc::new(move |_, payload| listener(payload)))
    }

    pub fn on_any(&self, listener: impl Fn(&str, &Value) + Send + Sync + 'static) -> ListenerId {
        self.add(None, Arc::new(listener))
    }

    pub fn off(&self, id: ListenerId) {
        self.listeners.lock().retain(|entry| entry.id != id.0);
    }

    fn add(&self, event: Option<String>, listener: Listener) -> ListenerId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.listeners.lock().push(Entry { id, event, listener });
        ListenerId(id)
    }
}

/// The process-wide bus.
pub fn bus() -> &'static EngineEventBus {
    static BUS: OnceLock<EngineEventBus> = OnceLock::new();
    BUS.get_or_init(EngineEventBus::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn listeners_receive_only_their_event_until_removed() {
        let bus = EngineEventBus::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        let id = bus.on("log.entry", move |payload| sink.lock().push(payload.clone()));
        bus.emit("log.entry", json!(1));
        bus.emit("ws.status", json!(2));
        bus.off(id);
        bus.emit("log.entry", json!(3));
        assert_eq!(*seen.lock(), vec![json!(1)]);
    }
}
