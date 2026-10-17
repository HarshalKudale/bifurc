//! `webhooks listen|stop|list|show|replay|save`: the engine's webhook receiver (see `engine/src/webhook.rs`). Without `--detach`
//! `listen` blocks and prints a line per inbound request; the engine ends the listener with this command's
//! connection, so Ctrl+C or a killed CI step never leaves the port busy.

use std::{collections::HashSet, io::Write, time::Instant};

use serde_json::{Value, json};

use crate::{
    capture,
    commands::{Context, created, need, unknown_verb, workspace_id},
    error::{CliError, NOT_FOUND, Result},
    format::js_template,
    response::{Response, present, size},
};

pub fn webhooks(ctx: &mut Context, sub: Option<&str>, out: &mut dyn Write) -> Result<String> {
    match sub {
        Some("listen") => listen(ctx, out),
        Some("stop") => stop(ctx),
        Some("list") => list(ctx),
        Some("show") => {
            let wanted = need(ctx.args.positional(2), "the webhook's id (see: bifurc webhooks list)")?;
            let workspace = workspace_id(ctx)?;
            webhook_id(ctx, &workspace, &wanted)?;
            capture::show(ctx)
        }
        Some("replay") => replay(ctx),
        Some("save") => save(ctx),
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

/// The full id of the webhook entry `wanted` names; an entry of another origin does not count.
fn webhook_id(ctx: &mut Context, workspace: &str, wanted: &str) -> Result<String> {
    let all = capture::summaries(ctx, workspace, json!({ "origin": "webhook" }))?;
    let ids: Vec<String> = all["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["origin"] == "webhook")
        .map(|e| js_template(e.get("id")))
        .collect();
    capture::entry_id(&ids, wanted).map_err(|err| match err {
        CliError::Exit(NOT_FOUND, _) => {
            CliError::not_found(format!("no webhook \"{wanted}\" (see: bifurc webhooks list)"))
        }
        other => other,
    })
}

/// `webhooks list [--name x] [--last N]`: the captured webhooks as `capture list` prints them, plus the listener.
fn list(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["name", "last"], 0)?;
    let workspace = workspace_id(ctx)?;
    let last = capture::number(ctx, "last")?.unwrap_or(20) as usize;
    let name = ctx.args.flag("name").map(|name| format!("webhook:{name}"));
    let mut payload = json!({ "origin": "webhook" });
    // The engine cuts to `last` before a name could pick, so a name filters here, over everything.
    if name.is_none() {
        payload["last"] = json!(last);
    }
    let mut answer = capture::summaries(ctx, &workspace, payload)?;
    if let Some(name) = name {
        let mut entries: Vec<Value> = answer["entries"].as_array().cloned().unwrap_or_default();
        entries.retain(|e| e["target"].as_str() == Some(&name));
        answer["matched"] = json!(entries.len());
        entries.truncate(last);
        answer["entries"] = json!(entries);
    }
    Ok(ctx.print(&answer, || {
        capture::lines_with(&answer, |e| {
            Some(e["target"].as_str().and_then(|t| t.strip_prefix("webhook:")).unwrap_or("-").to_string())
        })
    }))
}

/// `scheme://authority` of a URL, and what follows it.
fn split_origin(url: &str) -> (&str, &str) {
    let from = url.find("://").map_or(0, |at| at + 3);
    let end = url[from..].find(['/', '?', '#']).map_or(url.len(), |at| from + at);
    url.split_at(end)
}

/// Where a webhook is sent again: `--to` (host only: the entry's path and query go on it; with a path of its
/// own: used whole), else the listener's forward URL, else the URL it came in on.
fn replay_target(entry_url: &str, forward: Option<&str>, to: Option<&str>) -> Result<String> {
    let path = match split_origin(entry_url).1 {
        "" => "/",
        path => path,
    };
    if let Some(to) = to {
        let (origin, rest) = split_origin(to);
        if !to.contains("://") || origin.ends_with("://") {
            return Err(CliError::usage(format!("--to is a URL like http://127.0.0.1:4021, not \"{to}\"")));
        }
        return Ok(if matches!(rest, "" | "/") { format!("{origin}{path}") } else { to.to_string() });
    }
    // Like the listener's own forwarding: the request's path is appended to the forward URL's path.
    Ok(match forward {
        Some(forward) => {
            format!("{}{path}", forward.split(['?', '#']).next().unwrap_or_default().trim_end_matches('/'))
        }
        None => entry_url.to_string(),
    })
}

/// `webhooks replay <id> [--to <url>]`: the captured request again, with its method, headers and raw body bytes.
fn replay(ctx: &mut Context) -> Result<String> {
    ctx.accept(
        &["to", "body", "headers", "select", "out", "max-body", "raw", "quiet", "reveal", "expect-status", "expect"],
        1,
    )?;
    let wanted = need(ctx.args.positional(2), "the webhook's id (see: bifurc webhooks list)")?;
    let workspace = workspace_id(ctx)?;
    let id = webhook_id(ctx, &workspace, &wanted)?;
    let got = ctx.request("capture.get", json!({ "workspaceId": workspace, "id": id }))?;
    let Some(entry) = got.get("entry").filter(|entry| entry.is_object()) else {
        return Err(CliError::not_found(format!("no webhook \"{id}\" (see: bifurc webhooks list)")));
    };
    let to = ctx.args.flag("to").map(str::to_string);
    let mut forward = None;
    // shortcut: the forward URL is not stored on the entry, so a listener that has stopped is forgotten and the
    // replay goes to the original URL; keep the forward on the entry if that bites.
    if to.is_none()
        && let Some(name) = entry["target"].as_str().and_then(|t| t.strip_prefix("webhook:"))
    {
        let listeners = ctx.request("webhook.list", json!({}))?;
        forward = listeners["listeners"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|l| l["name"].as_str() == Some(name))
            .and_then(|l| l["forward"].as_str().map(str::to_string));
    }
    let url = replay_target(&js_template(entry.get("url")), forward.as_deref(), to.as_deref())?;
    // The body was read whole (no chunking), and the HTTP client sets its own host and length.
    let headers: serde_json::Map<String, Value> = entry["reqHeaders"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, _)| !["host", "content-length", "transfer-encoding"].contains(&name.to_lowercase().as_str()))
        .map(|(name, value)| (name.clone(), json!(js_template(Some(value)))))
        .collect();
    let payload = json!({
        "method": js_template(entry.get("method")), "url": url, "headers": headers,
        "body": entry["reqBody"].as_str().unwrap_or_default(),
    });
    let started = Instant::now();
    let mut sent = ctx.request("request.replay", payload)?;
    sent["durationMs"] = json!(started.elapsed().as_millis() as u64);
    present(ctx, &Response::from_rest(&sent))
}

/// `webhooks save <id> --as-request [--name x] [--folder f]`: a saved request like the webhook.
fn save(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["as-request", "name", "folder"], 1)?;
    if !ctx.args.has("as-request") {
        return Err(CliError::usage("webhooks save needs --as-request (see: bifurc webhooks --help)"));
    }
    let wanted = need(ctx.args.positional(2), "the webhook's id (see: bifurc webhooks list)")?;
    let workspace = workspace_id(ctx)?;
    let id = webhook_id(ctx, &workspace, &wanted)?;
    let built = ctx.request("capture.buildRequest", json!({ "workspaceId": workspace, "id": id }))?;
    let mut entity = built.get("request").cloned().unwrap_or(Value::Null);
    if let Some(name) = ctx.args.flag("name") {
        entity["name"] = json!(name);
    }
    if let Some(folder) = ctx.args.flag("folder") {
        let parts: Vec<&str> = folder.split('/').filter(|part| !part.is_empty()).collect();
        if parts.is_empty() {
            return Err(CliError::usage("--folder needs a folder name"));
        }
        entity["folderId"] = json!(crate::curl::folder_id(ctx, &workspace, "request", &parts)?);
    }
    let made =
        ctx.request("entity.create", json!({ "kind": "requests", "workspaceId": workspace, "entity": entity }))?;
    let saved = created(&made);
    Ok(ctx.print(&saved, || format!("Saved request \"{}\"", js_template(saved.get("name")))))
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

    fn hook(id: &str, name: Option<&str>, origin: &str) -> Value {
        json!({"id": id, "timestamp": 1_792_332_131_000_i64, "method": "POST", "origin": origin, "status": 200,
               "url": "http://127.0.0.1:4020/hooks/a?x=1", "durationMs": 3, "via": origin,
               "target": name.map(|n| format!("webhook:{n}"))})
    }

    /// Three webhooks (two of a listener named stripe, one unnamed) and a proxied request.
    fn buffer() -> Vec<Answer> {
        let entries = json!({"entries": [hook("mg1a1111", Some("stripe"), "webhook"), hook("mg1b2222", None, "webhook"),
            hook("mg1c3333", Some("stripe"), "webhook"), hook("mg1d4444", None, "server")], "matched": 4});
        vec![("config.get", Ok(config())), ("capture.list", Ok(entries))]
    }

    #[test]
    fn list_filters_by_listener_name_and_shows_it() {
        let (out, calls, _) = go(&["webhooks", "list"], buffer(), vec![]);
        let out = out.unwrap();
        assert!(out.lines().next().unwrap().ends_with("webhook  stripe"), "{out}");
        assert!(out.lines().nth(1).unwrap().ends_with("webhook  -"), "{out}");
        assert_eq!(
            payloads(&calls, "capture.list"),
            [json!({"workspaceId": "w1", "summary": true, "origin": "webhook", "last": 20})]
        );

        let (out, calls, _) = go(&["webhooks", "list", "--name", "stripe", "--last", "1"], buffer(), vec![]);
        let out = out.unwrap();
        assert!(out.starts_with("#1111") && out.ends_with("… 1 more — use --last or a filter"), "{out}");
        assert!(payloads(&calls, "capture.list")[0].get("last").is_none());
    }

    #[test]
    fn show_takes_a_webhook_and_refuses_another_entry() {
        let (out, _, _) = go(&["webhooks", "show", "d4444"], buffer(), vec![]);
        let err = out.unwrap_err();
        assert_eq!((err.code(), err.line().as_str()), (3, "bifurc: no webhook \"d4444\" (see: bifurc webhooks list)"));
        let mut answers = buffer();
        let entry = json!({"entry": {"id": "mg1a1111", "method": "POST", "url": "http://127.0.0.1:4020/hooks/a", "via": "webhook",
                                     "status": 200, "resStatus": 200, "resHeaders": {}, "resBody": ""}});
        answers.push(("capture.get", Ok(entry)));
        let (out, calls, _) = go(&["webhooks", "show", "#1111"], answers, vec![]);
        assert!(out.unwrap().starts_with("POST http://127.0.0.1:4020/hooks/a · webhook"));
        assert_eq!(payloads(&calls, "capture.get")[0]["id"], "mg1a1111");
    }

    #[test]
    fn the_replay_goes_to_the_forward_url_else_the_original_or_to() {
        let came = "http://127.0.0.1:4020/hooks/a?x=1";
        assert_eq!(replay_target(came, None, None).unwrap(), came);
        assert_eq!(
            replay_target(came, Some("http://api.localhost/webhooks/stripe/?y=2"), None).unwrap(),
            "http://api.localhost/webhooks/stripe/hooks/a?x=1"
        );
        for host_only in ["http://127.0.0.1:4021", "http://127.0.0.1:4021/"] {
            assert_eq!(
                replay_target(came, Some("http://f.test"), Some(host_only)).unwrap(),
                "http://127.0.0.1:4021/hooks/a?x=1"
            );
        }
        assert_eq!(replay_target(came, None, Some("https://app.test/in?z=1")).unwrap(), "https://app.test/in?z=1");
        assert_eq!(replay_target("http://h:1", None, Some("http://app.test")).unwrap(), "http://app.test/");
        assert_eq!(replay_target(came, None, Some("localhost:3000")).unwrap_err().code(), 2);
    }

    #[test]
    fn replay_sends_the_raw_body_and_headers_unchanged() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let raw = [0xff, 0xfe, 0x00, 0x80, b'{', 0xc3, 0xa9];
        let mut entry = hook("mg1a1111", Some("stripe"), "webhook");
        entry["reqBody"] = json!(STANDARD.encode(raw));
        entry["reqHeaders"] = json!({"Host": "127.0.0.1:4020", "Content-Length": "7", "stripe-signature": "t=1,v1=ab",
                                     "x-dup": "a, b", "content-type": "application/octet-stream"});
        let reply = json!({"status": 200, "headers": {"content-type": "application/json"}, "body": STANDARD.encode("{\"ok\":true}")});
        let mut answers = buffer();
        answers.extend([
            ("capture.get", Ok(json!({"entry": entry}))),
            ("webhook.list", Ok(json!({"listeners": [{"name": "stripe", "forward": "http://api.localhost/in"}]}))),
            ("request.replay", Ok(reply)),
        ]);
        let (out, calls, _) = go(&["webhooks", "replay", "1111"], answers.clone(), vec![]);
        assert!(out.unwrap().starts_with("200 OK"));
        let sent = &payloads(&calls, "request.replay")[0];
        assert_eq!(sent["url"], "http://api.localhost/in/hooks/a?x=1");
        assert_eq!(sent["method"], "POST");
        assert_eq!(sent["body"], STANDARD.encode(raw));
        assert_eq!(
            sent["headers"],
            json!({"stripe-signature": "t=1,v1=ab", "x-dup": "a, b", "content-type": "application/octet-stream"})
        );
        // --to skips the listener lookup.
        let (_, calls, _) =
            go(&["webhooks", "replay", "1111", "--to", "http://127.0.0.1:4021"], answers.clone(), vec![]);
        assert!(payloads(&calls, "webhook.list").is_empty());
        assert_eq!(payloads(&calls, "request.replay")[0]["url"], "http://127.0.0.1:4021/hooks/a?x=1");
        answers.insert(0, ("request.replay", Err(CliError::network("refused"))));
        let (out, _, _) = go(&["webhooks", "replay", "1111"], answers, vec![]);
        assert_eq!(out.unwrap_err().code(), 4);
    }

    #[test]
    fn save_builds_the_request_and_names_it() {
        let mut answers = buffer();
        answers.extend([
            ("capture.buildRequest", Ok(json!({"request": {"name": "POST /hooks/a", "method": "POST"}}))),
            ("entity.create", Ok(json!({"entity": {"id": "r1", "name": "Stripe hook"}}))),
        ]);
        let (out, calls, _) =
            go(&["webhooks", "save", "1111", "--as-request", "--name", "Stripe hook"], answers.clone(), vec![]);
        assert_eq!(out.unwrap(), "Saved request \"Stripe hook\"");
        assert_eq!(payloads(&calls, "entity.create")[0]["entity"]["name"], "Stripe hook");
        let (out, _, _) = go(&["webhooks", "save", "1111"], answers, vec![]);
        assert_eq!(out.unwrap_err().code(), 2);
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
