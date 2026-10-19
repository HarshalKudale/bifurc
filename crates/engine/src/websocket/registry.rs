//! The engine's live-socket registry: one outbound WebSocket per `(workspaceId, socketId)`, the bounded message
//! log kept for it, and the `ws.status` / `ws.message` events its life produces.
//!
//! Nothing here observes a client going away: [`disconnect_socket`] is the only lever that closes a socket. A
//! closed socket keeps its entry, so its final state and message log can still be read, and a reconnect
//! replaces the entry's connection instead of adding another one.

use std::{collections::HashMap, sync::LazyLock, time::Duration};

use futures::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Error as WsError, Message,
        client::IntoClientRequest,
        handshake::client::Request,
        http::{HeaderName, HeaderValue},
        protocol::{CloseFrame, frame::coding::CloseCode},
    },
};

use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars},
    kernel::streams::{
        Event, Stream, Streams, emit, header_name_invalid, header_value_invalid, key_of, not_connected, put_error,
        recorded, send_failed, trim_log,
    },
    protocol::EngineError,
    utils::id::now_ms,
};

/// Max concurrent live sockets per workspace.
pub const MAX_WS_CONNECTIONS: usize = 5;

/// The retained message log's ceiling per socket; older messages are dropped from the front.
pub const MAX_WS_MESSAGES: usize = 500;

/// How long a close handshake may take before the socket is reported as closed abnormally.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// What a caller of [`send_socket_message`], [`disconnect_socket`] and [`clear_socket_messages`] gets back.
pub type OkResult = Result<(), EngineError>;

enum Command {
    Send(String),
    Close { code: u16, reason: String },
}

struct Entry {
    workspace_id: String,
    socket_id: String,
    /// The url as the caller sent it, unresolved and trimmed.
    url: String,
    status: &'static str,
    error: Option<EngineError>,
    close_code: Option<u16>,
    close_reason: Option<String>,
    opened_at: Option<i64>,
    messages: Vec<Value>,
    /// The live connection's task, if any.
    tx: Option<UnboundedSender<Command>>,
    /// Bumped on every connect; a task whose generation is behind the entry's no-ops, so a superseded socket's
    /// late close cannot overwrite the new socket's state.
    generation: u64,
    /// Set by a disconnect: a close the caller asked for is not an error, whatever code the handshake ended with.
    close_requested: bool,
    /// Monotonic per entry; the suffix of each message's id.
    seq: u64,
}

impl Entry {
    fn status_event(&self, error: Option<&EngineError>, code: Option<u16>, reason: Option<&str>) -> Event {
        let mut payload = Map::new();
        payload.insert("workspaceId".into(), json!(self.workspace_id));
        payload.insert("socketId".into(), json!(self.socket_id));
        payload.insert("status".into(), json!(self.status));
        put_error(&mut payload, error);
        if let Some(code) = code {
            payload.insert("code".into(), json!(code));
        }
        if let Some(reason) = reason {
            payload.insert("reason".into(), json!(reason));
        }
        Event { name: "ws.status", payload: Value::Object(payload) }
    }

    /// Append a message to the log, trim it to the ceiling and return the `ws.message` event.
    fn push(&mut self, direction: &str, data: String) -> Event {
        self.seq += 1;
        // The log is what clients and files see; a secret that came back in a message is shown as its reference.
        let data = crate::vault::redact_text(&self.workspace_id, data);
        let message = json!({
            "id": format!("{}-{}", self.socket_id, self.seq),
            "ts": now_ms(),
            "direction": direction,
            "data": data,
        });
        self.messages.push(message.clone());
        trim_log(&mut self.messages, MAX_WS_MESSAGES);
        Event {
            name: "ws.message",
            payload: json!({ "workspaceId": self.workspace_id, "socketId": self.socket_id, "message": message }),
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
        out.insert("socketId".into(), json!(self.socket_id));
        out.insert("url".into(), json!(self.url));
        out.insert("status".into(), json!(self.status));
        put_error(&mut out, self.error.as_ref());
        if let Some(code) = self.close_code {
            out.insert("closeCode".into(), json!(code));
        }
        if let Some(reason) = &self.close_reason {
            out.insert("closeReason".into(), json!(reason));
        }
        out.insert("openedAt".into(), json!(self.opened_at));
        out.insert("messages".into(), Value::Array(self.messages.clone()));
        Value::Object(out)
    }

    fn clear_log(&mut self) {
        self.messages.clear();
    }
}

static SOCKETS: LazyLock<Streams<Entry>> = LazyLock::new(Streams::new);

fn no_socket(workspace_id: &str, socket_id: &str) -> EngineError {
    EngineError::not_found(format!("No socket \"{socket_id}\" in workspace \"{workspace_id}\"."))
        .keyed("websocket.not_found", json!({ "id": socket_id, "workspace": workspace_id }))
}

/// A fault the WebSocket library reported, with its own text.
fn failure(detail: &str) -> EngineError {
    EngineError::upstream(detail).keyed("websocket.failed", json!({ "detail": detail }))
}

/// The snapshot for one socket, live or finished, or `None` if the engine never opened that id.
pub fn get_socket(workspace_id: &str, socket_id: &str) -> Option<Value> {
    SOCKETS.get(workspace_id, socket_id)
}

/// The live sockets of a workspace. A finished socket keeps its entry but does not appear here.
pub fn list_sockets(workspace_id: &str) -> Vec<Value> {
    SOCKETS.list(workspace_id)
}

fn live_count(workspace_id: &str) -> usize {
    SOCKETS.live_count(workspace_id)
}

/// Open a socket, replacing any live one under the same key. The url and every header value are resolved
/// against the workspace's merged environment. Returns once the connection attempt has started; the open (or
/// the failure) arrives as `ws.status` events.
pub fn connect_socket(workspace_id: &str, socket_id: &str, url: &str, headers: Option<&Value>) -> OkResult {
    let key = key_of(workspace_id, socket_id);
    let env = load_merged_environment(workspace_id);

    // The limit applies only when the key has no live socket of its own: reconnecting to a socket you already
    // hold must not be refused by the socket you are replacing.
    let existing_live = SOCKETS.lock().get(&key).is_some_and(Entry::is_live);
    if !existing_live && live_count(workspace_id) >= MAX_WS_CONNECTIONS {
        return Err(EngineError::conflict(format!(
            "Max {MAX_WS_CONNECTIONS} active connections reached. \
             Disconnect another socket first (ws.disconnect), or list them with ws.list."
        ))
        .keyed("websocket.max_connections", json!({ "max": MAX_WS_CONNECTIONS })));
    }

    let resolved_url = resolve_vars(url.trim(), env.as_ref());
    let resolved_headers = headers.map(|h| resolve_headers(h, env.as_ref()));
    let request = build_request(&resolved_url, resolved_headers.as_ref());

    let mut events = Vec::new();
    let (tx, rx) = unbounded_channel();
    let outcome = {
        let mut sockets = SOCKETS.lock();
        let entry = sockets.entry(key.clone()).or_insert_with(|| Entry {
            workspace_id: workspace_id.to_string(),
            socket_id: socket_id.to_string(),
            url: String::new(),
            status: "connecting",
            error: None,
            close_code: None,
            close_reason: None,
            opened_at: None,
            messages: Vec::new(),
            tx: None,
            generation: 0,
            close_requested: false,
            seq: 0,
        });

        // Supersede the old socket before closing it, so its close cannot report after the `connecting` below.
        entry.generation += 1;
        if let Some(old) = entry.tx.take() {
            let _ = old.send(Command::Close { code: 1000, reason: "Reconnecting".into() });
        }

        // The retained message log deliberately survives a reconnect.
        entry.url = url.trim().to_string();
        entry.status = "connecting";
        entry.error = None;
        entry.opened_at = None;
        entry.close_code = None;
        entry.close_reason = None;
        entry.close_requested = false;
        events.push(entry.status_event(None, None, None));

        match request {
            Ok(request) => {
                entry.tx = Some(tx);
                Ok((request, entry.generation))
            }
            Err(error) => {
                let error = recorded(error);
                entry.status = "error";
                entry.error = Some(error.clone());
                events.push(entry.status_event(Some(&error), None, None));
                Err(error)
            }
        }
    };
    emit(events);

    let (request, generation) = outcome?;
    let workspace_id = crate::cookies::jar_of(workspace_id);
    tokio::spawn(async move { crate::cookies::scoped(&workspace_id, run(key, generation, request, rx)).await });
    Ok(())
}

/// Close a socket. `Ok` for a socket that is already closed; an error only when the id was never opened.
pub fn disconnect_socket(workspace_id: &str, socket_id: &str, code: Option<u16>, reason: Option<&str>) -> OkResult {
    let mut events = Vec::new();
    let result = {
        let mut sockets = SOCKETS.lock();
        let Some(entry) = sockets.get_mut(&key_of(workspace_id, socket_id)) else {
            return Err(no_socket(workspace_id, socket_id));
        };
        if let Some(tx) = entry.tx.clone() {
            entry.close_requested = true;
            let command =
                Command::Close { code: code.unwrap_or(1000), reason: reason.unwrap_or("User disconnected").into() };
            if tx.send(command).is_err() {
                // The connection task is gone, so no close will ever be reported: settle the state directly.
                entry.tx = None;
                entry.status = "disconnected";
                entry.error = None;
                events.push(entry.status_event(None, None, None));
            }
        }
        Ok(())
    };
    emit(events);
    result
}

/// Resolve `data` against the merged environment, write it to the socket and log the resolved text.
pub fn send_socket_message(workspace_id: &str, socket_id: &str, data: &str) -> OkResult {
    let key = key_of(workspace_id, socket_id);
    let open = |sockets: &HashMap<String, Entry>| {
        sockets.get(&key).is_some_and(|e| e.tx.is_some() && e.status == "connected" && !e.close_requested)
    };
    if !open(&SOCKETS.lock()) {
        return Err(not_connected());
    }

    // What is sent has the secrets in; what is logged has them as written (`push` also masks any value).
    let env = load_merged_environment(workspace_id);
    let resolved = resolve_vars(data, env.as_ref());
    let shown = {
        let mut display = env.clone();
        if let Some(display) = display.as_mut() {
            display["workspaceId"] = json!("");
        }
        resolve_vars(data, display.as_ref())
    };
    let event = {
        let mut sockets = SOCKETS.lock();
        if !open(&sockets) {
            return Err(not_connected());
        }
        let entry = sockets.get_mut(&key).expect("the entry was just found");
        let tx = entry.tx.as_ref().expect("an open socket has a task");
        if tx.send(Command::Send(resolved.clone())).is_err() {
            return Err(send_failed());
        }
        entry.push("sent", shown)
    };
    emit(vec![event]);
    Ok(())
}

/// Drop the retained log of one socket; the socket itself is untouched.
pub fn clear_socket_messages(workspace_id: &str, socket_id: &str) -> OkResult {
    if SOCKETS.clear_log(workspace_id, socket_id) { Ok(()) } else { Err(no_socket(workspace_id, socket_id)) }
}

// ── the connection task ───────────────────────────────────────────────────────────────────────────

/// Build the handshake request: `http(s)` urls are treated as `ws(s)`, and header names are taken verbatim.
fn build_request(url: &str, headers: Option<&Value>) -> Result<Request, EngineError> {
    let url = if let Some(rest) = url.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if let Some(rest) = url.strip_prefix("https://") {
        format!("wss://{rest}")
    } else {
        url.to_string()
    };
    let mut request = url.as_str().into_client_request().map_err(|e| failure(&e.to_string()))?;
    for (name, value) in headers.and_then(Value::as_object).into_iter().flatten() {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| header_name_invalid(name, &e))?;
        let value =
            HeaderValue::from_str(value.as_str().unwrap_or("")).map_err(|e| header_value_invalid(name.as_str(), &e))?;
        request.headers_mut().insert(name, value);
    }
    Ok(request)
}

/// Record a message the peer sent. `false` when the connection has been superseded.
fn record_received(key: &str, generation: u64, data: String) -> bool {
    SOCKETS.record(key, generation, |entry| vec![entry.push("received", data)])
}

/// The connection reported an error. A close the caller asked for is not a failure.
fn record_error(key: &str, generation: u64, message: &str) {
    SOCKETS.record(key, generation, |entry| {
        if entry.close_requested {
            return Vec::new();
        }
        let error = recorded(failure(message));
        entry.status = "error";
        entry.error = Some(error.clone());
        vec![entry.status_event(Some(&error), None, None)]
    });
}

/// The handshake completed. `false` when the connection has been superseded meanwhile.
fn record_open(key: &str, generation: u64) -> bool {
    SOCKETS.record(key, generation, |entry| {
        entry.status = "connected";
        entry.error = None;
        entry.opened_at = Some(now_ms());
        vec![entry.status_event(None, None, None)]
    })
}

/// The connection closed with `code`. Code 1000, or any close the caller asked for, is a clean disconnect;
/// anything else is an error carrying the peer's reason.
fn record_close(key: &str, generation: u64, code: u16, reason: &str) {
    SOCKETS.record(key, generation, |entry| {
        entry.tx = None;
        let clean = code == 1000 || entry.close_requested;
        entry.status = if clean { "disconnected" } else { "error" };
        entry.error = if clean {
            None
        } else if reason.is_empty() {
            Some(recorded(
                EngineError::upstream(format!("Connection closed (code {code})"))
                    .keyed("websocket.closed", json!({ "code": code })),
            ))
        } else {
            Some(recorded(failure(reason)))
        };
        entry.close_code = Some(code);
        entry.close_reason = (!reason.is_empty()).then(|| reason.to_string());
        let error = entry.error.clone();
        vec![entry.status_event(error.as_ref(), Some(code), (!reason.is_empty()).then_some(reason))]
    });
}

/// The code and reason of a received close frame; 1005 when the peer sent none.
fn close_parts(frame: Option<CloseFrame>) -> (u16, String) {
    match frame {
        Some(frame) => (u16::from(frame.code), frame.reason.to_string()),
        None => (1005, String::new()),
    }
}

/// Whether a stream error is the connection ending rather than a fault worth reporting.
fn is_plain_close(error: &WsError) -> bool {
    matches!(
        error,
        WsError::ConnectionClosed
            | WsError::AlreadyClosed
            | WsError::Protocol(tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake)
    )
}

/// Owns one socket from handshake to close. The registry reaches it through `rx`.
async fn run(key: String, generation: u64, mut request: Request, mut rx: UnboundedReceiver<Command>) {
    let url = request.uri().to_string();
    crate::cookies::attach(&url, request.headers_mut());
    let connect = connect_async(request);
    tokio::pin!(connect);
    let mut stream = loop {
        tokio::select! {
            result = &mut connect => match result {
                Ok((stream, response)) => {
                    crate::cookies::collect(&url, response.headers());
                    break stream;
                }
                Err(error) => {
                    record_error(&key, generation, &error.to_string());
                    record_close(&key, generation, 1006, "");
                    return;
                }
            },
            command = rx.recv() => match command {
                // Disconnecting mid-handshake aborts it, which a client reports as an abnormal close.
                Some(Command::Close { .. }) | None => {
                    record_close(&key, generation, 1006, "");
                    return;
                }
                Some(Command::Send(_)) => {}
            },
        }
    };

    if !record_open(&key, generation) {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, stream.close(None)).await;
        return;
    }

    loop {
        tokio::select! {
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    record_received(&key, generation, text.to_string());
                }
                Some(Ok(Message::Binary(_))) => {
                    record_received(&key, generation, "[binary data]".to_string());
                }
                Some(Ok(Message::Close(frame))) => {
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, stream.flush()).await;
                    let (code, reason) = close_parts(frame);
                    record_close(&key, generation, code, &reason);
                    return;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    if !is_plain_close(&error) {
                        record_error(&key, generation, &error.to_string());
                    }
                    record_close(&key, generation, 1006, "");
                    return;
                }
                None => {
                    record_close(&key, generation, 1006, "");
                    return;
                }
            },
            command = rx.recv() => match command {
                Some(Command::Send(data)) => {
                    if let Err(error) = stream.send(Message::text(data)).await {
                        record_error(&key, generation, &error.to_string());
                        record_close(&key, generation, 1006, "");
                        return;
                    }
                }
                Some(Command::Close { code, reason }) => {
                    let frame = CloseFrame { code: CloseCode::from(code), reason: reason.into() };
                    let (code, reason) = finish_close(&mut stream, frame).await;
                    record_close(&key, generation, code, &reason);
                    return;
                }
                None => {
                    let _ = tokio::time::timeout(CLOSE_TIMEOUT, stream.close(None)).await;
                    return;
                }
            },
        }
    }
}

/// Send our close frame and wait for the peer's. The peer's code and reason are what the socket closed with;
/// an unanswered close is abnormal (1006).
async fn finish_close<S>(stream: &mut tokio_tungstenite::WebSocketStream<S>, frame: CloseFrame) -> (u16, String)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let drain = async {
        if stream.close(Some(frame)).await.is_err() {
            return (1006, String::new());
        }
        loop {
            match stream.next().await {
                Some(Ok(Message::Close(frame))) => return close_parts(frame),
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return (1006, String::new()),
            }
        }
    };
    tokio::time::timeout(CLOSE_TIMEOUT, drain).await.unwrap_or((1006, String::new()))
}
