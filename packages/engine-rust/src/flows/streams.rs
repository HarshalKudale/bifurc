//! What a stream node does: connect its saved request once, then hand every message that arrives to the
//! nodes after it, one message at a time, until the stream ends or the node's limits say stop.

use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::{
    model::{Node, PORT_CLOSED, PORT_ERROR, PORT_MESSAGE},
    nodes::{after_execute, bindings},
    reference::render,
    run::{Live, Run, Token, fire, forward},
};
use crate::{
    kernel::{ListenerId, bus},
    utils::json::{opt_str, str_of},
};

/// How many messages may wait for the nodes after a stream before the run is stopped.
const MAX_BACKLOG: usize = 1_000;

/// The bus events of one live connection, in the order they happened. Listening ends when this is dropped.
pub struct Subscription {
    listeners: Vec<ListenerId>,
    events: UnboundedReceiver<(&'static str, Value)>,
}

impl Subscription {
    /// Listen to `events` whose payload names the connection `id` (under `id_field`) of the workspace.
    pub fn new(ws_id: &str, id_field: &'static str, id: &str, events: &[&'static str]) -> Self {
        let (tx, rx) = unbounded_channel();
        let listeners = events
            .iter()
            .map(|event| {
                let (tx, ws_id, id, event) = (tx.clone(), ws_id.to_string(), id.to_string(), *event);
                bus().on(event, move |payload| {
                    if str_of(payload, "workspaceId") == ws_id && str_of(payload, id_field) == id {
                        let _ = tx.send((event, payload.clone()));
                    }
                })
            })
            .collect();
        Self { listeners, events: rx }
    }

    pub async fn next(&mut self) -> Option<(&'static str, Value)> {
        self.events.recv().await
    }

    fn waiting(&self) -> usize {
        self.events.len()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        for listener in self.listeners.drain(..) {
            bus().off(listener);
        }
    }
}

/// The commands and events of one streaming request type.
struct Kind {
    connect: &'static str,
    disconnect: &'static str,
    id_field: &'static str,
    status: &'static str,
    message: &'static str,
}

fn kind_of(request_kind: &str) -> Option<Kind> {
    let kind = |connect, disconnect, id_field, status, message| Kind { connect, disconnect, id_field, status, message };
    Some(match request_kind {
        "sseRequests" => kind("sse.connect", "sse.disconnect", "streamId", "sse.status", "sse.message"),
        "wsConnections" => kind("ws.connect", "ws.disconnect", "socketId", "ws.status", "ws.message"),
        "socketioRequests" => {
            kind("socketio.connect", "socketio.disconnect", "connectionId", "socketio.status", "socketio.message")
        }
        // A live gRPC call: its request body is sent when it opens, and it reports `open` and `closed`.
        "grpcRequests" => {
            kind("grpc.stream.open", "grpc.stream.cancel", "streamId", "grpc.stream.status", "grpc.stream.message")
        }
        _ => return None,
    })
}

/// The id of the connection a flow's stream node holds. It is the flow's, not the run's: a flow has one run at
/// a time, and a new run takes the connection over instead of leaving the last one's behind.
fn live_id(run: &Run, node: &Node) -> String {
    format!("flow:{}:{}", run.flow.id, node.id)
}

/// Send the node's message over its open connection, if it has one to send and the type takes one.
async fn send(run: &Run, node: &Node, token: &Token) -> Result<(), String> {
    if node.message.trim().is_empty() {
        return Ok(());
    }
    let scope = run.scope(token);
    let text = render(&node.message, &scope)?;
    let id = live_id(run, node);
    let (command, payload) = match node.request_kind.as_str() {
        "wsConnections" => ("ws.send", json!({ "workspaceId": run.ws_id, "socketId": id, "data": text })),
        "socketioRequests" => {
            let event = render(&node.event, &scope)?;
            ("socketio.emit", json!({ "workspaceId": run.ws_id, "connectionId": id, "event": event, "args": text }))
        }
        "grpcRequests" => ("grpc.stream.send", json!({ "workspaceId": run.ws_id, "streamId": id, "data": text })),
        _ => return Ok(()),
    };
    let answer = run.invoke(command, payload, Vec::new()).await.map_err(|error| error.message)?;
    if answer["ok"] == true {
        Ok(())
    } else {
        Err(opt_str(&answer, "error").unwrap_or("The message was not sent").to_string())
    }
}

/// Wait until `at`, or for ever when there is no such moment.
async fn until(at: Option<tokio::time::Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Why a stream node stopped listening, and the port that says so.
struct End {
    port: &'static str,
    output: Value,
}

fn closed(reason: &str, messages: u64) -> End {
    End { port: PORT_CLOSED, output: json!({ "ok": true, "reason": reason, "messages": messages }) }
}

fn broken(error: impl Into<String>) -> End {
    End { port: PORT_ERROR, output: json!({ "ok": false, "error": error.into() }) }
}

/// Connect and listen until the stream ends. Each message is handed on before the next one is read.
async fn receive(run: &Arc<Run>, node: &Node, token: &Token, kind: &Kind) -> End {
    let variables = match bindings(run, node, token) {
        Ok(variables) => variables,
        Err(error) => return broken(error),
    };
    let id = live_id(run, node);
    let mut events = Subscription::new(&run.ws_id, kind.id_field, &id, &[kind.status, kind.message]);
    let mut payload = json!({ "workspaceId": run.ws_id, "id": node.request_id });
    payload[kind.id_field] = json!(id);
    match run.invoke(kind.connect, payload, variables).await {
        Err(error) => return broken(error.message),
        Ok(answer) if answer["ok"] != true => {
            return broken(opt_str(&answer, "error").unwrap_or("The connection failed"));
        }
        Ok(_) => {}
    }
    run.keep(Live { disconnect: kind.disconnect, id_field: kind.id_field, id: id.clone() });

    let after = |ms: u64| (ms > 0).then(|| tokio::time::Instant::now() + Duration::from_millis(ms));
    let listen_until = after(node.listen_ms);
    let mut received = 0u64;
    let mut opened = false;
    loop {
        let idle_until = after(node.idle_timeout_ms);
        let (event, payload) = tokio::select! {
            next = events.next() => match next {
                Some(next) => next,
                None => return closed("ended", received),
            },
            _ = until(idle_until) => return closed("idle", received),
            _ = until(listen_until) => return closed("time", received),
        };
        if event == kind.status {
            match str_of(&payload, "status") {
                // A gRPC call says `open` again when the server's metadata arrives.
                "connected" | "open" if !opened => {
                    opened = true;
                    run.waiting(node);
                    if let Err(error) = send(run, node, token).await {
                        return broken(error);
                    }
                }
                "error" => return broken(opt_str(&payload, "error").unwrap_or("The connection failed")),
                "disconnected" => return closed("closed", received),
                // A gRPC call ends with its status: anything but OK is a failure.
                "closed" => {
                    return match payload["grpcStatus"].as_i64().unwrap_or(0) {
                        0 => closed("closed", received),
                        code => broken(format!(
                            "The call ended with status {code}: {}",
                            opt_str(&payload, "statusMessage").unwrap_or("no message")
                        )),
                    };
                }
                _ => {}
            }
            continue;
        }
        let message = &payload["message"];
        // A two-way connection also reports what the flow itself sent.
        if str_of(message, "direction") == "sent" {
            continue;
        }
        received += 1;
        let arrived = (PORT_MESSAGE.to_string(), Arc::new(message.clone()));
        let (port, message) = after_execute(run, node, token, arrived).await;
        if port == PORT_ERROR {
            return broken(str_of(&message, "error"));
        }
        run.received(node, &message);
        forward(run, node, PORT_MESSAGE, message, token).await;

        let taken = run.events.fetch_add(1, Ordering::Relaxed) + 1;
        if (node.max_messages > 0 && received >= node.max_messages) || run.max_events.is_some_and(|max| taken >= max) {
            return closed("limit", received);
        }
        if events.waiting() > MAX_BACKLOG {
            run.fail(format!(
                "{}: more than {MAX_BACKLOG} messages are waiting for the nodes after it. They arrive faster than the flow handles them.",
                node.label()
            ));
            return closed("overrun", received);
        }
    }
}

/// Run a stream node. The first time a run reaches it, it connects and listens; reaching it again while it is
/// connected sends its message over the connection that is already open.
pub async fn listen(run: &Arc<Run>, node: &Node, token: &Token, started: Instant) {
    let Some(kind) = kind_of(&node.request_kind) else {
        let error = format!("A request of the kind \"{}\" cannot be listened to by a flow.", node.request_kind);
        return fire(run, node, PORT_ERROR, Arc::new(json!({ "ok": false, "error": error })), token, started.elapsed())
            .await;
    };
    if !run.listening.lock().insert(node.id.clone()) {
        run.waiting(node);
        if let Err(error) = send(run, node, token).await {
            run.note(node, error);
        }
        return;
    }
    let end = receive(run, node, token, &kind).await;
    run.listening.lock().remove(&node.id);
    let mut payload = json!({ "workspaceId": run.ws_id });
    payload[kind.id_field] = json!(live_id(run, node));
    let _ = run.ctx.registry.invoke(kind.disconnect, payload, run.ctx.clone()).await;
    fire(run, node, end.port, Arc::new(end.output), token, started.elapsed()).await;
}
