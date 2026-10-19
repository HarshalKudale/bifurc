//! One blocking session with the engine: 4-byte big-endian length-prefixed JSON frames over a named pipe
//! (Windows) or a unix socket, opened with a `hello` handshake that names this client `cli`.
//!
//! The CLI asks one question at a time, so a request is written and the reader waits for its response;
//! broadcast events the engine sends meanwhile are skipped.

use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::Path,
};

use bifurc_client::frame::{self, FrameError};
use serde_json::{Value, json};

use crate::{
    discovery::{RuntimeInfo, find_running_engine},
    error::{CliError, Result},
    protocol::{CLIENT_NAME, PROTOCOL_VERSION},
};

/// The largest frame either side may send.
pub use bifurc_client::frame::MAX_FRAME_BYTES;

/// What the commands need of an engine session; tests substitute their own.
pub trait Engine {
    /// The `runtime.json` of the engine this session is with.
    fn info(&self) -> &RuntimeInfo;
    /// Send `command` and return its `data`; a refusal is a [`CliError::Engine`].
    fn request(&mut self, command: &str, payload: Value) -> Result<Value>;
    /// The next event the engine sends on its own, waiting for it; `None` when the connection is closed.
    /// Only events subscribed to with the `subscribe` command arrive.
    fn next_event(&mut self) -> Result<Option<Value>>;
}

fn frame_error(err: FrameError) -> CliError {
    match err {
        FrameError::Io(e) => io_error(e),
        FrameError::Protocol(message) => CliError::msg(format!("protocol error: {message}")),
    }
}

/// Encode one frame: the length, then the JSON.
pub fn encode_frame(message: &Value) -> Result<Vec<u8>> {
    frame::encode_frame(message).map_err(frame_error)
}

/// Read one frame. `Ok(None)` is a clean end of stream between frames.
pub fn read_frame(reader: &mut impl Read) -> Result<Option<Value>> {
    frame::read_frame(reader).map_err(frame_error)
}

fn io_error(err: std::io::Error) -> CliError {
    CliError::msg(format!("i/o error talking to the engine: {err}"))
}

/// `details.reason` of the engine's refusal to serve the command line before the first-run setup is finished.
const SETUP_REQUIRED: &str = "setupRequired";

/// The error of a failed response.
fn engine_error(raw: &Value) -> CliError {
    let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_string);
    // The engine refuses the command line until the first-run setup is finished, and opens the interface
    // itself. Its message is the whole story; the code in front of it would only be noise.
    if raw.pointer("/details/reason").and_then(Value::as_str) == Some(SETUP_REQUIRED)
        && let Some(message) = text("message")
    {
        return CliError::msg(message);
    }
    match (text("code"), text("message")) {
        (Some(code), Some(message)) => CliError::Engine { code, message },
        _ => CliError::Engine {
            code: "ENGINE_ERROR".to_string(),
            message: "the engine returned an error without a code and a message".to_string(),
        },
    }
}

/// A request/response session over any byte stream.
pub struct Session<S: Read + Write> {
    stream: S,
    next_id: u64,
    /// Events that arrived while a call waited for its response, for [`Session::next_event`].
    pending: VecDeque<Value>,
}

impl<S: Read + Write> Session<S> {
    /// Wrap a connected stream. Nothing is sent until [`Session::hello`] or [`Session::call`].
    pub fn new(stream: S) -> Self {
        Self { stream, next_id: 1, pending: VecDeque::new() }
    }

    /// The next event, from those a call set aside first; `None` when the engine closed the connection.
    pub fn next_event(&mut self) -> Result<Option<Value>> {
        if let Some(event) = self.pending.pop_front() {
            return Ok(Some(event));
        }
        loop {
            match read_frame(&mut self.stream)? {
                None => return Ok(None),
                Some(record) if record.get("event").is_some_and(Value::is_string) => return Ok(Some(record)),
                Some(_) => {}
            }
        }
    }

    /// Send `action` and wait for its response; events that arrive meanwhile are kept for [`Session::next_event`].
    pub fn call(&mut self, action: &str, payload: Value) -> Result<Value> {
        let id = format!("cli-{}", self.next_id);
        self.next_id += 1;
        let frame = encode_frame(&json!({ "id": id, "action": action, "payload": payload }))?;
        self.stream.write_all(&frame).and_then(|()| self.stream.flush()).map_err(io_error)?;
        loop {
            let Some(record) = read_frame(&mut self.stream)? else {
                return Err(CliError::msg("the connection to the engine is closed"));
            };
            if record.get("event").is_some_and(Value::is_string) {
                self.pending.push_back(record);
                continue;
            }
            if record.get("id").and_then(Value::as_str) != Some(&id) {
                continue;
            }
            return match record.get("ok").and_then(Value::as_bool) {
                Some(true) => Ok(record.get("data").cloned().unwrap_or(Value::Null)),
                Some(false) => Err(engine_error(record.get("error").unwrap_or(&Value::Null))),
                None => Err(CliError::msg("protocol error: response frame has no boolean `ok`")),
            };
        }
    }

    /// The next frame the engine sends on its own (an event), waiting for it; `None` when the engine closed the
    /// connection. Use it on a session that makes no more calls: reads and writes on one pipe handle block each other.
    pub fn read_frame(&mut self) -> Result<Option<Value>> {
        read_frame(&mut self.stream)
    }

    /// The `hello` handshake.
    pub fn hello(&mut self, client_version: &str) -> Result<Value> {
        self.call(
            "hello",
            json!({ "protocolVersion": PROTOCOL_VERSION, "clientName": CLIENT_NAME, "clientVersion": client_version }),
        )
    }
}

#[cfg(windows)]
pub type Stream = std::fs::File;
#[cfg(unix)]
pub type Stream = std::os::unix::net::UnixStream;

/// `ERROR_PIPE_BUSY`: the pipe server has no free instance for this client right now.
#[cfg(windows)]
const PIPE_BUSY: i32 = 231;

#[cfg(windows)]
fn open_stream(address: &str) -> std::io::Result<Stream> {
    let mut attempts = 0;
    loop {
        match std::fs::OpenOptions::new().read(true).write(true).open(address) {
            Err(e) if e.raw_os_error() == Some(PIPE_BUSY) && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

#[cfg(unix)]
fn open_stream(address: &str) -> std::io::Result<Stream> {
    Stream::connect(Path::new(address))
}

/// A session with a live engine.
pub struct EngineConnection {
    info: RuntimeInfo,
    session: Session<Stream>,
}

impl EngineConnection {
    /// The bare session, for a command that keeps the connection open after its calls.
    pub fn into_session(self) -> Session<Stream> {
        self.session
    }
}

impl Engine for EngineConnection {
    fn info(&self) -> &RuntimeInfo {
        &self.info
    }

    fn request(&mut self, command: &str, payload: Value) -> Result<Value> {
        self.session.call(command, payload)
    }

    fn next_event(&mut self) -> Result<Option<Value>> {
        self.session.next_event()
    }
}

/// Why `info` cannot be reached with this client, if it cannot.
pub fn unreachable_reason(info: &RuntimeInfo, data_dir: &Path) -> Option<String> {
    match info.transport.as_str() {
        "stdio" => Some(format!(
            "The engine for {} (pid {}) serves stdio only — it belongs to the process that spawned it and cannot be \
             reached from here.",
            data_dir.display(),
            info.pid
        )),
        "ws" => Some(format!(
            "The engine for {} (pid {}) serves WebSocket at {}, which this CLI does not speak. Stop it and start one \
             with: bifurc engine start --transport socket",
            data_dir.display(),
            info.pid,
            info.address
        )),
        _ => None,
    }
}

/// Connect to the engine serving `data_dir`, found through its runtime.json, and open a session.
pub fn connect(data_dir: &Path, client_version: &str) -> Result<EngineConnection> {
    let Some(info) = find_running_engine(data_dir) else {
        return Err(CliError::msg(format!(
            "No engine is running for {}.\nStart one with: bifurc engine start",
            data_dir.display()
        )));
    };
    if let Some(reason) = unreachable_reason(&info, data_dir) {
        return Err(CliError::msg(reason));
    }
    let open_failed = |reason: String| {
        CliError::msg(format!("Could not open a session with the engine at {}: {reason}", info.address))
    };
    let stream = open_stream(&info.address).map_err(|e| open_failed(e.to_string()))?;
    let mut session = Session::new(stream);
    session.hello(client_version).map_err(|e| open_failed(e.to_string()))?;
    Ok(EngineConnection { info, session })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A stream that answers from a canned script and records what was written.
    struct Scripted {
        incoming: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl Scripted {
        fn new(frames: &[Value]) -> Self {
            let bytes = frames.iter().flat_map(|f| encode_frame(f).unwrap()).collect();
            Self { incoming: Cursor::new(bytes), written: Vec::new() }
        }
    }

    impl Read for Scripted {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.incoming.read(buf)
        }
    }

    impl Write for Scripted {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn sent(session: &Session<Scripted>) -> Vec<Value> {
        let mut reader = session.stream.written.as_slice();
        let mut frames = Vec::new();
        while let Some(frame) = read_frame(&mut reader).unwrap() {
            frames.push(frame);
        }
        frames
    }

    #[test]
    fn a_frame_round_trips() {
        let message = json!({"id": "1", "action": "config.get", "payload": {}});
        let bytes = encode_frame(&message).unwrap();
        assert_eq!(&bytes[..4], &(bytes.len() as u32 - 4).to_be_bytes());
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), Some(message));
    }

    #[test]
    fn multibyte_text_is_measured_in_bytes() {
        let message = json!({"text": "héllo — ✓"});
        let bytes = encode_frame(&message).unwrap();
        assert_eq!(u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize, bytes.len() - 4);
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), Some(message));
    }

    #[test]
    fn a_clean_end_of_stream_is_none_and_a_cut_frame_is_an_error() {
        assert_eq!(read_frame(&mut [].as_slice()).unwrap(), None);
        let mut bytes = encode_frame(&json!({"a": 1})).unwrap();
        bytes.pop();
        assert!(read_frame(&mut bytes.as_slice()).is_err());
    }

    #[test]
    fn oversized_and_non_object_frames_are_refused() {
        let header = ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes();
        assert!(read_frame(&mut header.as_slice()).is_err());
        let mut array = 5u32.to_be_bytes().to_vec();
        array.extend_from_slice(b"[1,2]");
        assert!(read_frame(&mut array.as_slice()).is_err());
    }

    #[test]
    fn a_call_returns_the_data_of_the_response_with_its_id() {
        let stream = Scripted::new(&[
            json!({"event": "event.entity.changed", "seq": 1, "payload": {}}),
            json!({"id": "other", "ok": true, "data": "not mine"}),
            json!({"id": "cli-1", "ok": true, "data": {"port": 80}}),
        ]);
        let mut session = Session::new(stream);
        assert_eq!(session.call("config.get", json!({})).unwrap(), json!({"port": 80}));
        let frames = sent(&session);
        assert_eq!(frames[0], json!({"id": "cli-1", "action": "config.get", "payload": {}}));
    }

    #[test]
    fn ids_advance_per_call() {
        let stream =
            Scripted::new(&[json!({"id": "cli-1", "ok": true, "data": 1}), json!({"id": "cli-2", "ok": true})]);
        let mut session = Session::new(stream);
        assert_eq!(session.call("a.b", json!({})).unwrap(), json!(1));
        assert_eq!(session.call("c.d", json!({})).unwrap(), Value::Null);
    }

    #[test]
    fn a_failed_response_is_an_engine_error_with_its_code() {
        let stream = Scripted::new(&[
            json!({"id": "cli-1", "ok": false, "error": {"code": "UNKNOWN_COMMAND", "message": "no such command"}}),
        ]);
        let mut session = Session::new(stream);
        let err = session.call("x.y", json!({})).unwrap_err();
        assert_eq!(err, CliError::Engine { code: "UNKNOWN_COMMAND".into(), message: "no such command".into() });
        assert_eq!(err.line(), "bifurc: UNKNOWN_COMMAND: no such command");
    }

    #[test]
    fn the_refusal_before_the_first_run_setup_is_shown_as_the_engine_worded_it() {
        let error = json!({"code": "FORBIDDEN", "message": "Bifurc has not been set up yet.",
            "details": {"reason": "setupRequired"}});
        let stream = Scripted::new(&[json!({"id": "cli-1", "ok": false, "error": error})]);
        let err = Session::new(stream).call("config.get", json!({})).unwrap_err();
        assert_eq!(err.line(), "bifurc: Bifurc has not been set up yet.");
        // Any other refusal keeps its code.
        let error = json!({"code": "FORBIDDEN", "message": "no", "details": {"required": "admin"}});
        let stream = Scripted::new(&[json!({"id": "cli-1", "ok": false, "error": error})]);
        let err = Session::new(stream).call("config.get", json!({})).unwrap_err();
        assert_eq!(err.line(), "bifurc: FORBIDDEN: no");
    }

    #[test]
    fn an_error_without_code_and_message_still_fails() {
        let stream = Scripted::new(&[json!({"id": "cli-1", "ok": false, "error": "boom"})]);
        let err = Session::new(stream).call("x.y", json!({})).unwrap_err();
        assert!(matches!(err, CliError::Engine { ref code, .. } if code == "ENGINE_ERROR"));
    }

    #[test]
    fn a_closed_connection_ends_the_call() {
        let mut session = Session::new(Scripted::new(&[]));
        let err = session.call("x.y", json!({})).unwrap_err();
        assert_eq!(err, CliError::msg("the connection to the engine is closed"));
    }

    #[test]
    fn hello_identifies_the_client_as_cli() {
        let stream = Scripted::new(&[json!({"id": "cli-1", "ok": true, "data": {"engineVersion": "0.4.0"}})]);
        let mut session = Session::new(stream);
        session.hello("9.9.9").unwrap();
        assert_eq!(
            sent(&session)[0],
            json!({"id": "cli-1", "action": "hello",
                   "payload": {"protocolVersion": "1.0.0", "clientName": "cli", "clientVersion": "9.9.9"}})
        );
    }

    #[test]
    fn connecting_without_an_engine_says_how_to_start_one() {
        let dir = tempfile::tempdir().unwrap();
        let err = connect(dir.path(), "t").err().unwrap();
        assert_eq!(
            err.to_string(),
            format!("No engine is running for {}.\nStart one with: bifurc engine start", dir.path().display())
        );
    }

    #[test]
    fn engines_that_cannot_be_reached_are_explained() {
        let dir = Path::new("/d");
        let mut info = RuntimeInfo::from_json(json!({"pid": 7, "transport": "stdio", "address": "-"})).unwrap();
        assert!(unreachable_reason(&info, dir).unwrap().contains("serves stdio only"));
        info.transport = "ws".into();
        assert!(unreachable_reason(&info, dir).unwrap().contains("WebSocket"));
        info.transport = "socket".into();
        assert_eq!(unreachable_reason(&info, dir), None);
    }
}
