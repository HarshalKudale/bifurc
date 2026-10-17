//! `requests send` for an SSE request: open the stream (unless the engine already holds it), listen, print the
//! events that arrived and close what was opened here.
//!
//! Unlike a WebSocket nothing is sent after connecting: the method, headers and body of the request go with the
//! connect. Listening ends after `--wait` seconds, or sooner when the server ends the stream.

use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    commands::{Context, refused},
    error::{CliError, Result},
    format::describe_stream,
    requests::{CONNECT_TIMEOUT, listen_seconds},
};

/// How long a stream is listened to when `--wait` does not say.
const DEFAULT_WAIT: f64 = 5.0;

/// How often the engine is asked for the stream's state while listening.
const POLL: Duration = Duration::from_millis(100);

fn status_of(snapshot: &Value) -> &str {
    snapshot.get("status").and_then(Value::as_str).unwrap_or_default()
}

/// One exchange over an SSE stream named `stream_id`: `target` is the request to open when it is not saved
/// (`url`, `method`, `headers`, `body`); a saved request is opened by its id.
pub fn stream_exchange(ctx: &mut Context, workspace: &str, stream_id: &str, target: Option<&Value>) -> Result<String> {
    let wait = listen_seconds(ctx.args, DEFAULT_WAIT)?;
    let key = json!({ "workspaceId": workspace, "streamId": stream_id });
    let before = ctx.request("sse.status", key.clone())?;
    let held = status_of(&before) == "connected";
    let seen: Vec<Value> = before
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("id").cloned())
        .collect();

    let mut snapshot = before;
    if !held {
        let mut connect = key.clone();
        if let Some(target) = target {
            for field in ["url", "method", "headers", "body"] {
                connect[field] = target.get(field).cloned().unwrap_or(Value::Null);
            }
            connect["headers"] = target.get("headers").cloned().unwrap_or_else(|| json!({}));
        }
        refused(ctx.request("sse.connect", connect)?)?;
        let started = Instant::now();
        loop {
            snapshot = ctx.request("sse.status", key.clone())?;
            if status_of(&snapshot) != "connecting" || started.elapsed() > CONNECT_TIMEOUT {
                break;
            }
            sleep(POLL);
        }
    }

    let listening = Instant::now();
    let budget = Duration::from_secs_f64(wait);
    while status_of(&snapshot) == "connected" && listening.elapsed() < budget {
        sleep(POLL.min(budget.saturating_sub(listening.elapsed())));
        snapshot = ctx.request("sse.status", key.clone())?;
    }
    if !held {
        ctx.request("sse.disconnect", key)?;
    }

    if let Some(messages) = snapshot.get_mut("messages").and_then(Value::as_array_mut) {
        messages.retain(|message| message.get("id").is_none_or(|id| !seen.contains(id)));
    }
    let opened = matches!(status_of(&snapshot), "connected" | "disconnected");
    let out = ctx.print(&snapshot, || describe_stream(&snapshot));
    if opened { Ok(out) } else { Err(CliError::Failed(out)) }
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

    /// An engine whose only saved request is an SSE one, `e1`.
    fn engine() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("entity.list:requests", listing(json!([]))),
            ("entity.list:graphqlRequests", listing(json!([]))),
            ("entity.list:grpcRequests", listing(json!([]))),
            ("entity.list:soapRequests", listing(json!([]))),
            ("entity.list:wsConnections", listing(json!([]))),
            ("entity.list:socketioRequests", listing(json!([]))),
            ("entity.list:mcpRequests", listing(json!([]))),
            (
                "entity.list:sseRequests",
                listing(json!([{"id": "e1", "name": "Feed", "url": "http://x/events", "method": "GET",
                                "headers": {}, "body": ""}])),
            ),
        ]
    }

    fn stream(status: &str, error: Value, messages: Value) -> Result<Value> {
        Ok(json!({"workspaceId": "w1", "streamId": "e1", "url": "http://x/events", "status": status, "error": error,
                  "httpStatus": 200, "messages": messages}))
    }

    #[test]
    fn sending_a_saved_stream_opens_it_prints_its_events_and_closes_it() {
        let events = json!([{"id": "e1-1", "event": "message", "data": "hello"}]);
        let mut answers = engine();
        answers.extend([
            ("sse.status", stream("disconnected", Value::Null, events)),
            ("sse.connect", Ok(json!({"ok": true}))),
            ("sse.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "Feed", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        // The fake answers the status asked before connecting with the same log, so every event counts as one
        // that was already there; the events of a real exchange are in the end-to-end test.
        assert_eq!(out.unwrap(), "SSE disconnected  http://x/events");
        let key = json!({"workspaceId": "w1", "streamId": "e1"});
        // A saved stream is opened by its id: the connect carries nothing of the request.
        assert_eq!(payloads(&calls, "sse.connect"), std::slice::from_ref(&key));
        assert_eq!(payloads(&calls, "sse.disconnect"), [key]);
    }

    #[test]
    fn events_that_were_already_in_the_log_are_not_this_sends() {
        let mut answers = engine();
        answers.extend([
            // The engine holds the stream and has an old event; the exchange is left open.
            (
                "sse.status",
                stream("connected", Value::Null, json!([{"id": "e1-1", "event": "message", "data": "old"}])),
            ),
        ]);
        let argv = ["requests", "send", "e1", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "SSE connected  http://x/events");
        assert!(payloads(&calls, "sse.connect").is_empty());
        assert!(payloads(&calls, "sse.disconnect").is_empty());
    }

    #[test]
    fn a_stream_that_does_not_open_fails_the_command() {
        let mut answers = engine();
        answers.extend([
            ("sse.status", stream("error", json!("HTTP 404 Not Found"), json!([]))),
            ("sse.connect", Ok(json!({"ok": true}))),
            ("sse.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "e1", "--wait", "0"];
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::Failed("SSE error  http://x/events\nerror: HTTP 404 Not Found".into()));
    }

    #[test]
    fn a_refused_connect_is_the_engines_message() {
        let mut answers = engine();
        answers.extend([
            ("sse.status", Ok(Value::Null)),
            ("sse.connect", Ok(json!({"ok": false, "error": "Max 5 active streams reached."}))),
        ]);
        let argv = ["requests", "send", "e1", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("Max 5 active streams reached."));
        assert!(payloads(&calls, "sse.disconnect").is_empty());
    }

    #[test]
    fn an_unsaved_stream_opens_the_request_given() {
        let answers = vec![
            ("config.get", Ok(config())),
            (
                "sse.status",
                stream("disconnected", Value::Null, json!([{"id": "x-1", "event": "message", "data": "bye"}])),
            ),
            ("sse.connect", Ok(json!({"ok": true}))),
            ("sse.disconnect", Ok(json!({"ok": true}))),
        ];
        let argv = [
            "requests",
            "send",
            "-t",
            "sse",
            "--url",
            "http://x/events",
            "-X",
            "post",
            "-H",
            "a: 1",
            "-d",
            "{\"q\":1}",
            "--wait",
            "30",
        ];
        // The stream is over when it is first asked about, so there is nothing to wait 30 seconds for.
        let started = Instant::now();
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(out.unwrap(), "SSE disconnected  http://x/events");
        let connect = &payloads(&calls, "sse.connect")[0];
        assert_eq!(connect["streamId"], format!("cli-{}", std::process::id()));
        assert_eq!(
            (connect["url"].clone(), connect["method"].clone(), connect["headers"].clone(), connect["body"].clone()),
            (json!("http://x/events"), json!("POST"), json!({"a": "1"}), json!("{\"q\":1}"))
        );
        assert_eq!(payloads(&calls, "sse.disconnect").len(), 1);
    }

    #[test]
    fn a_stream_that_stays_open_is_listened_to_for_the_wait_and_left_open_when_the_engine_held_it() {
        let mut answers = engine();
        answers.push(("sse.status", stream("connected", Value::Null, json!([]))));
        let argv = ["requests", "send", "e1", "--wait", "0.35"];
        let started = Instant::now();
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(started.elapsed() >= Duration::from_millis(350));
        assert_eq!(out.unwrap(), "SSE connected  http://x/events");
        // Asked once before listening, then every poll of the wait.
        assert!(payloads(&calls, "sse.status").len() >= 3);
        assert!(payloads(&calls, "sse.disconnect").is_empty());
    }

    #[test]
    fn options_that_do_not_belong_to_a_stream_send_are_refused_before_asking_the_engine() {
        for (argv, message) in [
            (
                vec!["requests", "send", "e1", "-d", "x"],
                "a saved request is sent as it is saved, without --body; change it with: bifurc requests update",
            ),
            (vec!["requests", "send", "e1", "--wait", "soon"], "--wait must be a number of seconds, not \"soon\""),
            (
                vec!["requests", "send", "-t", "sse", "--url", "http://x", "--query", "{a}"],
                "--query is not an option of a sse request (see: bifurc requests --help)",
            ),
            (
                vec!["requests", "send", "-t", "sse"],
                "missing request id or name, or --url for a request that is not saved",
            ),
        ] {
            let (out, calls) = run(&argv, engine(), |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap_err(), CliError::usage(message), "{argv:?}");
            assert!(payloads(&calls, "sse.connect").is_empty(), "{argv:?}");
        }
    }

    #[test]
    fn create_and_update_build_an_sse_request() {
        let mut answers = engine();
        answers.extend([
            ("entity.create", Ok(json!({"id": "n1", "entity": {"id": "n1"}}))),
            ("entity.update", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "create", "--type", "sse", "http://x/events", "-X", "post", "-H", "a: 1", "-d", "{}"];
        let (out, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved sse request n1");
        assert_eq!(
            payloads(&calls, "entity.create"),
            [json!({"kind": "sseRequests", "workspaceId": "w1", "entity": {
                "name": "SSE http://x/events", "url": "http://x/events", "method": "POST", "headers": {"a": "1"},
                "body": "{}"}})]
        );

        let argv = ["requests", "update", "Feed", "--url", "http://y/events", "-X", "put"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated sse request e1");
        let sent = &payloads(&calls, "entity.update")[0];
        assert_eq!(sent["kind"], "sseRequests");
        assert_eq!(
            (sent["entity"]["url"].clone(), sent["entity"]["method"].clone()),
            (json!("http://y/events"), json!("PUT"))
        );
    }

    #[test]
    fn list_shows_the_method_and_the_url() {
        let argv = ["requests", "list", "--type", "sse"];
        let (out, _) = run(&argv, engine(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "ID  TYPE  NAME  METHOD  URL\ne1  sse   Feed  GET     http://x/events");
    }
}
