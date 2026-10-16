//! The `socketio.*` commands: thin bindings of the connection registry to the wire.

use serde_json::{Value, json};

use super::registry::{
    MAX_SOCKETIO_CONNECTIONS, OkResult, Target, clear_connection_messages, connect_connection, disconnect_connection,
    emit_event, get_connection, list_connections, set_listeners,
};
use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    utils::json::{opt_str, str_of},
    workspace::config::load_entity,
};

fn ok_result(result: OkResult) -> Result<Value> {
    Ok(match result {
        Ok(()) => json!({ "ok": true }),
        Err(error) => json!({ "ok": false, "error": error }),
    })
}

/// The fields of a target besides `url`, which a draft may send and a saved request holds.
const TARGET_FIELDS: [&str; 5] = ["headers", "path", "namespace", "auth", "listeners"];

/// The events a target listens to: the names in its `listeners` (each a name, or an object with an `event`),
/// or `None` — every event — when it has no such list or says `listenAll`.
fn listeners_of(source: &Value) -> Option<Vec<String>> {
    if source.get("listenAll").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let listed = source.get("listeners")?.as_array()?;
    let name = |item: &Value| item.as_str().or_else(|| item.get("event")?.as_str()).map(|name| name.trim().to_string());
    Some(listed.iter().filter_map(name).filter(|name| !name.is_empty()).collect())
}

/// Where a connect gets its target: `url` (with optional `headers`, `path`, `namespace`, `auth`, `listeners`) for a draft,
/// or the saved `socketioRequests` entity named by `id` (by `connectionId` when `id` is omitted). The draft's
/// other fields without `url` are refused. Returns the object the target is read from.
fn resolve_target(params: &Value) -> Result<Value> {
    if opt_str(params, "url").is_some() {
        return Ok(params.clone());
    }
    if TARGET_FIELDS.iter().any(|field| params.get(*field).is_some()) {
        return Err(EngineError::bad_request(
            "socketio.connect takes `headers`, `path`, `namespace`, `auth` and `listeners` only together with `url`; \
             a saved request supplies its own.",
        ));
    }
    let workspace_id = str_of(params, "workspaceId");
    let id = opt_str(params, "id").unwrap_or_else(|| str_of(params, "connectionId"));
    load_entity(workspace_id, "socketioRequests", id).ok_or_else(|| {
        EngineError::not_found(format!(
            "No saved socketioRequests entity \"{id}\" in workspace \"{workspace_id}\". Pass `url` to connect a draft."
        ))
    })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("socketio.connect", |params, _| {
        let source = resolve_target(&params)?;
        let target = Target {
            url: str_of(&source, "url"),
            headers: source.get("headers"),
            path: str_of(&source, "path"),
            namespace: str_of(&source, "namespace"),
            auth: str_of(&source, "auth"),
            listeners: listeners_of(&source),
        };
        ok_result(connect_connection(str_of(&params, "workspaceId"), str_of(&params, "connectionId"), &target))
    });

    registry.register_sync("socketio.disconnect", |params, _| {
        ok_result(disconnect_connection(str_of(&params, "workspaceId"), str_of(&params, "connectionId")))
    });

    registry.register_sync("socketio.emit", |params, _| {
        ok_result(emit_event(
            str_of(&params, "workspaceId"),
            str_of(&params, "connectionId"),
            str_of(&params, "event"),
            str_of(&params, "args"),
            params.get("ack").and_then(Value::as_bool).unwrap_or(false),
        ))
    });

    registry.register_sync("socketio.listen", |params, _| {
        ok_result(set_listeners(str_of(&params, "workspaceId"), str_of(&params, "connectionId"), listeners_of(&params)))
    });

    registry.register_sync("socketio.status", |params, _| {
        Ok(get_connection(str_of(&params, "workspaceId"), str_of(&params, "connectionId")).unwrap_or(Value::Null))
    });

    registry.register_sync("socketio.list", |params, _| {
        Ok(json!({
            "connections": list_connections(str_of(&params, "workspaceId")),
            "max": MAX_SOCKETIO_CONNECTIONS,
        }))
    });

    registry.register_sync("socketio.clearMessages", |params, _| {
        ok_result(clear_connection_messages(str_of(&params, "workspaceId"), str_of(&params, "connectionId")))
    });
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use futures::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{
        accept_hdr_async,
        tungstenite::{
            Message,
            handshake::server::{Request, Response},
        },
    };

    use super::*;
    use crate::{
        kernel::{CommandContext, bus},
        protocol::ErrorCode,
        testing::scratch_data_root,
        workspace::fs::write_entity,
    };

    const WS: &str = "ws1";

    async fn call(action: &str, payload: Value) -> Value {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        registry.invoke(action, payload, ctx).await.expect("the command succeeds")
    }

    async fn call_err(action: &str, payload: Value) -> EngineError {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        registry.invoke(action, payload, ctx).await.expect_err("the command fails")
    }

    /// A loopback Socket.IO server, written against the wire format rather than with this module's codec, and
    /// the frames every client sent it.
    ///
    /// It answers the namespace connect with a sid and a `welcome` event that describes the handshake it saw
    /// (`/deny` is refused). It then echoes `echo` events (acknowledging the ones that ask), disconnects the
    /// client on `kick`, drops the socket on `drop`, pings on `ping-me` and sends a binary event on `binary`.
    /// The path chooses a behaviour: `/mute/` never sends the open packet, `/quiet/` announces a 200 ms
    /// heartbeat and then keeps silent.
    struct Server {
        url: String,
        frames: Arc<Mutex<Vec<String>>>,
    }

    impl Server {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let frames = Arc::new(Mutex::new(Vec::new()));
            let seen = frames.clone();
            tokio::spawn(async move {
                while let Ok((tcp, _)) = listener.accept().await {
                    tokio::spawn(serve(tcp, seen.clone()));
                }
            });
            Self { url, frames }
        }

        fn frames(&self) -> Vec<String> {
            self.frames.lock().unwrap().clone()
        }
    }

    /// Splits a packet's text after its type digit into the namespace prefix (`/chat,` or nothing), the ack id
    /// and the JSON.
    fn split(rest: &str) -> (String, String, String) {
        let (prefix, rest) = match rest.starts_with('/') {
            true => rest.split_at(rest.find(',').map_or(rest.len(), |at| at + 1)),
            false => ("", rest),
        };
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        (prefix.to_string(), rest[..digits].to_string(), rest[digits..].to_string())
    }

    async fn serve(tcp: tokio::net::TcpStream, frames: Arc<Mutex<Vec<String>>>) {
        let handshake = Arc::new(Mutex::new((String::new(), String::new())));
        let noted = handshake.clone();
        let Ok(mut ws) = accept_hdr_async(tcp, move |request: &Request, response: Response| {
            let token = request.headers().get("x-token").and_then(|v| v.to_str().ok()).unwrap_or_default();
            *noted.lock().unwrap() = (request.uri().to_string(), token.to_string());
            Ok(response)
        })
        .await
        else {
            return;
        };
        let (uri, token) = handshake.lock().unwrap().clone();
        if uri.starts_with("/mute/") {
            while ws.next().await.is_some() {}
            return;
        }
        let quiet = uri.starts_with("/quiet/");
        let (interval, timeout) = if quiet { (100, 100) } else { (25000, 20000) };
        let open = format!(
            "0{{\"sid\":\"eio-1\",\"upgrades\":[],\"pingInterval\":{interval},\"pingTimeout\":{timeout},\"maxPayload\":1000000}}"
        );
        if ws.send(Message::text(open)).await.is_err() {
            return;
        }
        while let Some(Ok(message)) = ws.next().await {
            let Message::Text(text) = message else { continue };
            let text = text.to_string();
            frames.lock().unwrap().push(text.clone());
            let mut replies: Vec<Message> = Vec::new();
            if let Some(rest) = text.strip_prefix("40") {
                let (ns, _, auth) = split(rest);
                if ns == "/deny," {
                    replies.push(Message::text("44/deny,{\"message\":\"Not authorized\"}"));
                } else {
                    let auth = if auth.is_empty() { "null".to_string() } else { auth };
                    replies.push(Message::text(format!("40{ns}{{\"sid\":\"sid-1\"}}")));
                    if !quiet {
                        let seen = json!({ "uri": uri, "token": token });
                        replies.push(Message::text(format!("42{ns}[\"welcome\",{seen},{auth}]")));
                    }
                }
            } else if let Some(rest) = text.strip_prefix("42") {
                let (ns, id, data) = split(rest);
                let args: Vec<Value> = serde_json::from_str(&data).unwrap();
                match args[0].as_str().unwrap() {
                    "echo" => {
                        replies.push(Message::text(format!("42{ns}{data}")));
                        if !id.is_empty() {
                            let answer = Value::Array(args[1..].to_vec());
                            replies.push(Message::text(format!("43{ns}{id}{answer}")));
                        }
                    }
                    "kick" => replies.push(Message::text(format!("41{ns}"))),
                    "drop" => return,
                    "ping-me" => replies.push(Message::text("2")),
                    "binary" => {
                        replies.push(Message::text("451-[\"file\",{\"_placeholder\":true,\"num\":0}]"));
                        replies.push(Message::Binary(vec![1u8, 2, 3].into()));
                        replies.push(Message::text("42[\"after\"]"));
                    }
                    "ask-me" => replies.push(Message::text(format!("42{ns}9[\"question\",1]"))),
                    _ => {}
                }
            } else if text.starts_with("41") {
                let _ = ws.close(None).await;
                return;
            }
            for reply in replies {
                if ws.send(reply).await.is_err() {
                    return;
                }
            }
        }
    }

    struct Events {
        status: Arc<Mutex<Vec<Value>>>,
        message: Arc<Mutex<Vec<Value>>>,
        ids: Vec<crate::kernel::ListenerId>,
    }

    impl Events {
        fn watch(connection_id: &'static str) -> Self {
            let status = Arc::new(Mutex::new(Vec::new()));
            let message = Arc::new(Mutex::new(Vec::new()));
            let (s, m) = (status.clone(), message.clone());
            let ids = vec![
                bus().on("socketio.status", move |p| {
                    if p["connectionId"] == connection_id {
                        s.lock().unwrap().push(p.clone());
                    }
                }),
                bus().on("socketio.message", move |p| {
                    if p["connectionId"] == connection_id {
                        m.lock().unwrap().push(p.clone());
                    }
                }),
            ];
            Self { status, message, ids }
        }

        fn statuses(&self) -> Vec<String> {
            self.status.lock().unwrap().iter().map(|p| p["status"].as_str().unwrap().to_string()).collect()
        }

        fn messages(&self) -> Vec<Value> {
            self.message.lock().unwrap().iter().map(|p| p["message"].clone()).collect()
        }

        /// The log as `(direction, type, event, data)`.
        fn lines(&self) -> Vec<(String, String, String, String)> {
            let text = |m: &Value, key: &str| m[key].as_str().unwrap().to_string();
            self.messages()
                .iter()
                .map(|m| (text(m, "direction"), text(m, "type"), text(m, "event"), text(m, "data")))
                .collect()
        }
    }

    impl Drop for Events {
        fn drop(&mut self) {
            for id in &self.ids {
                bus().off(*id);
            }
        }
    }

    fn line(direction: &str, kind: &str, event: &str, data: &str) -> (String, String, String, String) {
        (direction.into(), kind.into(), event.into(), data.into())
    }

    async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn status(connection_id: &str) -> Value {
        call("socketio.status", json!({ "workspaceId": WS, "connectionId": connection_id })).await
    }

    async fn wait_status(connection_id: &str, wanted: &str) -> Value {
        for _ in 0..200 {
            let snapshot = status(connection_id).await;
            if snapshot["status"] == wanted {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("connection {connection_id} never reached {wanted}: {}", status(connection_id).await);
    }

    async fn connect(connection_id: &str, url: &str) -> Value {
        call("socketio.connect", json!({ "workspaceId": WS, "connectionId": connection_id, "url": url })).await
    }

    async fn emit(connection_id: &str, event: &str, args: &str) -> Value {
        let payload = json!({ "workspaceId": WS, "connectionId": connection_id, "event": event, "args": args });
        call("socketio.emit", payload).await
    }

    async fn disconnect(connection_id: &str) -> Value {
        call("socketio.disconnect", json!({ "workspaceId": WS, "connectionId": connection_id })).await
    }

    #[tokio::test]
    async fn connect_emit_and_receive_on_the_main_namespace() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("main");

        assert_eq!(connect("main", &server.url).await, json!({ "ok": true }));
        let connected = wait_status("main", "connected").await;
        assert_eq!(events.statuses(), ["connecting", "connected"]);
        assert_eq!(connected["url"], server.url);
        assert_eq!(connected["namespace"], "/");
        assert_eq!(connected["sid"], "sid-1");
        assert_eq!(connected["error"], Value::Null);
        assert!(connected["openedAt"].is_i64());
        wait_for("the welcome", || events.messages().len() == 1).await;

        assert_eq!(emit("main", "echo", "[\"hello\", {\"n\": 1}]").await, json!({ "ok": true }));
        wait_for("the echo", || events.messages().len() == 3).await;
        let welcome = "[{\"uri\":\"/socket.io/?EIO=4&transport=websocket\",\"token\":\"\"},null]";
        assert_eq!(
            events.lines(),
            [
                line("received", "event", "welcome", welcome),
                line("sent", "event", "echo", "[\"hello\",{\"n\":1}]"),
                line("received", "event", "echo", "[\"hello\",{\"n\":1}]"),
            ]
        );
        let messages = events.messages();
        assert_eq!((messages[1]["id"].as_str(), messages[2]["id"].as_str()), (Some("main-2"), Some("main-3")));
        assert_eq!(messages[1]["ackId"], Value::Null);
        assert!(messages[1]["ts"].is_i64());
        assert_eq!(status("main").await["messages"].as_array().unwrap().len(), 3);
        // What went over the wire: the namespace connect, then the event.
        assert_eq!(server.frames(), ["40", "42[\"echo\",\"hello\",{\"n\":1}]"]);

        let listed = call("socketio.list", json!({ "workspaceId": WS })).await;
        assert_eq!(listed["max"], MAX_SOCKETIO_CONNECTIONS);
        assert!(listed["connections"].as_array().unwrap().iter().any(|c| c["connectionId"] == "main"));

        assert_eq!(disconnect("main").await, json!({ "ok": true }));
        let closed = wait_status("main", "disconnected").await;
        assert_eq!(closed["error"], Value::Null);
        assert_eq!(closed["closeReason"], "io client disconnect");
        assert_eq!(events.statuses(), ["connecting", "connected", "disconnected"]);
        // The server was told the client left the namespace.
        wait_for("the disconnect packet", || server.frames().last().is_some_and(|f| f == "41")).await;

        // A finished connection keeps its entry and log but is no longer live.
        let listed = call("socketio.list", json!({ "workspaceId": WS })).await;
        assert!(!listed["connections"].as_array().unwrap().iter().any(|c| c["connectionId"] == "main"));
        assert_eq!(status("main").await["messages"].as_array().unwrap().len(), 3);
        assert_eq!(disconnect("main").await, json!({ "ok": true }));
        assert_eq!(emit("main", "echo", "").await, json!({ "ok": false, "error": "Not connected" }));
    }

    #[tokio::test]
    async fn a_namespace_a_path_an_auth_payload_and_headers_reach_the_server() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("chat");
        let result = call(
            "socketio.connect",
            json!({ "workspaceId": WS, "connectionId": "chat", "url": format!("{}?room=7", server.url),
                    "path": "/realtime", "namespace": "chat", "auth": "{\"token\": \"abc\"}",
                    "headers": { "X-Token": "t-1" } }),
        )
        .await;
        assert_eq!(result, json!({ "ok": true }));
        assert_eq!(wait_status("chat", "connected").await["namespace"], "/chat");
        wait_for("the welcome", || events.messages().len() == 1).await;
        let welcome =
            "[{\"uri\":\"/realtime/?room=7&EIO=4&transport=websocket\",\"token\":\"t-1\"},{\"token\":\"abc\"}]";
        assert_eq!(events.lines(), [line("received", "event", "welcome", welcome)]);

        emit("chat", "echo", "one").await;
        wait_for("the echo", || events.messages().len() == 3).await;
        assert_eq!(server.frames(), ["40/chat,{\"token\":\"abc\"}", "42/chat,[\"echo\",\"one\"]"]);
        disconnect("chat").await;
        wait_status("chat", "disconnected").await;
        wait_for("the disconnect packet", || server.frames().last().is_some_and(|f| f == "41/chat,")).await;

        // The path of the address names the namespace when none is given.
        connect("by-path", &format!("{}/chat", server.url)).await;
        assert_eq!(wait_status("by-path", "connected").await["namespace"], "/chat");
        disconnect("by-path").await;
        wait_status("by-path", "disconnected").await;
    }

    #[tokio::test]
    async fn an_emit_can_ask_for_an_acknowledgement() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("ack");
        connect("ack", &server.url).await;
        wait_status("ack", "connected").await;
        wait_for("the welcome", || events.messages().len() == 1).await;

        for args in ["[1, \"two\"]", "{\"three\": 3}"] {
            let payload =
                json!({ "workspaceId": WS, "connectionId": "ack", "event": "echo", "args": args, "ack": true });
            assert_eq!(call("socketio.emit", payload).await, json!({ "ok": true }));
        }
        wait_for("both acknowledgements", || events.messages().len() == 7).await;
        assert_eq!(&server.frames()[1..], ["420[\"echo\",1,\"two\"]", "421[\"echo\",{\"three\":3}]"]);
        let acks: Vec<Value> = events.messages().into_iter().filter(|m| m["type"] == "ack").collect();
        // An acknowledgement is named after the event it answers and carries what the server answered with.
        assert_eq!(
            acks.iter()
                .map(|m| (m["event"].as_str().unwrap(), m["ackId"].as_u64().unwrap(), m["data"].as_str().unwrap()))
                .collect::<Vec<_>>(),
            [("echo", 0, "[1,\"two\"]"), ("echo", 1, "[{\"three\":3}]")]
        );
        let sent: Vec<Value> = events.messages().into_iter().filter(|m| m["direction"] == "sent").collect();
        assert_eq!((sent[0]["ackId"].clone(), sent[1]["ackId"].clone()), (json!(0), json!(1)));

        // An event the server wants acknowledged shows the id it asked with.
        emit("ack", "ask-me", "").await;
        wait_for("the question", || events.messages().iter().any(|m| m["event"] == "question")).await;
        let question = events.messages().into_iter().find(|m| m["event"] == "question").unwrap();
        assert_eq!((question["ackId"].clone(), question["data"].clone()), (json!(9), json!("[1]")));
        disconnect("ack").await;
        wait_status("ack", "disconnected").await;
    }

    #[tokio::test]
    async fn only_the_events_listened_to_are_logged() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("heard");
        let payload = json!({ "workspaceId": WS, "connectionId": "heard", "url": server.url, "listeners": ["echo"] });
        assert_eq!(call("socketio.connect", payload).await, json!({ "ok": true }));
        assert_eq!(wait_status("heard", "connected").await["listeners"], json!(["echo"]));

        // The server's welcome is not heard; what is sent always is, and so is the event listened to.
        emit("heard", "echo", "1").await;
        wait_for("the echo", || events.messages().len() == 2).await;
        assert_eq!(events.lines(), [line("sent", "event", "echo", "[1]"), line("received", "event", "echo", "[1]")]);

        // The listeners can change while connected. An acknowledgement is the answer to what was sent: it is
        // logged whatever is listened to.
        let listen = |listeners: Value| async move {
            let mut payload = json!({ "workspaceId": WS, "connectionId": "heard" });
            if !listeners.is_null() {
                payload["listeners"] = listeners;
            }
            call("socketio.listen", payload).await
        };
        assert_eq!(listen(json!(["question"])).await, json!({ "ok": true }));
        let payload = json!({ "workspaceId": WS, "connectionId": "heard", "event": "echo", "args": "2", "ack": true });
        call("socketio.emit", payload).await;
        wait_for("the acknowledgement", || events.messages().iter().any(|m| m["type"] == "ack")).await;
        emit("heard", "ask-me", "").await;
        wait_for("the question", || events.messages().iter().any(|m| m["event"] == "question")).await;
        assert_eq!(
            events.lines()[2..],
            [
                line("sent", "event", "echo", "[2]"),
                line("received", "ack", "echo", "[2]"),
                line("sent", "event", "ask-me", "[]"),
                line("received", "event", "question", "[1]"),
            ]
        );

        // No listeners at all hears nothing; no list hears everything.
        assert_eq!(listen(json!([])).await, json!({ "ok": true }));
        // The acknowledgement follows the echo, so once it is here the echo has gone by unheard.
        let payload = json!({ "workspaceId": WS, "connectionId": "heard", "event": "echo", "args": "3", "ack": true });
        call("socketio.emit", payload).await;
        wait_for("the second acknowledgement", || events.messages().iter().filter(|m| m["type"] == "ack").count() == 2)
            .await;
        assert_eq!(listen(Value::Null).await, json!({ "ok": true }));
        assert_eq!(status("heard").await["listeners"], Value::Null);
        emit("heard", "echo", "4").await;
        wait_for("the last echo", || events.lines().last() == Some(&line("received", "event", "echo", "[4]"))).await;
        assert_eq!(
            events.lines()[6..],
            [
                line("sent", "event", "echo", "[3]"),
                line("received", "ack", "echo", "[3]"),
                line("sent", "event", "echo", "[4]"),
                line("received", "event", "echo", "[4]"),
            ]
        );
        disconnect("heard").await;
        wait_status("heard", "disconnected").await;

        let unknown = listen_to("never", json!(["x"])).await;
        assert_eq!(unknown["ok"], false);
    }

    async fn listen_to(connection_id: &str, listeners: Value) -> Value {
        call("socketio.listen", json!({ "workspaceId": WS, "connectionId": connection_id, "listeners": listeners }))
            .await
    }

    #[tokio::test]
    async fn a_saved_request_listens_to_what_it_lists() {
        let root = scratch_data_root();
        let server = Server::start().await;
        let saved = |id: &str, extra: Value| {
            let mut entity =
                json!({ "id": id, "name": id, "url": server.url, "createdAt": 1, "workspaceId": root.ws_id });
            entity.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            write_entity(root.ws_id, "socketioRequests", id, &entity, None).unwrap();
        };
        // Names, or rows with a name; a blank one is no listener.
        saved("listed", json!({ "listeners": [{ "event": " echo ", "description": "what I sent" }, "question", ""] }));
        saved("all", json!({ "listeners": ["echo"], "listenAll": true }));
        saved("plain", json!({}));
        for (id, expected) in [("listed", json!(["echo", "question"])), ("all", Value::Null), ("plain", Value::Null)] {
            call("socketio.connect", json!({ "workspaceId": WS, "connectionId": id })).await;
            assert_eq!(wait_status(id, "connected").await["listeners"], expected, "{id}");
        }
        // The welcome reaches the two that hear everything, not the one with a list.
        wait_for("the welcomes", || {
            ["all", "plain"].iter().all(|id| get_connection(WS, id).unwrap()["messages"][0]["event"] == "welcome")
        })
        .await;
        assert_eq!(status("listed").await["messages"], json!([]));
        for id in ["listed", "all", "plain"] {
            disconnect(id).await;
            wait_status(id, "disconnected").await;
        }
    }

    #[tokio::test]
    async fn a_saved_request_resolves_variables() {
        let root = scratch_data_root();
        let server = Server::start().await;
        write_entity(
            root.ws_id,
            "environments",
            "env1",
            &json!({ "id": "env1", "name": "Env", "variables": [
                { "key": "BASE", "value": server.url, "enabled": true },
                { "key": "TOKEN", "value": "from-env", "enabled": true },
                { "key": "NS", "value": "chat", "enabled": true },
            ] }),
            None,
        )
        .unwrap();
        write_entity(
            root.ws_id,
            "socketioRequests",
            "saved1",
            &json!({ "id": "saved1", "name": "Saved", "url": "{{BASE}}", "namespace": "/{{NS}}", "path": "",
                     "auth": "{\"token\": \"{{TOKEN}}\"}", "headers": { "X-Token": "{{TOKEN}}" }, "createdAt": 1,
                     "workspaceId": root.ws_id }),
            None,
        )
        .unwrap();

        let events = Events::watch("saved1");
        let result = call("socketio.connect", json!({ "workspaceId": WS, "connectionId": "saved1" })).await;
        assert_eq!(result, json!({ "ok": true }));
        let connected = wait_status("saved1", "connected").await;
        assert_eq!((connected["url"].as_str(), connected["namespace"].as_str()), (Some("{{BASE}}"), Some("/chat")));
        wait_for("the welcome", || events.messages().len() == 1).await;
        let welcome: Value = serde_json::from_str(events.messages()[0]["data"].as_str().unwrap()).unwrap();
        assert_eq!((welcome[0]["token"].as_str(), welcome[1]["token"].as_str()), (Some("from-env"), Some("from-env")));

        // The event's name and arguments are resolved too.
        emit("saved1", "echo", "{\"token\": \"{{TOKEN}}\"}").await;
        wait_for("the echo", || events.messages().len() == 3).await;
        assert_eq!(server.frames()[1], "42/chat,[\"echo\",{\"token\":\"from-env\"}]");
        disconnect("saved1").await;
        wait_status("saved1", "disconnected").await;

        // The saved request is named by `id` when the connection has another name.
        call("socketio.connect", json!({ "workspaceId": WS, "connectionId": "tab-2", "id": "saved1" })).await;
        wait_status("tab-2", "connected").await;
        disconnect("tab-2").await;
        wait_status("tab-2", "disconnected").await;
    }

    #[tokio::test]
    async fn a_missing_saved_request_and_stray_draft_fields_are_refused() {
        let _root = scratch_data_root();
        let missing = call_err("socketio.connect", json!({ "workspaceId": WS, "connectionId": "nope" })).await;
        assert_eq!(missing.code, ErrorCode::NotFound);
        for stray in [
            json!({ "headers": { "A": "b" } }),
            json!({ "namespace": "/x" }),
            json!({ "auth": "{}" }),
            json!({ "path": "/p" }),
            json!({ "listeners": ["x"] }),
        ] {
            let mut payload = json!({ "workspaceId": WS, "connectionId": "x" });
            payload.as_object_mut().unwrap().extend(stray.as_object().unwrap().clone());
            assert_eq!(call_err("socketio.connect", payload).await.code, ErrorCode::BadRequest, "{stray}");
        }
    }

    #[tokio::test]
    async fn a_namespace_that_refuses_the_connection_is_an_error() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("denied");
        let payload = json!({ "workspaceId": WS, "connectionId": "denied", "url": server.url, "namespace": "/deny" });
        assert_eq!(call("socketio.connect", payload).await, json!({ "ok": true }));
        let failed = wait_status("denied", "error").await;
        assert_eq!(failed["error"], "Not authorized");
        assert_eq!(failed["closeReason"], "connect error");
        assert_eq!(failed["sid"], Value::Null);
        assert_eq!(events.statuses(), ["connecting", "error"]);
        let listed = call("socketio.list", json!({ "workspaceId": WS })).await;
        assert!(!listed["connections"].as_array().unwrap().iter().any(|c| c["connectionId"] == "denied"));
    }

    #[tokio::test]
    async fn the_server_ending_the_connection_is_told_apart_from_losing_it() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        connect("kicked", &server.url).await;
        wait_status("kicked", "connected").await;
        emit("kicked", "kick", "").await;
        let ended = wait_status("kicked", "disconnected").await;
        assert_eq!(
            (ended["error"].clone(), ended["closeReason"].clone()),
            (Value::Null, json!("io server disconnect"))
        );

        let events = Events::watch("dropped");
        connect("dropped", &server.url).await;
        wait_status("dropped", "connected").await;
        emit("dropped", "drop", "").await;
        let lost = wait_status("dropped", "error").await;
        assert!(lost["error"].as_str().unwrap().starts_with("The server closed the connection"), "{lost}");
        assert_eq!(events.statuses(), ["connecting", "connected", "error"]);
    }

    #[tokio::test]
    async fn pings_are_answered_and_binary_attachments_are_skipped() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("ping");
        connect("ping", &server.url).await;
        wait_status("ping", "connected").await;
        emit("ping", "ping-me", "").await;
        wait_for("the pong", || server.frames().last().is_some_and(|f| f == "3")).await;

        emit("ping", "binary", "").await;
        wait_for("the event after the attachment", || events.messages().iter().any(|m| m["event"] == "after")).await;
        let file = events.messages().into_iter().find(|m| m["event"] == "file").unwrap();
        assert_eq!(file["data"], "[{\"_placeholder\":true,\"num\":0}]");
        assert_eq!(status("ping").await["status"], "connected");
        disconnect("ping").await;
        wait_status("ping", "disconnected").await;
    }

    #[tokio::test]
    async fn a_server_that_never_shakes_hands_or_stops_its_heartbeat_times_out() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let payload = json!({ "workspaceId": WS, "connectionId": "mute", "url": server.url, "path": "/mute" });
        call("socketio.connect", payload).await;
        let failed = wait_status("mute", "error").await;
        assert!(
            failed["error"].as_str().unwrap().starts_with("Timed out waiting for the Socket.IO handshake"),
            "{failed}"
        );
        assert_eq!(failed["closeReason"], "timeout");

        let payload = json!({ "workspaceId": WS, "connectionId": "quiet", "url": server.url, "path": "/quiet" });
        call("socketio.connect", payload).await;
        wait_status("quiet", "connected").await;
        let failed = wait_status("quiet", "error").await;
        assert!(failed["error"].as_str().unwrap().starts_with("Ping timeout"), "{failed}");
        assert_eq!(failed["closeReason"], "ping timeout");
    }

    #[tokio::test]
    async fn a_refused_connection_and_a_bad_target_report_the_error() {
        let _root = scratch_data_root();
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let events = Events::watch("refused");
        assert_eq!(connect("refused", &format!("http://127.0.0.1:{port}")).await, json!({ "ok": true }));
        let failed = wait_status("refused", "error").await;
        assert!(failed["error"].is_string());
        assert_eq!(failed["openedAt"], Value::Null);
        assert_eq!(events.statuses(), ["connecting", "error"]);

        for (connection_id, url) in [("bad-url", "not a url"), ("bad-scheme", "ftp://127.0.0.1/x"), ("empty", "")] {
            let result = connect(connection_id, url).await;
            assert_eq!(result["ok"], false, "{url}");
            assert!(result["error"].as_str().unwrap().contains("Invalid URL"), "{result}");
            assert_eq!(status(connection_id).await["status"], "error");
        }
        let payload =
            json!({ "workspaceId": WS, "connectionId": "bad-auth", "url": "http://127.0.0.1", "auth": "nope" });
        let result = call("socketio.connect", payload).await;
        assert!(result["error"].as_str().unwrap().contains("Invalid auth"), "{result}");
    }

    #[tokio::test]
    async fn an_emit_needs_an_open_connection_and_an_event_name() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        assert_eq!(emit("ghost", "echo", "").await, json!({ "ok": false, "error": "Not connected" }));
        connect("named", &server.url).await;
        wait_status("named", "connected").await;
        assert_eq!(emit("named", "  ", "").await, json!({ "ok": false, "error": "An event name is required." }));
        disconnect("named").await;
        wait_status("named", "disconnected").await;
    }

    #[tokio::test]
    async fn a_reconnect_replaces_the_connection_and_keeps_the_log() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("again");
        connect("again", &server.url).await;
        wait_for("the first welcome", || events.messages().len() == 1).await;

        assert_eq!(connect("again", &server.url).await, json!({ "ok": true }));
        wait_for("the second welcome", || events.messages().len() == 2).await;
        // The superseded connection's close must not report a `disconnected` after the new `connecting`.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(events.statuses(), ["connecting", "connected", "connecting", "connected"]);

        let snapshot = status("again").await;
        assert_eq!(snapshot["status"], "connected");
        assert_eq!(snapshot["messages"].as_array().unwrap().len(), 2);
        assert_eq!(snapshot["messages"][1]["id"], "again-2");
        let listed = call("socketio.list", json!({ "workspaceId": WS })).await;
        assert_eq!(
            listed["connections"].as_array().unwrap().iter().filter(|c| c["connectionId"] == "again").count(),
            1
        );
        disconnect("again").await;
        wait_status("again", "disconnected").await;
    }

    #[tokio::test]
    async fn the_log_is_capped_and_clearable() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        connect("cap", &server.url).await;
        wait_status("cap", "connected").await;
        for n in 0..260 {
            assert_eq!(emit("cap", "echo", &n.to_string()).await, json!({ "ok": true }));
        }
        wait_for("all echoes", || {
            get_connection(WS, "cap")
                .is_some_and(|c| c["messages"][499]["data"] == "[259]" && c["messages"][499]["direction"] == "received")
        })
        .await;
        let messages = status("cap").await["messages"].as_array().unwrap().clone();
        assert_eq!(messages.len(), 500);
        // 521 messages were logged (the welcome and 260 echoes both ways); the oldest 21 were trimmed.
        assert_eq!(messages[0]["id"], "cap-22");
        assert_eq!(messages[499]["id"], "cap-521");

        let cleared = call("socketio.clearMessages", json!({ "workspaceId": WS, "connectionId": "cap" })).await;
        assert_eq!(cleared, json!({ "ok": true }));
        let snapshot = status("cap").await;
        assert_eq!((snapshot["status"].as_str(), snapshot["messages"].clone()), (Some("connected"), json!([])));
        disconnect("cap").await;
        wait_status("cap", "disconnected").await;
    }

    #[tokio::test]
    async fn unknown_ids_and_the_connection_limit() {
        let _root = scratch_data_root();
        assert_eq!(status("ghost").await, Value::Null);
        let expected = json!({ "ok": false, "error": "No connection \"ghost\" in workspace \"ws1\"." });
        assert_eq!(disconnect("ghost").await, expected);
        assert_eq!(
            call("socketio.clearMessages", json!({ "workspaceId": WS, "connectionId": "ghost" })).await,
            expected
        );

        let server = Server::start().await;
        for n in 0..MAX_SOCKETIO_CONNECTIONS {
            assert_eq!(connect(&format!("lim{n}"), &server.url).await, json!({ "ok": true }));
        }
        let refused = connect("lim-extra", &server.url).await;
        assert_eq!(refused["ok"], false);
        assert!(refused["error"].as_str().unwrap().starts_with("Max 5 active connections reached."), "{refused}");
        assert_eq!(status("lim-extra").await, Value::Null);
        // Reconnecting a connection already held is not counted against the limit.
        assert_eq!(connect("lim0", &server.url).await, json!({ "ok": true }));
        // Disconnecting while still connecting ends as a clean disconnect.
        for n in 0..MAX_SOCKETIO_CONNECTIONS {
            disconnect(&format!("lim{n}")).await;
        }
        for n in 0..MAX_SOCKETIO_CONNECTIONS {
            wait_status(&format!("lim{n}"), "disconnected").await;
        }
    }
}
