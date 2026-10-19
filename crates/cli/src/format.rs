//! Output formatting: tables, JSON, and the human renderings of a few payloads.

use serde_json::Value;

use crate::discovery::RuntimeInfo;

/// How a JavaScript template literal would print `value`; absent is `undefined`.
pub fn js_template(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(other) => js_string(other),
    }
}

/// `String(value)` as JavaScript does it, with `null` as the empty string of a table cell.
pub fn js_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 1e21 => format!("{f:.0}"),
            _ => n.to_string(),
        },
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// Pretty JSON with two-space indentation.
pub fn json(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "null".to_string())
}

/// Width in the units JavaScript's `padEnd` counts.
fn width(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Rows as aligned columns under upper-case headings; `(none)` for no rows.
pub fn table(rows: &[Value], columns: &[&str]) -> String {
    if rows.is_empty() {
        return "(none)".to_string();
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| columns.iter().map(|c| row.get(*c).map(js_string).unwrap_or_default()).collect())
        .collect();
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, heading)| cells.iter().map(|row| width(&row[i])).chain([width(heading)]).max().unwrap_or(0))
        .collect();
    let line = |values: Vec<&str>| {
        let padded: Vec<String> = values
            .iter()
            .enumerate()
            .map(|(i, v)| format!("{v}{}", " ".repeat(widths[i].saturating_sub(width(v)))))
            .collect();
        padded.join("  ").trim_end().to_string()
    };
    let headings: Vec<String> = columns.iter().map(|c| c.to_uppercase()).collect();
    let mut lines = vec![line(headings.iter().map(String::as_str).collect())];
    lines.extend(cells.iter().map(|row| line(row.iter().map(String::as_str).collect())));
    lines.join("\n")
}

/// Rows of cells as columns padded to the widest cell, two spaces apart, with no heading.
pub fn aligned(cells: &[Vec<String>]) -> String {
    let columns = cells.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> =
        (0..columns).map(|i| cells.iter().filter_map(|row| row.get(i)).map(|c| width(c)).max().unwrap_or(0)).collect();
    let lines: Vec<String> = cells
        .iter()
        .map(|row| {
            let padded: Vec<String> =
                row.iter().enumerate().map(|(i, c)| format!("{c}{}", " ".repeat(widths[i] - width(c)))).collect();
            padded.join("  ").trim_end().to_string()
        })
        .collect();
    lines.join("\n")
}

/// A WebSocket connection as printed by `requests send`: its state, then what was sent (`>`) and received (`<`).
pub fn describe_socket(snapshot: &Value) -> String {
    let mut lines =
        vec![format!("WebSocket {}  {}", js_template(snapshot.get("status")), js_template(snapshot.get("url")))];
    if let Some(error) = snapshot.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        lines.push(format!("error: {error}"));
    }
    for message in snapshot.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let mark = if message.get("direction").and_then(Value::as_str) == Some("sent") { ">" } else { "<" };
        lines.push(format!("{mark} {}", js_template(message.get("data"))));
    }
    lines.join("\n")
}

/// A Socket.IO connection as printed by `requests send`: its state and namespace, then each event emitted (`>`)
/// and received (`<`) with its arguments. `#n` is the id an acknowledgement was asked with, or answers.
pub fn describe_socketio(snapshot: &Value) -> String {
    let mut head = format!("Socket.IO {}  {}", js_template(snapshot.get("status")), js_template(snapshot.get("url")));
    if let Some(namespace) = snapshot.get("namespace").and_then(Value::as_str).filter(|ns| !ns.is_empty() && *ns != "/")
    {
        head.push_str(&format!("  {namespace}"));
    }
    let mut lines = vec![head];
    if let Some(error) = snapshot.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        lines.push(format!("error: {error}"));
    }
    for message in snapshot.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let mark = if message.get("direction").and_then(Value::as_str) == Some("sent") { ">" } else { "<" };
        let ack = message.get("ackId").and_then(Value::as_u64);
        let label = match (message.get("type").and_then(Value::as_str), ack) {
            (Some("ack"), Some(id)) => format!("ack #{id}"),
            (Some("ack"), None) => "ack".to_string(),
            (_, Some(id)) => format!("{} #{id}", js_template(message.get("event"))),
            (_, None) => js_template(message.get("event")),
        };
        lines.push(format!("{mark} [{label}] {}", js_template(message.get("data"))));
    }
    lines.join("\n")
}

/// The first line of an MCP connection as printed by `requests send`, with its error if it has one.
fn mcp_head(snapshot: &Value) -> Vec<String> {
    let head = format!(
        "MCP {}  {}  {}",
        js_template(snapshot.get("status")),
        js_template(snapshot.get("transport")),
        js_template(snapshot.get("url"))
    );
    let mut lines = vec![head];
    if let Some(error) = snapshot.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        lines.push(format!("error: {error}"));
    }
    lines
}

/// An MCP connection as printed by `requests send`: its state, the server, then what the server declares.
/// A tool or a prompt is shown with its arguments, the required ones starred.
pub fn describe_mcp(snapshot: &Value) -> String {
    let mut lines = mcp_head(snapshot);
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    if let Some(server) = snapshot.get("serverInfo").filter(|server| server.is_object()) {
        let version = snapshot.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
        let name = format!("{} {}", text(server, "name"), text(server, "version"));
        lines.push(format!("server: {} (protocol {version})", name.trim()));
    }
    let listed = |key: &str| snapshot.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let described = |signature: String, item: &Value| match text(item, "description").lines().next() {
        Some(description) if !description.trim().is_empty() => format!("  {signature}  {}", description.trim()),
        _ => format!("  {signature}"),
    };
    let starred = |name: &str, required: bool| if required { format!("{name}*") } else { name.to_string() };

    let tools = listed("tools");
    if !tools.is_empty() {
        lines.push("tools".to_string());
    }
    for tool in &tools {
        let schema = tool.get("inputSchema");
        let required: Vec<&str> = schema
            .and_then(|schema| schema.get("required")?.as_array())
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let arguments: Vec<String> = schema
            .and_then(|schema| schema.get("properties")?.as_object())
            .map(|properties| properties.keys().map(|name| starred(name, required.contains(&name.as_str()))).collect())
            .unwrap_or_default();
        lines.push(described(format!("{}({})", text(tool, "name"), arguments.join(", ")), tool));
    }
    let prompts = listed("prompts");
    if !prompts.is_empty() {
        lines.push("prompts".to_string());
    }
    for prompt in &prompts {
        let arguments: Vec<String> = prompt
            .get("arguments")
            .and_then(Value::as_array)
            .map(|arguments| {
                arguments.iter().map(|a| starred(&text(a, "name"), a.get("required").is_some_and(truthy))).collect()
            })
            .unwrap_or_default();
        lines.push(described(format!("{}({})", text(prompt, "name"), arguments.join(", ")), prompt));
    }
    for (key, heading, address) in
        [("resources", "resources", "uri"), ("resourceTemplates", "resource templates", "uriTemplate")]
    {
        let resources = listed(key);
        if !resources.is_empty() {
            lines.push(heading.to_string());
        }
        for resource in &resources {
            lines.push(format!("  {}  {}", text(resource, address), text(resource, "name")).trim_end().to_string());
        }
    }
    lines.join("\n")
}

/// One call over an MCP connection as printed by `requests send`: the connection's state, what was asked
/// (`>`), how it ended (`<`) and the text of the answer.
pub fn describe_mcp_call(snapshot: &Value, line: &Value) -> String {
    let mut lines = mcp_head(snapshot);
    let text = |key: &str| line.get(key).and_then(Value::as_str).unwrap_or_default();
    lines.push(format!("> {} {} {}", text("type"), text("name"), text("arguments")).trim_end().to_string());
    match line.get("durationMs").and_then(Value::as_u64) {
        Some(took) => lines.push(format!("< {}  ({took} ms)", text("status"))),
        None => lines.push(format!("< {}", text("status"))),
    }
    // The text of the answer is the engine's reading of it; an engine that gives none gave the answer.
    let answer = if text("text").is_empty() { text("data") } else { text("text") };
    if !answer.is_empty() {
        lines.push(answer.to_string());
    }
    lines.join("\n")
}

/// An SSE stream as printed by `requests send`: its state, then each event it delivered (`<`), named in
/// brackets unless it is a plain `message`.
pub fn describe_stream(snapshot: &Value) -> String {
    let mut lines = vec![format!("SSE {}  {}", js_template(snapshot.get("status")), js_template(snapshot.get("url")))];
    if let Some(error) = snapshot.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
        lines.push(format!("error: {error}"));
    }
    for message in snapshot.get("messages").and_then(Value::as_array).into_iter().flatten() {
        let name = message.get("event").and_then(Value::as_str).filter(|name| !name.is_empty() && *name != "message");
        let data = js_template(message.get("data")).replace('\n', "\n  ");
        lines.push(match name {
            Some(name) => format!("< [{name}] {data}"),
            None => format!("< {data}"),
        });
    }
    lines.join("\n")
}

/// The services and methods a server reflects, as `grpc reflect` prints them.
pub fn describe_reflection(result: &Value) -> String {
    let mut lines = Vec::new();
    for service in result.get("services").and_then(Value::as_array).into_iter().flatten() {
        lines.push(js_string(service.get("name").unwrap_or(&Value::Null)));
        for method in service.get("methods").and_then(Value::as_array).into_iter().flatten() {
            let streams = |key: &str| method.get(key).and_then(Value::as_bool).unwrap_or(false);
            let kind = match (streams("clientStreaming"), streams("serverStreaming")) {
                (false, false) => "unary",
                (false, true) => "server streaming",
                (true, false) => "client streaming",
                (true, true) => "bidirectional streaming",
            };
            lines.push(format!(
                "  {}({}) -> {}  [{kind}]",
                js_string(method.get("name").unwrap_or(&Value::Null)),
                js_string(method.get("inputType").unwrap_or(&Value::Null)),
                js_string(method.get("outputType").unwrap_or(&Value::Null)),
            ));
        }
    }
    lines.join("\n")
}

/// The answers `bifurc status` gathers; one that could not be had is `None` and its part is left out.
pub struct StatusParts {
    pub server: Option<Value>,
    pub config: Option<Value>,
    pub capture: Option<Value>,
    pub cookies: Option<Value>,
    /// `mocks.serveStatus`: the local mock servers that run.
    pub mock_servers: Option<Value>,
    /// `mail.status`: the local mail sink.
    pub mail: Option<Value>,
    /// `webhook.list`: the webhook receivers that run.
    pub webhooks: Option<Value>,
    /// `cassette.recordStatus`: the cassette being recorded, if any.
    pub recording: Option<Value>,
    /// `cassette.replayStatus`: the cassette being replayed, if any.
    pub replaying: Option<Value>,
    /// `approvals.list`: the calls a policy holds for a person.
    pub approvals: Option<Value>,
    /// `fault.list`: the fault rules in force.
    pub faults: Option<Value>,
}

impl StatusParts {
    /// How many held calls wait for a person (`None` when none do).
    pub fn approvals_pending(&self) -> Option<usize> {
        let n = self.approvals.as_ref()?.get("pending")?.as_array()?.len();
        (n > 0).then_some(n)
    }

    /// The name of the environment in force (`none` when no one is), from the engine's own choice.
    pub fn environment(&self) -> Option<String> {
        let config = self.config.as_ref()?;
        let id = config.get("activeEnvironmentId").filter(|id| !id.is_null());
        let found = config.get("environments").and_then(Value::as_array).and_then(|all| {
            all.iter().find(|e| e.get("id") == id && e.get("id").and_then(Value::as_str) != Some("__global__"))
        });
        Some(found.map_or("none".to_string(), |e| js_template(e.get("name"))))
    }

    /// How many mocks are enabled.
    pub fn mocks(&self) -> Option<usize> {
        let all = self.config.as_ref()?.get("mocks")?.as_array()?;
        Some(all.iter().filter(|m| m.get("enabled").is_some_and(truthy)).count())
    }

    /// How many domains hold cookies.
    pub fn cookies_domains(&self) -> Option<usize> {
        Some(self.cookies.as_ref()?.get("domains")?.as_array()?.len())
    }
}

/// The two lines of `bifurc status`.
pub fn describe_status(engine: &RuntimeInfo, active_workspace: &Value, parts: &StatusParts) -> String {
    let mut first = vec![format!("engine {}", engine.engine_version)];
    if let Some(server) = &parts.server {
        let port = parts.config.as_ref().and_then(|c| c.get("forwardProxyPort")).or(server.get("port"));
        let state = if server.get("running").is_some_and(truthy) { "on" } else { "off" };
        first.push(format!("proxy :{} {state}", js_template(port)));
    }
    if parts.config.is_some() {
        first.push(format!(
            "workspace {}",
            match active_workspace {
                Value::Object(w) => js_template(w.get("name")),
                other => js_string(other),
            }
        ));
    }
    first.extend(parts.environment().map(|name| format!("env {name}")));
    let mut second = Vec::new();
    second.extend(parts.approvals_pending().map(|n| format!("approvals {n} pending")));
    second.extend(parts.faults.as_ref().and_then(crate::faults::brief));
    second.extend(parts.mocks().map(|n| format!("mocks {n} on")));
    let ports: Vec<String> = parts
        .mock_servers
        .iter()
        .flat_map(|s| s["servers"].as_array())
        .flatten()
        .map(|s| format!(":{}", js_template(s.get("port"))))
        .collect();
    if !ports.is_empty() {
        second.push(format!("mock server {}", ports.join(",")));
    }
    if let Some(mail) = &parts.mail {
        let port = js_template(mail.get("port"));
        second.push(match (mail.get("running").is_some_and(truthy), mail.get("error").and_then(Value::as_str)) {
            (true, _) => format!("mail :{port} ({})", js_template(mail.get("count"))),
            (false, Some(reason)) => format!("mail :{port} off ({reason})"),
            (false, None) => format!("mail :{port} off"),
        });
    }
    let hooks: Vec<String> = parts
        .webhooks
        .iter()
        .flat_map(|w| w["listeners"].as_array())
        .flatten()
        .map(|l| format!(":{}", js_template(l.get("port"))))
        .collect();
    if !hooks.is_empty() {
        second.push(format!("webhooks {}", hooks.join(",")));
    }
    if let Some(rec) = parts.recording.as_ref().and_then(|r| r.get("recording")).filter(|r| r.is_object()) {
        second.push(format!("recording {}", crate::record::summary(rec)));
    }
    if let Some(rep) = parts.replaying.as_ref().and_then(|r| r.get("replaying")).filter(|r| r.is_object()) {
        second.push(format!("replaying {}", crate::replay::progress(rep)));
    }
    if let Some(capture) = &parts.capture {
        let limit = capture.get("limit").and_then(Value::as_u64).unwrap_or(MAX_CAPTURE);
        let paused = if capture.get("paused").is_some_and(truthy) { " paused" } else { "" };
        second.push(format!("capture {}/{}{paused}", js_template(capture.get("matched")), limit));
    }
    second.extend(parts.cookies_domains().map(|n| format!("cookies {n} domain{}", if n == 1 { "" } else { "s" })));
    [first, second].iter().filter(|line| !line.is_empty()).map(|line| line.join(" · ")).collect::<Vec<_>>().join(
        "
",
    )
}

/// What the engine keeps of captured traffic, per workspace, when it does not say.
const MAX_CAPTURE: u64 = 200;

/// JavaScript truthiness.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_table_aligns_columns_and_trims_line_ends() {
        let rows = vec![
            json!({"active": "*", "id": "w1", "name": "Default"}),
            json!({"active": "", "id": "workspace-2", "name": "B"}),
        ];
        assert_eq!(
            table(&rows, &["active", "id", "name"]),
            "ACTIVE  ID           NAME\n*       w1           Default\n        workspace-2  B"
        );
    }

    #[test]
    fn a_table_without_rows_is_none() {
        assert_eq!(table(&[], &["id"]), "(none)");
    }

    #[test]
    fn missing_and_null_cells_are_empty_and_values_are_stringified() {
        let rows = vec![json!({"port": 3000, "pid": null, "processName": "node", "ok": true})];
        assert_eq!(
            table(&rows, &["port", "pid", "processName", "address"]),
            "PORT  PID  PROCESSNAME  ADDRESS\n3000       node"
        );
        assert_eq!(table(&rows, &["ok"]), "OK\ntrue");
    }

    #[test]
    fn columns_are_as_wide_as_their_widest_cell_in_utf16_units() {
        let rows = vec![json!({"a": "héllo", "b": "x"}), json!({"a": "😀", "b": "y"})];
        assert_eq!(table(&rows, &["a", "b"]), "A      B\nhéllo  x\n😀     y");
    }

    #[test]
    fn javascript_string_conversion() {
        assert_eq!(js_string(&json!(1.0)), "1");
        assert_eq!(js_string(&json!(1.5)), "1.5");
        assert_eq!(js_string(&json!(42)), "42");
        assert_eq!(js_string(&json!([1, null, "a"])), "1,,a");
        assert_eq!(js_string(&json!({"a": 1})), "[object Object]");
        assert_eq!(js_template(None), "undefined");
        assert_eq!(js_template(Some(&Value::Null)), "null");
    }

    #[test]
    fn json_is_two_space_pretty() {
        assert_eq!(json(&json!({"a": [1, 2], "b": {}})), "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": {}\n}");
        assert_eq!(json(&json!([])), "[]");
    }

    #[test]
    fn a_socket_prints_its_state_and_its_messages_by_direction() {
        let snapshot = json!({"status": "connected", "url": "ws://h/", "error": null, "messages": [
            {"direction": "sent", "data": "ping"}, {"direction": "received", "data": "pong"}]});
        assert_eq!(describe_socket(&snapshot), "WebSocket connected  ws://h/\n> ping\n< pong");
        let failed = json!({"status": "error", "url": "ws://h/", "error": "refused", "messages": []});
        assert_eq!(describe_socket(&failed), "WebSocket error  ws://h/\nerror: refused");
    }

    #[test]
    fn a_socketio_connection_prints_its_namespace_and_each_event_with_its_arguments() {
        let snapshot = json!({"status": "connected", "url": "http://h", "namespace": "/chat", "error": null, "messages": [
            {"direction": "sent", "type": "event", "event": "say", "data": "[\"hi\"]", "ackId": 0},
            {"direction": "received", "type": "ack", "event": "say", "data": "[\"ok\"]", "ackId": 0},
            {"direction": "received", "type": "event", "event": "said", "data": "[\"hi\",1]", "ackId": null}]});
        assert_eq!(
            describe_socketio(&snapshot),
            "Socket.IO connected  http://h  /chat\n> [say #0] [\"hi\"]\n< [ack #0] [\"ok\"]\n< [said] [\"hi\",1]"
        );
        let failed =
            json!({"status": "error", "url": "http://h", "namespace": "/", "error": "Not authorized", "messages": []});
        assert_eq!(describe_socketio(&failed), "Socket.IO error  http://h\nerror: Not authorized");
    }

    #[test]
    fn an_mcp_connection_prints_what_the_server_declares() {
        let snapshot = json!({"status": "connected", "transport": "http", "url": "https://h/mcp", "error": null,
            "serverInfo": {"name": "everything", "version": "2.0"}, "protocolVersion": "2025-06-18",
            "tools": [
                {"name": "echo", "description": "Echoes.\nSecond line.",
                 "inputSchema": {"properties": {"message": {}, "loud": {}}, "required": ["message"]}},
                {"name": "now", "inputSchema": {"type": "object"}}],
            "prompts": [{"name": "greet", "arguments": [{"name": "who", "required": true}, {"name": "tone"}]}],
            "resources": [{"uri": "mem://a", "name": "alpha"}],
            "resourceTemplates": [{"uriTemplate": "mem://{id}", "name": "by id"}]});
        assert_eq!(
            describe_mcp(&snapshot),
            "MCP connected  http  https://h/mcp\nserver: everything 2.0 (protocol 2025-06-18)\n\
             tools\n  echo(message*, loud)  Echoes.\n  now()\nprompts\n  greet(who*, tone)\n\
             resources\n  mem://a  alpha\nresource templates\n  mem://{id}  by id"
        );
        let failed = json!({"status": "error", "transport": "stdio", "url": "npx x", "error": "it ended",
                            "serverInfo": null, "tools": []});
        assert_eq!(describe_mcp(&failed), "MCP error  stdio  npx x\nerror: it ended");
    }

    #[test]
    fn an_mcp_call_prints_what_was_asked_and_the_text_of_the_answer() {
        let snapshot = json!({"status": "connected", "transport": "stdio", "url": "npx x", "error": null});
        let head = "MCP connected  stdio  npx x";
        let line = |kind: &str, name: &str, arguments: &str, status: &str, text: &str| {
            json!({"type": kind, "name": name, "arguments": arguments, "status": status, "durationMs": 5,
                   "data": "{\"as\":\"it came\"}", "text": text})
        };
        let tool = line(
            "tool",
            "echo",
            "{\"a\":1}",
            "ok",
            "one
two",
        );
        assert_eq!(
            describe_mcp_call(&snapshot, &tool),
            format!(
                "{head}
> tool echo {{\"a\":1}}
< ok  (5 ms)
one
two"
            )
        );
        let resource = line("resource", "mem://a", "", "error", "boom");
        assert_eq!(
            describe_mcp_call(&snapshot, &resource),
            format!(
                "{head}
> resource mem://a
< error  (5 ms)
boom"
            )
        );
        // A call that is not answered yet has no time and no text.
        let pending = json!({"type": "tool", "name": "slow", "arguments": "{}", "status": "pending", "durationMs": null,
                             "data": "", "text": ""});
        assert_eq!(
            describe_mcp_call(&snapshot, &pending),
            format!(
                "{head}
> tool slow {{}}
< pending"
            )
        );
    }

    #[test]
    fn a_stream_prints_its_state_and_its_events_by_name() {
        let snapshot = json!({"status": "connected", "url": "http://h/e", "error": null, "messages": [
            {"event": "message", "data": "hi"}, {"event": "tick", "data": "1"}, {"event": "message", "data": "a\nb"}]});
        assert_eq!(describe_stream(&snapshot), "SSE connected  http://h/e\n< hi\n< [tick] 1\n< a\n  b");
        let failed = json!({"status": "error", "url": "http://h/e", "error": "HTTP 404 Not Found", "messages": []});
        assert_eq!(describe_stream(&failed), "SSE error  http://h/e\nerror: HTTP 404 Not Found");
    }

    fn none_again() -> StatusParts {
        StatusParts {
            server: None,
            config: None,
            capture: None,
            cookies: None,
            mock_servers: None,
            mail: None,
            webhooks: None,
            recording: None,
            replaying: None,
            approvals: None,
            faults: None,
        }
    }

    #[test]
    fn status_is_two_lines_and_leaves_out_what_could_not_be_had() {
        let info = RuntimeInfo::from_json(json!({
            "pid": 5, "transport": "socket", "address": "addr", "dataDir": "/d", "engineVersion": "0.6.0"
        }))
        .unwrap();
        let workspace = json!({"id": "w1", "name": "smoke"});
        let config = json!({"forwardProxyPort": 8080, "activeEnvironmentId": "e1", "environments": [
            {"id": "__global__", "name": "Global"}, {"id": "e1", "name": "dev"}],
            "mocks": [{"enabled": true}, {"enabled": false}, {"enabled": true}]});
        let parts = StatusParts {
            server: Some(json!({"running": true, "port": 1})),
            config: Some(config),
            capture: Some(json!({"matched": 143, "paused": false})),
            cookies: Some(json!({"domains": [{}, {}, {}]})),
            mock_servers: Some(json!({"servers": [{"port": 4010}]})),
            mail: Some(json!({"port": 1025, "running": true, "count": 3})),
            webhooks: Some(json!({"listeners": [{"port": 4020}]})),
            recording: None,
            replaying: None,
            approvals: None,
            faults: None,
        };
        assert_eq!(
            describe_status(&info, &workspace, &parts),
            "engine 0.6.0 · proxy :8080 on · workspace smoke · env dev
mocks 2 on · mock server :4010 · mail :1025 (3) · webhooks :4020 · capture 143/200 · cookies 3 domains"
        );
        let parts = StatusParts {
            server: Some(json!({"running": false, "port": 9})),
            config: None,
            capture: Some(json!({"matched": 0, "paused": true})),
            cookies: None,
            mock_servers: None,
            mail: Some(json!({"port": 1025, "running": false, "error": "Port 1025 is already in use", "count": 0})),
            webhooks: None,
            recording: None,
            replaying: None,
            approvals: None,
            faults: None,
        };
        assert_eq!(
            describe_status(&info, &json!(null), &parts),
            "engine 0.6.0 · proxy :9 off
mail :1025 off (Port 1025 is already in use) · capture 0/200 paused"
        );
        let none = StatusParts {
            server: None,
            config: Some(json!({"environments": []})),
            capture: None,
            cookies: None,
            mock_servers: None,
            mail: None,
            webhooks: None,
            recording: None,
            replaying: None,
            approvals: None,
            faults: None,
        };
        assert_eq!(describe_status(&info, &workspace, &none), "engine 0.6.0 · workspace smoke · env none");
        let recording = StatusParts {
            recording: Some(json!({"recording": {"name": "checkout", "interactions": 1, "hosts": ["a.com", "b.com"]}})),
            replaying: Some(json!({"replaying": {"name": "checkout", "served": 3, "unmatchedCount": 1}})),
            ..none
        };
        let held = StatusParts { approvals: Some(json!({"pending": [{}, {}]})), ..none_again() };
        assert!(describe_status(&info, &workspace, &held).ends_with("approvals 2 pending"));
        let faulty =
            StatusParts { faults: Some(json!({"faults": [{"target": "a.test", "reset": true}]})), ..none_again() };
        assert!(describe_status(&info, &workspace, &faulty).ends_with("faults 1 (a.test reset)"));
        assert!(describe_status(&info, &workspace, &recording).ends_with(
            "recording checkout · 1 interaction · a.com,b.com · replaying checkout · 3 served · 1 unmatched"
        ));
    }
}
