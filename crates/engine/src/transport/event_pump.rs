//! What a session's events pass through on the way to its connection. Traffic (`event.log.entry`) is
//! reduced to light entries and coalesced into batches; every other event goes out at once, after any
//! batch already waiting so order is kept.

use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use tokio::runtime::Handle;

use super::event_log::EngineEvent;

pub const COALESCED_EVENT: &str = "event.log.entry";
pub const DEFAULT_MAX_BATCH: usize = 100;
pub const DEFAULT_BATCH_WINDOW: Duration = Duration::from_millis(250);

/// The wire form of a captured exchange: what a traffic list shows. The full entry is `capture.get`.
pub fn to_log_entry_event(payload: &Value) -> Value {
    let empty = Map::new();
    let e = payload.as_object().unwrap_or(&empty);
    let mut out = Map::new();
    out.insert("id".into(), json!(e.get("id").and_then(Value::as_str).unwrap_or("")));
    out.insert("timestamp".into(), e.get("ts").filter(|v| v.is_number()).cloned().unwrap_or(json!(0)));
    out.insert("method".into(), json!(e.get("method").and_then(Value::as_str).unwrap_or("")));
    out.insert("url".into(), json!(e.get("url").and_then(Value::as_str).unwrap_or("")));
    out.insert("status".into(), e.get("status").filter(|v| v.is_number()).cloned().unwrap_or(Value::Null));
    out.insert("durationMs".into(), e.get("durationMs").filter(|v| v.is_number()).cloned().unwrap_or(Value::Null));
    for key in ["workspaceId", "via"] {
        if let Some(value) = e.get(key).filter(|v| v.is_string()) {
            out.insert(key.into(), value.clone());
        }
    }
    if e.get("pinned").and_then(Value::as_bool) == Some(true) {
        out.insert("pinned".into(), json!(true));
    }
    if let Some(origin) = e.get("origin").and_then(Value::as_str)
        && (origin == "forward" || origin == "reverse" || origin == "server")
    {
        out.insert("origin".into(), json!(origin));
    }
    if let Some(target) = e.get("target").filter(|v| v.is_string() || v.is_null()) {
        out.insert("target".into(), target.clone());
        // A mock's answer is logged with target `mock:<id>`.
        if let Some(id) = target.as_str().and_then(|t| t.strip_prefix("mock:")) {
            out.insert("mockId".into(), json!(id));
        }
    }
    if let Some(headers) = e.get("resHeaders").and_then(Value::as_object) {
        let content_type =
            headers.iter().find(|(k, v)| k.to_lowercase() == "content-type" && v.is_string()).map(|(_, v)| v.clone());
        if let Some(content_type) = content_type {
            out.insert("contentType".into(), content_type);
        }
    }
    out.insert("size".into(), json!(base64_size(e.get("resBody").and_then(Value::as_str).unwrap_or(""))));
    Value::Object(out)
}

fn base64_size(b64: &str) -> usize {
    if b64.is_empty() {
        return 0;
    }
    let padding = if b64.ends_with("==") {
        2
    } else if b64.ends_with('=') {
        1
    } else {
        0
    };
    (b64.len() * 3 / 4).saturating_sub(padding)
}

/// The entries of a `log.entry` payload, whether it is one raw entry or already a batch.
fn log_entries(payload: &Value) -> Vec<Value> {
    match payload.get("entries").and_then(Value::as_array) {
        Some(entries) => entries.clone(),
        None => vec![to_log_entry_event(payload)],
    }
}

#[derive(Default)]
struct State {
    pending: Vec<Value>,
    pending_seq: u64,
    /// Bumped on every flush, so a timer armed for an earlier batch does nothing.
    generation: u64,
    timer_armed: bool,
    closed: bool,
}

type Send = Arc<dyn Fn(Value) + std::marker::Send + Sync>;

#[derive(Clone)]
pub struct EventPump {
    state: Arc<Mutex<State>>,
    send: Send,
    runtime: Handle,
}

impl EventPump {
    /// `send` receives event envelopes (`{event, seq, payload}`). Must be created inside the runtime.
    pub fn new(send: impl Fn(Value) + std::marker::Send + Sync + 'static) -> Self {
        Self { state: Arc::new(Mutex::new(State::default())), send: Arc::new(send), runtime: Handle::current() }
    }

    pub fn push(&self, event: &EngineEvent) {
        let mut state = self.state.lock();
        if state.closed {
            return;
        }
        if event.event != COALESCED_EVENT {
            let batch = take_batch(&mut state);
            drop(state);
            if let Some(batch) = batch {
                (self.send)(batch);
            }
            (self.send)(json!({ "event": event.event, "seq": event.seq, "payload": event.payload }));
            return;
        }
        state.pending.extend(log_entries(&event.payload));
        state.pending_seq = state.pending_seq.max(event.seq);
        if state.pending.len() >= DEFAULT_MAX_BATCH {
            let batch = take_batch(&mut state);
            drop(state);
            if let Some(batch) = batch {
                (self.send)(batch);
            }
            return;
        }
        if !state.timer_armed {
            state.timer_armed = true;
            let generation = state.generation;
            let pump = self.clone();
            self.runtime.spawn(async move {
                tokio::time::sleep(DEFAULT_BATCH_WINDOW).await;
                let batch = {
                    let mut state = pump.state.lock();
                    if state.closed || state.generation != generation {
                        return;
                    }
                    take_batch(&mut state)
                };
                if let Some(batch) = batch {
                    (pump.send)(batch);
                }
            });
        }
    }

    /// Send what is waiting and accept nothing more.
    pub fn close(&self) {
        let batch = {
            let mut state = self.state.lock();
            if state.closed {
                return;
            }
            state.closed = true;
            take_batch(&mut state)
        };
        if let Some(batch) = batch {
            (self.send)(batch);
        }
    }
}

fn take_batch(state: &mut State) -> Option<Value> {
    state.generation += 1;
    state.timer_armed = false;
    if state.pending.is_empty() {
        return None;
    }
    let entries = std::mem::take(&mut state.pending);
    let seq = std::mem::take(&mut state.pending_seq);
    Some(json!({ "event": COALESCED_EVENT, "seq": seq, "payload": { "entries": entries } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collector() -> (EventPump, Arc<Mutex<Vec<Value>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sink = sent.clone();
        (EventPump::new(move |frame| sink.lock().push(frame)), sent)
    }

    fn entry(seq: u64) -> EngineEvent {
        EngineEvent {
            event: COALESCED_EVENT.into(),
            seq,
            payload: json!({ "id": format!("e{seq}"), "ts": 5, "method": "GET", "url": "http://a/", "status": 200,
                "durationMs": 3, "resHeaders": { "Content-Type": "text/plain" }, "resBody": "aGk=" }),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn traffic_is_batched_and_other_events_flush_it_first() {
        let (pump, sent) = collector();
        pump.push(&entry(1));
        pump.push(&entry(2));
        assert!(sent.lock().is_empty());
        pump.push(&EngineEvent { event: "event.ws.status".into(), seq: 3, payload: json!({}) });
        let frames = sent.lock().clone();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["seq"], 2);
        assert_eq!(frames[0]["payload"]["entries"].as_array().unwrap().len(), 2);
        assert_eq!(frames[0]["payload"]["entries"][0]["contentType"], "text/plain");
        assert_eq!(frames[0]["payload"]["entries"][0]["size"], 2);
        assert_eq!(frames[1]["event"], "event.ws.status");
    }

    #[tokio::test(start_paused = true)]
    async fn a_lone_entry_goes_out_after_the_window() {
        let (pump, sent) = collector();
        pump.push(&entry(1));
        tokio::time::sleep(DEFAULT_BATCH_WINDOW + Duration::from_millis(10)).await;
        assert_eq!(sent.lock().len(), 1);
    }
}
