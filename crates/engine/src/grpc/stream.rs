//! Live gRPC calls: `grpc.stream.open|send|end|cancel`.
//!
//! `grpc.execute` runs a call to its end and answers once. A call that streams needs to stay open: a server
//! that streams replies keeps sending them, and a call that streams from the client waits for the user's next
//! message. So a live call is a task, named by the caller (`streamId`, as a socket is named by `socketId`), that
//! reports through `grpc.stream.message` / `grpc.stream.status` events and is fed by the other commands.

use std::{
    collections::{BTreeMap, HashMap},
    sync::LazyLock,
};

use parking_lot::Mutex;
use prost_reflect::{DynamicMessage, MessageDescriptor, MethodDescriptor};
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tonic::{Code, Request, Status, Streaming, client::Grpc, metadata::MetadataMap, transport::Channel};

use super::call::{self, DynamicCodec};
use crate::{
    kernel::{CommandRegistry, bus},
    protocol::EngineError,
    utils::{
        id::now_ms,
        json::{str_of, string_map},
    },
    workspace::config::load_entity,
};

/// Live calls the engine keeps at once.
const MAX_LIVE: usize = 10;

enum Command {
    Send(DynamicMessage),
    End,
    Cancel,
}

struct Live {
    tx: UnboundedSender<Command>,
    input: MessageDescriptor,
    client_streaming: bool,
}

static LIVE: LazyLock<Mutex<HashMap<(String, String), Live>>> = LazyLock::new(Default::default);

fn kind_of(method: &MethodDescriptor) -> &'static str {
    match (method.is_client_streaming(), method.is_server_streaming()) {
        (false, false) => "unary",
        (false, true) => "server",
        (true, false) => "client",
        (true, true) => "bidi",
    }
}

fn ok_or_error(result: Result<(), EngineError>) -> Value {
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(error) => crate::runtime::failed(&error),
    }
}

fn not_open() -> EngineError {
    EngineError::not_found("That call is not open.").keyed("grpc.call_not_open", json!({}))
}

fn ended() -> EngineError {
    EngineError::conflict("That call has ended.").keyed("grpc.call_ended", json!({}))
}

fn emit_status(ws_id: &str, stream_id: &str, status: &str, detail: Value) {
    let mut payload = json!({ "workspaceId": ws_id, "streamId": stream_id, "status": status, "error": null });
    if let (Some(into), Some(from)) = (payload.as_object_mut(), detail.as_object()) {
        for (key, value) in from {
            into.insert(key.clone(), value.clone());
        }
    }
    bus().emit("grpc.stream.status", payload);
}

struct Log {
    ws_id: String,
    stream_id: String,
    seq: u64,
}

impl Log {
    fn message(&mut self, direction: &str, message: &DynamicMessage) {
        self.seq += 1;
        let data = crate::vault::redact_text(&self.ws_id, call::to_json(message));
        bus().emit(
            "grpc.stream.message",
            json!({
                "workspaceId": self.ws_id,
                "streamId": self.stream_id,
                "message": {
                    "id": format!("{}-{}", self.stream_id, self.seq),
                    "ts": now_ms(),
                    "direction": direction,
                    "data": data,
                },
            }),
        );
    }
}

/// Starts the call. Everything that can be refused is refused here; what happens to the call after that is
/// reported by events.
async fn open(ws_id: &str, stream_id: &str, request: &Value) -> Result<&'static str, EngineError> {
    let channel = call::connect(str_of(request, "serverAddress")).await?;
    let method = super::find_method(request, Some(&channel)).await?;
    let client_streaming = method.is_client_streaming();
    // A method that takes one message starts with it; one that streams from the client starts empty.
    let initial =
        if client_streaming { Vec::new() } else { call::parse_messages(str_of(request, "requestBody"), &method)? };
    let metadata = call::to_metadata(&string_map(request, "metadata"))?;

    let (tx, commands) = unbounded_channel();
    let key = (ws_id.to_owned(), stream_id.to_owned());
    {
        let mut live = LIVE.lock();
        if let Some(previous) = live.remove(&key) {
            let _ = previous.tx.send(Command::Cancel);
        }
        if live.len() >= MAX_LIVE {
            return Err(EngineError::conflict(format!("{MAX_LIVE} calls are already open. Cancel one first."))
                .keyed("grpc.max_calls", json!({ "max": MAX_LIVE })));
        }
        live.insert(key.clone(), Live { tx, input: method.input(), client_streaming });
    }
    let kind = kind_of(&method);
    tokio::spawn(async move {
        let (ws_id, stream_id) = key.clone();
        emit_status(&ws_id, &stream_id, "open", json!({}));
        let finish = drive(&ws_id, &stream_id, channel, &method, initial, metadata, commands).await;
        LIVE.lock().remove(&key);
        let (grpc_status, message, metadata) = finish;
        emit_status(
            &ws_id,
            &stream_id,
            "closed",
            json!({ "grpcStatus": grpc_status as i32, "statusMessage": message, "metadata": metadata }),
        );
    });
    Ok(kind)
}

/// Runs the call until it ends, however it ends. Returns the gRPC status, its message and the metadata the
/// server sent.
async fn drive(
    ws_id: &str,
    stream_id: &str,
    channel: Channel,
    method: &MethodDescriptor,
    initial: Vec<DynamicMessage>,
    metadata: MetadataMap,
    mut commands: UnboundedReceiver<Command>,
) -> (Code, String, BTreeMap<String, String>) {
    let mut log = Log { ws_id: ws_id.to_owned(), stream_id: stream_id.to_owned(), seq: 0 };
    let mut collected = BTreeMap::new();
    let client_streaming = method.is_client_streaming();

    let (out_tx, out_rx) = unbounded_channel::<DynamicMessage>();
    for message in initial {
        log.message("sent", &message);
        let _ = out_tx.send(message);
    }
    // The request stream ends when its last sender goes: at once for a method that takes one message, on
    // `end` for one that streams from the client.
    let mut out_tx = client_streaming.then_some(out_tx);
    let outbound = futures::stream::unfold(out_rx, |mut rx| async move { rx.recv().await.map(|m| (m, rx)) });

    let path = format!("/{}/{}", method.parent_service().full_name(), method.name());
    let Ok(path) = path.parse() else {
        return (Code::InvalidArgument, format!("\"{path}\" is not a valid method path."), collected);
    };
    let mut request = Request::new(outbound);
    *request.metadata_mut() = metadata;
    let mut grpc = Grpc::new(channel);
    // Opening the call waits for the server's response headers, and a server may send none until it has read
    // a message, so the call is polled beside the commands rather than awaited first.
    let opening = async {
        grpc.ready().await.map_err(|e| Status::unavailable(format!("The server is not reachable: {e}")))?;
        grpc.streaming(request, path, DynamicCodec(method.output())).await
    };
    tokio::pin!(opening);
    let mut opened = false;
    let mut inbound: Option<Streaming<DynamicMessage>> = None;

    let outcome: Result<(), Status> = loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Send(message)) => {
                    if let Some(tx) = &out_tx {
                        log.message("sent", &message);
                        let _ = tx.send(message);
                    }
                }
                Some(Command::End) => out_tx = None,
                Some(Command::Cancel) | None => break Err(Status::cancelled("Cancelled.")),
            },
            response = &mut opening, if !opened => {
                opened = true;
                match response {
                    Ok(response) => {
                        call::collect_metadata(&mut collected, response.metadata());
                        emit_status(ws_id, stream_id, "open", json!({ "metadata": collected }));
                        inbound = Some(response.into_inner());
                    }
                    Err(status) => break Err(status),
                }
            }
            next = async { inbound.as_mut().expect("guarded below").message().await }, if inbound.is_some() => match next {
                Ok(Some(message)) => log.message("received", &message),
                Ok(None) => {
                    if let Some(Ok(Some(trailers))) = match inbound.as_mut() { Some(stream) => Some(stream.trailers().await), None => None } {
                        call::collect_metadata(&mut collected, &trailers);
                    }
                    break Ok(());
                }
                Err(status) => break Err(status),
            },
        }
    };
    match outcome {
        Ok(()) => (Code::Ok, "OK".to_owned(), collected),
        Err(status) => {
            call::collect_metadata(&mut collected, status.metadata());
            (status.code(), status.message().to_owned(), collected)
        }
    }
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("grpc.stream.open", |params: Value, _| async move {
        let (ws_id, stream_id) = (str_of(&params, "workspaceId"), str_of(&params, "streamId"));
        let id = str_of(&params, "id");
        let stored = load_entity(ws_id, "grpcRequests", id).ok_or_else(|| {
            EngineError::not_found(format!("No saved grpcRequests entity \"{id}\" in workspace \"{ws_id}\"."))
                .keyed("request.saved_not_found", json!({ "kind": "grpcRequests", "id": id, "workspace": ws_id }))
        })?;
        let request = super::resolve_saved(ws_id, &stored);
        super::guard_call(&request).await?;
        Ok(match open(ws_id, stream_id, &request).await {
            Ok(kind) => json!({ "ok": true, "kind": kind }),
            Err(error) => crate::runtime::failed(&error),
        })
    });

    registry.register_sync("grpc.stream.send", |params, _| {
        let key = (str_of(&params, "workspaceId").to_owned(), str_of(&params, "streamId").to_owned());
        let live = LIVE.lock();
        let Some(entry) = live.get(&key) else { return Ok(ok_or_error(Err(not_open()))) };
        if !entry.client_streaming {
            return Ok(ok_or_error(Err(EngineError::bad_request(
                "This method takes one message, which is sent when the call starts.",
            )
            .keyed("grpc.single_message", json!({})))));
        }
        let text = str_of(&params, "data").trim();
        let text = if text.is_empty() { "{}" } else { text };
        let message = serde_json::from_str::<Value>(text).map_err(|e| call::invalid_json(&e)).and_then(|value| {
            DynamicMessage::deserialize(entry.input.clone(), value)
                .map_err(|e| call::mismatch(entry.input.full_name(), &e.to_string()))
        });
        Ok(ok_or_error(message.and_then(|m| entry.tx.send(Command::Send(m)).map_err(|_| ended()))))
    });

    for (action, command) in [("grpc.stream.end", false), ("grpc.stream.cancel", true)] {
        registry.register_sync(action, move |params, _| {
            let key = (str_of(&params, "workspaceId").to_owned(), str_of(&params, "streamId").to_owned());
            let live = LIVE.lock();
            let result = match live.get(&key) {
                Some(entry) => entry.tx.send(if command { Command::Cancel } else { Command::End }).map_err(|_| ended()),
                None => Err(not_open()),
            };
            Ok(ok_or_error(result))
        });
    }
}
