//! A WebSocket request's live connection, as the engine reports it.
//!
//! The connection belongs to the engine (it outlives this window), keyed by `{workspaceId, socketId}` where
//! `socketId` is the saved request's id. The UI holds a view: `ws.status` returns the whole snapshot with the
//! retained message log, and `event.ws.status` / `event.ws.message` keep it current.

use serde_json::Value;

use crate::i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SocketState {
    Connecting,
    Connected,
    #[default]
    Disconnected,
    Error,
}

impl SocketState {
    pub fn parse(text: &str) -> Self {
        match text {
            "connecting" => SocketState::Connecting,
            "connected" => SocketState::Connected,
            "error" => SocketState::Error,
            _ => SocketState::Disconnected,
        }
    }

    pub fn label(self) -> &'static str {
        t(match self {
            SocketState::Connecting => "model.state.connecting",
            SocketState::Connected => "model.state.connected",
            SocketState::Disconnected => "model.state.disconnected",
            SocketState::Error => "model.state.failed",
        })
    }

    /// A connection that is open or opening; the state in which Disconnect is the right command.
    pub fn is_live(self) -> bool {
        matches!(self, SocketState::Connecting | SocketState::Connected)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Sent,
    Received,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SocketMessage {
    pub id: String,
    pub ts: u64,
    pub direction: Direction,
    pub data: String,
}

impl SocketMessage {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            ts: value.get("ts").and_then(Value::as_u64).unwrap_or(0),
            direction: match value.get("direction")?.as_str()? {
                "sent" => Direction::Sent,
                _ => Direction::Received,
            },
            data: value.get("data").and_then(Value::as_str).unwrap_or_default().to_owned(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SocketView {
    pub state: SocketState,
    pub error: Option<String>,
    pub close_code: Option<u16>,
    pub messages: Vec<SocketMessage>,
}

/// The engine keeps at most this many messages per socket; the view mirrors the bound so a long session
/// cannot grow without limit on this side either.
pub const MAX_MESSAGES: usize = 500;

impl SocketView {
    /// From a `ws.status` result. `null` (no such socket yet) is a disconnected view with no history.
    pub fn from_status(result: &Value) -> Self {
        if result.is_null() {
            return Self::default();
        }
        Self {
            state: SocketState::parse(result.get("status").and_then(Value::as_str).unwrap_or("")),
            error: result.get("error").and_then(Value::as_str).map(str::to_owned),
            close_code: result.get("closeCode").and_then(Value::as_u64).and_then(|c| u16::try_from(c).ok()),
            messages: result
                .get("messages")
                .and_then(Value::as_array)
                .map(|m| m.iter().filter_map(SocketMessage::from_value).collect())
                .unwrap_or_default(),
        }
    }

    /// Applies an `event.ws.status` payload.
    pub fn apply_status_event(&mut self, payload: &Value) {
        self.state = SocketState::parse(payload.get("status").and_then(Value::as_str).unwrap_or(""));
        self.error = payload.get("error").and_then(Value::as_str).map(str::to_owned);
        self.close_code = payload.get("code").and_then(Value::as_u64).and_then(|c| u16::try_from(c).ok());
    }

    /// Applies an `event.ws.message` payload. A message already held (the same id) is not added twice: a
    /// client that attaches mid-stream can receive a message both in the snapshot and as an event.
    pub fn apply_message_event(&mut self, payload: &Value) {
        let Some(message) = payload.get("message").and_then(SocketMessage::from_value) else { return };
        if self.messages.iter().any(|m| m.id == message.id) {
            return;
        }
        self.messages.push(message);
        if self.messages.len() > MAX_MESSAGES {
            let excess = self.messages.len() - MAX_MESSAGES;
            self.messages.drain(..excess);
        }
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(id: &str, direction: &str) -> Value {
        json!({"id": id, "ts": 10, "direction": direction, "data": "hi"})
    }

    #[test]
    fn a_status_snapshot_carries_the_state_and_the_retained_log() {
        let view = SocketView::from_status(&json!({
            "status": "connected", "error": null, "messages": [message("s-1", "sent"), message("s-2", "received")],
        }));
        assert_eq!(view.state, SocketState::Connected);
        assert!(view.state.is_live());
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[1].direction, Direction::Received);
    }

    #[test]
    fn no_socket_yet_reads_as_disconnected_with_no_history() {
        let view = SocketView::from_status(&Value::Null);
        assert_eq!(view.state, SocketState::Disconnected);
        assert!(!view.state.is_live());
        assert!(view.messages.is_empty());
    }

    #[test]
    fn a_failed_close_keeps_the_reason_and_the_code() {
        let mut view = SocketView::default();
        view.apply_status_event(&json!({"status": "error", "error": "Connection closed (code 1006)", "code": 1006}));
        assert_eq!(view.state, SocketState::Error);
        assert_eq!(view.error.as_deref(), Some("Connection closed (code 1006)"));
        assert_eq!(view.close_code, Some(1006));
    }

    #[test]
    fn a_message_seen_in_the_snapshot_and_again_as_an_event_is_kept_once() {
        let mut view = SocketView::from_status(&json!({"status": "connected", "messages": [message("s-1", "sent")]}));
        view.apply_message_event(&json!({"message": message("s-1", "sent")}));
        view.apply_message_event(&json!({"message": message("s-2", "received")}));
        assert_eq!(view.messages.len(), 2);
    }

    #[test]
    fn the_log_is_bounded_like_the_engines() {
        let mut view = SocketView::default();
        for n in 0..(MAX_MESSAGES + 25) {
            view.apply_message_event(&json!({"message": message(&format!("s-{n}"), "received")}));
        }
        assert_eq!(view.messages.len(), MAX_MESSAGES);
        assert_eq!(view.messages[0].id, "s-25");
    }
}
