//! `capture list|show|watch`, and the entry ids every capture verb accepts: a full id, `#tail` or `tail`. Ids
//! start with the time, so entries close in time share their first characters; the end is what tells them apart.

use std::{
    collections::HashSet,
    io::Write,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use crate::{
    commands::{Context, need, workspace_id},
    error::{CliError, Result},
    format::{aligned, js_template},
    response::{Response, decode_base64, mask_header, present, preview, revealed, size},
};

/// The filters `list` and `watch` share.
const FILTERS: [&str; 6] = ["status", "method", "host", "path", "origin", "grep"];

/// The events that say the buffer has new entries.
const EVENT: &str = "event.log.entry";

pub(crate) fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

/// What an entry came from, in a word: real (the server), mock, mapped (a mapping), rule or error.
fn origin_word(entry: &Value) -> &'static str {
    match entry.get("via").and_then(Value::as_str) {
        Some("mock") => "mock",
        Some("mapping") => "mapped",
        Some("rule") => "rule",
        Some("error") => "error",
        _ => "real",
    }
}

/// `HH:MM:SS` of a time in ms since the epoch, in UTC (a local clock needs a platform call per OS), with a `Z`.
fn clock(ms: i64) -> String {
    let seconds = ms.div_euclid(1000).rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}Z", seconds / 3600, seconds % 3600 / 60, seconds % 60)
}

/// `host/path` without the scheme; a query or fragment shows as `?…`.
pub(crate) fn short_url(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    match rest.find(['?', '#']) {
        Some(at) => format!("{}?…", &rest[..at]),
        None => rest.to_string(),
    }
}

/// The shortest end of each id, at least 4 characters, that no other of `ids` ends with.
fn tails(ids: &[&str]) -> Vec<String> {
    ids.iter()
        .map(|id| {
            (4..=id.len())
                .filter_map(|n| id.get(id.len() - n..))
                .find(|tail| ids.iter().filter(|other| other.ends_with(tail)).count() == 1)
                .unwrap_or(id)
                .to_string()
        })
        .collect()
}

/// The full id `wanted` names among `ids`.
fn entry_id(ids: &[String], wanted: &str) -> Result<String> {
    let tail = wanted.strip_prefix('#').unwrap_or(wanted);
    if let Some(exact) = ids.iter().find(|id| *id == tail) {
        return Ok(exact.clone());
    }
    let hits: Vec<&String> = ids.iter().filter(|id| id.ends_with(tail)).collect();
    match hits.as_slice() {
        [] => Err(CliError::not_found(format!("no captured entry \"{wanted}\" (see: bifurc capture list)"))),
        [one] => Ok((*one).clone()),
        many => {
            let listed: Vec<&str> = many.iter().map(|id| id.as_str()).collect();
            Err(CliError::usage(format!(
                "{} entries end with \"{tail}\": {} — use more of the id",
                many.len(),
                listed.join(", ")
            )))
        }
    }
}

/// The entries of the workspace in the engine's light form, newest first.
pub(crate) fn summaries(ctx: &mut Context, workspace: &str, mut payload: Value) -> Result<Value> {
    payload["workspaceId"] = json!(workspace);
    payload["summary"] = json!(true);
    ctx.request("capture.list", payload)
}

/// The full ids the entries `wanted` name (each a full id, `#tail` or `tail`).
pub(crate) fn entry_ids(ctx: &mut Context, workspace: &str, wanted: &[String]) -> Result<Vec<String>> {
    let all = summaries(ctx, workspace, json!({}))?;
    let ids: Vec<String> = all["entries"].as_array().into_iter().flatten().map(|e| js_template(e.get("id"))).collect();
    wanted.iter().map(|one| entry_id(&ids, one)).collect()
}

/// The engine's `since`: `30s`, `2m`, `1h` ago, or just after the entry `#tail`.
fn since(ctx: &mut Context, workspace: &str, raw: &str) -> Result<i64> {
    if raw.starts_with('#') {
        let all = summaries(ctx, workspace, json!({}))?;
        let entries = all["entries"].as_array().map(Vec::as_slice).unwrap_or_default();
        let ids: Vec<String> = entries.iter().map(|e| js_template(e.get("id"))).collect();
        let id = entry_id(&ids, raw)?;
        let at = entries.iter().find(|e| js_template(e.get("id")) == id).and_then(|e| e["timestamp"].as_i64());
        return Ok(at.unwrap_or(0) + 1);
    }
    let (count, unit) = raw.split_at(raw.len().saturating_sub(1));
    match (count.parse::<i64>(), unit) {
        (Ok(n), "s") => Ok(now_ms() - n * 1000),
        (Ok(n), "m") => Ok(now_ms() - n * 60_000),
        (Ok(n), "h") => Ok(now_ms() - n * 3_600_000),
        _ => Err(CliError::usage(format!("--since is 30s, 2m, 1h or #<id tail>, not \"{raw}\""))),
    }
}

/// The payload fields of the filters given.
pub(crate) fn filters(ctx: &Context) -> Result<Value> {
    let mut payload = json!({});
    for name in FILTERS.iter().filter(|name| **name != "origin") {
        if let Some(value) = ctx.args.flag(name) {
            payload[*name] = json!(value);
        }
    }
    if let Some(origin) = ctx.args.flag("origin") {
        payload["via"] = json!(match origin {
            "real" => "proxy,rfc6761",
            "mock" => "mock",
            "mapped" => "mapping",
            "rule" => "rule",
            "error" => "error",
            other => {
                return Err(CliError::usage(format!(
                    "--origin must be one of real, mock, mapped, rule, error, not \"{other}\""
                )));
            }
        });
    }
    Ok(payload)
}

/// A whole number option.
fn number(ctx: &Context, flag: &str) -> Result<Option<u64>> {
    ctx.args
        .flag(flag)
        .map(|raw| raw.parse().map_err(|_| CliError::usage(format!("--{flag} must be a whole number, not \"{raw}\""))))
        .transpose()
}

/// `capture list`: one line per entry, newest first.
pub fn list(ctx: &mut Context) -> Result<String> {
    ctx.accept(&["last", "since", "status", "method", "host", "path", "origin", "grep"], 0)?;
    let workspace = workspace_id(ctx)?;
    let mut payload = filters(ctx)?;
    payload["last"] = json!(number(ctx, "last")?.unwrap_or(20));
    if let Some(raw) = ctx.args.flag("since") {
        payload["since"] = json!(since(ctx, &workspace, raw)?);
    }
    let answer = summaries(ctx, &workspace, payload)?;
    Ok(ctx.print(&answer, || {
        let entries = answer["entries"].as_array().map(Vec::as_slice).unwrap_or_default();
        if entries.is_empty() {
            return "(none)".to_string();
        }
        let ids: Vec<String> = entries.iter().map(|e| js_template(e.get("id"))).collect();
        let tails = tails(&ids.iter().map(String::as_str).collect::<Vec<_>>());
        let rows: Vec<Vec<String>> = entries
            .iter()
            .zip(tails)
            .map(|(entry, tail)| {
                vec![
                    format!("#{tail}"),
                    clock(entry["timestamp"].as_i64().unwrap_or(0)),
                    js_template(entry.get("method")),
                    short_url(&js_template(entry.get("url"))),
                    entry.get("status").and_then(Value::as_u64).map_or("ERR".to_string(), |s| s.to_string()),
                    entry.get("durationMs").and_then(Value::as_i64).map_or("-".to_string(), |ms| format!("{ms}ms")),
                    origin_word(entry).to_string(),
                ]
            })
            .collect();
        let mut text = aligned(&rows);
        let cut = (answer["matched"].as_u64().unwrap_or(0) as usize).saturating_sub(entries.len());
        if cut > 0 {
            text.push_str(&format!("\n… {cut} more — use --last or a filter"));
        }
        text
    }))
}

/// A request body as `> ` lines, cut at `--max-body`; a binary one is not shown.
fn request_body(entry: &Value, limit: usize) -> Vec<String> {
    let bytes = decode_base64(entry.get("reqBody"));
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return vec![format!("> binary body not shown ({})", size(bytes.len()))];
    };
    let (shown, left) = preview(text, limit);
    let mut lines: Vec<String> = shown.trim_end_matches(['\r', '\n']).lines().map(|line| format!("> {line}")).collect();
    if left > 0 {
        lines.push(format!("> … {} truncated — use --max-body 0", size(left)));
    }
    lines
}

/// `capture show <entry>`: the request line, then the response as every one-shot response prints.
pub fn show(ctx: &mut Context) -> Result<String> {
    let flags = ["body", "headers", "req-body", "req-headers", "select", "out", "max-body", "raw", "quiet"];
    ctx.accept(&[&flags[..], &["expect", "expect-status", "reveal"]].concat(), 1)?;
    let wanted = need(ctx.args.positional(2), "the entry's id (see: bifurc capture list)")?;
    let workspace = workspace_id(ctx)?;
    let id = entry_ids(ctx, &workspace, &[wanted])?.remove(0);
    let got = ctx.request("capture.get", json!({ "workspaceId": workspace, "id": id }))?;
    let Some(entry) = got.get("entry").filter(|entry| entry.is_object()) else {
        return Err(CliError::not_found(format!("no captured entry \"{id}\" (see: bifurc capture list)")));
    };
    let args = ctx.args;
    let (method, url, origin) = (js_template(entry.get("method")), js_template(entry.get("url")), origin_word(entry));
    let limit = number(ctx, "max-body")?.map_or(2048, |n| n as usize);
    let mut before = Vec::new();
    // `--raw` is the body and nothing else, `--quiet` nothing; `--select` is only the values.
    if !args.has("raw") && !args.has("quiet") {
        if !args.has("select") {
            before.push(format!("{method} {url} · {origin}"));
        }
        if args.has("req-headers") {
            let reveal = revealed(args);
            let headers = entry.get("reqHeaders").and_then(Value::as_object).into_iter().flatten();
            before.extend(headers.map(|(name, value)| {
                let value = js_template(Some(value));
                format!("> {name}: {}", if reveal { value } else { mask_header(name, &value) })
            }));
        }
        if args.has("req-body") {
            before.extend(request_body(entry, limit));
        }
    }
    let (text, passed) = match present(ctx, &Response::from_capture(entry)) {
        Ok(text) => (text, true),
        Err(CliError::Failed(text)) => (text, false),
        Err(other) => return Err(other),
    };
    let out = if ctx.json {
        // The JSON of the response, with what it answered.
        let mut out: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
        out["method"] = json!(method);
        out["url"] = json!(url);
        out["origin"] = json!(origin);
        crate::format::json(&out)
    } else {
        before.extend(Some(text).filter(|text| !text.is_empty()));
        before.join("\n")
    };
    if passed { Ok(out) } else { Err(CliError::Failed(out)) }
}

/// `capture watch`: one JSON line per new matching entry (the light form plus `tail`) until `--count` of them,
/// `--timeout` seconds, or the output closes.
pub fn watch(ctx: &mut Context, out: &mut dyn Write) -> Result<String> {
    ctx.accept(&["status", "method", "host", "path", "origin", "grep", "count", "timeout"], 0)?;
    let count = number(ctx, "count")?.filter(|n| *n > 0);
    let timeout = ctx.args.flag("timeout").map(|raw| {
        raw.parse::<f64>()
            .ok()
            .filter(|s| *s > 0.0)
            .ok_or_else(|| CliError::usage(format!("--timeout must be a number of seconds, not \"{raw}\"")))
    });
    let timeout = timeout.transpose()?;
    let workspace = workspace_id(ctx)?;
    let mut payload = filters(ctx)?;
    payload["since"] = json!(now_ms());
    ctx.request("subscribe", json!({ "events": [EVENT] }))?;
    if let Some(seconds) = timeout {
        // shortcut: the wait for an event cannot time out, so a timer ends the process; fine for a command
        // whose only job is this loop, to be reworked if the session ever gets a read timeout.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(seconds));
            std::process::exit(0);
        });
    }
    let (mut seen, mut printed) = (HashSet::new(), 0);
    loop {
        // An event only says the buffer changed; the engine's own filters pick what matches.
        let answer = summaries(ctx, &workspace, payload.clone())?;
        for entry in answer["entries"].as_array().into_iter().flatten().rev() {
            let id = js_template(entry.get("id"));
            if !seen.insert(id.clone()) {
                continue;
            }
            let mut line = entry.clone();
            line["tail"] = json!(id.get(id.len().saturating_sub(4)..).unwrap_or(&id));
            if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
                return Ok(String::new());
            }
            printed += 1;
            if count == Some(printed) {
                return Ok(String::new());
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
    use base64::{Engine as _, engine::general_purpose::STANDARD};

    fn summary(id: &str, ts: i64, method: &str, url: &str, status: Value, ms: i64, via: &str) -> Value {
        json!({"id": id, "timestamp": ts, "method": method, "url": url, "status": status, "durationMs": ms, "via": via})
    }

    /// 14:02:11 UTC on some day.
    const AT: i64 = 1_792_332_131_000;

    fn buffer() -> Value {
        json!({"entries": [
            summary("mgv2k3f9", AT, "POST", "http://api.localhost/orders?debug=1", json!(422), 38, "mock"),
            summary("mgv2k3a1", AT - 1000, "GET", "http://api.localhost/cart", json!(200), 12, "proxy"),
            summary("mgv1x3f9", AT - 5000, "GET", "http://down.localhost/", Value::Null, 3, "error"),
        ], "matched": 146, "paused": false})
    }

    fn go(argv: &[&str], answers: Vec<Answer>) -> (Result<String>, Vec<(String, Value)>) {
        run_with_events(argv, answers, vec![], |ctx| match argv[1] {
            "list" => list(ctx),
            _ => show(ctx),
        })
    }

    fn engine(buffer: Value) -> Vec<Answer> {
        vec![("config.get", Ok(config())), ("capture.list", Ok(buffer))]
    }

    #[test]
    fn list_is_one_aligned_line_per_entry_with_the_shortest_unique_tail() {
        let (out, calls) = go(&["capture", "list"], engine(buffer()));
        assert_eq!(
            out.unwrap(),
            "#k3f9  14:02:11Z  POST  api.localhost/orders?…  422  38ms  mock\n\
             #k3a1  14:02:10Z  GET   api.localhost/cart      200  12ms  real\n\
             #x3f9  14:02:06Z  GET   down.localhost/         ERR  3ms   error\n\
             … 143 more — use --last or a filter"
        );
        assert_eq!(payloads(&calls, "capture.list"), [json!({"workspaceId": "w1", "summary": true, "last": 20})]);
    }

    #[test]
    fn tails_are_four_characters_unless_that_is_not_unique() {
        assert_eq!(tails(&["aaa1k3f9", "aaa2k3f9", "aaa2x1b2"]), ["1k3f9", "2k3f9", "x1b2"]);
        assert_eq!(tails(&["ab"]), ["ab"]);
    }

    #[test]
    fn every_filter_goes_to_the_engine() {
        let argv = [
            "capture", "list", "--last", "5", "--status", "!2xx", "--method", "POST", "--host", "api.*", "--path",
            "/orders*", "--origin", "real", "--grep", "qty",
        ];
        let (_, calls) = go(&argv, engine(json!({"entries": []})));
        assert_eq!(
            payloads(&calls, "capture.list")[0],
            json!({"workspaceId": "w1", "summary": true, "last": 5, "status": "!2xx", "method": "POST",
                   "host": "api.*", "path": "/orders*", "via": "proxy,rfc6761", "grep": "qty"})
        );
        let (out, calls) = go(&["capture", "list", "--origin", "mapped"], engine(json!({"entries": []})));
        assert_eq!(out.unwrap(), "(none)");
        assert_eq!(payloads(&calls, "capture.list")[0]["via"], "mapping");
        let (out, _) = go(&["capture", "list", "--origin", "x"], engine(json!({})));
        assert_eq!(out.unwrap_err().code(), 2);
    }

    #[test]
    fn since_is_a_time_ago_or_just_after_an_entry() {
        let (_, calls) = go(&["capture", "list", "--since", "2m"], engine(json!({"entries": []})));
        let sent = payloads(&calls, "capture.list")[0]["since"].as_i64().unwrap();
        assert!((now_ms() - 120_000 - sent).abs() < 5000);
        let (_, calls) = go(&["capture", "list", "--since", "#3a1"], engine(buffer()));
        assert_eq!(payloads(&calls, "capture.list")[1]["since"], AT - 1000 + 1);
        let (out, _) = go(&["capture", "list", "--since", "soon"], engine(json!({})));
        assert_eq!(out.unwrap_err().code(), 2);
    }

    #[test]
    fn a_tail_names_one_entry_else_it_is_missing_or_ambiguous() {
        let ids: Vec<String> = ["mgv2k3f9", "mgv2k3a1", "mgv1x3f9"].map(String::from).to_vec();
        for wanted in ["mgv2k3a1", "#3a1", "3a1", "k3a1"] {
            assert_eq!(entry_id(&ids, wanted).unwrap(), "mgv2k3a1", "{wanted}");
        }
        let missing = entry_id(&ids, "#zzzz").unwrap_err();
        assert_eq!(missing.code(), 3);
        let ambiguous = entry_id(&ids, "3f9").unwrap_err();
        assert_eq!(ambiguous.code(), 2);
        assert!(ambiguous.line().contains("mgv2k3f9, mgv1x3f9"), "{}", ambiguous.line());
    }

    fn exchange(status: u16) -> Value {
        let body = |text: &str| STANDARD.encode(text);
        json!({"entry": {
            "id": "mgv2k3f9", "ts": AT, "method": "POST", "url": "http://api.localhost/orders", "via": "mock",
            "status": status, "resStatus": status, "durationMs": 38,
            "reqHeaders": {"Authorization": "Bearer abcdef", "Content-Type": "application/json"},
            "reqBody": body("{\"qty\":0}"),
            "resHeaders": {"Content-Type": "application/json", "Set-Cookie": "sid=1"},
            "resBody": body("{\"error\":{\"code\":\"qty\"},\"detail\":\"too small\"}"),
        }})
    }

    fn shown(status: u16, extra: &[&str]) -> (Result<String>, Vec<(String, Value)>) {
        let argv = [&["capture", "show", "#k3f9"][..], extra].concat();
        let mut answers = engine(buffer());
        answers.push(("capture.get", Ok(exchange(status))));
        go(&argv, answers)
    }

    #[test]
    fn show_is_the_request_line_then_the_response_summary() {
        // 422 is an error status, so the body follows the summary.
        let (out, calls) = shown(422, &[]);
        assert_eq!(
            out.unwrap(),
            "POST http://api.localhost/orders · mock\n422 Unprocessable Entity · 38ms · 45B json\n\
             {\"error\":{\"code\":\"qty\"},\"detail\":\"too small\"}"
        );
        assert_eq!(payloads(&calls, "capture.get"), [json!({"workspaceId": "w1", "id": "mgv2k3f9"})]);
    }

    #[test]
    fn show_takes_the_output_flags_of_a_response() {
        let (out, _) = shown(200, &["--select", ".error.code"]);
        assert_eq!(out.unwrap(), "qty");
        let (out, _) = shown(200, &["--body", "--max-body", "10"]);
        assert!(out.unwrap().ends_with("{\"error\":{\n… 35B truncated — use --out or --select"));
        let (out, _) = shown(200, &["--headers"]);
        let out = out.unwrap();
        assert!(out.contains("Content-Type: application/json") && out.contains("Set-Cookie: ••••(5)"), "{out}");
        let (out, _) = shown(422, &["--expect-status", "200"]);
        assert!(matches!(out, Err(CliError::Failed(text)) if text.contains("✗")));
    }

    #[test]
    fn show_prints_the_request_with_secrets_masked() {
        let (out, _) = shown(200, &["--req-headers", "--req-body", "--select", ".detail"]);
        assert_eq!(
            out.unwrap(),
            "> Authorization: ••••(13)\n> Content-Type: application/json\n> {\"qty\":0}\ntoo small"
        );
        let (out, _) = shown(200, &["--req-body", "--max-body", "4"]);
        assert!(out.unwrap().contains("> {\"qt\n> … 5B truncated"));
        let (out, _) = shown(200, &["--json"]);
        let out: Value = serde_json::from_str(&out.unwrap()).unwrap();
        assert_eq!((out["status"].clone(), out["origin"].clone()), (json!(200), json!("mock")));
    }

    #[test]
    fn watch_prints_each_new_entry_once_as_a_json_line_and_stops_at_count() {
        let argv = ["capture", "watch", "--status", "4xx", "--count", "2"];
        let fresh = json!({"entries": [
            summary("mgv2k3f9", AT, "POST", "http://api.localhost/orders", json!(422), 38, "mock"),
            summary("mgv2k3a1", AT - 1000, "GET", "http://api.localhost/cart", json!(404), 12, "proxy"),
        ]});
        let answers = vec![("config.get", Ok(config())), ("subscribe", Ok(json!({}))), ("capture.list", Ok(fresh))];
        let mut out = Vec::new();
        let (done, calls) =
            run_with_events(&argv, answers, vec![json!({"event": EVENT, "payload": {}})], |ctx| watch(ctx, &mut out));
        assert_eq!(done.unwrap(), "");
        let lines: Vec<Value> =
            String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.iter().map(|l| l["tail"].as_str().unwrap()).collect::<Vec<_>>(), ["k3a1", "k3f9"]);
        assert_eq!(payloads(&calls, "subscribe"), [json!({"events": [EVENT]})]);
        assert_eq!(payloads(&calls, "capture.list")[0]["status"], "4xx");
    }
}
