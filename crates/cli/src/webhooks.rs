//! `webhooks listen|stop|list|show|replay|save|fixtures|trigger`: the engine's webhook receiver (see `engine/src/webhook.rs`). Without `--detach`
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
        Some("fixtures") => {
            ctx.accept(&[], 0)?;
            let listed = ctx.request("webhook.fixtures", json!({}))?;
            Ok(ctx.print(&listed, || {
                let rows = listed["fixtures"].as_array().into_iter().flatten();
                rows.map(|f| format!("{}  {}", js_template(f.get("name")), js_template(f.get("description"))))
                    .collect::<Vec<_>>()
                    .join(
                        "
",
                    )
            }))
        }
        Some("trigger") => trigger(ctx),
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

/// A request URL aimed at a listener's forward URL: the request's path is appended to the forward URL's path,
/// like the listener's own forwarding.
fn forwarded_url(entry_url: &str, forward: &str) -> String {
    let path = match split_origin(entry_url).1 {
        "" => "/",
        path => path,
    };
    format!("{}{path}", forward.split(['?', '#']).next().unwrap_or_default().trim_end_matches('/'))
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
    let mut payload = json!({ "workspaceId": workspace, "id": id });
    if let Some(to) = ctx.args.flag("to") {
        payload["to"] = json!(to);
    }
    let started = Instant::now();
    let mut sent = ctx.request("webhook.replay", payload).map_err(|err| match err {
        CliError::Engine { code, message } if code == "BAD_REQUEST" => CliError::usage(message),
        CliError::Engine { code, message } if code == "NOT_FOUND" => CliError::not_found(message),
        other => other,
    })?;
    sent["durationMs"] = json!(started.elapsed().as_millis() as u64);
    present(ctx, &Response::from_rest(&sent))
}

/// `2026-10-18T09:30:00Z` for unix seconds (civil-from-days).
pub(crate) fn format_time(secs: u64) -> String {
    let (days, rem) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// `webhooks trigger <fixture> [--to <url>] [--name x] [--vault v [--secret KEY]] [--patch '<path>=<value>']`.
fn trigger(ctx: &mut Context) -> Result<String> {
    ctx.accept(
        &[
            "to",
            "name",
            "vault",
            "secret",
            "patch",
            "passphrase-stdin",
            "body",
            "headers",
            "select",
            "out",
            "max-body",
            "raw",
            "quiet",
            "reveal",
            "expect-status",
            "expect",
        ],
        1,
    )?;
    let wanted = need(ctx.args.positional(2), "the fixture's name (see: bifurc webhooks fixtures)")?;
    let vault = ctx.args.flag("vault").map(str::to_string);
    if vault.is_none() && ctx.args.flag("secret").is_some() {
        return Err(CliError::usage("--secret needs --vault <vault> to read the secret from"));
    }
    let mut patches = Vec::new();
    for patch in ctx.args.flags("patch") {
        let (path, value) = crate::select::patch_edit(patch).map_err(CliError::usage)?;
        patches.push(json!({ "path": path, "value": value }));
    }
    let workspace = workspace_id(ctx)?;
    let mut payload = json!({ "workspaceId": workspace, "fixture": wanted, "patch": patches });
    for flag in ["to", "name", "secret"] {
        if let Some(value) = ctx.args.flag(flag) {
            payload[flag] = json!(value);
        }
    }
    if let Some(vault) = &vault {
        // The engine reads the signing secret itself, so only the unsealing happens here.
        crate::vault::unseal_named(ctx, &workspace, vault)?;
        payload["vault"] = json!(vault);
    }
    let started = Instant::now();
    let mut sent = ctx.request("webhook.trigger", payload).map_err(|err| match err {
        CliError::Engine { code, message } if code == "BAD_REQUEST" => CliError::usage(message),
        CliError::Engine { code, message } if code == "NOT_FOUND" => CliError::not_found(message),
        other => other,
    })?;
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
    // Aim it at the handler the listener forwarded to, not the listener's own (short-lived) port.
    let got = ctx.request("capture.get", json!({ "workspaceId": workspace, "id": id }))?;
    if let (Some(forward), Some(url)) = (got["entry"]["forward"].as_str(), entity["url"].as_str()) {
        entity["url"] = json!(forwarded_url(url, forward));
    }
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
    use base64::{Engine as _, engine::general_purpose::STANDARD};

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
    fn replay_asks_the_engine_and_prints_its_answer_as_before() {
        let reply = json!({"status": 200, "headers": {"content-type": "application/json"}, "body": STANDARD.encode("{\"ok\":true}")});
        let mut answers = buffer();
        answers.push(("webhook.replay", Ok(reply)));
        let (out, calls, _) =
            go(&["webhooks", "replay", "1111", "--to", "http://127.0.0.1:4021"], answers.clone(), vec![]);
        assert!(out.unwrap().starts_with("200 OK"));
        assert_eq!(
            payloads(&calls, "webhook.replay"),
            [json!({"workspaceId": "w1", "id": "mg1a1111", "to": "http://127.0.0.1:4021"})]
        );
        let refused = |code: &str, message: &str| Err(CliError::Engine { code: code.into(), message: message.into() });
        for (code, exit, line) in [
            ("BAD_REQUEST", 2, "bifurc: --to is a URL"),
            ("NOT_FOUND", 3, "bifurc: no webhook"),
            ("UPSTREAM_FAILED", 4, "bifurc: UPSTREAM_FAILED: refused"),
        ] {
            let mut answers = buffer();
            answers.push((
                "webhook.replay",
                refused(code, line.trim_start_matches("bifurc: UPSTREAM_FAILED: ").trim_start_matches("bifurc: ")),
            ));
            let err = go(&["webhooks", "replay", "1111"], answers, vec![]).0.unwrap_err();
            assert_eq!((err.code(), err.line().as_str()), (exit, line));
        }
    }

    #[test]
    fn save_builds_the_request_and_names_it() {
        let mut answers = buffer();
        answers.extend([
            (
                "capture.buildRequest",
                Ok(json!({"request": {"name": "POST /hooks/a", "method": "POST", "url": "http://127.0.0.1:4020/hooks/a?x=1"}})),
            ),
            ("capture.get", Ok(json!({"entry": {"forward": "http://api.localhost/in"}}))),
            ("entity.create", Ok(json!({"entity": {"id": "r1", "name": "Stripe hook"}}))),
        ]);
        let (out, calls, _) =
            go(&["webhooks", "save", "1111", "--as-request", "--name", "Stripe hook"], answers.clone(), vec![]);
        assert_eq!(out.unwrap(), "Saved request \"Stripe hook\"");
        let saved = &payloads(&calls, "entity.create")[0]["entity"];
        assert_eq!(
            (saved["name"].clone(), saved["url"].clone()),
            (json!("Stripe hook"), json!("http://api.localhost/in/hooks/a?x=1"))
        );
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

    /// Answers for a trigger: a vault, and the engine's reply to `webhook.trigger`.
    fn triggering() -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("vault.list", Ok(json!([{"id": "v1", "name": "hooks", "state": "unsealed"}]))),
            ("webhook.trigger", Ok(json!({"status": 200, "headers": {}, "body": ""}))),
        ]
    }

    #[test]
    fn trigger_asks_the_engine_and_prints_its_answer_as_before() {
        let argv = ["webhooks", "trigger", "stripe", "--vault", "hooks", "--name", "s", "--patch", ".a.b[-1]=9"];
        let (out, calls, _) = go(&argv, triggering(), vec![]);
        assert!(out.unwrap().starts_with("200 OK"));
        assert_eq!(
            payloads(&calls, "webhook.trigger"),
            [json!({"workspaceId": "w1", "fixture": "stripe", "name": "s", "vault": "hooks",
                "patch": [{"path": ["a", "b", -1], "value": 9}]})]
        );
        let (_, calls, _) = go(&["webhooks", "trigger", "github.push", "--to", "http://h:1/x"], triggering(), vec![]);
        assert_eq!(
            payloads(&calls, "webhook.trigger"),
            [json!({"workspaceId": "w1", "fixture": "github.push", "to": "http://h:1/x", "patch": []})]
        );
        assert!(payloads(&calls, "vault.list").is_empty());
    }

    #[test]
    fn trigger_maps_the_engines_refusals_to_exit_codes() {
        let refused = |code: &str, message: &str| Err(CliError::Engine { code: code.into(), message: message.into() });
        for (code, exit) in [("BAD_REQUEST", 2), ("NOT_FOUND", 3), ("UPSTREAM_FAILED", 4)] {
            let mut answers = triggering();
            answers.pop();
            answers.push(("webhook.trigger", refused(code, "no")));
            assert_eq!(go(&["webhooks", "trigger", "stripe"], answers, vec![]).0.unwrap_err().code(), exit);
        }
        let err = |argv: &[&str]| go(argv, triggering(), vec![]).0.unwrap_err();
        assert_eq!(err(&["webhooks", "trigger", "stripe", "--secret", "K"]).code(), 2);
        assert_eq!(err(&["webhooks", "trigger", "stripe", "--patch", "nope"]).code(), 2);
    }

    #[test]
    fn fixtures_lists_both() {
        let listed = json!({"fixtures": [
            {"name": "stripe.checkout.session.completed", "description": "A paid Stripe Checkout session"},
            {"name": "github.push", "description": "A GitHub push event"}]});
        let (out, _, _) = go(&["webhooks", "fixtures"], vec![("webhook.fixtures", Ok(listed))], vec![]);
        assert_eq!(
            out.unwrap(),
            "stripe.checkout.session.completed  A paid Stripe Checkout session
github.push  A GitHub push event"
        );
    }
}
