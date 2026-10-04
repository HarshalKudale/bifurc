//! Output formatting: tables, JSON, and the human renderings of a few payloads.

use base64::Engine as _;
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

/// A saved or unsaved request's response as printed by `requests send`.
pub fn describe_send(result: &Value) -> String {
    let mut lines =
        vec![format!("HTTP {}  ({} ms)", js_template(result.get("status")), js_template(result.get("durationMs")))];
    if let Some(headers) = result.get("headers").and_then(Value::as_object) {
        lines.extend(headers.iter().map(|(k, v)| format!("{k}: {}", js_template(Some(v)))));
    }
    let body = result.get("body").and_then(Value::as_str).unwrap_or_default();
    let decoded =
        base64::engine::general_purpose::STANDARD_NO_PAD.decode(body.trim_end_matches('=')).unwrap_or_default();
    lines.push(String::new());
    lines.push(String::from_utf8_lossy(&decoded).into_owned());
    for (key, label) in [("preScriptError", "pre-script error"), ("postScriptError", "post-script error")] {
        if let Some(message) = result.get(key).and_then(Value::as_str).filter(|m| !m.is_empty()) {
            lines.push(String::new());
            lines.push(format!("{label}: {message}"));
        }
    }
    for test in result.get("tests").and_then(Value::as_array).into_iter().flatten() {
        let passed = test.get("passed").and_then(Value::as_bool).unwrap_or(false);
        let name = js_template(test.get("name"));
        let error = match test.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()) {
            Some(error) => format!(" — {error}"),
            None => String::new(),
        };
        lines.push(format!("{}  {name}{error}", if passed { "PASS" } else { "FAIL" }));
    }
    lines.join("\n")
}

/// A gRPC result as printed by `grpc call` and `grpc send`: the status line, the metadata, then each response.
pub fn describe_grpc(result: &Value) -> String {
    let status = result.get("status").and_then(Value::as_i64).unwrap_or(0);
    let message = result.get("statusMessage").and_then(Value::as_str).unwrap_or_default();
    let mut lines = vec![format!(
        "gRPC {}{}  ({} ms)",
        if status == 0 { "OK".to_string() } else { format!("status {status}") },
        if status == 0 || message.is_empty() { String::new() } else { format!(" {message}") },
        js_template(result.get("durationMs"))
    )];
    if let Some(metadata) = result.get("metadata").and_then(Value::as_object) {
        lines.extend(metadata.iter().map(|(k, v)| format!("{k}: {}", js_template(Some(v)))));
    }
    for response in result.get("responses").and_then(Value::as_array).into_iter().flatten() {
        lines.push(String::new());
        lines.push(js_string(response));
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

/// The four lines of `bifurc status`.
pub fn describe_status(engine: &RuntimeInfo, server: &Value, active_workspace: &Value) -> String {
    let proxy = if server.get("running").is_some_and(truthy) {
        format!("listening on {}", js_template(server.get("port")))
    } else {
        match server.get("error").filter(|e| truthy(e)) {
            Some(error) => format!("stopped ({})", js_string(error)),
            None => "stopped".to_string(),
        }
    };
    let workspace = match active_workspace {
        Value::Object(w) => format!("{} ({})", js_template(w.get("name")), js_template(w.get("id"))),
        other => js_string(other),
    };
    [
        format!("engine     pid {}, {} {}, v{}", engine.pid, engine.transport, engine.address, engine.engine_version),
        format!("data dir   {}", engine.data_dir),
        format!("proxy      {proxy}"),
        format!("workspace  {workspace}"),
    ]
    .join("\n")
}

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
    fn a_send_result_prints_status_headers_decoded_body_and_tests() {
        let result = json!({
            "status": 200,
            "durationMs": 12,
            "headers": {"content-type": "text/plain", "x-n": 1},
            "body": "aGVsbG8=",
            "tests": [{"name": "ok", "passed": true}, {"name": "bad", "passed": false, "error": "nope"}],
            "preScriptError": "pre boom",
            "postScriptError": "post boom",
        });
        assert_eq!(
            describe_send(&result),
            "HTTP 200  (12 ms)\ncontent-type: text/plain\nx-n: 1\n\nhello\n\npre-script error: pre boom\n\n\
             post-script error: post boom\nPASS  ok\nFAIL  bad — nope"
        );
    }

    #[test]
    fn an_empty_body_still_leaves_the_blank_separator() {
        let result = json!({"status": 204, "durationMs": 1, "headers": {}, "body": ""});
        assert_eq!(describe_send(&result), "HTTP 204  (1 ms)\n\n");
    }

    #[test]
    fn status_shows_engine_proxy_and_workspace() {
        let info = RuntimeInfo::from_json(json!({
            "pid": 5, "transport": "socket", "address": "addr", "dataDir": "/d", "engineVersion": "0.4.0"
        }))
        .unwrap();
        let workspace = json!({"id": "w1", "name": "Default"});
        assert_eq!(
            describe_status(&info, &json!({"running": true, "port": 8080}), &workspace),
            "engine     pid 5, socket addr, v0.4.0\ndata dir   /d\nproxy      listening on 8080\nworkspace  Default (w1)"
        );
        assert!(
            describe_status(&info, &json!({"running": false}), &json!("w9"))
                .ends_with("proxy      stopped\nworkspace  w9")
        );
        assert!(
            describe_status(&info, &json!({"running": false, "error": "EADDRINUSE"}), &workspace)
                .contains("proxy      stopped (EADDRINUSE)")
        );
    }
}
