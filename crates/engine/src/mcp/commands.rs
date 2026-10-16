//! The `mcp.*` commands: thin bindings of the connection registry to the wire.

use serde_json::{Value, json};

use super::{
    arguments::CallKind,
    registry::{
        MAX_MCP_CONNECTIONS, OkResult, Target, call_server, clear_connection_messages, connect_connection,
        disconnect_connection, get_connection, list_connections,
    },
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
const TARGET_FIELDS: [&str; 2] = ["transport", "headers"];

/// Where a connect gets its target: `url` (with optional `transport` and `headers`) for a draft, or the saved
/// `mcpRequests` entity named by `id` (by `connectionId` when `id` is omitted). The draft's other fields
/// without `url` are refused. Returns the object the target is read from.
fn resolve_target(params: &Value) -> Result<Value> {
    if opt_str(params, "url").is_some() {
        return Ok(params.clone());
    }
    if TARGET_FIELDS.iter().any(|field| params.get(*field).is_some()) {
        return Err(EngineError::bad_request(
            "mcp.connect takes `transport` and `headers` only together with `url`; a saved request supplies its own.",
        ));
    }
    let workspace_id = str_of(params, "workspaceId");
    let id = opt_str(params, "id").unwrap_or_else(|| str_of(params, "connectionId"));
    load_entity(workspace_id, "mcpRequests", id).ok_or_else(|| {
        EngineError::not_found(format!(
            "No saved mcpRequests entity \"{id}\" in workspace \"{workspace_id}\". Pass `url` to connect a draft."
        ))
    })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("mcp.connect", |params, _| {
        let source = resolve_target(&params)?;
        let target = Target {
            transport: str_of(&source, "transport"),
            url: str_of(&source, "url"),
            headers: source.get("headers"),
        };
        ok_result(connect_connection(str_of(&params, "workspaceId"), str_of(&params, "connectionId"), &target))
    });

    registry.register_sync("mcp.disconnect", |params, _| {
        ok_result(disconnect_connection(str_of(&params, "workspaceId"), str_of(&params, "connectionId")))
    });

    registry.register_sync("mcp.call", |params, _| {
        let kind = CallKind::named(str_of(&params, "type")).expect("the schema names the three kinds");
        let called = call_server(
            str_of(&params, "workspaceId"),
            str_of(&params, "connectionId"),
            kind,
            str_of(&params, "name"),
            params.get("arguments").and_then(Value::as_object),
        );
        Ok(match called {
            Ok(call_id) => json!({ "ok": true, "callId": call_id }),
            Err(error) => json!({ "ok": false, "error": error }),
        })
    });

    registry.register_sync("mcp.status", |params, _| {
        Ok(get_connection(str_of(&params, "workspaceId"), str_of(&params, "connectionId")).unwrap_or(Value::Null))
    });

    registry.register_sync("mcp.list", |params, _| {
        Ok(json!({
            "connections": list_connections(str_of(&params, "workspaceId")),
            "max": MAX_MCP_CONNECTIONS,
        }))
    });

    registry.register_sync("mcp.clearMessages", |params, _| {
        ok_result(clear_connection_messages(str_of(&params, "workspaceId"), str_of(&params, "connectionId")))
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

    /// One request a [`Server`] received.
    #[derive(Clone)]
    struct Seen {
        verb: String,
        headers: Value,
        /// The JSON-RPC message of a POST.
        message: Value,
    }

    /// A loopback MCP server over Streamable HTTP, written against the wire format rather than with the SDK,
    /// and the requests every client made of it.
    ///
    /// It opens a session, declares tools (in two pages), prompts and resources, and fails to list resource
    /// templates. `echo` answers on an event stream, after a log message; `fail` answers with a tool error,
    /// `boom` with a protocol error, `renew` announces a changed tool list, and `hang` never answers. The path
    /// chooses a behaviour: `/deny` wants credentials, `/html` is a web page, `/mute` never answers.
    struct Server {
        url: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Server {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let noted = seen.clone();
            tokio::spawn(async move {
                while let Ok((tcp, _)) = listener.accept().await {
                    tokio::spawn(serve(tcp, noted.clone()));
                }
            });
            Self { url, seen }
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        /// The JSON-RPC requests received for `method`.
        fn asked(&self, method: &str) -> Vec<Seen> {
            self.seen().into_iter().filter(|seen| seen.message["method"] == method).collect()
        }
    }

    /// Reads one request: its verb, path, headers (lower-cased names) and body.
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
        let (verb, path) = (first.next()?.to_string(), first.next()?.to_string());
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
        Some((verb, path, Value::Object(headers), body))
    }

    async fn reply(socket: &mut TcpStream, status: &str, extra: &str, content_type: &str, body: &str) {
        let head = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n",
            body.len()
        );
        let _ = socket.write_all(format!("{head}{body}").as_bytes()).await;
        let _ = socket.shutdown().await;
    }

    fn tool(name: &str, properties: Value, required: Value) -> Value {
        json!({ "name": name, "description": format!("The {name} tool"),
                "inputSchema": { "type": "object", "properties": properties, "required": required } })
    }

    /// What the server answers the JSON-RPC request `message` with: a result, or an error object.
    fn answer(message: &Value) -> std::result::Result<Value, Value> {
        let params = &message["params"];
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": { "listChanged": true }, "prompts": {}, "resources": {} },
                "serverInfo": { "name": "loopback", "version": "1.2.3" },
                "instructions": "Be kind.",
            })),
            "tools/list" if params["cursor"].is_null() => Ok(json!({
                "tools": [tool(
                    "echo",
                    json!({ "text": { "type": "string" }, "count": { "type": "integer" }, "loud": { "type": "boolean" } }),
                    json!(["text"]),
                )],
                "nextCursor": "page-2",
            })),
            "tools/list" => Ok(json!({ "tools": [
                tool("fail", json!({}), json!([])), tool("boom", json!({}), json!([])),
                tool("hang", json!({}), json!([])), tool("renew", json!({}), json!([])),
            ] })),
            "prompts/list" => Ok(json!({ "prompts": [
                { "name": "greet", "arguments": [{ "name": "who", "required": true }, { "name": "tone" }] },
            ] })),
            "resources/list" => Ok(json!({ "resources": [{ "uri": "mem://alpha", "name": "alpha" }] })),
            "tools/call" => match params["name"].as_str().unwrap_or_default() {
                "fail" => Ok(json!({ "content": [{ "type": "text", "text": "it broke" }], "isError": true })),
                "boom" => Err(json!({ "code": -32000, "message": "boom" })),
                _ => Ok(json!({ "content": [{ "type": "text", "text": params["arguments"].to_string() }] })),
            },
            "prompts/get" => {
                let who = params["arguments"]["who"].as_str().unwrap_or("nobody");
                let text = format!("Hello {who}");
                Ok(json!({ "messages": [{ "role": "user", "content": { "type": "text", "text": text } }] }))
            }
            "resources/read" => Ok(json!({ "contents": [{ "uri": params["uri"], "text": "alpha's text" }] })),
            other => Err(json!({ "code": -32601, "message": format!("Method not found: {other}") })),
        }
    }

    async fn serve(mut socket: TcpStream, seen: Arc<Mutex<Vec<Seen>>>) {
        let Some((verb, path, headers, body)) = read_request(&mut socket).await else { return };
        let message: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        seen.lock().unwrap().push(Seen { verb: verb.clone(), headers, message: message.clone() });
        match path.as_str() {
            "/deny" => {
                let challenge = "www-authenticate: Bearer realm=\"loopback\"\r\n";
                return reply(&mut socket, "401 Unauthorized", challenge, "text/plain", "no").await;
            }
            "/html" => return reply(&mut socket, "200 OK", "", "text/html", "<html>hello</html>").await,
            "/mute" => {
                let mut sink = [0u8; 64];
                while matches!(socket.read(&mut sink).await, Ok(n) if n > 0) {}
                return;
            }
            _ => {}
        }
        if verb != "POST" {
            // No stream of the server's own, and nothing to do when a session is deleted.
            let status = if verb == "DELETE" { "200 OK" } else { "405 Method Not Allowed" };
            return reply(&mut socket, status, "", "text/plain", "").await;
        }
        if message["id"].is_null() {
            return reply(&mut socket, "202 Accepted", "", "text/plain", "").await;
        }
        let called = message["params"]["name"].as_str().unwrap_or_default();
        if message["method"] == "tools/call" && called == "hang" {
            let mut sink = [0u8; 64];
            while matches!(socket.read(&mut sink).await, Ok(n) if n > 0) {}
            return;
        }
        let answered = match answer(&message) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": message["id"], "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": message["id"], "error": error }),
        };
        if message["method"] == "tools/call" && matches!(called, "echo" | "renew") {
            // An event stream: what the server has to say first, then the answer.
            let first = match called {
                "renew" => json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }),
                _ => json!({ "jsonrpc": "2.0", "method": "notifications/message",
                             "params": { "level": "info", "data": "working on it" } }),
            };
            let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
            let events = format!("{head}event: message\ndata: {first}\n\nevent: message\ndata: {answered}\n\n");
            let _ = socket.write_all(events.as_bytes()).await;
            let _ = socket.shutdown().await;
            return;
        }
        let session = if message["method"] == "initialize" { "mcp-session-id: session-1\r\n" } else { "" };
        reply(&mut socket, "200 OK", session, "application/json", &answered.to_string()).await;
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
                bus().on("mcp.status", move |p| {
                    if p["connectionId"] == connection_id {
                        s.lock().unwrap().push(p.clone());
                    }
                }),
                bus().on("mcp.message", move |p| {
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
    }

    impl Drop for Events {
        fn drop(&mut self) {
            for id in &self.ids {
                bus().off(*id);
            }
        }
    }

    async fn status(connection_id: &str) -> Value {
        call("mcp.status", json!({ "workspaceId": WS, "connectionId": connection_id })).await
    }

    async fn wait_status(connection_id: &str, wanted: &str) -> Value {
        for _ in 0..400 {
            let snapshot = status(connection_id).await;
            if snapshot["status"] == wanted {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("connection {connection_id} never reached {wanted}: {}", status(connection_id).await);
    }

    async fn connect(connection_id: &str, url: &str) -> Value {
        call("mcp.connect", json!({ "workspaceId": WS, "connectionId": connection_id, "url": url })).await
    }

    async fn disconnect(connection_id: &str) -> Value {
        call("mcp.disconnect", json!({ "workspaceId": WS, "connectionId": connection_id })).await
    }

    /// Ask for something and return its log line once the server has answered.
    async fn ask(connection_id: &str, kind: &str, name: &str, arguments: Value) -> Value {
        let mut payload = json!({ "workspaceId": WS, "connectionId": connection_id, "type": kind, "name": name });
        if !arguments.is_null() {
            payload["arguments"] = arguments;
        }
        let called = call("mcp.call", payload).await;
        assert_eq!(called["ok"], true, "{called}");
        let call_id = called["callId"].as_str().unwrap().to_string();
        for _ in 0..400 {
            let snapshot = status(connection_id).await;
            let line = snapshot["messages"].as_array().unwrap().iter().find(|m| m["id"] == call_id.as_str()).cloned();
            if let Some(line) = line.filter(|line| line["status"] != "pending") {
                return line;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("{kind} {name} was never answered: {}", status(connection_id).await);
    }

    fn data(line: &Value) -> Value {
        serde_json::from_str(line["data"].as_str().unwrap()).unwrap()
    }

    fn names(list: &Value) -> Vec<String> {
        list.as_array().unwrap().iter().map(|item| item["name"].as_str().unwrap().to_string()).collect()
    }

    #[tokio::test]
    async fn connecting_reads_what_the_server_declares() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("main");

        assert_eq!(connect("main", &server.url).await, json!({ "ok": true }));
        let connected = wait_status("main", "connected").await;
        assert_eq!(events.statuses(), ["connecting", "connected"]);
        assert_eq!(connected["transport"], "http");
        assert_eq!(connected["url"], server.url);
        assert_eq!(connected["error"], Value::Null);
        assert_eq!(connected["serverInfo"]["name"], "loopback");
        assert_eq!(connected["serverInfo"]["version"], "1.2.3");
        assert_eq!(connected["protocolVersion"], "2025-06-18");
        assert_eq!(connected["instructions"], "Be kind.");
        assert!(connected["capabilities"]["tools"].is_object());
        assert!(connected["openedAt"].is_i64());
        // Both pages of the tools; the list the server could not produce is empty.
        assert_eq!(names(&connected["tools"]), ["echo", "fail", "boom", "hang", "renew"]);
        assert_eq!(connected["tools"][0]["inputSchema"]["required"], json!(["text"]));
        assert_eq!(names(&connected["prompts"]), ["greet"]);
        assert_eq!(connected["resources"][0]["uri"], "mem://alpha");
        assert_eq!(connected["resourceTemplates"], json!([]));
        // The status event carries what was declared, so a client need not ask again.
        let announced = events.status.lock().unwrap().last().cloned().unwrap();
        assert_eq!(names(&announced["tools"]).len(), 5);
        assert!(announced.get("messages").is_none());

        // The handshake came first, and the session it opened is named on what followed.
        let seen = server.seen();
        assert_eq!(seen[0].message["method"], "initialize");
        assert_eq!(seen[0].message["params"]["clientInfo"]["name"], "bifurc");
        assert_eq!(server.asked("notifications/initialized").len(), 1);
        let listed = &server.asked("prompts/list")[0];
        assert_eq!(listed.headers["mcp-session-id"], "session-1");
        assert_eq!(listed.headers["mcp-protocol-version"], "2025-06-18");

        let listed = call("mcp.list", json!({ "workspaceId": WS })).await;
        assert_eq!(listed["max"], MAX_MCP_CONNECTIONS);
        assert!(listed["connections"].as_array().unwrap().iter().any(|c| c["connectionId"] == "main"));

        assert_eq!(disconnect("main").await, json!({ "ok": true }));
        let closed = wait_status("main", "disconnected").await;
        assert_eq!(closed["error"], Value::Null);
        assert_eq!(events.statuses(), ["connecting", "connected", "disconnected"]);
        // The server was told the session is over.
        for _ in 0..200 {
            if server.seen().iter().any(|seen| seen.verb == "DELETE") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let deleted = server.seen().into_iter().find(|seen| seen.verb == "DELETE").expect("the session is deleted");
        assert_eq!(deleted.headers["mcp-session-id"], "session-1");
        // A finished connection keeps its entry but is no longer live, and what it declared is still shown.
        let listed = call("mcp.list", json!({ "workspaceId": WS })).await;
        assert!(!listed["connections"].as_array().unwrap().iter().any(|c| c["connectionId"] == "main"));
        assert_eq!(names(&closed["tools"]).len(), 5);
        assert_eq!(disconnect("main").await, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn a_tool_is_called_with_arguments_of_the_declared_types() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("tools");
        connect("tools", &server.url).await;
        wait_status("tools", "connected").await;

        let line = ask("tools", "tool", "echo", json!({ "text": "7", "count": "7", "loud": "true" })).await;
        assert_eq!(line["type"], "tool");
        assert_eq!(line["name"], "echo");
        assert_eq!(line["status"], "ok");
        assert!(line["durationMs"].is_u64());
        assert_eq!(line["arguments"], "{\"text\":\"7\",\"count\":7,\"loud\":true}");
        // The server echoes the arguments it was given: text stayed text, the others became what they are.
        assert_eq!(data(&line)["content"][0]["text"], "{\"text\":\"7\",\"count\":7,\"loud\":true}");
        // Beside the answer as it came is the text in it.
        assert_eq!(line["text"], "{\"text\":\"7\",\"count\":7,\"loud\":true}");
        let sent = &server.asked("tools/call")[0].message["params"];
        assert_eq!(sent["name"], "echo");
        assert_eq!(sent["arguments"], json!({ "text": "7", "count": 7, "loud": true }));

        // The line is reported when the call is made and again when it is answered; what the server said
        // meanwhile is a line of its own.
        let reported = events.messages();
        let of_call: Vec<&Value> = reported.iter().filter(|m| m["id"] == line["id"]).collect();
        assert_eq!(of_call.iter().map(|m| m["status"].as_str().unwrap()).collect::<Vec<_>>(), ["pending", "ok"]);
        let said = reported.iter().find(|m| m["type"] == "notification").expect("the log message");
        assert_eq!(said["name"], "notifications/message");
        assert_eq!(data(said)["data"], "working on it");

        // An argument that is not what the tool declares is refused before anything is sent.
        let payload = json!({ "workspaceId": WS, "connectionId": "tools", "type": "tool", "name": "echo",
                              "arguments": { "text": "a", "count": "many" } });
        let refused = call("mcp.call", payload).await;
        assert_eq!(
            refused,
            json!({ "ok": false, "error": "Argument \"count\" must be a whole number, not \"many\"." })
        );
        assert_eq!(server.asked("tools/call").len(), 1);

        // A tool that fails and a call the server rejects both end as errors, with what the server said.
        let failed = ask("tools", "tool", "fail", json!({})).await;
        assert_eq!(failed["status"], "error");
        assert_eq!(data(&failed)["content"][0]["text"], "it broke");
        let rejected = ask("tools", "tool", "boom", json!({})).await;
        assert_eq!(rejected["status"], "error");
        assert_eq!(data(&rejected)["message"], "boom");
        assert_eq!(data(&rejected)["code"], -32000);
        assert_eq!((failed["text"].as_str(), rejected["text"].as_str()), (Some("it broke"), Some("boom")));

        disconnect("tools").await;
        wait_status("tools", "disconnected").await;
    }

    #[tokio::test]
    async fn a_prompt_is_got_and_a_resource_is_read() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        connect("more", &server.url).await;
        wait_status("more", "connected").await;

        let prompt = ask("more", "prompt", "greet", json!({ "who": "Ada", "tone": "" })).await;
        assert_eq!((prompt["type"].as_str(), prompt["status"].as_str()), (Some("prompt"), Some("ok")));
        assert_eq!(data(&prompt)["messages"][0]["content"]["text"], "Hello Ada");
        assert_eq!(prompt["text"], "user: Hello Ada");
        // An argument left empty that the prompt does not require is not sent.
        assert_eq!(server.asked("prompts/get")[0].message["params"]["arguments"], json!({ "who": "Ada" }));

        let resource = ask("more", "resource", " mem://alpha ", Value::Null).await;
        assert_eq!((resource["type"].as_str(), resource["name"].as_str()), (Some("resource"), Some("mem://alpha")));
        assert_eq!(resource["arguments"], "");
        assert_eq!(data(&resource)["contents"][0]["text"], "alpha's text");
        assert_eq!(server.asked("resources/read")[0].message["params"]["uri"], "mem://alpha");

        let payload = json!({ "workspaceId": WS, "connectionId": "more", "type": "resource", "name": " " });
        assert_eq!(call("mcp.call", payload).await, json!({ "ok": false, "error": "A resource URI is required." }));
        disconnect("more").await;
        wait_status("more", "disconnected").await;
    }

    #[tokio::test]
    async fn a_changed_list_is_read_again() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("renew");
        connect("renew", &server.url).await;
        wait_status("renew", "connected").await;
        assert_eq!(server.asked("tools/list").len(), 2);

        ask("renew", "tool", "renew", json!({})).await;
        for _ in 0..200 {
            if events.statuses().len() == 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        // Both pages were read once more, and the list was announced again.
        assert_eq!(server.asked("tools/list").len(), 4);
        assert_eq!(events.statuses(), ["connecting", "connected", "connected"]);
        assert!(events.messages().iter().any(|m| m["name"] == "notifications/tools/list_changed"));
        disconnect("renew").await;
        wait_status("renew", "disconnected").await;
    }

    #[tokio::test]
    async fn a_saved_request_resolves_variables_and_sends_its_headers() {
        let root = scratch_data_root();
        let server = Server::start().await;
        write_entity(
            root.ws_id,
            "environments",
            "env1",
            &json!({ "id": "env1", "name": "Env", "variables": [
                { "key": "BASE", "value": server.url, "enabled": true },
                { "key": "TOKEN", "value": "from-env", "enabled": true },
                { "key": "N", "value": "3", "enabled": true },
            ] }),
            None,
        )
        .unwrap();
        write_entity(
            root.ws_id,
            "mcpRequests",
            "saved1",
            &json!({ "id": "saved1", "name": "Saved", "transport": "http", "url": "{{BASE}}",
                     "headers": { "Authorization": "Bearer {{TOKEN}}" }, "createdAt": 1, "workspaceId": root.ws_id }),
            None,
        )
        .unwrap();

        let result = call("mcp.connect", json!({ "workspaceId": WS, "connectionId": "saved1" })).await;
        assert_eq!(result, json!({ "ok": true }));
        let connected = wait_status("saved1", "connected").await;
        assert_eq!(connected["url"], "{{BASE}}");
        assert!(server.seen().iter().all(|seen| seen.headers["authorization"] == "Bearer from-env"));

        // What is typed for an argument is resolved before it is read as the declared type.
        let line = ask("saved1", "tool", "echo", json!({ "text": "n={{N}}", "count": "{{N}}" })).await;
        assert_eq!(server.asked("tools/call")[0].message["params"]["arguments"], json!({ "text": "n=3", "count": 3 }));
        assert_eq!(line["arguments"], "{\"text\":\"n=3\",\"count\":3}");
        disconnect("saved1").await;
        wait_status("saved1", "disconnected").await;

        // The saved request is named by `id` when the connection has another name.
        call("mcp.connect", json!({ "workspaceId": WS, "connectionId": "tab-2", "id": "saved1" })).await;
        wait_status("tab-2", "connected").await;
        disconnect("tab-2").await;
        wait_status("tab-2", "disconnected").await;
    }

    #[tokio::test]
    async fn a_missing_saved_request_and_stray_draft_fields_are_refused() {
        let _root = scratch_data_root();
        let missing = call_err("mcp.connect", json!({ "workspaceId": WS, "connectionId": "nope" })).await;
        assert_eq!(missing.code, ErrorCode::NotFound);
        for stray in [json!({ "headers": { "A": "b" } }), json!({ "transport": "http" })] {
            let mut payload = json!({ "workspaceId": WS, "connectionId": "x" });
            payload.as_object_mut().unwrap().extend(stray.as_object().unwrap().clone());
            assert_eq!(call_err("mcp.connect", payload).await.code, ErrorCode::BadRequest, "{stray}");
        }
    }

    #[tokio::test]
    async fn a_server_that_cannot_be_reached_or_is_not_one_reports_the_error() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("denied");
        connect("denied", &format!("{}/deny", server.url)).await;
        let failed = wait_status("denied", "error").await;
        assert!(failed["error"].as_str().unwrap().starts_with("The server wants credentials"), "{failed}");
        assert_eq!(events.statuses(), ["connecting", "error"]);
        let listed = call("mcp.list", json!({ "workspaceId": WS })).await;
        assert!(!listed["connections"].as_array().unwrap().iter().any(|c| c["connectionId"] == "denied"));

        connect("html", &format!("{}/html", server.url)).await;
        let failed = wait_status("html", "error").await;
        assert!(failed["error"].as_str().unwrap().ends_with("Is this an MCP server?"), "{failed}");

        connect("mute", &format!("{}/mute", server.url)).await;
        let failed = wait_status("mute", "error").await;
        assert!(failed["error"].as_str().unwrap().starts_with("Timed out waiting for the MCP handshake"), "{failed}");

        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        connect("refused", &format!("http://127.0.0.1:{port}/mcp")).await;
        let failed = wait_status("refused", "error").await;
        assert!(failed["error"].as_str().unwrap().contains("Nothing is listening at"), "{failed}");
        assert_eq!(failed["openedAt"], Value::Null);

        for (connection_id, transport, url, expected) in [
            ("bad-url", "http", "not a url", "Invalid URL"),
            ("bad-scheme", "http", "ws://127.0.0.1/x", "Invalid URL"),
            ("empty", "", "", "A command is required"),
        ] {
            let payload =
                json!({ "workspaceId": WS, "connectionId": connection_id, "url": url, "transport": transport });
            let payload = if transport.is_empty() {
                json!({ "workspaceId": WS, "connectionId": connection_id, "url": url })
            } else {
                payload
            };
            let result = call("mcp.connect", payload).await;
            assert_eq!(result["ok"], false, "{url}");
            assert!(result["error"].as_str().unwrap().starts_with(expected), "{result}");
            assert_eq!(status(connection_id).await["status"], "error");
        }
        let payload = json!({ "workspaceId": WS, "connectionId": "reserved", "url": "http://127.0.0.1/mcp",
                              "headers": { "Mcp-Session-Id": "mine" } });
        let result = call("mcp.connect", payload).await;
        assert!(result["error"].as_str().unwrap().contains("set by the MCP transport"), "{result}");
    }

    #[cfg(windows)]
    const SAY_AND_LEAVE: &str = "echo key is %API_KEY% 1>&2";
    #[cfg(not(windows))]
    const SAY_AND_LEAVE: &str = "echo key is $API_KEY 1>&2";

    #[tokio::test]
    async fn a_program_gets_the_headers_as_its_environment_and_its_last_words_explain_its_end() {
        let _root = scratch_data_root();
        let events = Events::watch("program");
        let payload = json!({ "workspaceId": WS, "connectionId": "program", "transport": "stdio",
                              "url": SAY_AND_LEAVE, "headers": { "API_KEY": "k-123" } });
        assert_eq!(call("mcp.connect", payload).await, json!({ "ok": true }));
        let failed = wait_status("program", "error").await;
        assert_eq!(failed["transport"], "stdio");
        assert_eq!(failed["error"], "The program ended before the MCP handshake finished: key is k-123");
        assert_eq!(events.statuses(), ["connecting", "error"]);
        // What the program wrote for itself is in the log.
        let said = failed["messages"].as_array().unwrap().iter().find(|m| m["type"] == "stderr").expect("its words");
        assert_eq!(said["data"], "key is k-123");

        // A command that is not one fails the same way, in the shell's words.
        connect("no-program", "bifurc-no-such-mcp-server --stdio").await;
        let failed = wait_status("no-program", "error").await;
        assert!(
            failed["error"].as_str().unwrap().starts_with("The program ended before the MCP handshake"),
            "{failed}"
        );
        assert!(failed["error"].as_str().unwrap().contains("bifurc-no-such-mcp-server"), "{failed}");
    }

    #[tokio::test]
    async fn a_call_needs_an_open_connection_and_one_left_waiting_ends_with_the_connection() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let payload = json!({ "workspaceId": WS, "connectionId": "ghost", "type": "tool", "name": "echo" });
        assert_eq!(call("mcp.call", payload).await, json!({ "ok": false, "error": "Not connected" }));

        let events = Events::watch("waiting");
        connect("waiting", &server.url).await;
        wait_status("waiting", "connected").await;
        let payload = json!({ "workspaceId": WS, "connectionId": "waiting", "type": "tool", "name": "hang" });
        let called = call("mcp.call", payload).await;
        assert_eq!(called["ok"], true);
        assert_eq!(status("waiting").await["messages"][0]["status"], "pending");

        disconnect("waiting").await;
        let closed = wait_status("waiting", "disconnected").await;
        let line = &closed["messages"][0];
        assert_eq!(line["id"], called["callId"]);
        assert_eq!(
            (line["status"].as_str(), line["text"].as_str()),
            (Some("error"), Some("The connection ended before the server answered."))
        );
        assert_eq!(events.messages().last().unwrap()["status"], "error");

        let payload = json!({ "workspaceId": WS, "connectionId": "waiting", "type": "tool", "name": "echo" });
        assert_eq!(call("mcp.call", payload).await, json!({ "ok": false, "error": "Not connected" }));
    }

    #[tokio::test]
    async fn a_reconnect_replaces_the_connection_and_keeps_the_log() {
        let _root = scratch_data_root();
        let server = Server::start().await;
        let events = Events::watch("again");
        connect("again", &server.url).await;
        wait_status("again", "connected").await;
        ask("again", "resource", "mem://alpha", Value::Null).await;

        assert_eq!(connect("again", &server.url).await, json!({ "ok": true }));
        for _ in 0..200 {
            if events.statuses().len() == 4 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        // The superseded connection's close must not report a `disconnected` after the new `connecting`.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(events.statuses(), ["connecting", "connected", "connecting", "connected"]);
        let line = ask("again", "resource", "mem://alpha", Value::Null).await;
        assert_eq!(line["id"], "again-2");
        let snapshot = status("again").await;
        assert_eq!(snapshot["messages"].as_array().unwrap().len(), 2);

        let cleared = call("mcp.clearMessages", json!({ "workspaceId": WS, "connectionId": "again" })).await;
        assert_eq!(cleared, json!({ "ok": true }));
        let snapshot = status("again").await;
        assert_eq!((snapshot["status"].as_str(), snapshot["messages"].clone()), (Some("connected"), json!([])));
        disconnect("again").await;
        wait_status("again", "disconnected").await;
    }

    #[tokio::test]
    async fn unknown_ids_and_the_connection_limit() {
        let _root = scratch_data_root();
        assert_eq!(status("ghost").await, Value::Null);
        let expected = json!({ "ok": false, "error": "No connection \"ghost\" in workspace \"ws1\"." });
        assert_eq!(disconnect("ghost").await, expected);
        assert_eq!(call("mcp.clearMessages", json!({ "workspaceId": WS, "connectionId": "ghost" })).await, expected);

        let server = Server::start().await;
        for n in 0..MAX_MCP_CONNECTIONS {
            assert_eq!(connect(&format!("lim{n}"), &server.url).await, json!({ "ok": true }));
        }
        let refused = connect("lim-extra", &server.url).await;
        assert_eq!(refused["ok"], false);
        assert!(refused["error"].as_str().unwrap().starts_with("Max 5 active connections reached."), "{refused}");
        assert_eq!(status("lim-extra").await, Value::Null);
        // Reconnecting a connection already held is not counted against the limit.
        assert_eq!(connect("lim0", &server.url).await, json!({ "ok": true }));
        // Disconnecting while still connecting ends as a clean disconnect.
        for n in 0..MAX_MCP_CONNECTIONS {
            disconnect(&format!("lim{n}")).await;
        }
        for n in 0..MAX_MCP_CONNECTIONS {
            wait_status(&format!("lim{n}"), "disconnected").await;
        }
    }
}
