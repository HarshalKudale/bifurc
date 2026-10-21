//! `requests send` for an MCP request: connect to the server (unless the engine already holds the connection),
//! then either print what the server declares, or call a tool (`--tool`), get a prompt (`--prompt`) or read a
//! resource (`--resource`) and print the answer. What was connected here is disconnected.
//!
//! `--arg name=value` gives an argument of the tool or the prompt; the engine reads the value as the type the
//! tool declares for it. `--wait` is how many seconds the answer is waited for.

use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};

use crate::{
    commands::{Context, refused},
    error::{CliError, Result},
    format::{describe_mcp, describe_mcp_call},
    requests::listen_seconds,
};

/// How long an answer is waited for when `--wait` does not say.
const DEFAULT_WAIT: f64 = 60.0;

/// How long a server is given to start and shake hands. The engine gives up sooner on an endpoint, and a
/// program may first have to be downloaded.
const OPEN_TIMEOUT: Duration = Duration::from_secs(125);

/// How often the engine is asked for the connection's state.
const POLL: Duration = Duration::from_millis(100);

/// The options that say what to ask the server for; one at most.
pub(crate) const ASKS: [&str; 3] = ["tool", "prompt", "resource"];

fn status_of(snapshot: &Value) -> &str {
    snapshot.get("status").and_then(Value::as_str).unwrap_or_default()
}

/// What `--tool`, `--prompt` or `--resource` asks for: the kind and the name.
fn asked(ctx: &Context) -> Result<Option<(&'static str, String)>> {
    let given: Vec<&'static str> = ASKS.into_iter().filter(|option| ctx.args.has(option)).collect();
    match given.as_slice() {
        [] if ctx.args.has("arg") => Err(CliError::usage("--arg goes with --tool <name> or --prompt <name>")),
        [] => Ok(None),
        [kind] => {
            if *kind == "resource" && ctx.args.has("arg") {
                return Err(CliError::usage("--arg goes with --tool <name> or --prompt <name>"));
            }
            Ok(Some((kind, ctx.args.flag(kind).unwrap_or_default().to_string())))
        }
        _ => Err(CliError::usage("give one of --tool, --prompt and --resource")),
    }
}

/// `--arg name=value`, repeated, as the arguments the engine takes: every value as it was typed.
fn arguments(ctx: &Context) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for raw in ctx.args.flags("arg") {
        match raw.split_once('=') {
            Some((name, value)) if !name.trim().is_empty() => out.insert(name.trim().to_string(), json!(value)),
            _ => return Err(CliError::usage(format!("argument \"{raw}\" must be \"name=value\""))),
        };
    }
    Ok(out)
}

/// The log line `call_id` of a connection, if it is still there.
fn line_of(snapshot: &Value, call_id: &str) -> Option<Value> {
    let messages = snapshot.get("messages").and_then(Value::as_array)?;
    messages.iter().find(|message| message.get("id").and_then(Value::as_str) == Some(call_id)).cloned()
}

/// One exchange over an MCP connection named `connection_id`: `target` is the request to open when it is not
/// saved (`url`, `transport`, `headers`); a saved request is opened by its id.
pub fn mcp_exchange(ctx: &mut Context, workspace: &str, connection_id: &str, target: Option<&Value>) -> Result<String> {
    let wait = listen_seconds(ctx.args, DEFAULT_WAIT)?;
    let ask = asked(ctx)?;
    let typed = arguments(ctx)?;

    let key = json!({ "workspaceId": workspace, "connectionId": connection_id });
    let mut snapshot = ctx.request("mcp.status", key.clone())?;
    let held = status_of(&snapshot) == "connected";
    if !held {
        let mut connect = key.clone();
        if let Some(target) = target {
            connect["url"] = target.get("url").cloned().unwrap_or_else(|| json!(""));
            if let Some(transport) = target.get("transport").and_then(Value::as_str).filter(|t| !t.is_empty()) {
                connect["transport"] = json!(transport);
            }
            connect["headers"] = target.get("headers").cloned().unwrap_or_else(|| json!({}));
        }
        refused(ctx.request("mcp.connect", connect)?)?;
        let started = Instant::now();
        loop {
            snapshot = ctx.request("mcp.status", key.clone())?;
            if status_of(&snapshot) != "connecting" || started.elapsed() > OPEN_TIMEOUT {
                break;
            }
            sleep(POLL);
        }
    }

    // Something is only asked of a connection that opened; one that did not is the failure to report.
    let mut failure = None;
    let mut line = None;
    if let (Some((kind, name)), "connected") = (&ask, status_of(&snapshot)) {
        let mut call = key.clone();
        call["type"] = json!(kind);
        call["name"] = json!(name);
        if *kind != "resource" {
            call["arguments"] = Value::Object(typed);
        }
        let answer = ctx.request("mcp.call", call)?;
        let call_id = answer.get("callId").and_then(Value::as_str).map(str::to_string);
        match (refused(answer), call_id) {
            (Err(error), _) => failure = Some(error),
            (Ok(_), None) => failure = Some(CliError::msg("the engine did not say which call it made")),
            (Ok(_), Some(call_id)) => {
                let asked_at = Instant::now();
                let budget = Duration::from_secs_f64(wait);
                loop {
                    snapshot = ctx.request("mcp.status", key.clone())?;
                    line = line_of(&snapshot, &call_id);
                    let pending = line.as_ref().is_some_and(|line| line.get("status") == Some(&json!("pending")));
                    if !pending || status_of(&snapshot) != "connected" || asked_at.elapsed() >= budget {
                        break;
                    }
                    sleep(POLL.min(budget.saturating_sub(asked_at.elapsed())));
                }
            }
        }
    }
    if !held {
        ctx.request("mcp.disconnect", key)?;
    }

    // The log of this exchange is the call it made; what the engine logged before is not.
    if let Some(messages) = snapshot.get_mut("messages").and_then(Value::as_array_mut) {
        *messages = line.iter().cloned().collect();
    }
    let opened = matches!(status_of(&snapshot), "connected" | "disconnected");
    let answered = line.as_ref().is_none_or(|line| line.get("status") == Some(&json!("ok")));
    let asked_and_unanswered = ask.is_some() && line.is_none();
    let out = ctx.print(&snapshot, || match &line {
        Some(line) => describe_mcp_call(&snapshot, line),
        None => describe_mcp(&snapshot),
    });
    match failure {
        Some(error) if !ctx.json => Err(error),
        None if opened && answered && !asked_and_unanswered => Ok(out),
        _ => Err(CliError::Failed(out)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        requests::requests,
        testing::{Answer, config, payloads, run, sub},
    };

    fn listing(entities: Value) -> Result<Value> {
        Ok(json!({ "entities": entities }))
    }

    /// An engine whose only saved request is an MCP one, `m1`.
    fn engine() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("entity.list:requests", listing(json!([]))),
            ("entity.list:graphqlRequests", listing(json!([]))),
            ("entity.list:grpcRequests", listing(json!([]))),
            ("entity.list:soapRequests", listing(json!([]))),
            ("entity.list:wsConnections", listing(json!([]))),
            ("entity.list:sseRequests", listing(json!([]))),
            ("entity.list:socketioRequests", listing(json!([]))),
            (
                "entity.list:mcpRequests",
                listing(
                    json!([{"id": "m1", "name": "Tools", "transport": "stdio", "url": "npx my-server", "headers": {}}]),
                ),
            ),
        ]
    }

    fn connection(status: &str, error: Value, messages: Value) -> Result<Value> {
        Ok(json!({"workspaceId": "w1", "connectionId": "m1", "transport": "stdio", "url": "npx my-server",
                  "status": status, "error": error, "serverInfo": {"name": "mine", "version": "1.0"},
                  "protocolVersion": "2025-06-18",
                  "tools": [{"name": "echo", "description": "Says it back",
                             "inputSchema": {"properties": {"text": {}, "count": {}}, "required": ["text"]}}],
                  "prompts": [], "resources": [], "resourceTemplates": [], "messages": messages}))
    }

    fn line(status: &str, text: &str) -> Value {
        json!({"id": "m1-4", "type": "tool", "name": "echo", "arguments": "{\"text\":\"hi\"}", "status": status,
               "durationMs": 12, "data": "{}", "text": text})
    }

    #[test]
    fn sending_a_saved_request_connects_shows_what_the_server_declares_and_disconnects() {
        let mut answers = engine();
        answers.extend([
            ("mcp.status", connection("disconnected", Value::Null, json!([]))),
            ("mcp.connect", Ok(json!({"ok": true}))),
            ("mcp.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "Tools"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap(),
            "MCP disconnected  stdio  npx my-server\nserver: mine 1.0 (protocol 2025-06-18)\ntools\n  echo(text*, count)  Says it back"
        );
        let key = json!({"workspaceId": "w1", "connectionId": "m1"});
        // A saved request is opened by its id: the connect carries nothing of the request.
        assert_eq!(payloads(&calls, "mcp.connect"), std::slice::from_ref(&key));
        assert!(payloads(&calls, "mcp.call").is_empty());
        assert_eq!(payloads(&calls, "mcp.disconnect"), [key]);
    }

    #[test]
    fn a_tool_is_called_over_a_connection_the_engine_holds_which_is_left_open() {
        let mut answers = engine();
        answers.extend([
            (
                "mcp.status",
                connection(
                    "connected",
                    Value::Null,
                    json!([
                {"id": "m1-1", "type": "tool", "name": "old", "arguments": "{}", "status": "ok", "data": "{}"},
                line("ok", "hi")]),
                ),
            ),
            ("mcp.call", Ok(json!({"ok": true, "callId": "m1-4"}))),
        ]);
        let argv = ["requests", "send", "m1", "--tool", "echo", "--arg", "text=hi", "--arg", "count=2"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        // Only this call is printed, not what was already in the log.
        assert_eq!(
            out.unwrap(),
            "MCP connected  stdio  npx my-server\n> tool echo {\"text\":\"hi\"}\n< ok  (12 ms)\nhi"
        );
        // Every value goes as it was typed: the engine reads it as the type the tool declares.
        assert_eq!(
            payloads(&calls, "mcp.call"),
            [json!({"workspaceId": "w1", "connectionId": "m1", "type": "tool", "name": "echo",
                    "arguments": {"text": "hi", "count": "2"}})]
        );
        assert!(payloads(&calls, "mcp.connect").is_empty());
        assert!(payloads(&calls, "mcp.disconnect").is_empty());
    }

    #[test]
    fn a_prompt_and_a_resource_are_asked_for_by_name() {
        for (argv, expected) in [
            (
                vec!["requests", "send", "m1", "--prompt", "greet", "--arg", "who=Ada"],
                json!({"workspaceId": "w1", "connectionId": "m1", "type": "prompt", "name": "greet",
                    "arguments": {"who": "Ada"}}),
            ),
            (
                vec!["requests", "send", "m1", "--resource", "mem://a"],
                json!({"workspaceId": "w1", "connectionId": "m1", "type": "resource", "name": "mem://a"}),
            ),
        ] {
            let mut answers = engine();
            answers.extend([
                ("mcp.status", connection("connected", Value::Null, json!([line("ok", "{}")]))),
                ("mcp.call", Ok(json!({"ok": true, "callId": "m1-4"}))),
            ]);
            let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
            assert!(out.is_ok(), "{argv:?}");
            assert_eq!(payloads(&calls, "mcp.call"), [expected]);
        }
    }

    #[test]
    fn a_call_that_fails_fails_the_command_and_still_prints_the_answer() {
        let mut answers = engine();
        answers.extend([
            ("mcp.status", connection("connected", Value::Null, json!([line("error", "it broke")]))),
            ("mcp.call", Ok(json!({"ok": true, "callId": "m1-4"}))),
        ]);
        let argv = ["requests", "send", "m1", "--tool", "echo"];
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::Failed(
                "MCP connected  stdio  npx my-server\n> tool echo {\"text\":\"hi\"}\n< error  (12 ms)\nit broke".into()
            )
        );
    }

    #[test]
    fn an_answer_is_waited_for_no_longer_than_the_wait() {
        let mut answers = engine();
        answers.extend([
            ("mcp.status", connection("connected", Value::Null, json!([line("pending", "")]))),
            ("mcp.call", Ok(json!({"ok": true, "callId": "m1-4"}))),
        ]);
        let argv = ["requests", "send", "m1", "--tool", "echo", "--wait", "0.3"];
        let started = Instant::now();
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(matches!(out.unwrap_err(), CliError::Failed(report) if report.contains("
< pending")));
    }

    #[test]
    fn a_connection_that_does_not_open_fails_the_command_without_calling() {
        let mut answers = engine();
        answers.extend([
            ("mcp.status", connection("error", json!("The program ended"), json!([]))),
            ("mcp.connect", Ok(json!({"ok": true}))),
            ("mcp.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "m1", "--tool", "echo"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(matches!(out.unwrap_err(), CliError::Failed(report)
            if report.starts_with("MCP error  stdio  npx my-server\nerror: The program ended")));
        assert!(payloads(&calls, "mcp.call").is_empty());
    }

    #[test]
    fn a_refused_connect_or_call_is_the_engines_message() {
        let mut answers = engine();
        answers.extend([
            ("mcp.status", Ok(Value::Null)),
            ("mcp.connect", Ok(json!({"ok": false, "error": "Max 5 active connections reached."}))),
        ]);
        let argv = ["requests", "send", "m1"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("Max 5 active connections reached."));
        assert!(payloads(&calls, "mcp.disconnect").is_empty());

        let mut answers = engine();
        answers.extend([
            ("mcp.status", connection("connected", Value::Null, json!([]))),
            ("mcp.call", Ok(json!({"ok": false, "error": "Argument \"count\" must be a whole number, not \"x\"."}))),
        ]);
        let argv = ["requests", "send", "m1", "--tool", "echo", "--arg", "count=x"];
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("Argument \"count\" must be a whole number, not \"x\"."));
    }

    #[test]
    fn an_unsaved_request_connects_to_what_the_options_describe() {
        let answers = vec![
            ("config.get", Ok(config())),
            ("mcp.status", connection("disconnected", Value::Null, json!([]))),
            ("mcp.connect", Ok(json!({"ok": true}))),
            ("mcp.disconnect", Ok(json!({"ok": true}))),
        ];
        let argv = ["requests", "send", "-t", "mcp", "--url", "npx my-server", "-H", "API_KEY: k"];
        let (out, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert!(out.unwrap().starts_with("MCP disconnected"));
        assert_eq!(
            payloads(&calls, "mcp.connect"),
            [
                json!({"workspaceId": "w1", "connectionId": format!("cli-{}", std::process::id()), "url": "npx my-server",
                    "transport": "stdio", "headers": {"API_KEY": "k"}})
            ]
        );
        assert_eq!(payloads(&calls, "mcp.disconnect").len(), 1);

        // An address is an endpoint, unless the transport is named.
        let argv = ["requests", "send", "-t", "mcp", "--url", "https://x/mcp"];
        let (_, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(payloads(&calls, "mcp.connect")[0]["transport"], "http");
        let argv = ["requests", "send", "-t", "mcp", "--url", "https-server --stdio", "--transport", "stdio"];
        let (_, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(payloads(&calls, "mcp.connect")[0]["transport"], "stdio");
    }

    #[test]
    fn options_that_do_not_belong_to_the_send_are_refused() {
        for (argv, message) in [
            (
                vec!["requests", "send", "m1", "--transport", "http"],
                "a saved request is sent as it is saved, without --transport; change it with: bifurc requests update",
            ),
            (vec!["requests", "send", "m1", "--arg", "a=1"], "--arg goes with --tool <name> or --prompt <name>"),
            (
                vec!["requests", "send", "m1", "--resource", "mem://a", "--arg", "a=1"],
                "--arg goes with --tool <name> or --prompt <name>",
            ),
            (
                vec!["requests", "send", "m1", "--tool", "a", "--prompt", "b"],
                "give one of --tool, --prompt and --resource",
            ),
            (
                vec!["requests", "send", "m1", "--tool", "a", "--arg", "novalue"],
                "argument \"novalue\" must be \"name=value\"",
            ),
            (vec!["requests", "send", "m1", "--wait", "soon"], "--wait must be a number of seconds, not \"soon\""),
            (
                vec!["requests", "send", "-t", "mcp", "--url", "x", "-X", "POST"],
                "--method is not an option of a mcp request (see: bifurc requests --help)",
            ),
            (
                vec!["requests", "send", "-t", "rest", "--url", "http://x", "--tool", "echo"],
                "--tool is not an option of a rest request (see: bifurc requests --help)",
            ),
            (
                vec!["requests", "create", "-t", "mcp", "x", "--transport", "pigeon"],
                "--transport is stdio or http, not \"pigeon\"",
            ),
        ] {
            let (out, calls) = run(&argv, engine(), |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap_err(), CliError::usage(message), "{argv:?}");
            assert!(payloads(&calls, "mcp.connect").is_empty(), "{argv:?}");
        }
    }

    #[test]
    fn create_and_update_build_an_mcp_request() {
        let mut answers = engine();
        answers.extend([
            ("entity.create", Ok(json!({"id": "n1", "entity": {"id": "n1"}}))),
            ("entity.update", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "create", "--type", "mcp", "npx -y my-server", "-H", "API_KEY: {{vault.key}}"];
        let (out, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved mcp request n1");
        assert_eq!(
            payloads(&calls, "entity.create"),
            [json!({"kind": "mcpRequests", "workspaceId": "w1", "entity": {
                "name": "MCP npx -y my-server", "transport": "stdio", "url": "npx -y my-server",
                "headers": {"API_KEY": "{{vault.key}}"}}})]
        );

        // Changing the address changes the transport with it, unless one is named.
        let argv = ["requests", "update", "Tools", "--url", "https://x/mcp"];
        let (out, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated mcp request m1");
        let sent = &payloads(&calls, "entity.update")[0];
        assert_eq!(sent["kind"], "mcpRequests");
        assert_eq!(
            (sent["entity"]["url"].clone(), sent["entity"]["transport"].clone()),
            (json!("https://x/mcp"), json!("http"))
        );
        let argv = ["requests", "update", "Tools", "--name", "Mine"];
        let (_, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(payloads(&calls, "entity.update")[0]["entity"]["transport"], "stdio");
    }

    #[test]
    fn list_shows_the_transport_and_the_command() {
        let argv = ["requests", "list", "--type", "mcp"];
        let (out, _) = run(&argv, engine(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "MCP  npx my-server  Tools");
    }
}
