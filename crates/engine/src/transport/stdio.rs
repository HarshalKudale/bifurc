//! The stdio transport: one session on this process's stdin and stdout, for a parent that spawned the engine
//! as its sidecar. The parent is the user, so there is no handshake to pass: `hello` is answered for the
//! parent's convenience and every command runs with the full scope set. Events are numbered by this
//! connection alone (no replay), and a frame that is not a request ends the session.

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc::{UnboundedSender, unbounded_channel},
};
use uuid::Uuid;

use super::{
    event_log::EngineEvent,
    event_pump::EventPump,
    framing::{FrameDecoder, encode_frame},
    session::SessionOptions,
};
use crate::{
    kernel::{CommandContext, EngineEventBus, ListenerId, bus},
    protocol::{self, EngineError, HELLO_ACTION, PROTOCOL_VERSION},
};

/// Serve one session on this process's stdin and stdout, until stdin closes.
pub async fn run_stdio_server(opts: SessionOptions) {
    serve_stdio(tokio::io::stdin(), tokio::io::stdout(), opts, bus()).await
}

struct Connection {
    opts: SessionOptions,
    bus: &'static EngineEventBus,
    out: UnboundedSender<Value>,
    pump: EventPump,
    seq: Arc<AtomicU64>,
    attached: Mutex<HashMap<String, ListenerId>>,
}

/// The failed response frame for `id`, logged by `command` (when it is one), code and key — never the message.
fn failure(id: &str, command: Option<&str>, error: &EngineError) -> Value {
    crate::runtime::log_failure(command, error);
    json!({ "id": id, "ok": false, "error": error.to_rpc() })
}

fn success(id: &str, data: Value) -> Value {
    json!({ "id": id, "ok": true, "data": data })
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

impl Connection {
    fn reply(&self, frame: Value) {
        let _ = self.out.send(frame);
    }

    fn detach_all(&self) {
        for (_, listener) in self.attached.lock().drain() {
            self.bus.off(listener);
        }
    }

    /// Handle one request; `false` means the frame was not a request and the session is over.
    fn handle_frame(self: &Arc<Self>, frame: Value) -> bool {
        let (Some(id), Some(action)) =
            (frame.get("id").and_then(Value::as_str), frame.get("action").and_then(Value::as_str))
        else {
            return false;
        };
        let payload = frame.get("payload").cloned().unwrap_or(Value::Null);
        match action {
            "subscribe" => self.reply(match self.subscribe(&payload) {
                Ok(events) => success(id, json!({ "events": events })),
                Err(error) => failure(id, Some(action), &error),
            }),
            "unsubscribe" => self.reply(match event_names(&payload, "unsubscribe") {
                Ok(events) => {
                    self.unsubscribe(&events);
                    success(id, json!({ "events": events }))
                }
                Err(error) => failure(id, Some(action), &error),
            }),
            HELLO_ACTION => self.reply(success(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "engineVersion": self.opts.engine_version,
                    "capabilities": [],
                    "sessionId": Uuid::new_v4().to_string(),
                    "hostDomain": "localhost",
                }),
            )),
            _ => {
                let (connection, id, action) = (self.clone(), id.to_string(), action.to_string());
                tokio::spawn(async move {
                    let registry = connection.opts.registry.clone();
                    let ctx = CommandContext::internal(registry.clone());
                    connection.reply(
                        match crate::runtime::with_command(&action, registry.invoke(&action, payload, ctx)).await {
                            Ok(data) => success(&id, data),
                            Err(error) => failure(&id, Some(&action), &error),
                        },
                    );
                });
            }
        }
        true
    }

    fn subscribe(&self, payload: &Value) -> Result<Vec<String>, EngineError> {
        let events = event_names(payload, "subscribe")?;
        if let Some(unknown) = events.iter().find(|wire| !protocol::is_wire_event(wire)) {
            return Err(EngineError::unsupported(format!(
                "This transport cannot deliver \"{unknown}\". It is either not a known event or not yet on the wire."
            ))
            .keyed("transport.event_unsupported", json!({ "wire": unknown })));
        }
        let mut attached = self.attached.lock();
        for wire in &events {
            if attached.contains_key(wire) {
                continue;
            }
            let Some(bus_name) = protocol::bus_name_for(wire) else { continue };
            let (pump, seq, wire_name) = (self.pump.clone(), self.seq.clone(), wire.clone());
            let listener = self.bus.on(bus_name, move |payload| {
                let seq = seq.fetch_add(1, Ordering::Relaxed) + 1;
                pump.push(&EngineEvent { event: wire_name.clone(), seq, payload: payload.clone() });
            });
            attached.insert(wire.clone(), listener);
        }
        Ok(events)
    }

    fn unsubscribe(&self, events: &[String]) {
        let mut attached = self.attached.lock();
        for wire in events {
            if let Some(listener) = attached.remove(wire) {
                self.bus.off(listener);
            }
        }
    }
}

/// Serve one session over any byte stream of length-prefixed frames, until the reader ends, a frame is
/// malformed or the writer fails. `bus` is where the events a client subscribes to come from.
pub async fn serve_stdio<R, W>(mut reader: R, mut writer: W, opts: SessionOptions, bus: &'static EngineEventBus)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (out, mut outbound) = unbounded_channel::<Value>();
    let pump_out = out.clone();
    let connection = Arc::new(Connection {
        opts,
        bus,
        out,
        pump: EventPump::new(move |frame| {
            let _ = pump_out.send(frame);
        }),
        seq: Arc::new(AtomicU64::new(0)),
        attached: Mutex::new(HashMap::new()),
    });
    let mut decoder = FrameDecoder::new();
    let mut buffer = vec![0u8; 64 * 1024];
    'serve: loop {
        tokio::select! {
            read = reader.read(&mut buffer) => {
                let Ok(n) = read else { break };
                if n == 0 {
                    break;
                }
                let Ok(frames) = decoder.push(&buffer[..n]) else { break };
                for frame in frames {
                    if !connection.handle_frame(frame) {
                        break 'serve;
                    }
                }
            }
            frame = outbound.recv() => {
                let Some(frame) = frame else { break };
                let Ok(bytes) = encode_frame(&frame) else { break };
                if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
        }
    }
    connection.detach_all();
    connection.pump.close();
    let _ = writer.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{kernel::CommandRegistry, transport::event_log::EventLog};
    use tokio::io::{DuplexStream, duplex};

    struct Client {
        stream: DuplexStream,
        decoder: FrameDecoder,
        pending: Vec<Value>,
    }

    impl Client {
        async fn send(&mut self, frame: Value) {
            self.stream.write_all(&encode_frame(&frame).unwrap()).await.unwrap();
        }

        /// The next frame, or `None` when the server closed the stream.
        async fn next(&mut self) -> Option<Value> {
            loop {
                if !self.pending.is_empty() {
                    return Some(self.pending.remove(0));
                }
                let mut buffer = [0u8; 4096];
                let n = self.stream.read(&mut buffer).await.unwrap();
                if n == 0 {
                    return None;
                }
                self.pending = self.decoder.push(&buffer[..n]).unwrap();
            }
        }
    }

    fn start() -> (Client, &'static EngineEventBus, tokio::task::JoinHandle<()>) {
        let bus: &'static EngineEventBus = Box::leak(Box::new(EngineEventBus::new()));
        let mut registry = CommandRegistry::new();
        registry
            .register_sync("proxy.status", |_, ctx| Ok(json!({ "running": false, "scoped": ctx.session.is_some() })));
        registry.register_sync("engine.shutdown", |_, _| Ok(json!({ "ok": true })));
        let opts = SessionOptions {
            registry: Arc::new(registry),
            log: EventLog::new(bus),
            engine_version: "9.9.9".into(),
            host_domain: Some("localhost".into()),
            capabilities: Vec::new(),
            scopes_for: None,
            on_auth_failure: None,
            on_session_start: None,
            on_session_end: None,
            refuse_command: None,
            peer: None,
            allowed_events: None,
        };
        let (client, server) = duplex(64 * 1024);
        let (reader, writer) = tokio::io::split(server);
        let task = tokio::spawn(serve_stdio(reader, writer, opts, bus));
        (Client { stream: client, decoder: FrameDecoder::new(), pending: Vec::new() }, bus, task)
    }

    #[tokio::test]
    async fn commands_run_without_a_handshake_and_hello_is_answered() {
        let (mut client, _, task) = start();
        client.send(json!({ "id": "1", "action": "proxy.status", "payload": {} })).await;
        assert_eq!(
            client.next().await.unwrap(),
            json!({ "id": "1", "ok": true, "data": { "running": false, "scoped": false } })
        );
        client.send(json!({ "id": "2", "action": "engine.shutdown", "payload": {} })).await;
        assert_eq!(client.next().await.unwrap()["ok"], true);
        client.send(json!({ "id": "3", "action": "nope.nope", "payload": {} })).await;
        assert_eq!(client.next().await.unwrap()["error"]["code"], "UNKNOWN_COMMAND");

        client.send(json!({ "id": "4", "action": "hello", "payload": {} })).await;
        let hello = client.next().await.unwrap();
        assert_eq!(hello["data"]["capabilities"], json!([]));
        assert_eq!(hello["data"]["engineVersion"], "9.9.9");
        assert_eq!(hello["data"]["hostDomain"], "localhost");
        assert_eq!(hello["data"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(hello["data"]["sessionId"].is_string());

        drop(client);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn events_are_numbered_by_the_connection() {
        let (mut client, bus, task) = start();
        client.send(json!({ "id": "1", "action": "subscribe", "payload": { "events": ["event.ws.status"] } })).await;
        assert_eq!(
            client.next().await.unwrap(),
            json!({ "id": "1", "ok": true, "data": { "events": ["event.ws.status"] } })
        );

        bus.emit("ws.status", json!({ "n": 1 }));
        bus.emit("ws.message", json!({ "ignored": true }));
        bus.emit("ws.status", json!({ "n": 2 }));
        assert_eq!(
            client.next().await.unwrap(),
            json!({ "event": "event.ws.status", "seq": 1, "payload": { "n": 1 } })
        );
        assert_eq!(
            client.next().await.unwrap(),
            json!({ "event": "event.ws.status", "seq": 2, "payload": { "n": 2 } })
        );

        client.send(json!({ "id": "2", "action": "subscribe", "payload": { "events": ["event.bogus"] } })).await;
        assert_eq!(client.next().await.unwrap()["error"]["code"], "UNSUPPORTED");

        client.send(json!({ "id": "3", "action": "unsubscribe", "payload": { "events": ["event.ws.status"] } })).await;
        assert_eq!(client.next().await.unwrap()["ok"], true);
        bus.emit("ws.status", json!({ "n": 3 }));
        client.send(json!({ "id": "4", "action": "proxy.status", "payload": {} })).await;
        assert_eq!(client.next().await.unwrap()["id"], "4");

        drop(client);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_frame_that_is_not_a_request_ends_the_session() {
        let (mut client, _, task) = start();
        client.send(json!({ "id": 7, "action": "proxy.status" })).await;
        assert!(client.next().await.is_none());
        task.await.unwrap();
    }

    #[tokio::test]
    async fn end_of_input_ends_the_session() {
        let (client, _, task) = start();
        drop(client);
        task.await.unwrap();
    }
}
