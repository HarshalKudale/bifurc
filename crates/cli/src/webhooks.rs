//! `webhooks listen|stop`: the engine's webhook receiver (see `engine/src/webhook.rs`). Without `--detach`
//! `listen` blocks and prints a line per inbound request; the engine ends the listener with this command's
//! connection, so Ctrl+C or a killed CI step never leaves the port busy.

use std::{collections::HashSet, io::Write};

use serde_json::{Value, json};

use crate::{
    capture,
    commands::{Context, unknown_verb, workspace_id},
    error::{CliError, Result},
    format::js_template,
    response::size,
};

pub fn webhooks(ctx: &mut Context, sub: Option<&str>, out: &mut dyn Write) -> Result<String> {
    match sub {
        Some("listen") => listen(ctx, out),
        Some("stop") => stop(ctx),
        other => Err(unknown_verb("webhooks", other.unwrap_or(""))),
    }
}

/// `listening http://127.0.0.1:4020 (stripe) → http://api.localhost/webhooks/stripe`
fn start_line(url: &str, name: Option<&str>, forward: Option<&str>) -> String {
    let name = name.map_or(String::new(), |n| format!(" ({n})"));
    let forward = forward.map_or(String::new(), |f| format!(" → {f}"));
    format!("listening {url}{name}{forward}")
}

/// `POST /hooks/stripe → 200 · 11B`
fn request_line(entry: &Value) -> String {
    let url = js_template(entry.get("url"));
    let rest = url.split_once("://").map_or(url.as_str(), |(_, rest)| rest);
    let path = rest.find('/').map_or("/", |at| &rest[at..]);
    let bytes = entry["size"].as_u64().unwrap_or(0) as usize;
    format!("{} {path} → {} · {}", js_template(entry.get("method")), js_template(entry.get("status")), size(bytes))
}

fn listen(ctx: &mut Context, out: &mut dyn Write) -> Result<String> {
    ctx.accept(&["port", "name", "forward", "detach", "quiet"], 0)?;
    let args = ctx.args;
    let port = capture::number(ctx, "port")?;
    if port.is_some_and(|p| p > 65535) {
        return Err(CliError::usage("--port is a number from 0 to 65535"));
    }
    let workspace = workspace_id(ctx)?;
    let mut payload = json!({ "workspaceId": workspace, "detach": args.has("detach") });
    if let Some(port) = port {
        payload["port"] = json!(port);
    }
    for key in ["name", "forward"] {
        if let Some(value) = args.flag(key) {
            payload[key] = json!(value);
        }
    }
    let started = ctx.request("webhook.listen", payload).map_err(|err| match err {
        CliError::Engine { code, .. } if code == "CONFLICT" => {
            CliError::msg(format!("port {} is in use (see: bifurc services list)", port.unwrap_or(0)))
        }
        CliError::Engine { code, message } if code == "BAD_REQUEST" => CliError::usage(message),
        other => other,
    })?;
    let line = start_line(&js_template(started.get("url")), args.flag("name"), args.flag("forward"));
    if args.has("detach") {
        return Ok(ctx.print(&started, || line.clone()));
    }

    let _ = writeln!(out, "{line} (Ctrl+C to stop)");
    let _ = out.flush();
    let quiet = args.has("quiet");
    let (mut seen, suffix) = (HashSet::new(), format!(":{}", js_template(started.get("port"))));
    let since = capture::now_ms();
    if !quiet {
        ctx.request("subscribe", json!({ "events": [capture::EVENT] }))?;
    }
    // The listener ends when this connection does; until then, report what it receives.
    loop {
        if !quiet {
            let listed = capture::summaries(ctx, &workspace, json!({ "origin": "webhook", "since": since }))?;
            for entry in listed["entries"].as_array().into_iter().flatten().rev() {
                let url = js_template(entry.get("url"));
                let authority =
                    url.split_once("://").map_or(url.as_str(), |(_, rest)| rest).split('/').next().unwrap_or_default();
                if !authority.ends_with(&suffix) || !seen.insert(js_template(entry.get("id"))) {
                    continue;
                }
                if writeln!(out, "{}", request_line(entry)).and_then(|()| out.flush()).is_err() {
                    return Ok(String::new());
                }
            }
        }
        if ctx.engine()?.next_event()?.is_none() {
            return Ok(String::new());
        }
    }
}

/// `webhooks stop [<name|port>]`: one listener by its name or port, or every one when none is given.
fn stop(ctx: &mut Context) -> Result<String> {
    ctx.accept(&[], 1)?;
    let wanted = ctx.args.positional(2).map(str::to_string);
    let mut ids = Vec::new();
    if let Some(wanted) = &wanted {
        let listed = ctx.request("webhook.list", json!({}))?;
        for l in listed["listeners"].as_array().into_iter().flatten() {
            if l["name"].as_str() == Some(wanted) || js_template(l.get("port")) == *wanted {
                ids.push(js_template(l.get("id")));
            }
        }
        if ids.is_empty() {
            return Err(CliError::not_found(format!("no webhook listener \"{wanted}\" is running")));
        }
    }
    let mut stopped = Vec::new();
    for payload in if ids.is_empty() { vec![json!({})] } else { ids.iter().map(|id| json!({ "id": id })).collect() } {
        let answer = ctx.request("webhook.stop", payload)?;
        stopped.extend(answer["stopped"].as_array().cloned().unwrap_or_default());
    }
    let ports: Vec<String> = stopped.iter().map(|l| format!(":{}", js_template(l.get("port")))).collect();
    Ok(ctx.print(&json!({ "stopped": stopped }), || format!("stopped webhook listener {}", ports.join(", "))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Answer, config, payloads, run_with_events};

    fn go(argv: &[&str], answers: Vec<Answer>, events: Vec<Value>) -> (Result<String>, Vec<(String, Value)>, String) {
        let mut out = Vec::new();
        let sub = argv.get(1).copied();
        let (result, calls) = run_with_events(argv, answers, events, |ctx| webhooks(ctx, sub, &mut out));
        (result, calls, String::from_utf8(out).unwrap())
    }

    fn started() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("webhook.listen", Ok(json!({"id": "k1", "port": 4020, "url": "http://127.0.0.1:4020"}))),
        ]
    }

    #[test]
    fn detach_sends_the_options_and_prints_one_line() {
        let argv = ["webhooks", "listen", "--detach", "--name", "stripe", "--forward", "http://api.localhost/hook"];
        let (out, calls, printed) = go(&argv, started(), vec![]);
        assert_eq!(out.unwrap(), "listening http://127.0.0.1:4020 (stripe) → http://api.localhost/hook");
        assert_eq!(
            payloads(&calls, "webhook.listen"),
            [json!({"workspaceId": "w1", "detach": true, "name": "stripe", "forward": "http://api.localhost/hook"})]
        );
        assert_eq!(printed, "");
        let (out, _, _) = go(&["webhooks", "listen", "--detach"], started(), vec![]);
        assert_eq!(out.unwrap(), "listening http://127.0.0.1:4020");
    }

    #[test]
    fn a_busy_port_and_a_bad_forward_are_reported() {
        let busy = Err(CliError::Engine { code: "CONFLICT".into(), message: "Port 4020 is already in use".into() });
        let (out, _, _) = go(
            &["webhooks", "listen", "--port", "4020"],
            vec![("config.get", Ok(config())), ("webhook.listen", busy)],
            vec![],
        );
        assert_eq!(out.unwrap_err().line(), "bifurc: port 4020 is in use (see: bifurc services list)");
        let bad = Err(CliError::Engine { code: "BAD_REQUEST".into(), message: "--forward is a URL".into() });
        let (out, _, _) = go(
            &["webhooks", "listen", "--forward", "x"],
            vec![("config.get", Ok(config())), ("webhook.listen", bad)],
            vec![],
        );
        assert_eq!(out.unwrap_err().code(), 2);
    }

    #[test]
    fn stop_by_name_or_port_or_all() {
        let listing = json!({"listeners": [{"id": "k1", "port": 4020, "name": "stripe"}, {"id": "k2", "port": 4021}]});
        let stopped = json!({"stopped": [{"id": "k1", "port": 4020, "name": "stripe"}]});
        for wanted in ["stripe", "4020"] {
            let answers = vec![("webhook.list", Ok(listing.clone())), ("webhook.stop", Ok(stopped.clone()))];
            let (out, calls, _) = go(&["webhooks", "stop", wanted], answers, vec![]);
            assert_eq!(out.unwrap(), "stopped webhook listener :4020");
            assert_eq!(payloads(&calls, "webhook.stop"), [json!({"id": "k1"})]);
        }
        let (out, _, _) = go(&["webhooks", "stop", "nope"], vec![("webhook.list", Ok(listing))], vec![]);
        assert_eq!(out.unwrap_err().code(), 3);
        let all = json!({"stopped": [{"port": 4020}, {"port": 4021}]});
        let (out, calls, _) = go(&["webhooks", "stop"], vec![("webhook.stop", Ok(all))], vec![]);
        assert_eq!(out.unwrap(), "stopped webhook listener :4020, :4021");
        assert_eq!(payloads(&calls, "webhook.stop"), [json!({})]);
    }

    #[test]
    fn the_foreground_prints_a_line_per_request_of_its_own_port() {
        let mut answers = started();
        answers.push(("subscribe", Ok(json!({}))));
        let own = |id: &str, url: &str| json!({"id": id, "method": "POST", "url": url, "status": 200, "size": 11});
        answers.push((
            "capture.list",
            Ok(
                json!({"entries": [own("b", "http://127.0.0.1:4020/hooks/b"), own("a", "http://127.0.0.1:4020/hooks/a"),
                                   own("c", "http://127.0.0.1:9999/other")]}),
            ),
        ));
        let (out, calls, printed) = go(&["webhooks", "listen"], answers, vec![json!({"event": "event.log.entry"})]);
        assert_eq!(out.unwrap(), "");
        assert_eq!(
            printed,
            "listening http://127.0.0.1:4020 (Ctrl+C to stop)\nPOST /hooks/a → 200 · 11B\nPOST /hooks/b → 200 · 11B\n"
        );
        assert_eq!(payloads(&calls, "capture.list")[0]["origin"], "webhook");
    }
}
