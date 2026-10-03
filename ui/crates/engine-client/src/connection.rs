//! One session with the engine over its local socket: request/response correlation and the event stream.
//!
//! The engine's wire is symmetric JSON frames. A request is `{id, action, payload}` and is answered by
//! `{id, ok: true, data}` or `{id, ok: false, error}`; an event is `{event, seq, payload}` and carries no
//! id. A dedicated reader thread owns the receive half and routes both; everything else is the send half
//! behind a mutex. There is no async runtime here on purpose: a call returns a plain future over a oneshot
//! channel, so the caller (GPUI's executor, a test) chooses where it runs.

use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use futures::channel::{mpsc, oneshot};
use interprocess::local_socket::{GenericFilePath, Stream, prelude::*};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    error::{ClientError, EngineError, Result},
    frame::{read_frame, write_frame},
};

/// The protocol version this client speaks. The engine accepts any 1.x.
pub use bifurc_protocol::PROTOCOL_VERSION;
/// A client name the protocol registers (`bifurc_protocol::CLIENT_NAMES`).
pub const CLIENT_NAME: &str = "desktop-ui";

type Pending = oneshot::Sender<Result<Value>>;

/// A broadcast from the engine: `event` is the full wire name, e.g. `event.entity.changed`.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineEvent {
    pub event: String,
    pub seq: u64,
    pub payload: Value,
}

/// What a subscriber receives. `Closed` is delivered once, when the connection ends for any reason.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Event(EngineEvent),
    Closed,
}

/// The engine's answer to `hello`.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub protocol_version: String,
    pub engine_version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub session_id: String,
    #[serde(default)]
    pub host_domain: Option<String>,
}

struct Shared {
    writer: Mutex<Box<dyn Write + Send>>,
    pending: Mutex<HashMap<String, Pending>>,
    subscribers: Mutex<Vec<mpsc::UnboundedSender<Incoming>>>,
    next_id: AtomicU64,
    closed: AtomicBool,
}

impl Shared {
    fn fail_everything(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let waiting: Vec<Pending> = self.pending.lock().unwrap().drain().map(|(_, p)| p).collect();
        for sender in waiting {
            let _ = sender.send(Err(ClientError::Closed));
        }
        for subscriber in self.subscribers.lock().unwrap().drain(..) {
            let _ = subscriber.unbounded_send(Incoming::Closed);
        }
    }

    fn deliver_event(&self, record: &Value) {
        let Some(event) = record.get("event").and_then(Value::as_str) else { return };
        let incoming = Incoming::Event(EngineEvent {
            event: event.to_owned(),
            seq: record.get("seq").and_then(Value::as_u64).unwrap_or(0),
            payload: record.get("payload").cloned().unwrap_or(Value::Null),
        });
        self.subscribers.lock().unwrap().retain(|s| s.unbounded_send(incoming.clone()).is_ok());
    }

    fn deliver_response(&self, record: &Value) {
        let Some(id) = record.get("id").and_then(Value::as_str) else { return };
        let Some(sender) = self.pending.lock().unwrap().remove(id) else { return };
        let outcome = match record.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(record.get("data").cloned().unwrap_or(Value::Null)),
            Some(false) => Err(EngineError::from_wire(record.get("error").unwrap_or(&Value::Null)).into()),
            None => Err(ClientError::Protocol("response frame has no boolean `ok`".to_owned())),
        };
        let _ = sender.send(outcome);
    }
}

fn read_loop(shared: &Shared, mut reader: impl Read) {
    // A frame that cannot be read (end of stream, a broken pipe, a malformed frame) ends the session.
    while let Ok(Some(record)) = read_frame(&mut reader) {
        if record.get("event").is_some_and(Value::is_string) {
            shared.deliver_event(&record);
        } else {
            shared.deliver_response(&record);
        }
    }
    shared.fail_everything();
}

/// A handle on one engine session. Cheap to clone; every clone shares the connection.
#[derive(Clone)]
pub struct EngineClient {
    shared: Arc<Shared>,
}

impl EngineClient {
    /// Opens the socket at `address` (a named pipe on Windows, a socket path elsewhere) and says `hello`.
    pub fn connect(address: &str, client_version: &str) -> Result<(Self, Hello)> {
        Self::connect_attempts(address, client_version, 20)
    }

    /// `connect` with a bound on how often a refused connection is retried (100 ms apart). One attempt is
    /// right for probing a possibly stale `runtime.json`; the default rides out a busy named pipe.
    pub fn connect_attempts(address: &str, client_version: &str, attempts: u32) -> Result<(Self, Hello)> {
        let stream = connect_with_retry(address, attempts)?;
        let (reader, writer) = stream.split();
        let client = Self::from_halves(reader, writer);
        let hello = client.call_blocking(
            "hello",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientName": CLIENT_NAME,
                "clientVersion": client_version,
            }),
        )?;
        Ok((client, serde_json::from_value(hello)?))
    }

    /// Builds a client over any byte stream pair. The seam the tests use; `connect` is the only other caller.
    pub fn from_halves(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Self {
        let shared = Arc::new(Shared {
            writer: Mutex::new(Box::new(writer)),
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            closed: AtomicBool::new(false),
        });
        let for_reader = Arc::clone(&shared);
        thread::Builder::new()
            .name("engine-reader".to_owned())
            .spawn(move || read_loop(&for_reader, reader))
            .expect("spawning the engine reader thread");
        Self { shared }
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }

    /// Sends `action` and resolves with its `data`. The frame is written when the future is first polled,
    /// so the write — which can block on a full pipe — happens on whichever thread the caller runs it on.
    pub async fn call(&self, action: &str, payload: Value) -> Result<Value> {
        let receiver = self.send_request(action, payload)?;
        receiver.await.map_err(|_| ClientError::Closed)?
    }

    /// `call`, with the result decoded into `T`.
    pub async fn call_as<T: DeserializeOwned>(&self, action: &str, payload: Value) -> Result<T> {
        Ok(serde_json::from_value(self.call(action, payload).await?)?)
    }

    /// For code that has no executor to hand: the handshake, tests, shutdown on exit.
    pub fn call_blocking(&self, action: &str, payload: Value) -> Result<Value> {
        futures::executor::block_on(self.call(action, payload))
    }

    /// Starts delivery of `events` (full wire names) to the returned receiver. Every subscriber sees every
    /// subscribed event: filtering by name is the receiver's job.
    pub async fn subscribe(&self, events: &[&str]) -> Result<mpsc::UnboundedReceiver<Incoming>> {
        let (sender, receiver) = mpsc::unbounded();
        if self.is_closed() {
            return Err(ClientError::Closed);
        }
        self.shared.subscribers.lock().unwrap().push(sender);
        self.call("subscribe", json!({ "events": events })).await?;
        Ok(receiver)
    }

    fn send_request(&self, action: &str, payload: Value) -> Result<oneshot::Receiver<Result<Value>>> {
        if self.is_closed() {
            return Err(ClientError::Closed);
        }
        let id = format!("ui-{}", self.shared.next_id.fetch_add(1, Ordering::Relaxed));
        let (sender, receiver) = oneshot::channel();
        self.shared.pending.lock().unwrap().insert(id.clone(), sender);

        let frame = json!({ "id": id, "action": action, "payload": payload });
        let mut writer = self.shared.writer.lock().unwrap();
        let written = write_frame(&mut *writer, &frame);
        drop(writer);
        if let Err(err) = written {
            self.shared.pending.lock().unwrap().remove(&id);
            self.shared.fail_everything();
            return Err(err);
        }
        Ok(receiver)
    }
}

/// A named pipe server accepts one client per pending instance, so a connect that races another can see
/// `ERROR_PIPE_BUSY`; the engine's own clients retry the same way.
fn connect_with_retry(address: &str, attempts: u32) -> Result<Stream> {
    let mut last = None;
    for attempt in 0..attempts.max(1) {
        let name = address.to_fs_name::<GenericFilePath>()?;
        match Stream::connect(name) {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                last = Some(err);
                if attempt + 1 < attempts {
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
    Err(last.expect("at least one attempt ran").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{encode_frame, read_frame};
    use std::io::{self, Cursor};

    /// A reader the test feeds from another thread, standing in for the engine's side of the pipe.
    struct Channel(std::sync::mpsc::Receiver<Vec<u8>>, Cursor<Vec<u8>>);
    impl Read for Channel {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            loop {
                let n = self.1.read(buf)?;
                if n > 0 {
                    return Ok(n);
                }
                match self.0.recv() {
                    Ok(chunk) => self.1 = Cursor::new(chunk),
                    Err(_) => return Ok(0),
                }
            }
        }
    }

    /// A writer that records every frame the client sends.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);
    impl Write for Sink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn harness() -> (EngineClient, std::sync::mpsc::Sender<Vec<u8>>, Sink) {
        let (to_client, from_engine) = std::sync::mpsc::channel();
        let sink = Sink::default();
        let client = EngineClient::from_halves(Channel(from_engine, Cursor::new(Vec::new())), sink.clone());
        (client, to_client, sink)
    }

    fn sent_frames(sink: &Sink) -> Vec<Value> {
        let bytes = sink.0.lock().unwrap().clone();
        let mut reader = bytes.as_slice();
        let mut frames = Vec::new();
        while let Some(frame) = read_frame(&mut reader).unwrap() {
            frames.push(frame);
        }
        frames
    }

    #[test]
    fn a_response_resolves_the_call_with_the_matching_id() {
        let (client, engine, sink) = harness();
        let pending = std::thread::spawn({
            let client = client.clone();
            move || client.call_blocking("config.get", json!({}))
        });
        while sent_frames(&sink).is_empty() {
            thread::sleep(Duration::from_millis(5));
        }
        let id = sent_frames(&sink)[0]["id"].as_str().unwrap().to_owned();
        engine.send(encode_frame(&json!({"id": id, "ok": true, "data": {"port": 80}})).unwrap()).unwrap();
        assert_eq!(pending.join().unwrap().unwrap(), json!({"port": 80}));
    }

    #[test]
    fn a_failed_response_becomes_an_engine_error_with_its_code() {
        let (client, engine, sink) = harness();
        let pending = std::thread::spawn({
            let client = client.clone();
            move || client.call_blocking("entity.load", json!({}))
        });
        while sent_frames(&sink).is_empty() {
            thread::sleep(Duration::from_millis(5));
        }
        let id = sent_frames(&sink)[0]["id"].as_str().unwrap().to_owned();
        engine
            .send(
                encode_frame(
                    &json!({"id": id, "ok": false, "error": {"code": "NOT_FOUND", "message": "no such entity"}}),
                )
                .unwrap(),
            )
            .unwrap();
        match pending.join().unwrap() {
            Err(ClientError::Engine(e)) => {
                assert_eq!(e.code, "NOT_FOUND");
                assert_eq!(e.message, "no such entity");
            }
            other => panic!("expected an engine error, got {other:?}"),
        }
    }

    #[test]
    fn events_reach_every_subscriber_and_the_end_of_the_stream_is_announced_once() {
        let (client, engine, _sink) = harness();
        let (a, mut a_rx) = mpsc::unbounded();
        let (b, mut b_rx) = mpsc::unbounded();
        client.shared.subscribers.lock().unwrap().extend([a, b]);

        engine
            .send(encode_frame(&json!({"event": "event.ws.message", "seq": 7, "payload": {"x": 1}})).unwrap())
            .unwrap();
        drop(engine);

        let expected =
            Incoming::Event(EngineEvent { event: "event.ws.message".to_owned(), seq: 7, payload: json!({"x": 1}) });
        for rx in [&mut a_rx, &mut b_rx] {
            assert_eq!(futures::executor::block_on(futures::StreamExt::next(rx)), Some(expected.clone()));
            assert_eq!(futures::executor::block_on(futures::StreamExt::next(rx)), Some(Incoming::Closed));
            assert_eq!(futures::executor::block_on(futures::StreamExt::next(rx)), None);
        }
    }

    #[test]
    fn calls_in_flight_fail_with_closed_when_the_engine_goes_away() {
        let (client, engine, sink) = harness();
        let pending = std::thread::spawn({
            let client = client.clone();
            move || client.call_blocking("config.get", json!({}))
        });
        while sent_frames(&sink).is_empty() {
            thread::sleep(Duration::from_millis(5));
        }
        drop(engine);
        assert!(matches!(pending.join().unwrap(), Err(ClientError::Closed)));
        assert!(matches!(client.call_blocking("config.get", json!({})), Err(ClientError::Closed)));
    }
}
