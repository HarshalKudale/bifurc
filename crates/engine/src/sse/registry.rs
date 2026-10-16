//! The engine's live-stream registry: one outbound SSE connection per `(workspaceId, streamId)`, the bounded
//! event log kept for it, and the `sse.status` / `sse.message` events its life produces.
//!
//! Like a socket, a stream belongs to the engine, not to the client that opened it: only [`disconnect_stream`]
//! closes one. A finished stream keeps its entry so its final state and log can still be read, and a reconnect
//! replaces the connection under the same key. The engine does not reconnect by itself: a stream the server
//! ends is shown as ended, so what the server did is visible.

use std::sync::LazyLock;

use reqwest::{
    Method,
    header::{ACCEPT, CACHE_CONTROL, HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use super::parser::{Parser, SseEvent};
use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars},
    kernel::streams::{Event, Stream, Streams, emit, key_of, trim_log},
    rest::direct_http::{client, describe_error},
    utils::id::now_ms,
};

/// Max concurrent live streams per workspace.
pub const MAX_SSE_STREAMS: usize = 5;

/// The retained event log's ceiling per stream; older events are dropped from the front.
pub const MAX_SSE_MESSAGES: usize = 500;

/// What a caller of [`disconnect_stream`] and [`clear_stream_messages`] gets back.
pub type OkResult = Result<(), String>;

/// Everything a connect needs, already resolved.
struct Prepared {
    method: Method,
    url: url::Url,
    headers: HeaderMap,
    body: Option<String>,
}

struct Entry {
    workspace_id: String,
    stream_id: String,
    /// The url as the caller sent it, unresolved and trimmed.
    url: String,
    method: String,
    status: &'static str,
    error: Option<String>,
    /// The HTTP status of the response, once there is one.
    http_status: Option<u16>,
    opened_at: Option<i64>,
    messages: Vec<Value>,
    /// The live connection's task, if any: a message tells it to stop.
    tx: Option<UnboundedSender<()>>,
    /// Bumped on every connect; a task whose generation is behind the entry's no-ops, so a superseded stream's
    /// late end cannot overwrite the new stream's state.
    generation: u64,
    /// Set by a disconnect: a close the caller asked for is not an error.
    close_requested: bool,
    /// Monotonic per entry; the suffix of each message's id.
    seq: u64,
}

impl Entry {
    fn status_event(&self) -> Event {
        Event {
            name: "sse.status",
            payload: json!({
                "workspaceId": self.workspace_id,
                "streamId": self.stream_id,
                "status": self.status,
                "error": self.error,
                "httpStatus": self.http_status,
            }),
        }
    }

    /// Append an event to the log, trim it to the ceiling and return the `sse.message` event.
    fn push(&mut self, event: SseEvent) -> Event {
        self.seq += 1;
        // The log is what clients and files see; a secret that came back in an event is shown as its reference.
        let data = crate::vault::redact_text(&self.workspace_id, event.data);
        let message = json!({
            "id": format!("{}-{}", self.stream_id, self.seq),
            "ts": now_ms(),
            "event": event.event,
            "data": data,
            "eventId": event.id,
            "retry": event.retry,
        });
        self.messages.push(message.clone());
        trim_log(&mut self.messages, MAX_SSE_MESSAGES);
        Event {
            name: "sse.message",
            payload: json!({ "workspaceId": self.workspace_id, "streamId": self.stream_id, "message": message }),
        }
    }
}

impl Stream for Entry {
    fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    fn is_live(&self) -> bool {
        self.tx.is_some()
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn snapshot(&self) -> Value {
        let mut out = Map::new();
        out.insert("workspaceId".into(), json!(self.workspace_id));
        out.insert("streamId".into(), json!(self.stream_id));
        out.insert("url".into(), json!(self.url));
        out.insert("method".into(), json!(self.method));
        out.insert("status".into(), json!(self.status));
        out.insert("error".into(), json!(self.error));
        out.insert("httpStatus".into(), json!(self.http_status));
        out.insert("openedAt".into(), json!(self.opened_at));
        out.insert("messages".into(), Value::Array(self.messages.clone()));
        Value::Object(out)
    }

    fn clear_log(&mut self) {
        self.messages.clear();
    }
}

static STREAMS: LazyLock<Streams<Entry>> = LazyLock::new(Streams::new);

fn no_stream(workspace_id: &str, stream_id: &str) -> String {
    format!("No stream \"{stream_id}\" in workspace \"{workspace_id}\".")
}

/// The snapshot for one stream, live or finished, or `None` if the engine never opened that id.
pub fn get_stream(workspace_id: &str, stream_id: &str) -> Option<Value> {
    STREAMS.get(workspace_id, stream_id)
}

/// The live streams of a workspace. A finished stream keeps its entry but does not appear here.
pub fn list_streams(workspace_id: &str) -> Vec<Value> {
    STREAMS.list(workspace_id)
}

fn live_count(workspace_id: &str) -> usize {
    STREAMS.live_count(workspace_id)
}

/// The request a connect makes: the url, header values and body resolved against the environment, the
/// `Accept` and `Cache-Control` an event stream needs unless the caller set them.
fn prepare(url: &str, method: &str, headers: Option<&Value>, body: Option<&str>, env: Option<&Value>) -> Result<Prepared, String> {
    let url = url::Url::parse(&resolve_vars(url.trim(), env)).map_err(|_| "Invalid URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none_or(str::is_empty) {
        return Err("Invalid URL: an SSE request needs an http:// or https:// address.".to_string());
    }
    let method = Method::from_bytes(method.trim().to_ascii_uppercase().as_bytes())
        .map_err(|_| format!("Invalid method \"{method}\""))?;
    let mut map = HeaderMap::new();
    map.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
    map.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    if let Some(headers) = headers {
        for (name, value) in resolve_headers(headers, env).as_object().into_iter().flatten() {
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| format!("Invalid character in header name \"{name}\""))?;
            let value = HeaderValue::from_str(value.as_str().unwrap_or(""))
                .map_err(|_| format!("Invalid character in header content [\"{name}\"]"))?;
            map.insert(name, value);
        }
    }
    Ok(Prepared { method, url, headers: map, body: body.map(|text| resolve_vars(text, env)) })
}

/// Open a stream, replacing any live one under the same key. Returns once the connection attempt has started;
/// the open (or the failure) arrives as `sse.status` events.
pub fn connect_stream(
    workspace_id: &str,
    stream_id: &str,
    url: &str,
    method: &str,
    headers: Option<&Value>,
    body: Option<&str>,
) -> Result<(), String> {
    let key = key_of(workspace_id, stream_id);
    let env = load_merged_environment(workspace_id);

    // The limit applies only when the key has no live stream of its own: reconnecting to a stream you already
    // hold must not be refused by the stream you are replacing.
    let existing_live = STREAMS.lock().get(&key).is_some_and(Entry::is_live);
    if !existing_live && live_count(workspace_id) >= MAX_SSE_STREAMS {
        return Err(format!(
            "Max {MAX_SSE_STREAMS} active streams reached. \
             Disconnect another stream first (sse.disconnect), or list them with sse.list."
        ));
    }

    let prepared = prepare(url, method, headers, body, env.as_ref());
    let mut events = Vec::new();
    let (tx, rx) = unbounded_channel();
    let outcome = {
        let mut streams = STREAMS.lock();
        let entry = streams.entry(key.clone()).or_insert_with(|| Entry {
            workspace_id: workspace_id.to_string(),
            stream_id: stream_id.to_string(),
            url: String::new(),
            method: String::new(),
            status: "connecting",
            error: None,
            http_status: None,
            opened_at: None,
            messages: Vec::new(),
            tx: None,
            generation: 0,
            close_requested: false,
            seq: 0,
        });

        // Supersede the old stream before stopping it, so its end cannot report after the `connecting` below.
        entry.generation += 1;
        if let Some(old) = entry.tx.take() {
            let _ = old.send(());
        }

        // The retained log deliberately survives a reconnect.
        entry.url = url.trim().to_string();
        entry.method = method.trim().to_ascii_uppercase();
        entry.status = "connecting";
        entry.error = None;
        entry.http_status = None;
        entry.opened_at = None;
        entry.close_requested = false;
        events.push(entry.status_event());

        match prepared {
            Ok(prepared) => {
                entry.tx = Some(tx);
                Ok((prepared, entry.generation))
            }
            Err(message) => {
                entry.status = "error";
                entry.error = Some(message.clone());
                events.push(entry.status_event());
                Err(message)
            }
        }
    };
    emit(events);

    let (prepared, generation) = outcome?;
    let workspace_id = crate::cookies::jar_of(workspace_id);
    tokio::spawn(async move { crate::cookies::scoped(&workspace_id, run(key, generation, prepared, rx)).await });
    Ok(())
}

/// Close a stream. `Ok` for a stream that is already closed; an error only when the id was never opened.
pub fn disconnect_stream(workspace_id: &str, stream_id: &str) -> OkResult {
    let mut events = Vec::new();
    let result = {
        let mut streams = STREAMS.lock();
        let Some(entry) = streams.get_mut(&key_of(workspace_id, stream_id)) else {
            return Err(no_stream(workspace_id, stream_id));
        };
        if let Some(tx) = entry.tx.clone() {
            entry.close_requested = true;
            if tx.send(()).is_err() {
                // The task is gone, so no end will ever be reported: settle the state directly.
                entry.tx = None;
                entry.status = "disconnected";
                entry.error = None;
                events.push(entry.status_event());
            }
        }
        Ok(())
    };
    emit(events);
    result
}

/// Drop the retained log of one stream; the stream itself is untouched.
pub fn clear_stream_messages(workspace_id: &str, stream_id: &str) -> OkResult {
    if STREAMS.clear_log(workspace_id, stream_id) { Ok(()) } else { Err(no_stream(workspace_id, stream_id)) }
}

// ── the connection task ───────────────────────────────────────────────────────────────────────────

/// Record an event the server sent. `false` when the stream has been superseded.
fn record_event(key: &str, generation: u64, event: SseEvent) -> bool {
    STREAMS.record(key, generation, |entry| vec![entry.push(event)])
}

/// The response headers arrived with a 2xx status. `false` when the stream has been superseded meanwhile.
fn record_open(key: &str, generation: u64, http_status: u16) -> bool {
    STREAMS.record(key, generation, |entry| {
        entry.status = "connected";
        entry.error = None;
        entry.http_status = Some(http_status);
        entry.opened_at = Some(now_ms());
        vec![entry.status_event()]
    })
}

/// The stream is over. `error` is why it failed; with none it ended cleanly, whether the server ended it or
/// the caller closed it. A close the caller asked for is never an error.
fn record_end(key: &str, generation: u64, http_status: Option<u16>, error: Option<String>) {
    STREAMS.record(key, generation, |entry| {
        entry.tx = None;
        if http_status.is_some() {
            entry.http_status = http_status;
        }
        match error.filter(|_| !entry.close_requested) {
            Some(message) => {
                entry.status = "error";
                entry.error = Some(message);
            }
            None => {
                entry.status = "disconnected";
                entry.error = None;
            }
        }
        vec![entry.status_event()]
    });
}

/// Owns one stream from request to end. The registry reaches it through `stop`.
async fn run(key: String, generation: u64, prepared: Prepared, mut stop: UnboundedReceiver<()>) {
    let host = prepared.url.host_str().unwrap_or_default().to_owned();
    let port = prepared.url.port_or_known_default().unwrap_or(80);
    let http = match client() {
        Ok(http) => http,
        Err(message) => return record_end(&key, generation, None, Some(message)),
    };
    let (url, mut headers) = (prepared.url, prepared.headers);
    crate::cookies::attach(url.as_str(), &mut headers);
    let mut request = http.request(prepared.method, url.clone()).headers(headers);
    if let Some(body) = prepared.body.filter(|body| !body.is_empty()) {
        request = request.body(body);
    }

    let mut response = tokio::select! {
        result = request.send() => match result {
            Ok(response) => response,
            Err(error) => return record_end(&key, generation, None, Some(describe_error(&error, &host, port))),
        },
        _ = stop.recv() => return record_end(&key, generation, None, None),
    };
    let status = response.status();
    crate::cookies::collect(url.as_str(), response.headers());
    if !status.is_success() {
        let reason = status.canonical_reason().unwrap_or("");
        return record_end(&key, generation, Some(status.as_u16()), Some(format!("HTTP {} {reason}", status.as_u16()).trim().to_owned()));
    }
    if !record_open(&key, generation, status.as_u16()) {
        return;
    }

    let mut parser = Parser::new();
    loop {
        tokio::select! {
            chunk = response.chunk() => match chunk {
                Ok(Some(bytes)) => {
                    for event in parser.feed(&bytes) {
                        if !record_event(&key, generation, event) {
                            return;
                        }
                    }
                }
                Ok(None) => return record_end(&key, generation, None, None),
                Err(error) => return record_end(&key, generation, None, Some(describe_error(&error, &host, port))),
            },
            _ = stop.recv() => return record_end(&key, generation, None, None),
        }
    }
}
