//! `mocks serve`: the workspace's enabled mocks over plain HTTP on a local port (the engine runs the server).
//! Without `--detach` the command blocks and prints a line per request; the engine ends the server with this
//! command's connection, so Ctrl+C or a killed CI step never leaves the port busy.

use std::{
    collections::{HashMap, HashSet},
    io::Write,
};

use serde_json::{Value, json};

use crate::{
    capture,
    commands::{Context, id_of, list_entities, workspace_id},
    error::{CliError, Result},
    format::js_template,
};

const DEFAULT_PORT: u64 = 4010;

fn mocks_words(n: u64) -> String {
    format!("{n} mock{}", if n == 1 { "" } else { "s" })
}

/// The name of each mock by id.
fn mock_names(ctx: &mut Context, workspace: &str) -> Result<HashMap<String, String>> {
    let rows = list_entities(ctx, workspace, "mocks")?;
    Ok(rows.iter().map(|row| (id_of(row), js_template(row.get("name")))).collect())
}

/// `GET /orders → mock "Get orders" 200 · 2ms`, or `→ 404 no mock`, or `→ fallthrough 200 · 12ms`.
fn request_line(entry: &Value, names: &HashMap<String, String>) -> String {
    let url = js_template(entry.get("url"));
    let rest = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let path = rest.find('/').map_or("/", |at| &rest[at..]);
    let status = js_template(entry.get("status"));
    let ms = entry.get("durationMs").and_then(Value::as_i64).map_or(String::new(), |ms| format!(" · {ms}ms"));
    let head = format!("{} {path} →", js_template(entry.get("method")));
    match (entry.get("via").and_then(Value::as_str), entry.get("mockId").and_then(Value::as_str)) {
        (Some("mock"), id) => {
            let name = id.and_then(|id| names.get(id)).map_or("?", String::as_str);
            format!("{head} mock \"{name}\" {status}{ms}")
        }
        (Some("proxy"), _) => format!("{head} fallthrough {status}{ms}"),
        _ => format!("{head} {status} no mock"),
    }
}

/// `mocks serve [--port N] [--host H] [--fallthrough URL] [--detach] [--quiet]`, `--stop [--port N]`, `--status`.
pub fn serve(ctx: &mut Context, out: &mut dyn Write) -> Result<String> {
    ctx.accept(&["port", "host", "fallthrough", "detach", "stop", "status", "quiet"], 1)?;
    let args = ctx.args;
    let port = capture::number(ctx, "port")?;
    if port.is_some_and(|p| p > 65535) {
        return Err(CliError::usage("--port is a number from 0 to 65535"));
    }
    if args.has("status") {
        let answer = ctx.request("mocks.serveStatus", json!({}))?;
        return Ok(ctx.print(&answer, || {
            let rows: Vec<String> = answer["servers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| {
                    let fallthrough =
                        s["fallthrough"].as_str().map_or(String::new(), |url| format!(" · fallthrough {url}"));
                    let kind = if s["detached"] == true { "detached" } else { "foreground" };
                    format!(
                        ":{} · {kind} · {} hits{fallthrough}",
                        js_template(s.get("port")),
                        js_template(s.get("hits"))
                    )
                })
                .collect();
            if rows.is_empty() { "(no mock server running)".to_string() } else { rows.join("\n") }
        }));
    }
    if args.has("stop") || args.positional(2) == Some("stop") {
        let answer = ctx.request("mocks.serveStop", port.map_or(json!({}), |p| json!({ "port": p })))?;
        let ports: Vec<String> =
            answer["stopped"].as_array().into_iter().flatten().map(|p| format!(":{}", js_template(Some(p)))).collect();
        return Ok(ctx.print(&answer, || format!("stopped mock server {}", ports.join(", "))));
    }

    let workspace = workspace_id(ctx)?;
    let wanted = port.unwrap_or(DEFAULT_PORT);
    let mut payload = json!({ "workspaceId": workspace, "port": wanted, "detach": args.has("detach") });
    for (flag, key) in [("host", "host"), ("fallthrough", "fallthrough")] {
        if let Some(value) = args.flag(flag) {
            payload[key] = json!(value);
        }
    }
    let started = ctx.request("mocks.serve", payload).map_err(|err| match err {
        CliError::Engine { code, .. } if code == "CONFLICT" => {
            CliError::msg(format!("port {wanted} is in use (see: bifurc services list)"))
        }
        other => other,
    })?;
    let (url, count, bound) =
        (js_template(started.get("url")), started["mocks"].as_u64().unwrap_or(0), started["port"].as_u64());
    if args.has("detach") {
        return Ok(ctx.print(&started, || {
            format!(
                "serving {} on {url} (stop with: bifurc mocks serve --stop --port {})",
                mocks_words(count),
                bound.unwrap_or(wanted)
            )
        }));
    }

    let _ = writeln!(out, "serving {} on {url} (Ctrl+C to stop)", mocks_words(count));
    let _ = out.flush();
    let quiet = args.has("quiet");
    let (mut names, mut seen) = (mock_names(ctx, &workspace)?, HashSet::new());
    let suffix = format!(":{}", bound.unwrap_or(wanted));
    let since = capture::now_ms();
    if !quiet {
        ctx.request("subscribe", json!({ "events": [capture::EVENT] }))?;
    }
    // The server ends when this connection does; until then, report what it serves.
    loop {
        if !quiet {
            let listed = capture::summaries(ctx, &workspace, json!({ "origin": "server", "since": since }))?;
            for entry in listed["entries"].as_array().into_iter().flatten().rev() {
                let url = js_template(entry.get("url"));
                let authority =
                    url.split_once("://").map_or(url.as_str(), |(_, rest)| rest).split('/').next().unwrap_or_default();
                if !authority.ends_with(&suffix) || !seen.insert(js_template(entry.get("id"))) {
                    continue;
                }
                if entry["mockId"].as_str().is_some_and(|id| !names.contains_key(id)) {
                    names = mock_names(ctx, &workspace)?;
                }
                if writeln!(out, "{}", request_line(entry, &names)).and_then(|()| out.flush()).is_err() {
                    return Ok(String::new());
                }
            }
        }
        if ctx.engine()?.next_event()?.is_none() {
            return Ok(String::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run_with_events};

    fn go(argv: &[&str], answers: Vec<Answer>, events: Vec<Value>) -> (Result<String>, Vec<(String, Value)>, String) {
        let mut out = Vec::new();
        let (result, calls) = run_with_events(argv, answers, events, |ctx| serve(ctx, &mut out));
        (result, calls, String::from_utf8(out).unwrap())
    }

    fn started() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("mocks.serve", Ok(json!({"port": 4010, "url": "http://127.0.0.1:4010", "mocks": 5}))),
        ]
    }

    #[test]
    fn detach_sends_the_options_and_says_how_to_stop() {
        let argv = [
            "mocks",
            "serve",
            "--detach",
            "--port",
            "4010",
            "--host",
            "0.0.0.0",
            "--fallthrough",
            "http://localhost:3000",
        ];
        let (out, calls, printed) = go(&argv, started(), vec![]);
        assert_eq!(
            out.unwrap(),
            "serving 5 mocks on http://127.0.0.1:4010 (stop with: bifurc mocks serve --stop --port 4010)"
        );
        assert_eq!(
            payloads(&calls, "mocks.serve"),
            [
                json!({"workspaceId": "w1", "port": 4010, "detach": true, "host": "0.0.0.0", "fallthrough": "http://localhost:3000"})
            ]
        );
        assert_eq!(printed, "");
    }

    #[test]
    fn the_default_port_is_4010_and_a_busy_port_is_an_environment_problem() {
        let busy = Err(CliError::Engine { code: "CONFLICT".into(), message: "Port 4010 is already in use".into() });
        let (out, calls, _) =
            go(&["mocks", "serve"], vec![("config.get", Ok(config())), ("mocks.serve", busy)], vec![]);
        let err = out.unwrap_err();
        assert_eq!(err.line(), "bifurc: port 4010 is in use (see: bifurc services list)");
        assert_eq!(err.code(), 1);
        assert_eq!(payloads(&calls, "mocks.serve")[0]["port"], 4010);
        assert_eq!(payloads(&calls, "mocks.serve")[0]["detach"], false);
    }

    #[test]
    fn stop_and_status() {
        let stopped = vec![("mocks.serveStop", Ok(json!({"stopped": [4010]})))];
        let (out, calls, _) = go(&["mocks", "serve", "--stop", "--port", "4010"], stopped, vec![]);
        assert_eq!(out.unwrap(), "stopped mock server :4010");
        assert_eq!(payloads(&calls, "mocks.serveStop"), [json!({"port": 4010})]);

        let missing = Err(CliError::Engine { code: "NOT_FOUND".into(), message: "no mock server is running".into() });
        let (out, calls, _) = go(&["mocks", "serve", "--stop"], vec![("mocks.serveStop", missing)], vec![]);
        assert_eq!(out.unwrap_err().code(), 3);
        assert_eq!(payloads(&calls, "mocks.serveStop"), [json!({})]);

        let listing = json!({"servers": [{"port": 4010, "workspaceId": "w1", "fallthrough": "http://x", "detached": true, "hits": 3}]});
        let (out, _, _) = go(&["mocks", "serve", "--status"], vec![("mocks.serveStatus", Ok(listing))], vec![]);
        assert_eq!(out.unwrap(), ":4010 · detached · 3 hits · fallthrough http://x");
        let (out, _, _) =
            go(&["mocks", "serve", "--status"], vec![("mocks.serveStatus", Ok(json!({"servers": []})))], vec![]);
        assert_eq!(out.unwrap(), "(no mock server running)");
    }

    fn entry(id: &str, path: &str, status: u64, via: &str, mock: Option<&str>) -> Value {
        let mut entry = json!({"id": id, "method": "GET", "url": format!("http://127.0.0.1:4010{path}"), "status": status, "durationMs": 2, "via": via});
        if let Some(mock) = mock {
            entry["mockId"] = json!(mock);
        }
        entry
    }

    #[test]
    fn the_foreground_prints_the_start_line_then_a_line_per_request_until_the_connection_ends() {
        let mut answers = started();
        answers.push(("entity.list:mocks", Ok(json!({"entities": [{"id": "m1", "name": "Get orders"}]}))));
        answers.push(("subscribe", Ok(json!({}))));
        let listed = json!({"entries": [
            entry("b", "/x", 404, "error", None),
            entry("a", "/orders", 200, "mock", Some("m1")),
            entry("c", "/real", 200, "proxy", None),
            json!({"id": "d", "method": "GET", "url": "http://127.0.0.1:9999/other", "status": 200, "via": "mock"}),
        ]});
        answers.push(("capture.list", Ok(listed)));
        let (out, calls, printed) = go(&["mocks", "serve"], answers, vec![json!({"event": "event.log.entry"})]);
        assert_eq!(out.unwrap(), "");
        assert_eq!(
            printed,
            "serving 5 mocks on http://127.0.0.1:4010 (Ctrl+C to stop)\n\
             GET /real → fallthrough 200 · 2ms\n\
             GET /orders → mock \"Get orders\" 200 · 2ms\n\
             GET /x → 404 no mock\n"
        );
        assert_eq!(payloads(&calls, "mocks.serve")[0]["detach"], false);
        let listing = &payloads(&calls, "capture.list")[0];
        assert_eq!((&listing["origin"], &listing["workspaceId"]), (&json!("server"), &json!("w1")));
    }

    #[test]
    fn quiet_prints_only_the_start_line() {
        let (out, calls, printed) = go(
            &["mocks", "serve", "--quiet"],
            {
                let mut answers = started();
                answers.push(("entity.list:mocks", Ok(json!({"entities": []}))));
                answers
            },
            vec![json!({"event": "event.log.entry"})],
        );
        assert_eq!(out.unwrap(), "");
        assert_eq!(printed, "serving 5 mocks on http://127.0.0.1:4010 (Ctrl+C to stop)\n");
        assert!(payloads(&calls, "capture.list").is_empty() && payloads(&calls, "subscribe").is_empty());
    }
}
