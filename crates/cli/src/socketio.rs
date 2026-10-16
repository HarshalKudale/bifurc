//! `requests send` for a Socket.IO request: connect (unless the engine already holds the connection), emit
//! `--event` with the arguments in `--body`, listen, print what was emitted and received and disconnect what
//! was connected here.
//!
//! Listening ends after `--wait` seconds, or sooner when the server ends the connection.

use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    commands::{Context, refused},
    error::{CliError, Result},
    format::describe_socketio,
    requests::{CONNECT_TIMEOUT, listen_seconds},
};

/// How long a connection is listened to when `--wait` does not say.
const DEFAULT_WAIT: f64 = 2.0;

/// How often the engine is asked for the connection's state while listening.
const POLL: Duration = Duration::from_millis(100);

fn status_of(snapshot: &Value) -> &str {
    snapshot.get("status").and_then(Value::as_str).unwrap_or_default()
}

/// One exchange over a Socket.IO connection named `connection_id`: `target` is the request to open when it is
/// not saved (`url`, `headers`, `path`, `namespace`, `auth`); a saved request is opened by its id.
pub fn socketio_exchange(
    ctx: &mut Context,
    workspace: &str,
    connection_id: &str,
    target: Option<&Value>,
) -> Result<String> {
    let wait = listen_seconds(ctx.args, DEFAULT_WAIT)?;
    let event = ctx.args.flag("event").map(str::to_string);
    if event.is_none() {
        if let Some(option) = ["body", "ack"].into_iter().find(|option| ctx.args.has(option)) {
            return Err(CliError::msg(format!("--{option} goes with --event <name>, the event to emit")));
        }
    }

    let key = json!({ "workspaceId": workspace, "connectionId": connection_id });
    let before = ctx.request("socketio.status", key.clone())?;
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
            for field in ["url", "path", "namespace", "auth"] {
                connect[field] = target.get(field).cloned().unwrap_or_else(|| json!(""));
            }
            connect["headers"] = target.get("headers").cloned().unwrap_or_else(|| json!({}));
            // Without --listen every event the server sends is printed.
            if let Some(listeners) = target.get("listeners").filter(|listeners| listeners.is_array()) {
                connect["listeners"] = listeners.clone();
            }
        }
        refused(ctx.request("socketio.connect", connect)?)?;
        let started = Instant::now();
        loop {
            snapshot = ctx.request("socketio.status", key.clone())?;
            if status_of(&snapshot) != "connecting" || started.elapsed() > CONNECT_TIMEOUT {
                break;
            }
            sleep(POLL);
        }
    }

    // An event is only emitted over a connection that opened; one that did not is the failure to report.
    let mut failure = None;
    if let (Some(event), "connected") = (&event, status_of(&snapshot)) {
        let mut emit = key.clone();
        emit["event"] = json!(event);
        emit["args"] = json!(ctx.args.flag("body").unwrap_or_default());
        emit["ack"] = json!(ctx.args.has("ack"));
        failure = refused(ctx.request("socketio.emit", emit)?).err();
    }

    let listening = Instant::now();
    let budget = Duration::from_secs_f64(wait);
    while failure.is_none() && status_of(&snapshot) == "connected" && listening.elapsed() < budget {
        sleep(POLL.min(budget.saturating_sub(listening.elapsed())));
        snapshot = ctx.request("socketio.status", key.clone())?;
    }
    if event.is_some() && failure.is_none() {
        // With no time to listen the emit is still what this send did.
        snapshot = ctx.request("socketio.status", key.clone())?;
    }
    if !held {
        ctx.request("socketio.disconnect", key)?;
    }

    if let Some(messages) = snapshot.get_mut("messages").and_then(Value::as_array_mut) {
        messages.retain(|message| message.get("id").is_none_or(|id| !seen.contains(id)));
    }
    let opened = matches!(status_of(&snapshot), "connected" | "disconnected");
    let out = ctx.print(&snapshot, || describe_socketio(&snapshot));
    match failure {
        Some(error) if !ctx.json => Err(error),
        None if opened => Ok(out),
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

    /// An engine whose only saved request is a Socket.IO one, `i1`.
    fn engine() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("entity.list:requests", listing(json!([]))),
            ("entity.list:graphqlRequests", listing(json!([]))),
            ("entity.list:grpcRequests", listing(json!([]))),
            ("entity.list:soapRequests", listing(json!([]))),
            ("entity.list:wsConnections", listing(json!([]))),
            ("entity.list:sseRequests", listing(json!([]))),
            ("entity.list:mcpRequests", listing(json!([]))),
            (
                "entity.list:socketioRequests",
                listing(json!([{"id": "i1", "name": "Chat", "url": "http://x", "namespace": "/chat", "path": "",
                                "auth": "", "headers": {}}])),
            ),
        ]
    }

    fn connection(status: &str, error: Value, messages: Value) -> Result<Value> {
        Ok(json!({"workspaceId": "w1", "connectionId": "i1", "url": "http://x", "namespace": "/chat",
                  "status": status, "error": error, "sid": "s1", "messages": messages}))
    }

    #[test]
    fn sending_a_saved_request_connects_emits_listens_and_disconnects() {
        let mut answers = engine();
        answers.extend([
            ("socketio.status", connection("disconnected", Value::Null, json!([]))),
            ("socketio.connect", Ok(json!({"ok": true}))),
            ("socketio.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "Chat", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Socket.IO disconnected  http://x  /chat");
        let key = json!({"workspaceId": "w1", "connectionId": "i1"});
        // A saved request is opened by its id: the connect carries nothing of the request.
        assert_eq!(payloads(&calls, "socketio.connect"), [key.clone()]);
        assert!(payloads(&calls, "socketio.emit").is_empty());
        assert_eq!(payloads(&calls, "socketio.disconnect"), [key]);
    }

    #[test]
    fn an_event_is_emitted_over_a_connection_the_engine_holds_which_is_left_open() {
        let mut answers = engine();
        answers.extend([
            // The engine holds the connection and has an old message.
            (
                "socketio.status",
                connection(
                    "connected",
                    Value::Null,
                    json!([
                {"id": "i1-1", "direction": "received", "type": "event", "event": "old", "data": "[]", "ackId": null}]),
                ),
            ),
            ("socketio.emit", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "i1", "--event", "say", "-d", "[\"hi\"]", "--ack", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        // The message that was already in the log is not this send's.
        assert_eq!(out.unwrap(), "Socket.IO connected  http://x  /chat");
        assert_eq!(
            payloads(&calls, "socketio.emit"),
            [json!({"workspaceId": "w1", "connectionId": "i1", "event": "say", "args": "[\"hi\"]", "ack": true})]
        );
        assert!(payloads(&calls, "socketio.connect").is_empty());
        assert!(payloads(&calls, "socketio.disconnect").is_empty());
    }

    #[test]
    fn an_event_without_arguments_or_acknowledgement_says_so() {
        let mut answers = engine();
        answers.extend([
            ("socketio.status", connection("connected", Value::Null, json!([]))),
            ("socketio.emit", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "i1", "--event", "ping", "--wait", "0"];
        let (_, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        let emitted = &payloads(&calls, "socketio.emit")[0];
        assert_eq!((emitted["args"].clone(), emitted["ack"].clone()), (json!(""), json!(false)));
    }

    #[test]
    fn a_connection_that_does_not_open_fails_the_command_without_emitting() {
        let mut answers = engine();
        answers.extend([
            ("socketio.status", connection("error", json!("Not authorized"), json!([]))),
            ("socketio.connect", Ok(json!({"ok": true}))),
            ("socketio.disconnect", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "send", "i1", "--event", "say", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::Failed("Socket.IO error  http://x  /chat\nerror: Not authorized".into())
        );
        assert!(payloads(&calls, "socketio.emit").is_empty());
    }

    #[test]
    fn a_refused_connect_or_emit_is_the_engines_message() {
        let mut answers = engine();
        answers.extend([
            ("socketio.status", Ok(Value::Null)),
            ("socketio.connect", Ok(json!({"ok": false, "error": "Max 5 active connections reached."}))),
        ]);
        let argv = ["requests", "send", "i1", "--wait", "0"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap_err(), CliError::msg("Max 5 active connections reached."));
        assert!(payloads(&calls, "socketio.disconnect").is_empty());

        let mut answers = engine();
        answers.extend([
            ("socketio.status", connection("connected", Value::Null, json!([]))),
            ("socketio.emit", Ok(json!({"ok": false, "error": "An event name is required."}))),
        ]);
        let argv = ["requests", "send", "i1", "--event", " ", "--wait", "30"];
        // A refused emit is not listened after.
        let started = Instant::now();
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(out.unwrap_err(), CliError::msg("An event name is required."));
    }

    #[test]
    fn an_unsaved_request_connects_to_what_the_options_describe() {
        let answers = vec![
            ("config.get", Ok(config())),
            ("socketio.status", connection("disconnected", Value::Null, json!([]))),
            ("socketio.connect", Ok(json!({"ok": true}))),
            ("socketio.disconnect", Ok(json!({"ok": true}))),
        ];
        let argv = [
            "requests",
            "send",
            "-t",
            "socket.io",
            "--url",
            "http://x",
            "--namespace",
            "/chat",
            "--path",
            "/rt",
            "--auth",
            "{\"token\":\"t\"}",
            "-H",
            "a: 1",
            "--wait",
            "30",
        ];
        // The connection is over when it is first asked about, so there is nothing to wait 30 seconds for.
        let started = Instant::now();
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(out.unwrap(), "Socket.IO disconnected  http://x  /chat");
        assert_eq!(
            payloads(&calls, "socketio.connect"),
            [json!({"workspaceId": "w1", "connectionId": format!("cli-{}", std::process::id()), "url": "http://x",
                    "path": "/rt", "namespace": "/chat", "auth": "{\"token\":\"t\"}", "headers": {"a": "1"}})]
        );
        assert_eq!(payloads(&calls, "socketio.disconnect").len(), 1);
    }

    #[test]
    fn listen_names_the_events_that_are_printed() {
        let answers = vec![
            ("config.get", Ok(config())),
            ("socketio.status", connection("disconnected", Value::Null, json!([]))),
            ("socketio.connect", Ok(json!({"ok": true}))),
            ("socketio.disconnect", Ok(json!({"ok": true}))),
        ];
        let argv = [
            "requests", "send", "-t", "socketio", "--url", "http://x", "--listen", "message", "--listen", "tick",
            "--wait", "0",
        ];
        let (_, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(payloads(&calls, "socketio.connect")[0]["listeners"], json!(["message", "tick"]));

        let mut answers = engine();
        answers.extend([
            ("entity.create", Ok(json!({"id": "n1", "entity": {"id": "n1"}}))),
            ("entity.update", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "create", "-t", "socketio", "http://x", "--listen", "message"];
        let (_, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        let entity = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!((entity["listeners"].clone(), entity["listenAll"].clone()), (json!(["message"]), json!(false)));

        let argv = ["requests", "update", "Chat", "--listen", "tick"];
        let (_, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(payloads(&calls, "entity.update")[0]["entity"]["listeners"], json!(["tick"]));

        // A saved request listens to what it has saved.
        let argv = ["requests", "send", "Chat", "--listen", "tick"];
        let (out, _) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(
            out.unwrap_err(),
            CliError::msg(
                "a saved request is sent as it is saved, without --listen; change it with: bifurc requests update"
            )
        );
    }

    #[test]
    fn a_connection_that_stays_open_is_listened_to_for_the_wait() {
        let mut answers = engine();
        answers.push(("socketio.status", connection("connected", Value::Null, json!([]))));
        let argv = ["requests", "send", "i1", "--wait", "0.35"];
        let started = Instant::now();
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert!(started.elapsed() >= Duration::from_millis(350));
        assert_eq!(out.unwrap(), "Socket.IO connected  http://x  /chat");
        assert!(payloads(&calls, "socketio.status").len() >= 3);
    }

    #[test]
    fn options_that_do_not_belong_to_the_send_are_refused() {
        for (argv, message) in [
            (
                vec!["requests", "send", "i1", "--namespace", "/x"],
                "a saved request is sent as it is saved, without --namespace; change it with: bifurc requests update",
            ),
            (vec!["requests", "send", "i1", "-d", "[1]"], "--body goes with --event <name>, the event to emit"),
            (vec!["requests", "send", "i1", "--ack"], "--ack goes with --event <name>, the event to emit"),
            (vec!["requests", "send", "i1", "--wait", "soon"], "--wait must be a number of seconds, not \"soon\""),
            (
                vec!["requests", "send", "-t", "socketio", "--url", "http://x", "-X", "POST"],
                "--method is not an option of a socketio request (see: bifurc requests --help)",
            ),
            (
                vec!["requests", "send", "-t", "rest", "--url", "http://x", "--event", "say"],
                "--event is not an option of a rest request (see: bifurc requests --help)",
            ),
            (
                vec!["requests", "send", "-t", "sse", "--url", "http://x", "--ack"],
                "--ack is not an option of a sse request (see: bifurc requests --help)",
            ),
        ] {
            let (out, calls) = run(&argv, engine(), |c| requests(c, sub(&argv)));
            assert_eq!(out.unwrap_err(), CliError::msg(message), "{argv:?}");
            assert!(payloads(&calls, "socketio.connect").is_empty(), "{argv:?}");
        }
    }

    #[test]
    fn create_and_update_build_a_socketio_request() {
        let mut answers = engine();
        answers.extend([
            ("entity.create", Ok(json!({"id": "n1", "entity": {"id": "n1"}}))),
            ("entity.update", Ok(json!({"ok": true}))),
        ]);
        let argv = ["requests", "create", "--type", "socketio", "http://x", "--namespace", "/chat", "-H", "a: 1"];
        let (out, calls) = run(&argv, answers.clone(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Saved socketio request n1");
        assert_eq!(
            payloads(&calls, "entity.create"),
            [json!({"kind": "socketioRequests", "workspaceId": "w1", "entity": {
                "name": "Socket.IO http://x", "url": "http://x", "headers": {"a": "1"}, "path": "",
                "namespace": "/chat", "auth": ""}})]
        );

        let argv = ["requests", "update", "Chat", "--auth", "{\"token\":\"t\"}", "--path", "/rt"];
        let (out, calls) = run(&argv, answers, |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "Updated socketio request i1");
        let sent = &payloads(&calls, "entity.update")[0];
        assert_eq!(sent["kind"], "socketioRequests");
        assert_eq!(
            (sent["entity"]["auth"].clone(), sent["entity"]["path"].clone(), sent["entity"]["namespace"].clone()),
            (json!("{\"token\":\"t\"}"), json!("/rt"), json!("/chat"))
        );
    }

    #[test]
    fn list_shows_the_namespace_and_the_url() {
        let argv = ["requests", "list", "--type", "sio"];
        let (out, _) = run(&argv, engine(), |c| requests(c, sub(&argv)));
        assert_eq!(out.unwrap(), "ID  TYPE      NAME  METHOD  URL\ni1  socketio  Chat  /chat   http://x");
    }
}
