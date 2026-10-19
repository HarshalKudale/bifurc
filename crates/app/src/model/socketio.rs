//! A Socket.IO request's live connection, as the engine reports it.
//!
//! The connection belongs to the engine (it outlives this window), keyed by `{workspaceId, connectionId}` where
//! `connectionId` is the saved request's id. The UI holds a view: `socketio.status` returns the whole snapshot
//! with the retained message log, and `event.socketio.status` / `event.socketio.message` keep it current.
//!
//! The states and directions are the WebSocket's ([`SocketState`], [`Direction`]); what differs is the message,
//! which is a named event with arguments, or the acknowledgement of one.

use crate::engine::error_text;
use serde_json::Value;

use super::socket::{Direction, SocketState};
use crate::i18n::{t, t_with};

/// What a line of the log is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Event,
    /// The acknowledgement of an event, which carries what the other side answered with.
    Ack,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SocketIoMessage {
    pub id: String,
    pub ts: u64,
    pub direction: Direction,
    pub kind: MessageKind,
    /// The event's name; for an acknowledgement, the name of the event it answers (empty when unknown).
    pub event: String,
    /// The arguments, as the text of a JSON array.
    pub data: String,
    /// On an event, the id an acknowledgement was asked with; on an acknowledgement, the id it answers.
    pub ack_id: Option<u64>,
}

impl SocketIoMessage {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            ts: value.get("ts").and_then(Value::as_u64).unwrap_or(0),
            direction: match value.get("direction")?.as_str()? {
                "sent" => Direction::Sent,
                _ => Direction::Received,
            },
            kind: match value.get("type").and_then(Value::as_str) {
                Some("ack") => MessageKind::Ack,
                _ => MessageKind::Event,
            },
            event: value.get("event").and_then(Value::as_str).unwrap_or_default().to_owned(),
            data: value.get("data").and_then(Value::as_str).unwrap_or_default().to_owned(),
            ack_id: value.get("ackId").and_then(Value::as_u64),
        })
    }

    /// What labels the line: the event's name, or `ack` for an acknowledgement.
    pub fn label(&self) -> String {
        match self.kind {
            MessageKind::Event => self.event.clone(),
            MessageKind::Ack => t("model.socketio.ack").to_owned(),
        }
    }

    /// What is said beside the line: that an acknowledgement was asked for, or which event one answers.
    pub fn note(&self) -> Option<String> {
        match (self.kind, self.ack_id) {
            (MessageKind::Event, Some(id)) => Some(t_with("model.socketio.ack_asked", &[("id", &id)])),
            (MessageKind::Ack, Some(id)) if self.event.is_empty() => {
                Some(t_with("model.socketio.ack_answer_unknown", &[("id", &id)]))
            }
            (MessageKind::Ack, Some(id)) => {
                Some(t_with("model.socketio.ack_answer", &[("event", &self.event), ("id", &id)]))
            }
            _ => None,
        }
    }

    /// The arguments as they are read: the one argument by itself (a string without its quotes), nothing
    /// for none, the whole array for several.
    pub fn shown(&self) -> String {
        match serde_json::from_str::<Value>(&self.data) {
            Ok(Value::Array(mut args)) => match args.len() {
                0 => String::new(),
                1 => match args.remove(0) {
                    Value::String(text) => text,
                    other => other.to_string(),
                },
                _ => self.data.clone(),
            },
            _ => self.data.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SocketIoView {
    pub state: SocketState,
    pub error: Option<String>,
    /// The namespace joined; empty before the first connect.
    pub namespace: String,
    /// The id the server gave this client.
    pub sid: Option<String>,
    /// Why the connection ended, in Socket.IO's words.
    pub close_reason: Option<String>,
    pub messages: Vec<SocketIoMessage>,
}

/// The engine keeps at most this many messages per connection; the view mirrors the bound so a long session
/// cannot grow without limit on this side either.
pub const MAX_MESSAGES: usize = 500;

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

impl SocketIoView {
    /// From a `socketio.status` result. `null` (no such connection yet) is a disconnected view with no history.
    pub fn from_status(result: &Value) -> Self {
        if result.is_null() {
            return Self::default();
        }
        Self {
            state: SocketState::parse(result.get("status").and_then(Value::as_str).unwrap_or("")),
            error: error_text(result),
            namespace: text(result, "namespace").unwrap_or_default(),
            sid: text(result, "sid"),
            close_reason: text(result, "closeReason"),
            messages: result
                .get("messages")
                .and_then(Value::as_array)
                .map(|m| m.iter().filter_map(SocketIoMessage::from_value).collect())
                .unwrap_or_default(),
        }
    }

    /// Applies an `event.socketio.status` payload.
    pub fn apply_status_event(&mut self, payload: &Value) {
        self.state = SocketState::parse(payload.get("status").and_then(Value::as_str).unwrap_or(""));
        self.error = error_text(payload);
        self.sid = text(payload, "sid");
        self.close_reason = text(payload, "reason");
        if let Some(namespace) = text(payload, "namespace") {
            self.namespace = namespace;
        }
    }

    /// Applies an `event.socketio.message` payload. A message already held (the same id) is not added twice: a
    /// client that attaches mid-stream can receive a message both in the snapshot and as an event.
    pub fn apply_message_event(&mut self, payload: &Value) {
        let Some(message) = payload.get("message").and_then(SocketIoMessage::from_value) else { return };
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
        json!({"id": id, "ts": 10, "direction": direction, "type": "event", "event": "chat", "data": "[\"hi\"]", "ackId": null})
    }

    fn parsed(value: Value) -> SocketIoMessage {
        SocketIoMessage::from_value(&value).unwrap()
    }

    #[test]
    fn a_status_snapshot_carries_the_state_the_namespace_and_the_retained_log() {
        let view = SocketIoView::from_status(&json!({
            "status": "connected", "error": null, "namespace": "/chat", "sid": "s1", "closeReason": null,
            "messages": [message("c-1", "sent"), message("c-2", "received")],
        }));
        assert_eq!(view.state, SocketState::Connected);
        assert!(view.state.is_live());
        assert_eq!(
            (view.namespace.as_str(), view.sid.as_deref(), view.close_reason.as_deref()),
            ("/chat", Some("s1"), None)
        );
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[0].direction, Direction::Sent);
        assert_eq!(view.messages[1].direction, Direction::Received);
    }

    #[test]
    fn no_connection_yet_reads_as_disconnected_with_no_history() {
        let view = SocketIoView::from_status(&Value::Null);
        assert_eq!(view.state, SocketState::Disconnected);
        assert!(view.messages.is_empty());
        assert_eq!(view.namespace, "");
    }

    #[test]
    fn a_status_event_keeps_the_reason_and_the_namespace() {
        let mut view = SocketIoView::default();
        view.apply_status_event(
            &json!({"status": "connected", "error": null, "namespace": "/chat", "sid": "s1", "reason": null}),
        );
        assert_eq!(
            (view.state, view.namespace.as_str(), view.sid.as_deref()),
            (SocketState::Connected, "/chat", Some("s1"))
        );
        view.apply_status_event(
            &json!({"status": "error", "error": "Not authorized", "sid": null, "reason": "connect error"}),
        );
        assert_eq!(view.state, SocketState::Error);
        assert_eq!(view.error.as_deref(), Some("Not authorized"));
        assert_eq!(view.close_reason.as_deref(), Some("connect error"));
        // An event that does not name the namespace leaves the one known.
        assert_eq!(view.namespace, "/chat");
        assert_eq!(view.sid, None);
    }

    #[test]
    fn an_event_and_an_acknowledgement_are_labelled_for_a_reader() {
        let event = parsed(message("c-1", "sent"));
        assert_eq!((event.kind, event.label().as_str(), event.note()), (MessageKind::Event, "chat", None));

        let asking = parsed(
            json!({"id": "c-2", "direction": "sent", "type": "event", "event": "save", "data": "[]", "ackId": 3}),
        );
        assert_eq!((asking.label().as_str(), asking.note().as_deref()), ("save", Some("ack #3")));

        let ack = parsed(
            json!({"id": "c-3", "direction": "received", "type": "ack", "event": "save", "data": "[\"ok\"]", "ackId": 3}),
        );
        assert_eq!((ack.kind, ack.label().as_str(), ack.note().as_deref()), (MessageKind::Ack, "ack", Some("save #3")));
        let unknown =
            parsed(json!({"id": "c-4", "direction": "received", "type": "ack", "event": "", "data": "[]", "ackId": 9}));
        assert_eq!(unknown.note().as_deref(), Some("#9"));

        assert!(SocketIoMessage::from_value(&json!({"direction": "sent"})).is_none());
        assert!(SocketIoMessage::from_value(&json!({"id": "c-5"})).is_none());
    }

    #[test]
    fn the_arguments_are_shown_as_they_are_read() {
        let with = |data: &str| {
            let mut value = message("c-1", "received");
            value["data"] = json!(data);
            parsed(value).shown()
        };
        assert_eq!(with("[]"), "");
        assert_eq!(with("[\"hello\"]"), "hello");
        assert_eq!(with("[{\"n\":1}]"), "{\"n\":1}");
        assert_eq!(with("[42]"), "42");
        assert_eq!(with("[\"a\",1]"), "[\"a\",1]");
        assert_eq!(with("not json"), "not json");
    }

    #[test]
    fn a_message_seen_in_the_snapshot_and_again_as_an_event_is_kept_once() {
        let mut view = SocketIoView::from_status(&json!({"status": "connected", "messages": [message("c-1", "sent")]}));
        view.apply_message_event(&json!({"message": message("c-1", "sent")}));
        view.apply_message_event(&json!({"message": message("c-2", "received")}));
        assert_eq!(view.messages.len(), 2);
        view.apply_message_event(&json!({"nothing": true}));
        assert_eq!(view.messages.len(), 2);
    }

    #[test]
    fn the_log_is_bounded_like_the_engines_and_can_be_cleared() {
        let mut view = SocketIoView::default();
        for n in 0..(MAX_MESSAGES + 25) {
            view.apply_message_event(&json!({"message": message(&format!("c-{n}"), "received")}));
        }
        assert_eq!(view.messages.len(), MAX_MESSAGES);
        assert_eq!(view.messages[0].id, "c-25");
        view.clear_messages();
        assert!(view.messages.is_empty());
    }
}
