//! An SSE request's live stream, as the engine reports it.
//!
//! The stream belongs to the engine (it outlives this window), keyed by `{workspaceId, streamId}` where
//! `streamId` is the saved request's id. The UI holds a view: `sse.status` returns the whole snapshot with the
//! retained event log, and `event.sse.status` / `event.sse.message` keep it current.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamState {
    Connecting,
    Connected,
    #[default]
    Disconnected,
    Error,
}

impl StreamState {
    pub fn parse(text: &str) -> Self {
        match text {
            "connecting" => StreamState::Connecting,
            "connected" => StreamState::Connected,
            "error" => StreamState::Error,
            _ => StreamState::Disconnected,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StreamState::Connecting => "Connecting",
            StreamState::Connected => "Streaming",
            StreamState::Disconnected => "Ended",
            StreamState::Error => "Failed",
        }
    }

    /// A stream that is open or opening; the state in which Disconnect is the right command.
    pub fn is_live(self) -> bool {
        matches!(self, StreamState::Connecting | StreamState::Connected)
    }
}

/// One event the server sent.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamMessage {
    pub id: String,
    pub ts: u64,
    /// The `event:` name; `message` when the server named none.
    pub event: String,
    pub data: String,
    /// The `id:` the server set, if any.
    pub event_id: Option<String>,
    pub retry: Option<u64>,
}

impl StreamMessage {
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            ts: value.get("ts").and_then(Value::as_u64).unwrap_or(0),
            event: value.get("event").and_then(Value::as_str).unwrap_or("message").to_owned(),
            data: value.get("data").and_then(Value::as_str).unwrap_or_default().to_owned(),
            event_id: value.get("eventId").and_then(Value::as_str).map(str::to_owned),
            retry: value.get("retry").and_then(Value::as_u64),
        })
    }

    /// Whether the server named the event; a plain `message` needs no label.
    pub fn is_named(&self) -> bool {
        self.event != "message"
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamView {
    pub state: StreamState,
    pub error: Option<String>,
    pub http_status: Option<u16>,
    pub messages: Vec<StreamMessage>,
}

/// The engine keeps at most this many events per stream; the view mirrors the bound so a long session
/// cannot grow without limit on this side either.
pub const MAX_MESSAGES: usize = 500;

impl StreamView {
    /// From a `sse.status` result. `null` (no such stream yet) is an ended view with no history.
    pub fn from_status(result: &Value) -> Self {
        if result.is_null() {
            return Self::default();
        }
        Self {
            state: StreamState::parse(result.get("status").and_then(Value::as_str).unwrap_or("")),
            error: result.get("error").and_then(Value::as_str).map(str::to_owned),
            http_status: http_status(result),
            messages: result
                .get("messages")
                .and_then(Value::as_array)
                .map(|m| m.iter().filter_map(StreamMessage::from_value).collect())
                .unwrap_or_default(),
        }
    }

    /// Applies an `event.sse.status` payload.
    pub fn apply_status_event(&mut self, payload: &Value) {
        self.state = StreamState::parse(payload.get("status").and_then(Value::as_str).unwrap_or(""));
        self.error = payload.get("error").and_then(Value::as_str).map(str::to_owned);
        self.http_status = http_status(payload);
    }

    /// Applies an `event.sse.message` payload. An event already held (the same id) is not added twice: a
    /// client that attaches mid-stream can receive one both in the snapshot and as an event.
    pub fn apply_message_event(&mut self, payload: &Value) {
        let Some(message) = payload.get("message").and_then(StreamMessage::from_value) else { return };
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

fn http_status(value: &Value) -> Option<u16> {
    value.get("httpStatus").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(id: &str, event: &str) -> Value {
        json!({"id": id, "ts": 10, "event": event, "data": "hi", "eventId": null, "retry": null})
    }

    #[test]
    fn a_status_snapshot_carries_the_state_and_the_retained_log() {
        let view = StreamView::from_status(&json!({
            "status": "connected", "error": null, "httpStatus": 200,
            "messages": [message("s-1", "message"), message("s-2", "tick")],
        }));
        assert_eq!(view.state, StreamState::Connected);
        assert!(view.state.is_live());
        assert_eq!(view.http_status, Some(200));
        assert_eq!(view.messages.len(), 2);
        assert!(!view.messages[0].is_named());
        assert!(view.messages[1].is_named());
    }

    #[test]
    fn no_stream_yet_reads_as_ended_with_no_history() {
        let view = StreamView::from_status(&Value::Null);
        assert_eq!(view.state, StreamState::Disconnected);
        assert!(!view.state.is_live());
        assert!(view.messages.is_empty());
        assert_eq!(view.http_status, None);
    }

    #[test]
    fn the_states_are_named_for_a_reader() {
        for (wire, state, live) in [
            ("connecting", StreamState::Connecting, true),
            ("connected", StreamState::Connected, true),
            ("disconnected", StreamState::Disconnected, false),
            ("error", StreamState::Error, false),
            ("anything else", StreamState::Disconnected, false),
        ] {
            assert_eq!(StreamState::parse(wire), state, "{wire}");
            assert_eq!(state.is_live(), live, "{wire}");
        }
        assert_eq!(StreamState::Connected.label(), "Streaming");
        assert_eq!(StreamState::Disconnected.label(), "Ended");
    }

    #[test]
    fn an_event_keeps_the_id_and_retry_the_server_set() {
        let parsed = StreamMessage::from_value(
            &json!({"id": "s-1", "ts": 5, "event": "tick", "data": "1", "eventId": "42", "retry": 3000}),
        )
        .unwrap();
        assert_eq!((parsed.event.as_str(), parsed.event_id.as_deref(), parsed.retry), ("tick", Some("42"), Some(3000)));
        // An event with no name is a plain message.
        let plain = StreamMessage::from_value(&json!({"id": "s-2", "data": "x"})).unwrap();
        assert_eq!(plain.event, "message");
        assert!(StreamMessage::from_value(&json!({"data": "no id"})).is_none());
    }

    #[test]
    fn a_failed_stream_keeps_the_reason_and_the_http_status() {
        let mut view = StreamView::default();
        view.apply_status_event(&json!({"status": "error", "error": "HTTP 404 Not Found", "httpStatus": 404}));
        assert_eq!(view.state, StreamState::Error);
        assert_eq!(view.error.as_deref(), Some("HTTP 404 Not Found"));
        assert_eq!(view.http_status, Some(404));
        // A later attempt clears them.
        view.apply_status_event(&json!({"status": "connecting", "error": null, "httpStatus": null}));
        assert_eq!((view.state, view.error.clone(), view.http_status), (StreamState::Connecting, None, None));
    }

    #[test]
    fn an_event_seen_in_the_snapshot_and_again_as_an_event_is_kept_once() {
        let mut view = StreamView::from_status(&json!({"status": "connected", "messages": [message("s-1", "message")]}));
        view.apply_message_event(&json!({"message": message("s-1", "message")}));
        view.apply_message_event(&json!({"message": message("s-2", "message")}));
        assert_eq!(view.messages.len(), 2);
        view.apply_message_event(&json!({"nothing": true}));
        assert_eq!(view.messages.len(), 2);
    }

    #[test]
    fn the_log_is_bounded_like_the_engines_and_can_be_cleared() {
        let mut view = StreamView::default();
        for n in 0..(MAX_MESSAGES + 25) {
            view.apply_message_event(&json!({"message": message(&format!("s-{n}"), "message")}));
        }
        assert_eq!(view.messages.len(), MAX_MESSAGES);
        assert_eq!(view.messages[0].id, "s-25");
        view.clear_messages();
        assert!(view.messages.is_empty());
    }
}
