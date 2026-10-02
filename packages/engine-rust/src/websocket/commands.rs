//! The `ws.*` commands: thin bindings of the socket registry to the wire.

use serde_json::{Value, json};

use super::registry::{
    MAX_WS_CONNECTIONS, OkResult, clear_socket_messages, connect_socket, disconnect_socket, get_socket, list_sockets,
    send_socket_message,
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

/// Where a connect gets its target: `url` (and optional `headers`) for a draft, or the saved `sockets` entity
/// named by `socketId`. `headers` without `url` is refused.
fn resolve_target(params: &Value) -> Result<(String, Option<Value>)> {
    if let Some(url) = opt_str(params, "url") {
        return Ok((url.to_string(), params.get("headers").cloned()));
    }
    if params.get("headers").is_some() {
        return Err(EngineError::bad_request(
            "ws.connect takes `headers` only together with `url`; a saved request supplies its own.",
        ));
    }
    let (workspace_id, socket_id) = (str_of(params, "workspaceId"), str_of(params, "socketId"));
    let Some(saved) = load_entity(workspace_id, "sockets", socket_id) else {
        return Err(EngineError::not_found(format!(
            "No saved sockets entity \"{socket_id}\" in workspace \"{workspace_id}\". Pass `url` to connect a draft."
        )));
    };
    Ok((str_of(&saved, "url").to_string(), saved.get("headers").cloned()))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("ws.connect", |params, _| {
        let (url, headers) = resolve_target(&params)?;
        ok_result(connect_socket(str_of(&params, "workspaceId"), str_of(&params, "socketId"), &url, headers.as_ref()))
    });

    registry.register_sync("ws.disconnect", |params, _| {
        let code = params.get("code").and_then(Value::as_i64).and_then(|code| u16::try_from(code).ok());
        ok_result(disconnect_socket(
            str_of(&params, "workspaceId"),
            str_of(&params, "socketId"),
            code,
            opt_str(&params, "reason"),
        ))
    });

    registry.register_sync("ws.send", |params, _| {
        ok_result(send_socket_message(
            str_of(&params, "workspaceId"),
            str_of(&params, "socketId"),
            str_of(&params, "data"),
        ))
    });

    registry.register_sync("ws.status", |params, _| {
        Ok(get_socket(str_of(&params, "workspaceId"), str_of(&params, "socketId")).unwrap_or(Value::Null))
    });

    registry.register_sync("ws.list", |params, _| {
        Ok(json!({ "connections": list_sockets(str_of(&params, "workspaceId")), "max": MAX_WS_CONNECTIONS }))
    });

    registry.register_sync("ws.clearMessages", |params, _| {
        ok_result(clear_socket_messages(str_of(&params, "workspaceId"), str_of(&params, "socketId")))
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
        accept_async,
        tungstenite::{
            Message,
            protocol::{CloseFrame, frame::coding::CloseCode},
        },
    };

    use super::*;
    use crate::{
        kernel::{CommandContext, bus},
        testing::scratch_data_root,
        workspace::fs::write_entity,
    };

    const WS: &str = "ws1";

    fn registry() -> (Arc<CommandRegistry>, CommandContext) {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let ctx = CommandContext::internal(registry.clone());
        (registry, ctx)
    }

    async fn call(action: &str, payload: Value) -> Value {
        let (registry, ctx) = registry();
        registry.invoke(action, payload, ctx).await.expect("the command succeeds")
    }

    /// What a test server does with the first connection it accepts.
    #[derive(Clone, Copy)]
    enum Behavior {
        Echo,
        CloseAfterFirst,
    }

    /// An echo server on a loopback port. Serves any number of connections.
    async fn server(behavior: Behavior) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let Ok(mut ws) = accept_async(tcp).await else { return };
                    while let Some(Ok(message)) = ws.next().await {
                        match (behavior, message) {
                            (Behavior::Echo, message @ (Message::Text(_) | Message::Binary(_))) => {
                                if ws.send(message).await.is_err() {
                                    return;
                                }
                            }
                            (Behavior::CloseAfterFirst, Message::Text(_)) => {
                                let frame =
                                    CloseFrame { code: CloseCode::from(4000), reason: "server says bye".into() };
                                let _ = ws.close(Some(frame)).await;
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        format!("ws://{addr}")
    }

    /// Collects `ws.status` and `ws.message` events for one socket.
    struct Events {
        status: Arc<Mutex<Vec<Value>>>,
        message: Arc<Mutex<Vec<Value>>>,
        ids: Vec<crate::kernel::bus::ListenerId>,
    }

    impl Events {
        fn watch(socket_id: &'static str) -> Self {
            let status = Arc::new(Mutex::new(Vec::new()));
            let message = Arc::new(Mutex::new(Vec::new()));
            let (s, m) = (status.clone(), message.clone());
            let ids = vec![
                bus().on("ws.status", move |p| {
                    if p["socketId"] == socket_id {
                        s.lock().unwrap().push(p.clone());
                    }
                }),
                bus().on("ws.message", move |p| {
                    if p["socketId"] == socket_id {
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
    }

    impl Drop for Events {
        fn drop(&mut self) {
            for id in &self.ids {
                bus().off(*id);
            }
        }
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

    async fn wait_status(socket_id: &str, status: &str) {
        for _ in 0..200 {
            let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": socket_id })).await;
            if snapshot["status"] == status {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("socket {socket_id} never reached {status}");
    }

    async fn connect(socket_id: &str, url: &str) -> Value {
        call("ws.connect", json!({ "workspaceId": WS, "socketId": socket_id, "url": url })).await
    }

    async fn send(socket_id: &str, data: &str) -> Value {
        call("ws.send", json!({ "workspaceId": WS, "socketId": socket_id, "data": data })).await
    }

    async fn disconnect(socket_id: &str) -> Value {
        let result = call("ws.disconnect", json!({ "workspaceId": WS, "socketId": socket_id })).await;
        if result["ok"] == true {
            wait_for("the socket to close", || {
                get_socket(WS, socket_id).is_some_and(|s| s["status"] != "connecting" && s["status"] != "connected")
            })
            .await;
        }
        result
    }

    #[tokio::test]
    async fn connect_send_and_echo() {
        let _root = scratch_data_root();
        let url = server(Behavior::Echo).await;
        let events = Events::watch("echo");

        assert_eq!(connect("echo", &url).await, json!({ "ok": true }));
        wait_status("echo", "connected").await;
        assert_eq!(events.statuses(), ["connecting", "connected"]);
        let connected = call("ws.status", json!({ "workspaceId": WS, "socketId": "echo" })).await;
        assert_eq!(connected["url"], url);
        assert!(connected["openedAt"].is_i64());
        assert_eq!(connected["error"], Value::Null);

        assert_eq!(send("echo", "hello").await, json!({ "ok": true }));
        wait_for("the echo", || events.messages().len() == 2).await;
        let messages = events.messages();
        assert_eq!((messages[0]["direction"].as_str(), messages[0]["data"].as_str()), (Some("sent"), Some("hello")));
        assert_eq!(
            (messages[1]["direction"].as_str(), messages[1]["data"].as_str()),
            (Some("received"), Some("hello"))
        );
        assert_eq!((messages[0]["id"].as_str(), messages[1]["id"].as_str()), (Some("echo-1"), Some("echo-2")));
        assert!(messages[0]["ts"].is_i64());

        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "echo" })).await;
        assert_eq!(snapshot["messages"].as_array().unwrap().len(), 2);

        let listed = call("ws.list", json!({ "workspaceId": WS })).await;
        assert_eq!(listed["max"], 5);
        assert!(listed["connections"].as_array().unwrap().iter().any(|c| c["socketId"] == "echo"));

        assert_eq!(disconnect("echo").await, json!({ "ok": true }));
        wait_status("echo", "disconnected").await;
        let closed = call("ws.status", json!({ "workspaceId": WS, "socketId": "echo" })).await;
        assert_eq!(closed["closeCode"], 1000);
        assert_eq!(closed["error"], Value::Null);
        assert_eq!(events.statuses().last().map(String::as_str), Some("disconnected"));
        let last = events.status.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last["code"], 1000);

        // A finished socket keeps its entry and log but is no longer live.
        let listed = call("ws.list", json!({ "workspaceId": WS })).await;
        assert!(!listed["connections"].as_array().unwrap().iter().any(|c| c["socketId"] == "echo"));
        assert_eq!(disconnect("echo").await, json!({ "ok": true }));
        assert_eq!(send("echo", "late").await, json!({ "ok": false, "error": "Not connected" }));
    }

    #[tokio::test]
    async fn binary_echo_is_logged_as_a_placeholder() {
        let _root = scratch_data_root();
        let url = server(Behavior::Echo).await;
        let events = Events::watch("bin");
        connect("bin", &url).await;
        wait_status("bin", "connected").await;

        // The wire sends text only, so a server that pushes a binary frame stands in for a binary echo.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(tcp).await.unwrap();
            ws.send(Message::Binary(vec![9u8, 9].into())).await.unwrap();
            while ws.next().await.is_some() {}
        });
        connect("bin", &format!("ws://{addr}")).await;
        wait_for("the binary frame", || events.messages().iter().any(|m| m["data"] == "[binary data]")).await;
        let message = events.messages().into_iter().find(|m| m["data"] == "[binary data]").unwrap();
        assert_eq!(message["direction"], "received");
        disconnect("bin").await;
        wait_status("bin", "disconnected").await;
    }

    #[tokio::test]
    async fn server_close_reports_code_and_reason() {
        let _root = scratch_data_root();
        let url = server(Behavior::CloseAfterFirst).await;
        let events = Events::watch("srvclose");
        connect("srvclose", &url).await;
        wait_status("srvclose", "connected").await;
        send("srvclose", "bye?").await;
        wait_status("srvclose", "error").await;

        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "srvclose" })).await;
        assert_eq!(snapshot["error"], "server says bye");
        assert_eq!(snapshot["closeCode"], 4000);
        assert_eq!(snapshot["closeReason"], "server says bye");
        let last = events.status.lock().unwrap().last().cloned().unwrap();
        assert_eq!(last["status"], "error");
        assert_eq!(last["code"], 4000);
        assert_eq!(last["reason"], "server says bye");
        assert_eq!(last["error"], "server says bye");
    }

    #[tokio::test]
    async fn reconnect_replaces_the_socket_and_keeps_the_log() {
        let _root = scratch_data_root();
        let url = server(Behavior::Echo).await;
        let events = Events::watch("again");
        connect("again", &url).await;
        wait_status("again", "connected").await;
        send("again", "one").await;
        wait_for("the first echo", || events.messages().len() == 2).await;

        assert_eq!(connect("again", &url).await, json!({ "ok": true }));
        // The superseded socket's close must not report a `disconnected` after the new `connecting`.
        wait_for("the second connection", || events.statuses().iter().filter(|s| *s == "connected").count() == 2).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(events.statuses(), ["connecting", "connected", "connecting", "connected"]);

        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "again" })).await;
        assert_eq!(snapshot["status"], "connected");
        assert_eq!(snapshot["messages"].as_array().unwrap().len(), 2);
        send("again", "two").await;
        wait_for("the second echo", || events.messages().len() == 4).await;
        assert_eq!(events.messages()[3]["id"], "again-4");
        let listed = call("ws.list", json!({ "workspaceId": WS })).await;
        assert_eq!(listed["connections"].as_array().unwrap().iter().filter(|c| c["socketId"] == "again").count(), 1);
        disconnect("again").await;
    }

    #[tokio::test]
    async fn saved_socket_resolves_variables() {
        let root = scratch_data_root();
        let url = server(Behavior::Echo).await;
        let addr = url.trim_start_matches("ws://").to_string();
        write_entity(
            root.ws_id,
            "environments",
            "env1",
            &json!({
                "id": "env1", "name": "Env", "variables": [
                    { "key": "HOST", "value": addr, "enabled": true },
                    { "key": "MSG", "value": "from-env", "enabled": true },
                ]
            }),
            None,
        )
        .unwrap();
        write_entity(
            root.ws_id,
            "sockets",
            "saved1",
            &json!({ "id": "saved1", "name": "Saved", "url": "ws://{{HOST}}", "headers": { "X-Test": "{{MSG}}" },
                     "createdAt": 1, "workspaceId": root.ws_id }),
            None,
        )
        .unwrap();

        let events = Events::watch("saved1");
        let result = call("ws.connect", json!({ "workspaceId": WS, "socketId": "saved1" })).await;
        assert_eq!(result, json!({ "ok": true }));
        wait_status("saved1", "connected").await;
        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "saved1" })).await;
        assert_eq!(snapshot["url"], "ws://{{HOST}}");

        send("saved1", "{{MSG}}").await;
        wait_for("the echo", || events.messages().len() == 2).await;
        assert_eq!(events.messages()[1]["data"], "from-env");
        disconnect("saved1").await;

        let (registry, ctx) = registry();
        let missing =
            registry.invoke("ws.connect", json!({ "workspaceId": WS, "socketId": "nope" }), ctx.clone()).await;
        assert_eq!(missing.unwrap_err().code, crate::protocol::ErrorCode::NotFound);
        let headers_only = registry
            .invoke("ws.connect", json!({ "workspaceId": WS, "socketId": "x", "headers": { "A": "b" } }), ctx)
            .await;
        assert_eq!(headers_only.unwrap_err().code, crate::protocol::ErrorCode::BadRequest);
    }

    #[tokio::test]
    async fn refused_connection_reports_the_error() {
        let _root = scratch_data_root();
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let events = Events::watch("refused");
        assert_eq!(connect("refused", &format!("ws://127.0.0.1:{port}")).await, json!({ "ok": true }));
        wait_status("refused", "error").await;
        wait_for("the close", || events.statuses().len() == 3).await;
        assert_eq!(events.statuses(), ["connecting", "error", "error"]);
        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "refused" })).await;
        assert_eq!(snapshot["closeCode"], 1006);
        assert_eq!(snapshot["error"], "Connection closed (code 1006)");
        assert_eq!(snapshot["openedAt"], Value::Null);

        let invalid = connect("badurl", "not a url").await;
        assert_eq!(invalid["ok"], false);
        assert!(invalid["error"].is_string());
        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "badurl" })).await;
        assert_eq!(snapshot["status"], "error");
    }

    #[tokio::test]
    async fn message_log_is_capped_and_clearable() {
        let _root = scratch_data_root();
        let url = server(Behavior::Echo).await;
        connect("cap", &url).await;
        wait_status("cap", "connected").await;
        for i in 0..260 {
            assert_eq!(send("cap", &format!("m{i}")).await, json!({ "ok": true }));
        }
        wait_for("all echoes", || get_socket(WS, "cap").is_some_and(|s| s["messages"][499]["data"] == "m259")).await;
        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "cap" })).await;
        let messages = snapshot["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 500);
        // 520 messages were logged; the oldest 20 were trimmed from the front.
        assert_eq!(messages[0]["id"], "cap-21");
        assert_eq!(messages[499]["id"], "cap-520");

        assert_eq!(
            call("ws.clearMessages", json!({ "workspaceId": WS, "socketId": "cap" })).await,
            json!({ "ok": true })
        );
        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "cap" })).await;
        assert_eq!(snapshot["messages"], json!([]));
        assert_eq!(snapshot["status"], "connected");
        send("cap", "after").await;
        wait_for("a new message", || {
            get_socket(WS, "cap").is_some_and(|s| s["messages"].as_array().unwrap().len() == 2)
        })
        .await;
        let snapshot = call("ws.status", json!({ "workspaceId": WS, "socketId": "cap" })).await;
        assert_eq!(snapshot["messages"][0]["id"], "cap-521");
        disconnect("cap").await;
    }

    #[tokio::test]
    async fn unknown_ids_and_the_connection_limit() {
        let _root = scratch_data_root();
        assert_eq!(call("ws.status", json!({ "workspaceId": WS, "socketId": "ghost" })).await, Value::Null);
        let expected = json!({ "ok": false, "error": "No socket \"ghost\" in workspace \"ws1\"." });
        assert_eq!(disconnect("ghost").await, expected);
        assert_eq!(call("ws.clearMessages", json!({ "workspaceId": WS, "socketId": "ghost" })).await, expected);
        assert_eq!(send("ghost", "x").await, json!({ "ok": false, "error": "Not connected" }));

        let url = server(Behavior::Echo).await;
        for i in 0..5 {
            assert_eq!(connect(&format!("lim{i}"), &url).await, json!({ "ok": true }));
        }
        let refused = connect("lim5", &url).await;
        assert_eq!(refused["ok"], false);
        assert!(refused["error"].as_str().unwrap().starts_with("Max 5 active connections reached."));
        assert_eq!(call("ws.status", json!({ "workspaceId": WS, "socketId": "lim5" })).await, Value::Null);
        // Reconnecting a socket we already hold is not refused by the limit.
        assert_eq!(connect("lim0", &url).await, json!({ "ok": true }));
        for i in 0..5 {
            disconnect(&format!("lim{i}")).await;
        }
        // Disconnecting while still connecting aborts the handshake and ends as a clean disconnect.
        for i in 0..5 {
            wait_status(&format!("lim{i}"), "disconnected").await;
        }
    }
}
