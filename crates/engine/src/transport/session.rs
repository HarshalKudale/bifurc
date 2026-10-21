//! One client session, independent of the bytes it arrives on. Every transport (socket, WebSocket, stdio)
//! decodes frames into JSON objects and hands them here; what comes back goes out through one channel so a
//! connection has a single writer.
//!
//! A session starts unauthenticated. Its first request must be `hello`; anything else ends the connection.
//! After the handshake each command is checked against the session's scopes and run on the registry, and
//! `subscribe` / `unsubscribe` attach the session to the event log.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use futures::FutureExt as _;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use super::{
    event_log::{EventLog, ReplayOutcome, SubscriptionId},
    event_pump::EventPump,
    scopes::{command_is_allowed, required_scope_for, shell_scopes},
};
use crate::{
    kernel::{CommandContext, CommandRegistry, Scope, SessionIdentity},
    protocol::{
        self, CLIENT_NAMES, EngineError, ErrorCode, HELLO_ACTION, PROTOCOL_VERSION, VersionCompatibility, capability,
    },
};

/// What a session asks its connection to do.
#[derive(Debug)]
pub enum Outbound {
    Frame(Value),
    /// Write the frame, then close: the reply to a request that ended the session.
    FrameThenClose(Value),
    Close,
}

/// Picks the scopes a client gets from its validated `hello`.
pub type ScopesFor = Arc<dyn Fn(&Value) -> HashSet<Scope> + Send + Sync>;
/// Called with the refusal and the client's name, if it gave one.
pub type AuthFailureHook = Arc<dyn Fn(&EngineError, Option<&str>) + Send + Sync>;
pub type SessionHook = Arc<dyn Fn(&SessionIdentity) + Send + Sync>;

#[derive(Clone)]
pub struct SessionOptions {
    pub registry: Arc<CommandRegistry>,
    pub log: Arc<EventLog>,
    pub engine_version: String,
    pub host_domain: Option<String>,
    /// Advertised in the handshake; `events.replay` is added.
    pub capabilities: Vec<&'static str>,
    /// The scopes a client gets, from its validated `hello`. Defaults to every scope.
    pub scopes_for: Option<ScopesFor>,
    /// Called when a handshake is refused: the error code, its message, and the client's name if it gave one.
    pub on_auth_failure: Option<AuthFailureHook>,
    pub on_session_start: Option<SessionHook>,
    pub on_session_end: Option<SessionHook>,
    /// Asked before a command of an identified session runs: the error to refuse it with, if it is refused.
    pub refuse_command: Option<CommandGate>,
    pub peer: Option<String>,
    /// The wire names a session may subscribe to; `None` is every event.
    pub allowed_events: Option<&'static [&'static str]>,
}

/// Given the session and the command's name, why the command is refused, if it is.
pub type CommandGate = Arc<dyn Fn(&SessionIdentity, &str) -> Option<EngineError> + Send + Sync>;

#[derive(Default)]
struct State {
    identity: Option<SessionIdentity>,
    resume_from: Option<u64>,
    subscriptions: HashMap<String, (usize, SubscriptionId)>,
    closed: bool,
}

struct Inner {
    opts: SessionOptions,
    out: UnboundedSender<Outbound>,
    pump: EventPump,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

/// The failed response frame for `id`, logged by `command` (when it is one), code and key — never the message.
fn failure(id: &str, command: Option<&str>, error: &EngineError) -> Value {
    crate::runtime::log_failure(command, error);
    json!({ "id": id, "ok": false, "error": error.to_rpc() })
}

fn success(id: &str, data: Value) -> Value {
    json!({ "id": id, "ok": true, "data": data })
}

fn unsupported_event(wire: &str) -> EngineError {
    EngineError::unsupported(format!(
        "This transport cannot deliver \"{wire}\". It is either not a known event or not yet on the wire."
    ))
    .keyed("transport.event_unsupported", json!({ "wire": wire }))
}

/// The `events` of a subscribe / unsubscribe payload.
fn event_names(payload: &Value, what: &str) -> Result<Vec<String>, EngineError> {
    let invalid = || {
        EngineError::bad_request(format!("Invalid {what} payload: events must be an array of strings"))
            .keyed("transport.events_invalid", json!({ "what": what }))
    };
    let events = payload.get("events").and_then(Value::as_array).ok_or_else(invalid)?;
    events.iter().map(|e| e.as_str().map(str::to_string).ok_or_else(invalid)).collect()
}

impl Session {
    /// Must be called inside the runtime. `out` is the connection's single writer.
    pub fn new(opts: SessionOptions, out: UnboundedSender<Outbound>) -> Self {
        let events_out = out.clone();
        let pump = EventPump::new(move |envelope| {
            let _ = events_out.send(Outbound::Frame(envelope));
        });
        Self { inner: Arc::new(Inner { opts, out, pump, state: Mutex::new(State::default()) }) }
    }

    fn send(&self, frame: Value) {
        let _ = self.inner.out.send(Outbound::Frame(frame));
    }

    /// Handle one inbound frame. Commands run concurrently; their replies are sent when they finish.
    pub fn handle_frame(&self, frame: Value) {
        if self.inner.state.lock().closed {
            return;
        }
        let (Some(id), Some(action)) =
            (frame.get("id").and_then(Value::as_str), frame.get("action").and_then(Value::as_str))
        else {
            let _ = self.inner.out.send(Outbound::Close);
            return;
        };
        let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
        match action {
            "subscribe" => {
                let reply = match self.subscribe(&payload) {
                    Ok(events) => success(id, json!({ "events": events })),
                    Err(error) => failure(id, Some(action), &error),
                };
                self.send(reply);
            }
            "unsubscribe" => {
                let reply = match self.unsubscribe(&payload) {
                    Ok(events) => success(id, json!({ "events": events })),
                    Err(error) => failure(id, Some(action), &error),
                };
                self.send(reply);
            }
            HELLO_ACTION => match self.handshake(&payload) {
                Ok(data) => self.send(success(id, data)),
                Err(error) => self.refuse(id, error),
            },
            _ => self.command(id, action, payload),
        }
    }

    /// Reply with an error; one that says the client has no valid identity also ends the connection.
    fn refuse(&self, id: &str, error: EngineError) {
        let frame = failure(id, None, &error);
        if matches!(error.code, ErrorCode::Unauthorized | ErrorCode::Unsupported) {
            let _ = self.inner.out.send(Outbound::FrameThenClose(frame));
        } else {
            self.send(frame);
        }
    }

    fn handshake(&self, payload: &Value) -> Result<Value, EngineError> {
        let opts = &self.inner.opts;
        if self.inner.state.lock().identity.is_some() {
            return Err(EngineError::conflict(format!(
                "this session is already authenticated; \"{HELLO_ACTION}\" is only valid once"
            ))
            .keyed("transport.already_authenticated", json!({ "action": HELLO_ACTION })));
        }
        let invalid = |reason: &str| {
            EngineError::bad_request(format!("invalid {HELLO_ACTION} payload: {reason}"))
                .keyed("transport.hello_invalid", json!({ "action": HELLO_ACTION, "reason": reason }))
        };
        let protocol_version = payload
            .get("protocolVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("protocolVersion: required"))?;
        let client_name = payload
            .get("clientName")
            .and_then(Value::as_str)
            .filter(|name| CLIENT_NAMES.contains(name))
            .ok_or_else(|| invalid("clientName: expected one of the known client names"))?;
        let client_version =
            payload.get("clientVersion").and_then(Value::as_str).ok_or_else(|| invalid("clientVersion: required"))?;
        let last_seq = match payload.get("lastSeq") {
            None => None,
            Some(value) => Some(value.as_u64().ok_or_else(|| invalid("lastSeq: expected a non-negative integer"))?),
        };

        if protocol::check_version_compatibility(protocol_version, PROTOCOL_VERSION)
            == VersionCompatibility::Incompatible
        {
            let error = EngineError::with_details(
                ErrorCode::Unsupported,
                format!(
                    "protocol version \"{protocol_version}\" is not compatible with this engine's \
                     \"{PROTOCOL_VERSION}\" (major version mismatch)"
                ),
                json!({ "client": protocol_version, "engine": PROTOCOL_VERSION }),
            )
            .keyed("transport.version_incompatible", json!({ "client": protocol_version, "engine": PROTOCOL_VERSION }));
            if let Some(hook) = &opts.on_auth_failure {
                hook(&error, Some(client_name));
            }
            return Err(error);
        }

        let session = match payload.get("session") {
            None | Some(Value::Null) => None,
            Some(Value::String(name))
                if (1..=40).contains(&name.len())
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) =>
            {
                Some(name.clone())
            }
            Some(_) => return Err(invalid("session: expected 1 to 40 letters, digits, '-', '_' or '.'")),
        };
        let identity = SessionIdentity {
            session_id: uuid::Uuid::new_v4().to_string(),
            client_name: Some(client_name.to_string()),
            client_version: Some(client_version.to_string()),
            peer: opts.peer.clone(),
            client_info: payload.get("clientInfo").and_then(super::companions::sanitize_client_info),
            session,
            scopes: Some(opts.scopes_for.as_ref().map(|f| f(payload)).unwrap_or_else(shell_scopes)),
        };

        let mut response = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "engineVersion": opts.engine_version,
            "capabilities": self.capabilities(),
            "sessionId": identity.session_id,
        });
        if let Some(domain) = &opts.host_domain {
            response["hostDomain"] = json!(domain);
        }
        let mut resume_from = None;
        if let Some(last_seq) = last_seq {
            match opts.log.replay_from(last_seq, None) {
                ReplayOutcome::Replay(_) => {
                    response["replayedFrom"] = json!(last_seq);
                    resume_from = Some(last_seq);
                }
                ReplayOutcome::Resync(_) => response["resyncRequired"] = json!(true),
            }
        }

        {
            let mut state = self.inner.state.lock();
            state.identity = Some(identity.clone());
            state.resume_from = resume_from;
        }
        if let Some(hook) = &opts.on_session_start {
            hook(&identity);
        }
        Ok(response)
    }

    fn capabilities(&self) -> Vec<&'static str> {
        let mut capabilities = self.inner.opts.capabilities.clone();
        if !capabilities.contains(&capability::EVENTS_REPLAY) {
            capabilities.push(capability::EVENTS_REPLAY);
        }
        capabilities
    }

    fn command(&self, id: &str, action: &str, payload: Value) {
        let identity = self.inner.state.lock().identity.clone();
        let Some(identity) = identity else {
            let error = EngineError::new(
                ErrorCode::Unauthorized,
                format!(
                    "the first request must be \"{HELLO_ACTION}\"; refusing \"{action}\" on an unauthenticated session"
                ),
            )
            .keyed("transport.hello_required", json!({ "hello": HELLO_ACTION, "action": action }));
            if let Some(hook) = &self.inner.opts.on_auth_failure {
                hook(&error, None);
            }
            self.refuse(id, error);
            return;
        };

        if let Some(error) = self.inner.opts.refuse_command.as_ref().and_then(|gate| gate(&identity, action)) {
            self.send(failure(id, Some(action), &error));
            return;
        }

        let granted = identity.scopes.clone().unwrap_or_else(shell_scopes);
        if !command_is_allowed(&granted, action) {
            // A name with no scope is not a command: let the registry say so.
            if let Some(required) = required_scope_for(action) {
                let mut names: Vec<&str> = granted.iter().map(|s| s.as_str()).collect();
                names.sort_unstable();
                self.send(failure(
                    id,
                    Some(action),
                    &EngineError::with_details(
                        ErrorCode::Forbidden,
                        format!(
                            "this session's scopes do not include \"{}\", which \"{action}\" requires",
                            required.as_str()
                        ),
                        json!({ "required": required.as_str(), "granted": names }),
                    )
                    .keyed("transport.scope_missing", json!({ "required": required.as_str(), "action": action })),
                ));
                return;
            }
        }

        let session = self.clone();
        let (id, action) = (id.to_string(), action.to_string());
        let ctx = CommandContext { registry: self.inner.opts.registry.clone(), session: Some(identity) };
        tokio::spawn(async move {
            let registry = session.inner.opts.registry.clone();
            let client = ctx.session.as_ref().and_then(|s| s.client_name.clone());
            let command = crate::analytics::describe(&action, &payload);
            // The command line runs in a session: a named one of its own, or the default one (see `env::session`).
            let in_session = ctx
                .session
                .as_ref()
                .filter(|s| s.client_name.as_deref() == Some("cli"))
                .map(|s| (crate::env::session::workspace_of(&payload), s.session.clone().unwrap_or_default()));
            let invoking = registry.invoke(&action, payload, ctx);
            let invoking = async move {
                match in_session {
                    Some((ws_id, name)) => crate::env::session::enter(&ws_id, &name, invoking).await,
                    None => invoking.await,
                }
            };
            // Changes the command records say which client (and session) made them.
            let invoking = crate::workspace::changes::as_client(client.as_deref(), invoking);
            // A handler that panics still answers: the client is not left waiting, and the failure is counted.
            let result = std::panic::AssertUnwindSafe(crate::runtime::with_command(&action, invoking))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| {
                    Err(EngineError::engine(format!("\"{action}\" crashed inside the engine"))
                        .keyed("transport.handler_crashed", json!({ "action": action })))
                });
            if let Some(client) = client.filter(|_| registry.is_registered(&action)) {
                let outcome = result.as_ref().map_err(|error| error.code.as_str());
                crate::analytics::record_command(command, &action, &client, outcome);
            }
            let reply = match result {
                Ok(data) => success(&id, data),
                Err(error) => failure(&id, Some(&action), &error),
            };
            session.send(reply);
        });
    }

    fn subscribe(&self, payload: &Value) -> Result<Vec<String>, EngineError> {
        let events = event_names(payload, "subscribe")?;
        if let Some(unknown) = events.iter().find(|wire| !protocol::is_wire_event(wire)) {
            return Err(unsupported_event(unknown));
        }
        if let Some(denied) =
            self.inner.opts.allowed_events.and_then(|ok| events.iter().find(|w| !ok.contains(&w.as_str())))
        {
            return Err(EngineError::new(ErrorCode::Forbidden, format!("this endpoint cannot deliver \"{denied}\""))
                .keyed("transport.event_forbidden", json!({ "event": denied })));
        }
        let mut state = self.inner.state.lock();
        if state.identity.is_none() {
            return Err(EngineError::new(
                ErrorCode::Unauthorized,
                format!("cannot subscribe before the \"{HELLO_ACTION}\" handshake"),
            )
            .keyed("transport.subscribe_before_hello", json!({ "hello": HELLO_ACTION })));
        }
        let log = &self.inner.opts.log;
        for wire in &events {
            if let Some((count, _)) = state.subscriptions.get_mut(wire) {
                *count += 1;
                continue;
            }
            let names = std::slice::from_ref(wire);
            let backlog = match state.resume_from {
                None => Vec::new(),
                Some(resume_from) => match log.replay_from(resume_from, Some(names)) {
                    ReplayOutcome::Replay(events) => events,
                    ReplayOutcome::Resync(reason) => {
                        return Err(EngineError::with_details(
                            ErrorCode::Conflict,
                            format!(
                                "this session's resume point (seq {resume_from}) can no longer be served: {reason}. \
                                 Reconnect without \"lastSeq\" and re-fetch state instead of waiting for events."
                            ),
                            json!({ "lastSeq": resume_from, "reason": reason }),
                        )
                        .keyed("transport.resume_unavailable", json!({ "seq": resume_from, "reason": reason })));
                    }
                },
            };
            let pump = self.inner.pump.clone();
            let subscription = log.subscribe(names, move |event| pump.push(event));
            for event in &backlog {
                self.inner.pump.push(event);
            }
            state.subscriptions.insert(wire.clone(), (1, subscription));
        }
        Ok(events)
    }

    fn unsubscribe(&self, payload: &Value) -> Result<Vec<String>, EngineError> {
        let events = event_names(payload, "unsubscribe")?;
        let mut state = self.inner.state.lock();
        for wire in &events {
            let Some((count, _)) = state.subscriptions.get_mut(wire) else { continue };
            *count -= 1;
            if *count == 0
                && let Some((_, subscription)) = state.subscriptions.remove(wire)
            {
                self.inner.opts.log.unsubscribe(&subscription);
            }
        }
        Ok(events)
    }

    /// The connection ended. Idempotent.
    pub fn close(&self) {
        let identity = {
            let mut state = self.inner.state.lock();
            if state.closed {
                return;
            }
            state.closed = true;
            for (_, (_, subscription)) in state.subscriptions.drain() {
                self.inner.opts.log.unsubscribe(&subscription);
            }
            state.identity.clone()
        };
        self.inner.pump.close();
        if let Some(identity) = &identity {
            crate::mocks::stop_owned_by(&identity.session_id);
            crate::webhook::stop_owned_by(&identity.session_id);
            crate::workspace::claims::release_owned_by(&identity.session_id);
        }
        if let (Some(identity), Some(hook)) = (identity, &self.inner.opts.on_session_end) {
            hook(&identity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::EngineEventBus;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    fn session() -> (Session, UnboundedReceiver<Outbound>, &'static EngineEventBus) {
        let bus: &'static EngineEventBus = Box::leak(Box::new(EngineEventBus::new()));
        let mut registry = CommandRegistry::new();
        registry.register_sync("proxy.status", |_, _| Ok(json!({ "running": false })));
        crate::mocks::register(&mut registry);
        let (tx, rx) = unbounded_channel();
        let opts = SessionOptions {
            registry: Arc::new(registry),
            log: EventLog::new(bus),
            engine_version: "0.0.0".into(),
            host_domain: Some("localhost".into()),
            capabilities: vec![capability::SOCKET],
            scopes_for: None,
            on_auth_failure: None,
            on_session_start: None,
            on_session_end: None,
            // Only `gated.command` is ever refused, so the other tests see no gate.
            refuse_command: Some(Arc::new(|identity, action| {
                (action == "gated.command")
                    .then(|| EngineError::new(ErrorCode::Forbidden, format!("not for {:?}", identity.client_name)))
            })),
            peer: None,
            allowed_events: None,
        };
        (Session::new(opts, tx), rx, bus)
    }

    #[tokio::test]
    async fn a_command_the_gate_refuses_is_answered_with_the_gates_error_and_the_session_goes_on() {
        let (session, mut rx, _) = session();
        session.handle_frame(hello());
        frame(&mut rx).await;
        session.handle_frame(json!({ "id": "1", "action": "gated.command", "payload": {} }));
        let reply = frame(&mut rx).await;
        assert_eq!((reply["ok"].as_bool(), reply["error"]["code"].as_str()), (Some(false), Some("FORBIDDEN")));
        assert_eq!(reply["error"]["message"], "not for Some(\"cli\")");
        session.handle_frame(json!({ "id": "2", "action": "proxy.status", "payload": {} }));
        assert_eq!(frame(&mut rx).await["ok"], true);
    }

    #[tokio::test]
    async fn a_companion_session_gets_status_and_the_extensions_events_only() {
        use crate::transport::scopes::{COMPANION_EVENTS, companion_refusal, companion_scopes};
        let bus: &'static EngineEventBus = Box::leak(Box::new(EngineEventBus::new()));
        let mut registry = CommandRegistry::new();
        for name in ["server.status", "capture.get", "env.getMerged", "entity.list"] {
            registry.register_sync(name, |_, _| Ok(json!({})));
        }
        let (tx, mut rx) = unbounded_channel();
        let session = Session::new(
            SessionOptions {
                registry: Arc::new(registry),
                log: EventLog::new(bus),
                engine_version: "0.0.0".into(),
                host_domain: None,
                capabilities: vec![],
                scopes_for: Some(Arc::new(|_| companion_scopes())),
                on_auth_failure: None,
                on_session_start: None,
                on_session_end: None,
                refuse_command: Some(Arc::new(|_, action| companion_refusal(action))),
                peer: None,
                allowed_events: Some(COMPANION_EVENTS),
            },
            tx,
        );
        session.handle_frame(hello());
        frame(&mut rx).await;
        let mut ask = async |action: &str, payload: Value| {
            session.handle_frame(json!({ "id": "x", "action": action, "payload": payload }));
            frame(&mut rx).await
        };
        assert_eq!(ask("server.status", json!({})).await["ok"], true);
        for action in ["capture.get", "env.getMerged", "entity.list"] {
            let reply = ask(action, json!({})).await;
            assert_eq!(reply["error"]["code"], "FORBIDDEN", "{action}");
            assert_eq!(reply["error"]["message"], "the companion endpoint only serves status");
        }
        let events = json!({ "events": ["event.settings.changed", "event.server.error", "event.engine.stopping"] });
        assert_eq!(ask("subscribe", events).await["ok"], true);
        let reply = ask("subscribe", json!({ "events": ["event.ws.status"] })).await;
        assert_eq!(reply["error"]["code"], "FORBIDDEN");
    }

    fn hello() -> Value {
        json!({ "id": "h", "action": "hello",
            "payload": { "protocolVersion": "1.0.0", "clientName": "cli", "clientVersion": "1" } })
    }

    async fn frame(rx: &mut UnboundedReceiver<Outbound>) -> Value {
        match rx.recv().await.expect("a frame") {
            Outbound::Frame(frame) | Outbound::FrameThenClose(frame) => frame,
            Outbound::Close => panic!("closed"),
        }
    }

    #[tokio::test]
    async fn a_command_before_hello_ends_the_session() {
        let (session, mut rx, _) = session();
        session.handle_frame(json!({ "id": "1", "action": "proxy.status", "payload": {} }));
        match rx.recv().await.unwrap() {
            Outbound::FrameThenClose(frame) => assert_eq!(frame["error"]["code"], "UNAUTHORIZED"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn a_mock_server_started_in_a_session_stops_when_the_session_ends() {
        let _root = crate::testing::scratch_data_root();
        let (session, mut rx, _) = session();
        session.handle_frame(hello());
        frame(&mut rx).await;
        session.handle_frame(
            json!({ "id": "1", "action": "mocks.serve", "payload": { "workspaceId": "ws1", "port": 0 } }),
        );
        let port = frame(&mut rx).await["data"]["port"].as_u64().unwrap() as u16;
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok());
        session.close();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }

    #[tokio::test]
    async fn hello_then_commands_and_events() {
        let (session, mut rx, bus) = session();
        session.handle_frame(hello());
        let reply = frame(&mut rx).await;
        assert_eq!(reply["data"]["capabilities"], json!(["socket", "events.replay"]));
        assert_eq!(reply["data"]["hostDomain"], "localhost");

        session.handle_frame(json!({ "id": "1", "action": "proxy.status", "payload": {} }));
        assert_eq!(frame(&mut rx).await, json!({ "id": "1", "ok": true, "data": { "running": false } }));

        session.handle_frame(json!({ "id": "2", "action": "nope.nope", "payload": {} }));
        assert_eq!(frame(&mut rx).await["error"]["code"], "UNKNOWN_COMMAND");

        session.handle_frame(json!({ "id": "3", "action": "subscribe", "payload": { "events": ["event.ws.status"] } }));
        assert_eq!(frame(&mut rx).await["ok"], true);
        bus.emit("ws.status", json!({ "socketId": "s" }));
        let event = frame(&mut rx).await;
        assert_eq!(event, json!({ "event": "event.ws.status", "seq": 1, "payload": { "socketId": "s" } }));

        session.handle_frame(json!({ "id": "4", "action": "subscribe", "payload": { "events": ["event.bogus"] } }));
        assert_eq!(frame(&mut rx).await["error"]["code"], "UNSUPPORTED");
    }
}
