//! The engine's live Socket.IO registry: one outbound connection per `(workspaceId, connectionId)`, the bounded
//! message log kept for it, and the `socketio.status` / `socketio.message` events its life produces.
//!
//! Like a WebSocket, a connection belongs to the engine, not to the client that opened it: only
//! [`disconnect_connection`] closes one. A finished connection keeps its entry so its final state and log can
//! still be read, and a reconnect replaces the connection under the same key. The engine does not reconnect by
//! itself: a connection the server ends is shown as ended, so what the server did is visible.
//!
//! The transport is a WebSocket speaking Engine.IO v4 (Socket.IO servers v3 and v4); HTTP long-polling is not
//! used. A connection joins one namespace.
//!
//! What the server sends is logged for the events the connection listens to, as a Socket.IO client only hears
//! the events it has a listener for. Without a list of listeners every event is logged.

use std::{collections::HashMap, sync::LazyLock, time::Duration};

use futures::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
    time::Instant,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        Message,
        client::IntoClientRequest,
        handshake::client::Request,
        http::{HeaderName, HeaderValue},
    },
};

use super::packet::{
    DEFAULT_HEARTBEAT, Incoming, PONG, REFUSED_BY_DEFAULT, connect_frame, disconnect_frame, event_frame,
    normalize_namespace, parse_args, read_frame,
};
use crate::{
    env::{load_merged_environment, resolve_headers, resolve_vars},
    kernel::streams::{
        Event, Stream, Streams, emit, header_name_invalid, header_value_invalid, key_of, not_connected, put_error,
        recorded, send_failed, trim_log,
    },
    protocol::{EngineError, ErrorCode},
    utils::id::now_ms,
};

/// Max concurrent live connections per workspace.
pub const MAX_SOCKETIO_CONNECTIONS: usize = 5;

/// The retained message log's ceiling per connection; older messages are dropped from the front.
pub const MAX_SOCKETIO_MESSAGES: usize = 500;

/// The path a Socket.IO server listens on unless it was configured otherwise.
pub const DEFAULT_PATH: &str = "/socket.io";

/// How long the server is given, once the WebSocket is open, to accept the namespace.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(if cfg!(test) { 600 } else { 10_000 });

/// How long the closing of the WebSocket may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// What a caller of [`emit_event`], [`disconnect_connection`] and [`clear_connection_messages`] gets back.
pub type OkResult = Result<(), EngineError>;

/// What a connect is asked to open, as the caller wrote it (variables unresolved).
pub struct Target<'a> {
    pub url: &'a str,
    pub headers: Option<&'a Value>,
    /// The server's path; [`DEFAULT_PATH`] when empty.
    pub path: &'a str,
    /// The namespace to join; when empty, the path of `url` if it has one, the main namespace otherwise.
    pub namespace: &'a str,
    /// The JSON object sent with the namespace connect, or empty for none.
    pub auth: &'a str,
    /// The events to log when the server sends them; `None` logs every event.
    pub listeners: Option<Vec<String>>,
}

/// Everything a connect needs, already resolved.
struct Prepared {
    request: Request,
    namespace: String,
    auth: Option<Value>,
}

enum Command {
    /// A frame to write: an event this side emits.
    Frame(String),
    Close,
}

struct Entry {
    workspace_id: String,
    connection_id: String,
    /// The url as the caller sent it, unresolved and trimmed.
    url: String,
    /// The namespace joined, resolved.
    namespace: String,
    status: &'static str,
    error: Option<EngineError>,
    /// The id the server gave this client in the namespace.
    sid: Option<String>,
    /// Why the connection ended, in Socket.IO's words (`io server disconnect`, `transport close`, ...).
    close_reason: Option<&'static str>,
    opened_at: Option<i64>,
    messages: Vec<Value>,
    /// The live connection's task, if any.
    tx: Option<UnboundedSender<Command>>,
    /// Bumped on every connect; a task whose generation is behind the entry's no-ops, so a superseded
    /// connection's late end cannot overwrite the new connection's state.
    generation: u64,
    /// Set by a disconnect: a close the caller asked for is not an error.
    close_requested: bool,
    /// Monotonic per entry; the suffix of each message's id.
    seq: u64,
    /// The id the next emit that asks for an acknowledgement is sent with.
    next_ack: u64,
    /// The events waiting for the server's acknowledgement, by the id they were sent with.
    pending_acks: HashMap<u64, String>,
    /// The events logged when the server sends them; `None` logs every event.
    listeners: Option<Vec<String>>,
}

/// One line of the log: an event or an acknowledgement, in either direction.
struct Logged<'a> {
    direction: &'static str,
    kind: &'static str,
    event: &'a str,
    args: &'a [Value],
    ack_id: Option<u64>,
}

impl Entry {
    fn status_event(&self) -> Event {
        let mut payload = json!({
            "workspaceId": self.workspace_id,
            "connectionId": self.connection_id,
            "namespace": self.namespace,
            "status": self.status,
            "sid": self.sid,
            "reason": self.close_reason,
        });
        if let Some(map) = payload.as_object_mut() {
            put_error(map, self.error.as_ref());
        }
        Event { name: "socketio.status", payload }
    }

    /// Append a message to the log, trim it to the ceiling and return the `socketio.message` event.
    fn push(&mut self, logged: Logged) -> Event {
        self.seq += 1;
        // The log is what clients and files see; a secret that came back in a message is shown as its reference.
        let data = crate::vault::redact_text(&self.workspace_id, Value::Array(logged.args.to_vec()).to_string());
        let message = json!({
            "id": format!("{}-{}", self.connection_id, self.seq),
            "ts": now_ms(),
            "direction": logged.direction,
            "type": logged.kind,
            "event": logged.event,
            "data": data,
            "ackId": logged.ack_id,
        });
        self.messages.push(message.clone());
        trim_log(&mut self.messages, MAX_SOCKETIO_MESSAGES);
        Event {
            name: "socketio.message",
            payload: json!({
                "workspaceId": self.workspace_id, "connectionId": self.connection_id, "message": message,
            }),
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
        out.insert("connectionId".into(), json!(self.connection_id));
        out.insert("url".into(), json!(self.url));
        out.insert("namespace".into(), json!(self.namespace));
        out.insert("status".into(), json!(self.status));
        put_error(&mut out, self.error.as_ref());
        out.insert("sid".into(), json!(self.sid));
        out.insert("closeReason".into(), json!(self.close_reason));
        out.insert("listeners".into(), json!(self.listeners));
        out.insert("openedAt".into(), json!(self.opened_at));
        out.insert("messages".into(), Value::Array(self.messages.clone()));
        Value::Object(out)
    }

    fn clear_log(&mut self) {
        self.messages.clear();
    }
}

static CONNECTIONS: LazyLock<Streams<Entry>> = LazyLock::new(Streams::new);

fn no_connection(workspace_id: &str, connection_id: &str) -> EngineError {
    EngineError::not_found(format!("No connection \"{connection_id}\" in workspace \"{workspace_id}\"."))
        .keyed("socketio.not_found", json!({ "id": connection_id, "workspace": workspace_id }))
}

/// A fault the transport reported, with its own text.
fn failure(detail: &str) -> EngineError {
    EngineError::upstream(detail).keyed("socketio.failed", json!({ "detail": detail }))
}

/// The server's own words for refusing the namespace, or ours when it gave none.
fn refused(message: &str) -> EngineError {
    if message == REFUSED_BY_DEFAULT {
        EngineError::upstream(message).keyed("socketio.refused_default", json!({}))
    } else {
        failure(message)
    }
}

/// The snapshot for one connection, live or finished, or `None` if the engine never opened that id.
pub fn get_connection(workspace_id: &str, connection_id: &str) -> Option<Value> {
    CONNECTIONS.get(workspace_id, connection_id)
}

/// The live connections of a workspace. A finished connection keeps its entry but does not appear here.
pub fn list_connections(workspace_id: &str) -> Vec<Value> {
    CONNECTIONS.list(workspace_id)
}

fn live_count(workspace_id: &str) -> usize {
    CONNECTIONS.live_count(workspace_id)
}

/// The WebSocket request a connect makes, the namespace it joins and what it authenticates with, everything
/// resolved against the environment. `http(s)` addresses are treated as `ws(s)`; the address's query is kept
/// and given the `EIO` and `transport` parameters a Socket.IO server expects.
fn prepare(target: &Target, env: Option<&Value>) -> Result<Prepared, EngineError> {
    let needs = || {
        EngineError::bad_request(
            "Invalid URL: a Socket.IO request needs an http://, https://, ws:// or wss:// address.",
        )
        .keyed("socketio.invalid_url", json!({}))
    };
    let mut url = url::Url::parse(&resolve_vars(target.url.trim(), env)).map_err(|_| needs())?;
    let scheme = match url.scheme() {
        "http" | "ws" => "ws",
        "https" | "wss" => "wss",
        _ => return Err(needs()),
    };
    if url.host_str().is_none_or(str::is_empty) {
        return Err(needs());
    }

    // `https://host/admin` names the namespace `/admin`, as it does for a Socket.IO client.
    let named = resolve_vars(target.namespace.trim(), env);
    let namespace = normalize_namespace(if named.trim().is_empty() { url.path() } else { &named });

    let path = resolve_vars(target.path.trim(), env);
    let path = path.trim().trim_end_matches('/');
    let path = if path.is_empty() { DEFAULT_PATH } else { path };
    let path = if path.starts_with('/') { format!("{path}/") } else { format!("/{path}/") };

    let has = |name: &str| url.query_pairs().any(|(key, _)| key == name);
    let (has_version, has_transport) = (has("EIO"), has("transport"));
    url.set_scheme(scheme).map_err(|_| needs())?;
    url.set_path(&path);
    url.set_fragment(None);
    {
        let mut query = url.query_pairs_mut();
        if !has_version {
            query.append_pair("EIO", "4");
        }
        if !has_transport {
            query.append_pair("transport", "websocket");
        }
    }

    let auth = resolve_vars(target.auth.trim(), env);
    let auth = match auth.trim() {
        "" => None,
        text => match serde_json::from_str::<Value>(text) {
            Ok(value @ Value::Object(_)) => Some(value),
            _ => {
                return Err(EngineError::bad_request(
                    "Invalid auth: it must be a JSON object, such as {\"token\": \"...\"}.",
                )
                .keyed("socketio.invalid_auth", json!({})));
            }
        },
    };

    let mut request = url.as_str().into_client_request().map_err(|e| failure(&e.to_string()))?;
    if let Some(headers) = target.headers {
        for (name, value) in resolve_headers(headers, env).as_object().into_iter().flatten() {
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| header_name_invalid(name, &e))?;
            let value = HeaderValue::from_str(value.as_str().unwrap_or(""))
                .map_err(|e| header_value_invalid(name.as_str(), &e))?;
            request.headers_mut().insert(name, value);
        }
    }
    Ok(Prepared { request, namespace, auth })
}

/// Open a connection, replacing any live one under the same key. Returns once the connection attempt has
/// started; the open (or the failure) arrives as `socketio.status` events.
pub fn connect_connection(workspace_id: &str, connection_id: &str, target: &Target) -> OkResult {
    let key = key_of(workspace_id, connection_id);
    let env = load_merged_environment(workspace_id);

    // The limit applies only when the key has no live connection of its own: reconnecting to one you already
    // hold must not be refused by the connection you are replacing.
    let existing_live = CONNECTIONS.lock().get(&key).is_some_and(Entry::is_live);
    if !existing_live && live_count(workspace_id) >= MAX_SOCKETIO_CONNECTIONS {
        return Err(EngineError::conflict(format!(
            "Max {MAX_SOCKETIO_CONNECTIONS} active connections reached. \
             Disconnect another one first (socketio.disconnect), or list them with socketio.list."
        ))
        .keyed("socketio.max_connections", json!({ "max": MAX_SOCKETIO_CONNECTIONS })));
    }

    let prepared = prepare(target, env.as_ref());
    let mut events = Vec::new();
    let (tx, rx) = unbounded_channel();
    let outcome = {
        let mut connections = CONNECTIONS.lock();
        let entry = connections.entry(key.clone()).or_insert_with(|| Entry {
            workspace_id: workspace_id.to_string(),
            connection_id: connection_id.to_string(),
            url: String::new(),
            namespace: "/".to_string(),
            status: "connecting",
            error: None,
            sid: None,
            close_reason: None,
            opened_at: None,
            messages: Vec::new(),
            tx: None,
            generation: 0,
            close_requested: false,
            seq: 0,
            next_ack: 0,
            pending_acks: HashMap::new(),
            listeners: None,
        });

        // Supersede the old connection before closing it, so its end cannot report after the `connecting` below.
        entry.generation += 1;
        if let Some(old) = entry.tx.take() {
            let _ = old.send(Command::Close);
        }

        // The retained message log deliberately survives a reconnect.
        entry.url = target.url.trim().to_string();
        entry.status = "connecting";
        entry.error = None;
        entry.sid = None;
        entry.close_reason = None;
        entry.opened_at = None;
        entry.close_requested = false;
        entry.pending_acks.clear();
        entry.listeners = target.listeners.clone();
        if let Ok(prepared) = &prepared {
            entry.namespace = prepared.namespace.clone();
        }
        events.push(entry.status_event());

        match prepared {
            Ok(prepared) => {
                entry.tx = Some(tx);
                Ok((prepared, entry.generation))
            }
            Err(error) => {
                entry.status = "error";
                entry.error = Some(recorded(error.clone()));
                events.push(entry.status_event());
                Err(error)
            }
        }
    };
    emit(events);

    let (prepared, generation) = outcome?;
    let workspace_id = crate::cookies::jar_of(workspace_id);
    tokio::spawn(async move { crate::cookies::scoped(&workspace_id, run(key, generation, prepared, rx)).await });
    Ok(())
}

/// Close a connection. `Ok` for one that is already closed; an error only when the id was never opened.
pub fn disconnect_connection(workspace_id: &str, connection_id: &str) -> OkResult {
    let mut events = Vec::new();
    let result = {
        let mut connections = CONNECTIONS.lock();
        let Some(entry) = connections.get_mut(&key_of(workspace_id, connection_id)) else {
            return Err(no_connection(workspace_id, connection_id));
        };
        if let Some(tx) = entry.tx.clone() {
            entry.close_requested = true;
            if tx.send(Command::Close).is_err() {
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

/// Emit `event` with the arguments written in `args` (see [`parse_args`]), both resolved against the merged
/// environment. With `ack` the server is asked to acknowledge it; the acknowledgement arrives as a message.
pub fn emit_event(workspace_id: &str, connection_id: &str, event: &str, args: &str, ack: bool) -> OkResult {
    let key = key_of(workspace_id, connection_id);
    let open = |connections: &HashMap<String, Entry>| {
        connections.get(&key).is_some_and(|e| e.tx.is_some() && e.status == "connected" && !e.close_requested)
    };
    if !open(&CONNECTIONS.lock()) {
        return Err(not_connected());
    }

    // What is sent has the secrets in; what is logged has them as written (`push` also masks any value).
    let env = load_merged_environment(workspace_id);
    let display = {
        let mut display = env.clone();
        if let Some(display) = display.as_mut() {
            display["workspaceId"] = json!("");
        }
        display
    };
    let name = resolve_vars(event.trim(), env.as_ref());
    if name.trim().is_empty() {
        return Err(
            EngineError::bad_request("An event name is required.").keyed("socketio.event_name_required", json!({}))
        );
    }
    let sent_args = parse_args(&resolve_vars(args, env.as_ref()));
    let shown_name = resolve_vars(event.trim(), display.as_ref());
    let shown_args = parse_args(&resolve_vars(args, display.as_ref()));

    let logged = {
        let mut connections = CONNECTIONS.lock();
        if !open(&connections) {
            return Err(not_connected());
        }
        let entry = connections.get_mut(&key).expect("the entry was just found");
        let ack_id = ack.then_some(entry.next_ack);
        let frame = event_frame(&entry.namespace, name.trim(), &sent_args, ack_id);
        let tx = entry.tx.as_ref().expect("an open connection has a task");
        if tx.send(Command::Frame(frame)).is_err() {
            return Err(send_failed());
        }
        if let Some(id) = ack_id {
            entry.next_ack += 1;
            entry.pending_acks.insert(id, shown_name.trim().to_string());
        }
        entry.push(Logged { direction: "sent", kind: "event", event: shown_name.trim(), args: &shown_args, ack_id })
    };
    emit(vec![logged]);
    Ok(())
}

/// Change the events a connection listens to, live or not: `None` listens to every event. What is already in
/// the log stays.
pub fn set_listeners(workspace_id: &str, connection_id: &str, listeners: Option<Vec<String>>) -> OkResult {
    match CONNECTIONS.lock().get_mut(&key_of(workspace_id, connection_id)) {
        Some(entry) => {
            entry.listeners = listeners;
            Ok(())
        }
        None => Err(no_connection(workspace_id, connection_id)),
    }
}

/// Drop the retained log of one connection; the connection itself is untouched.
pub fn clear_connection_messages(workspace_id: &str, connection_id: &str) -> OkResult {
    if CONNECTIONS.clear_log(workspace_id, connection_id) {
        Ok(())
    } else {
        Err(no_connection(workspace_id, connection_id))
    }
}

// ── the connection task ───────────────────────────────────────────────────────────────────────────

/// Record an event or an acknowledgement the server sent. `false` when the connection has been superseded.
fn record_received(key: &str, generation: u64, incoming: Incoming) -> bool {
    CONNECTIONS.record(key, generation, |entry| {
        let event = match incoming {
            Incoming::Event { name, args, ack_id } => {
                // An event nothing listens to is not heard, as in a Socket.IO client.
                if entry.listeners.as_ref().is_some_and(|heard| !heard.contains(&name)) {
                    return Vec::new();
                }
                entry.push(Logged { direction: "received", kind: "event", event: &name, args: &args, ack_id })
            }
            Incoming::Ack { id, args } => {
                // An acknowledgement is named after the event it answers.
                let answered = entry.pending_acks.remove(&id).unwrap_or_default();
                entry.push(Logged {
                    direction: "received",
                    kind: "ack",
                    event: &answered,
                    args: &args,
                    ack_id: Some(id),
                })
            }
            _ => return Vec::new(),
        };
        vec![event]
    })
}

/// The namespace accepted the connection. `false` when the connection has been superseded meanwhile.
fn record_open(key: &str, generation: u64, sid: Option<String>) -> bool {
    CONNECTIONS.record(key, generation, |entry| {
        entry.status = "connected";
        entry.error = None;
        entry.sid = sid;
        entry.opened_at = Some(now_ms());
        vec![entry.status_event()]
    })
}

/// The connection is over, for `reason`. `error` is why it failed; with none it ended cleanly. A close the
/// caller asked for is never an error.
fn record_end(key: &str, generation: u64, error: Option<EngineError>, reason: &'static str) {
    CONNECTIONS.record(key, generation, |entry| {
        entry.tx = None;
        entry.pending_acks.clear();
        match error.filter(|_| !entry.close_requested) {
            Some(error) => {
                entry.status = "error";
                entry.error = Some(recorded(error));
                entry.close_reason = Some(reason);
            }
            None => {
                entry.status = "disconnected";
                entry.error = None;
                entry.close_reason = Some(if entry.close_requested { "io client disconnect" } else { reason });
            }
        }
        vec![entry.status_event()]
    });
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn close(stream: &mut Socket) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, stream.close(None)).await;
}

/// Owns one connection from the WebSocket handshake to its end. The registry reaches it through `rx`.
async fn run(key: String, generation: u64, prepared: Prepared, mut rx: UnboundedReceiver<Command>) {
    let Prepared { mut request, namespace, auth } = prepared;
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
                    let error = EngineError::upstream(format!(
                        "No Socket.IO server answered at this address and path ({error})."
                    ))
                    .keyed("socketio.no_server", json!({ "detail": error.to_string() }));
                    return record_end(&key, generation, Some(error), "transport error");
                }
            },
            command = rx.recv() => match command {
                Some(Command::Close) | None => return record_end(&key, generation, None, "io client disconnect"),
                Some(Command::Frame(_)) => {}
            },
        }
    };

    // Until the namespace answers, the timer is the handshake's; after that it is the server's heartbeat.
    let mut connected = false;
    let mut heartbeat = DEFAULT_HEARTBEAT;
    let timer = tokio::time::sleep(HANDSHAKE_TIMEOUT);
    tokio::pin!(timer);

    loop {
        tokio::select! {
            _ = &mut timer => {
                close(&mut stream).await;
                let (error, reason) = if connected {
                    (
                        EngineError::new(ErrorCode::Timeout, "Ping timeout: the server stopped sending its heartbeat.")
                            .keyed("socketio.ping_timeout", json!({})),
                        "ping timeout",
                    )
                } else {
                    (
                        EngineError::new(
                            ErrorCode::Timeout,
                            "Timed out waiting for the Socket.IO handshake. Is this a Socket.IO server (v3 or v4)?",
                        )
                        .keyed("socketio.handshake_timeout", json!({})),
                        "timeout",
                    )
                };
                return record_end(&key, generation, Some(error), reason);
            }
            incoming = stream.next() => {
                let frame = match incoming {
                    Some(Ok(Message::Text(text))) => text.to_string(),
                    Some(Ok(Message::Close(_))) | None => {
                        return record_end(&key, generation, Some(lost(connected, None)), "transport close");
                    }
                    Some(Err(error)) => {
                        let message = lost(connected, Some(error.to_string()));
                        return record_end(&key, generation, Some(message), "transport error");
                    }
                    // The binary attachments of a binary event: the event itself is logged with placeholders.
                    Some(Ok(_)) => continue,
                };
                if connected {
                    timer.as_mut().reset(Instant::now() + heartbeat);
                }
                match read_frame(&frame, &namespace) {
                    Incoming::Open { heartbeat: told } => {
                        heartbeat = told;
                        if stream.send(Message::text(connect_frame(&namespace, auth.as_ref()))).await.is_err() {
                            return record_end(&key, generation, Some(lost(connected, None)), "transport error");
                        }
                    }
                    Incoming::Ping
                        if stream.send(Message::text(PONG)).await.is_err() => {
                            return record_end(&key, generation, Some(lost(connected, None)), "transport error");
                        }
                    Incoming::Connected { sid } if !connected => {
                        if !record_open(&key, generation, sid) {
                            return close(&mut stream).await;
                        }
                        connected = true;
                        timer.as_mut().reset(Instant::now() + heartbeat);
                    }
                    Incoming::Refused(message) => {
                        close(&mut stream).await;
                        return record_end(&key, generation, Some(refused(&message)), "connect error");
                    }
                    Incoming::Disconnected => {
                        close(&mut stream).await;
                        return record_end(&key, generation, None, "io server disconnect");
                    }
                    Incoming::Closed => {
                        return record_end(&key, generation, Some(lost(connected, None)), "transport close");
                    }
                    // The guard would move `received`, so the `if` cannot be folded into it.
                    #[allow(clippy::collapsible_match)]
                    received @ (Incoming::Event { .. } | Incoming::Ack { .. }) if connected => {
                        if !record_received(&key, generation, received) {
                            return close(&mut stream).await;
                        }
                    }
                    _ => {}
                }
            }
            command = rx.recv() => match command {
                Some(Command::Frame(frame)) => {
                    if let Err(error) = stream.send(Message::text(frame)).await {
                        let message = lost(connected, Some(error.to_string()));
                        return record_end(&key, generation, Some(message), "transport error");
                    }
                }
                Some(Command::Close) | None => {
                    if connected {
                        let _ = stream.send(Message::text(disconnect_frame(&namespace))).await;
                    }
                    close(&mut stream).await;
                    return record_end(&key, generation, None, "io client disconnect");
                }
            },
        }
    }
}

/// What is said when the server goes away without disconnecting, with the transport's own words if it had any.
fn lost(connected: bool, detail: Option<String>) -> EngineError {
    let what = if connected {
        "The server closed the connection"
    } else {
        "The connection closed before the Socket.IO handshake finished"
    };
    let error = match &detail {
        Some(detail) => EngineError::upstream(format!("{what} ({detail}).")),
        None => EngineError::upstream(format!("{what}.")),
    };
    let params = json!({ "detail": detail });
    match (connected, detail.is_some()) {
        (true, false) => error.keyed("socketio.closed_connected", json!({})),
        (true, true) => error.keyed("socketio.closed_connected_detail", params),
        (false, false) => error.keyed("socketio.closed_handshake", json!({})),
        (false, true) => error.keyed("socketio.closed_handshake_detail", params),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target<'a>(url: &'a str, path: &'a str, namespace: &'a str, auth: &'a str) -> Target<'a> {
        Target { url, headers: None, path, namespace, auth, listeners: None }
    }

    fn address(prepared: &Prepared) -> String {
        prepared.request.uri().to_string()
    }

    #[test]
    fn an_address_becomes_the_websocket_address_of_the_server() {
        let prepared = prepare(&target("http://localhost:3000", "", "", ""), None).unwrap();
        assert_eq!(address(&prepared), "ws://localhost:3000/socket.io/?EIO=4&transport=websocket");
        assert_eq!(prepared.namespace, "/");
        assert_eq!(prepared.auth, None);

        let prepared = prepare(&target(" https://example.test ", "", "", ""), None).unwrap();
        assert_eq!(address(&prepared), "wss://example.test/socket.io/?EIO=4&transport=websocket");
        let prepared = prepare(&target("wss://example.test/", "", "", ""), None).unwrap();
        assert_eq!(address(&prepared), "wss://example.test/socket.io/?EIO=4&transport=websocket");
    }

    #[test]
    fn the_path_of_the_address_is_the_namespace_unless_one_is_named() {
        let prepared = prepare(&target("http://h/admin", "", "", ""), None).unwrap();
        assert_eq!(prepared.namespace, "/admin");
        assert_eq!(address(&prepared), "ws://h/socket.io/?EIO=4&transport=websocket");

        let prepared = prepare(&target("http://h/admin", "", "chat", ""), None).unwrap();
        assert_eq!(prepared.namespace, "/chat");
    }

    #[test]
    fn the_servers_path_and_the_query_of_the_address_are_kept() {
        let prepared = prepare(&target("http://h?token=abc&x=1", "/realtime/", "", ""), None).unwrap();
        assert_eq!(address(&prepared), "ws://h/realtime/?token=abc&x=1&EIO=4&transport=websocket");
        let prepared = prepare(&target("http://h?EIO=3", "ws", "", ""), None).unwrap();
        assert_eq!(address(&prepared), "ws://h/ws/?EIO=3&transport=websocket");
    }

    #[test]
    fn variables_are_resolved_everywhere() {
        let env = json!({ "variables": [
            { "key": "HOST", "value": "example.test", "enabled": true },
            { "key": "NS", "value": "chat", "enabled": true },
            { "key": "TOKEN", "value": "secret", "enabled": true },
        ] });
        let headers = json!({ "X-Token": "{{TOKEN}}" });
        let prepared = prepare(
            &Target {
                url: "https://{{HOST}}",
                headers: Some(&headers),
                path: "/{{NS}}-io",
                namespace: "/{{NS}}",
                auth: "{\"token\": \"{{TOKEN}}\"}",
                listeners: None,
            },
            Some(&env),
        )
        .unwrap();
        assert_eq!(address(&prepared), "wss://example.test/chat-io/?EIO=4&transport=websocket");
        assert_eq!(prepared.namespace, "/chat");
        assert_eq!(prepared.auth, Some(json!({ "token": "secret" })));
        assert_eq!(prepared.request.headers().get("x-token").unwrap(), "secret");
    }

    #[test]
    fn what_cannot_be_connected_is_refused_with_a_reason() {
        for url in ["", "not a url", "ftp://h/x", "localhost:3000"] {
            let error = prepare(&target(url, "", "", ""), None).err().expect(url);
            assert!(error.message.starts_with("Invalid URL"), "{url}: {error:?}");
            assert_eq!(error.key(), Some("socketio.invalid_url"));
        }
        for auth in ["token", "[1]", "\"text\"", "{\"open\": "] {
            let error = prepare(&target("http://h", "", "", auth), None).err().expect(auth);
            assert!(error.message.starts_with("Invalid auth"), "{auth}: {error:?}");
            assert_eq!(error.key(), Some("socketio.invalid_auth"));
        }
        let headers = json!({ "bad name": "v" });
        let refused = prepare(
            &Target { url: "http://h", headers: Some(&headers), path: "", namespace: "", auth: "", listeners: None },
            None,
        );
        assert!(refused.err().unwrap().message.contains("header name"));
    }
}
