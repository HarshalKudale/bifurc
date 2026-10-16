//! The `sse.*` commands: thin bindings of the stream registry to the wire.

use serde_json::{Value, json};

use super::registry::{
    MAX_SSE_STREAMS, OkResult, clear_stream_messages, connect_stream, disconnect_stream, get_stream, list_streams,
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

/// What a connect sends: the url, method, headers and body.
struct Target {
    url: String,
    method: String,
    headers: Option<Value>,
    body: Option<String>,
}

/// Where a connect gets its target: `url` (with optional `method`, `headers`, `body`) for a draft, or the saved
/// `sseRequests` entity named by `id` (by `streamId` when `id` is omitted). The draft's other fields without
/// `url` are refused.
fn resolve_target(params: &Value) -> Result<Target> {
    if let Some(url) = opt_str(params, "url") {
        return Ok(Target {
            url: url.to_string(),
            method: opt_str(params, "method").unwrap_or("GET").to_string(),
            headers: params.get("headers").cloned(),
            body: opt_str(params, "body").map(str::to_string),
        });
    }
    if ["method", "headers", "body"].iter().any(|field| params.get(*field).is_some()) {
        return Err(EngineError::bad_request(
            "sse.connect takes `method`, `headers` and `body` only together with `url`; a saved request supplies its own.",
        ));
    }
    let workspace_id = str_of(params, "workspaceId");
    let id = opt_str(params, "id").unwrap_or_else(|| str_of(params, "streamId"));
    let Some(saved) = load_entity(workspace_id, "sseRequests", id) else {
        return Err(EngineError::not_found(format!(
            "No saved sseRequests entity \"{id}\" in workspace \"{workspace_id}\". Pass `url` to connect a draft."
        )));
    };
    Ok(Target {
        url: str_of(&saved, "url").to_string(),
        method: opt_str(&saved, "method").filter(|m| !m.is_empty()).unwrap_or("GET").to_string(),
        headers: saved.get("headers").cloned(),
        body: opt_str(&saved, "body").map(str::to_string),
    })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("sse.connect", |params, _| {
        let target = resolve_target(&params)?;
        ok_result(connect_stream(
            str_of(&params, "workspaceId"),
            str_of(&params, "streamId"),
            &target.url,
            &target.method,
            target.headers.as_ref(),
            target.body.as_deref(),
        ))
    });

    registry.register_sync("sse.disconnect", |params, _| {
        ok_result(disconnect_stream(str_of(&params, "workspaceId"), str_of(&params, "streamId")))
    });

    registry.register_sync("sse.status", |params, _| {
        Ok(get_stream(str_of(&params, "workspaceId"), str_of(&params, "streamId")).unwrap_or(Value::Null))
    });

    registry.register_sync("sse.list", |params, _| {
        Ok(json!({ "streams": list_streams(str_of(&params, "workspaceId")), "max": MAX_SSE_STREAMS }))
    });

    registry.register_sync("sse.clearMessages", |params, _| {
        ok_result(clear_stream_messages(str_of(&params, "workspaceId"), str_of(&params, "streamId")))
    });
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
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

    async fn pause() {
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    /// Reads one request: its head (request line and headers, lower-cased names) and its body.
    async fn read_request(socket: &mut TcpStream) -> Option<(String, String, Value, String)> {
        let mut data = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => return None,
                Ok(n) => data.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split(' ');
        let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
        let mut headers = serde_json::Map::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), json!(value.trim()));
            }
        }
        let length: usize =
            headers.get("content-length").and_then(Value::as_str).and_then(|v| v.parse().ok()).unwrap_or(0);
        while data.len() < head_end + 4 + length {
            match socket.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => data.extend_from_slice(&chunk[..n]),
            }
        }
        let body = String::from_utf8_lossy(&data[(head_end + 4).min(data.len())..]).into_owned();
        Some((method, path, Value::Object(headers), body))
    }

    const STREAM_HEAD: &str =
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n";

    /// A loopback SSE server. `/events` sends a few events, one of them split mid-line across writes, then ends;
    /// `/hold` sends one event and stays open until the client leaves; `/echo` answers with one event describing
    /// the request it got; `/missing` is a 404; anything else is a 200 that is not an event stream.
    async fn server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let Some((method, path, headers, body)) = read_request(&mut socket).await else { return };
                    match path.as_str() {
                        "/events" => {
                            let _ = socket.write_all(STREAM_HEAD.as_bytes()).await;
                            for piece in [
                                ": hello\n\nid: 1\nevent: greeting\ndata: hel",
                                "lo\n\ndata: a\r\ndata: b\r\n\r\n",
                                "retry: 3000\ndata: last\n\n",
                            ] {
                                let _ = socket.write_all(piece.as_bytes()).await;
                                let _ = socket.flush().await;
                                pause().await;
                            }
                            let _ = socket.shutdown().await;
                        }
                        "/hold" => {
                            let _ = socket.write_all(format!("{STREAM_HEAD}data: first\n\n").as_bytes()).await;
                            let mut sink = [0u8; 64];
                            while matches!(socket.read(&mut sink).await, Ok(n) if n > 0) {}
                        }
                        "/echo" => {
                            let described = json!({ "method": method, "headers": headers, "body": body });
                            let _ = socket.write_all(format!("{STREAM_HEAD}data: {described}\n\n").as_bytes()).await;
                            let _ = socket.shutdown().await;
                        }
                        "/missing" => {
                            let _ = socket
                                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                                .await;
                            let _ = socket.shutdown().await;
                        }
                        _ => {
                            let _ = socket
                                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
                                .await;
                            let _ = socket.shutdown().await;
                        }
                    }
                });
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    struct Events {
        status: Arc<Mutex<Vec<Value>>>,
        message: Arc<Mutex<Vec<Value>>>,
        ids: Vec<crate::kernel::ListenerId>,
    }

    impl Events {
        fn watch(stream_id: &'static str) -> Self {
            let status = Arc::new(Mutex::new(Vec::new()));
            let message = Arc::new(Mutex::new(Vec::new()));
            let (s, m) = (status.clone(), message.clone());
            let ids = vec![
                bus().on("sse.status", move |p| {
                    if p["streamId"] == stream_id {
                        s.lock().unwrap().push(p.clone());
                    }
                }),
                bus().on("sse.message", move |p| {
                    if p["streamId"] == stream_id {
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

    async fn wait_status(stream_id: &str, status: &str) -> Value {
        for _ in 0..200 {
            let snapshot = call("sse.status", json!({ "workspaceId": WS, "streamId": stream_id })).await;
            if snapshot["status"] == status {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("stream {stream_id} never reached {status}");
    }

    async fn connect(stream_id: &str, url: &str) -> Value {
        call("sse.connect", json!({ "workspaceId": WS, "streamId": stream_id, "url": url })).await
    }

    #[tokio::test]
    async fn a_stream_delivers_events_in_order_and_ends_cleanly() {
        let _root = scratch_data_root();
        let base = server().await;
        let events = Events::watch("flow");
        assert_eq!(connect("flow", &format!("{base}/events")).await, json!({ "ok": true }));
        let done = wait_status("flow", "disconnected").await;

        assert_eq!(events.statuses(), ["connecting", "connected", "disconnected"]);
        let logged = done["messages"].as_array().unwrap();
        let shown: Vec<(&str, &str)> =
            logged.iter().map(|m| (m["event"].as_str().unwrap(), m["data"].as_str().unwrap())).collect();
        assert_eq!(shown, [("greeting", "hello"), ("message", "a\nb"), ("message", "last")]);
        assert_eq!(logged[0]["eventId"], "1");
        assert_eq!(logged[2]["retry"], 3000);
        assert_eq!(logged[0]["id"], "flow-1");
        assert_eq!(events.messages(), *logged);
        assert_eq!(done["httpStatus"], 200);
        assert_eq!(done["error"], Value::Null);
        assert!(done["openedAt"].is_i64());
    }

    #[tokio::test]
    async fn the_request_asks_for_an_event_stream_and_carries_method_headers_and_body() {
        let _root = scratch_data_root();
        let base = server().await;
        let result = call(
            "sse.connect",
            json!({ "workspaceId": WS, "streamId": "echo", "url": format!("{base}/echo"), "method": "post",
                    "headers": { "X-Token": "abc", "Accept": "text/plain" }, "body": "{\"q\":1}" }),
        )
        .await;
        assert_eq!(result, json!({ "ok": true }));
        let done = wait_status("echo", "disconnected").await;
        let seen: Value = serde_json::from_str(done["messages"][0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(seen["method"], "POST");
        assert_eq!(seen["body"], "{\"q\":1}");
        assert_eq!(seen["headers"]["x-token"], "abc");
        // The caller's header wins over the default; the default fills what the caller left out.
        assert_eq!(seen["headers"]["accept"], "text/plain");
        assert_eq!(seen["headers"]["cache-control"], "no-cache");
        assert_eq!(done["method"], "POST");

        connect("echo-default", &format!("{base}/echo")).await;
        let done = wait_status("echo-default", "disconnected").await;
        let seen: Value = serde_json::from_str(done["messages"][0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(
            (seen["method"].as_str(), seen["headers"]["accept"].as_str()),
            (Some("GET"), Some("text/event-stream"))
        );
    }

    #[tokio::test]
    async fn a_saved_request_resolves_variables() {
        let root = scratch_data_root();
        let base = server().await;
        write_entity(
            root.ws_id,
            "environments",
            "env1",
            &json!({ "id": "env1", "name": "Env", "variables": [
                { "key": "BASE", "value": base, "enabled": true },
                { "key": "TOKEN", "value": "from-env", "enabled": true },
            ] }),
            None,
        )
        .unwrap();
        write_entity(
            root.ws_id,
            "sseRequests",
            "saved1",
            &json!({ "id": "saved1", "name": "Saved", "url": "{{BASE}}/echo", "method": "POST",
                     "headers": { "X-Test": "{{TOKEN}}" }, "body": "{{TOKEN}}", "createdAt": 1, "workspaceId": root.ws_id }),
            None,
        )
        .unwrap();

        assert_eq!(
            call("sse.connect", json!({ "workspaceId": WS, "streamId": "saved1" })).await,
            json!({ "ok": true })
        );
        let done = wait_status("saved1", "disconnected").await;
        assert_eq!(done["url"], "{{BASE}}/echo");
        let seen: Value = serde_json::from_str(done["messages"][0]["data"].as_str().unwrap()).unwrap();
        assert_eq!(
            (seen["method"].as_str(), seen["body"].as_str(), seen["headers"]["x-test"].as_str()),
            (Some("POST"), Some("from-env"), Some("from-env"))
        );

        // The saved request is named by `id` when the stream has another name.
        call("sse.connect", json!({ "workspaceId": WS, "streamId": "tab-2", "id": "saved1" })).await;
        let done = wait_status("tab-2", "disconnected").await;
        assert_eq!(done["messages"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_missing_saved_request_and_stray_draft_fields_are_refused() {
        let _root = scratch_data_root();
        assert_eq!(
            call_err("sse.connect", json!({ "workspaceId": WS, "streamId": "nope" })).await.code,
            ErrorCode::NotFound
        );
        let stray =
            call_err("sse.connect", json!({ "workspaceId": WS, "streamId": "x", "headers": { "A": "b" } })).await;
        assert_eq!(stray.code, ErrorCode::BadRequest);
        let stray = call_err("sse.connect", json!({ "workspaceId": WS, "streamId": "x", "body": "b" })).await;
        assert_eq!(stray.code, ErrorCode::BadRequest);
    }

    #[tokio::test]
    async fn an_http_error_status_is_an_error() {
        let _root = scratch_data_root();
        let base = server().await;
        let events = Events::watch("missing");
        connect("missing", &format!("{base}/missing")).await;
        let done = wait_status("missing", "error").await;
        assert_eq!(done["error"], "HTTP 404 Not Found");
        assert_eq!(done["httpStatus"], 404);
        assert_eq!(done["messages"], json!([]));
        assert_eq!(events.statuses(), ["connecting", "error"]);
    }

    #[tokio::test]
    async fn a_refused_connection_and_a_bad_url_report_the_error() {
        let _root = scratch_data_root();
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        connect("refused", &format!("http://127.0.0.1:{port}/")).await;
        let done = wait_status("refused", "error").await;
        assert!(done["error"].as_str().unwrap().contains("Nothing is listening at"), "{done}");

        for (stream_id, url) in [("bad-url", "not a url"), ("bad-scheme", "ws://127.0.0.1/x"), ("empty", "")] {
            let result = connect(stream_id, url).await;
            assert_eq!(result["ok"], false, "{url}");
            assert!(result["error"].as_str().unwrap().contains("Invalid URL"), "{result}");
            assert_eq!(
                call("sse.status", json!({ "workspaceId": WS, "streamId": stream_id })).await["status"],
                "error"
            );
        }
        let result = call(
            "sse.connect",
            json!({ "workspaceId": WS, "streamId": "bad-method", "url": "http://127.0.0.1/x", "method": "NO PE" }),
        )
        .await;
        assert_eq!(result["ok"], false);
        let result = call(
            "sse.connect",
            json!({ "workspaceId": WS, "streamId": "bad-header", "url": "http://127.0.0.1/x", "headers": { "bad name": "v" } }),
        )
        .await;
        assert!(result["error"].as_str().unwrap().contains("header name"), "{result}");
    }

    #[tokio::test]
    async fn disconnect_closes_a_live_stream_without_an_error() {
        let _root = scratch_data_root();
        let base = server().await;
        let events = Events::watch("held");
        connect("held", &format!("{base}/hold")).await;
        wait_for("the first event", || events.messages().len() == 1).await;

        let listed = call("sse.list", json!({ "workspaceId": WS })).await;
        assert!(listed["streams"].as_array().unwrap().iter().any(|s| s["streamId"] == "held"));
        assert_eq!(listed["max"], MAX_SSE_STREAMS);

        assert_eq!(
            call("sse.disconnect", json!({ "workspaceId": WS, "streamId": "held" })).await,
            json!({ "ok": true })
        );
        let done = wait_status("held", "disconnected").await;
        assert_eq!(done["error"], Value::Null);
        assert_eq!(events.statuses(), ["connecting", "connected", "disconnected"]);
        // The log is still readable, and the stream is no longer listed.
        assert_eq!(done["messages"].as_array().unwrap().len(), 1);
        let listed = call("sse.list", json!({ "workspaceId": WS })).await;
        assert!(!listed["streams"].as_array().unwrap().iter().any(|s| s["streamId"] == "held"));
        // Closing it again is fine; a stream the engine never opened is not.
        assert_eq!(
            call("sse.disconnect", json!({ "workspaceId": WS, "streamId": "held" })).await,
            json!({ "ok": true })
        );
        let unknown = call("sse.disconnect", json!({ "workspaceId": WS, "streamId": "never" })).await;
        assert_eq!(unknown["ok"], false);
        assert!(unknown["error"].as_str().unwrap().contains("never"));
    }

    #[tokio::test]
    async fn a_reconnect_replaces_the_connection_and_keeps_the_log() {
        let _root = scratch_data_root();
        let base = server().await;
        let events = Events::watch("again");
        connect("again", &format!("{base}/hold")).await;
        wait_for("the first event", || events.messages().len() == 1).await;
        connect("again", &format!("{base}/hold")).await;
        wait_for("the second event", || events.messages().len() == 2).await;

        let snapshot = wait_status("again", "connected").await;
        assert_eq!(snapshot["messages"].as_array().unwrap().len(), 2);
        assert_eq!(snapshot["messages"][1]["id"], "again-2");
        let listed = call("sse.list", json!({ "workspaceId": WS })).await;
        assert_eq!(listed["streams"].as_array().unwrap().iter().filter(|s| s["streamId"] == "again").count(), 1);
        // The superseded connection's late end must not overwrite the new connection's state.
        pause().await;
        assert_eq!(call("sse.status", json!({ "workspaceId": WS, "streamId": "again" })).await["status"], "connected");
        call("sse.disconnect", json!({ "workspaceId": WS, "streamId": "again" })).await;
        wait_status("again", "disconnected").await;
    }

    #[tokio::test]
    async fn the_log_can_be_cleared_without_touching_the_stream() {
        let _root = scratch_data_root();
        let base = server().await;
        let events = Events::watch("clear");
        connect("clear", &format!("{base}/hold")).await;
        wait_for("the first event", || events.messages().len() == 1).await;
        assert_eq!(
            call("sse.clearMessages", json!({ "workspaceId": WS, "streamId": "clear" })).await,
            json!({ "ok": true })
        );
        let snapshot = call("sse.status", json!({ "workspaceId": WS, "streamId": "clear" })).await;
        assert_eq!((snapshot["status"].as_str(), snapshot["messages"].clone()), (Some("connected"), json!([])));
        let unknown = call("sse.clearMessages", json!({ "workspaceId": WS, "streamId": "never" })).await;
        assert_eq!(unknown["ok"], false);
        call("sse.disconnect", json!({ "workspaceId": WS, "streamId": "clear" })).await;
        wait_status("clear", "disconnected").await;
    }

    #[tokio::test]
    async fn the_number_of_live_streams_is_limited_per_workspace() {
        let _root = scratch_data_root();
        let base = server().await;
        for n in 0..MAX_SSE_STREAMS {
            assert_eq!(connect(&format!("cap-{n}"), &format!("{base}/hold")).await, json!({ "ok": true }));
            wait_status(&format!("cap-{n}"), "connected").await;
        }
        let refused = connect("cap-extra", &format!("{base}/hold")).await;
        assert_eq!(refused["ok"], false);
        assert!(refused["error"].as_str().unwrap().contains("Max 5"), "{refused}");
        // Reconnecting a stream already held is not counted against the limit.
        assert_eq!(connect("cap-0", &format!("{base}/hold")).await, json!({ "ok": true }));
        for n in 0..MAX_SSE_STREAMS {
            call("sse.disconnect", json!({ "workspaceId": WS, "streamId": format!("cap-{n}") })).await;
            wait_status(&format!("cap-{n}"), "disconnected").await;
        }
    }

    #[tokio::test]
    async fn the_status_of_an_unopened_stream_is_null() {
        assert_eq!(call("sse.status", json!({ "workspaceId": WS, "streamId": "unopened" })).await, Value::Null);
    }
}
