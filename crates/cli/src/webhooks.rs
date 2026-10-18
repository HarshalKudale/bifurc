//! `webhooks listen|stop|list|show|replay|save|fixtures|trigger`: the engine's webhook receiver (see `engine/src/webhook.rs`). Without `--detach`
//! `listen` blocks and prints a line per inbound request; the engine ends the listener with this command's
//! connection, so Ctrl+C or a killed CI step never leaves the port busy.

use std::{collections::HashSet, io::Write, time::Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    capture,
    commands::{Context, created, find_one, need, unknown_verb, workspace_id},
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
            let rows: Vec<Value> =
                FIXTURES.iter().map(|(name, about)| json!({ "name": name, "description": about })).collect();
            Ok(ctx.print(&json!({ "fixtures": rows }), || {
                FIXTURES.iter().map(|(name, about)| format!("{name}  {about}")).collect::<Vec<_>>().join("\n")
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
    // The entry's own forward (new entries); older entries fall back to the running listener's.
    let mut forward = entry["forward"].as_str().map(str::to_string);
    let mut stopped = None;
    if to.is_none()
        && forward.is_none()
        && let Some(name) = entry["target"].as_str().and_then(|t| t.strip_prefix("webhook:"))
    {
        let listeners = ctx.request("webhook.list", json!({}))?;
        let running =
            listeners["listeners"].as_array().into_iter().flatten().find(|l| l["name"].as_str() == Some(name));
        forward = running.and_then(|l| l["forward"].as_str().map(str::to_string));
        if running.is_none() {
            stopped = Some(name.to_string());
        }
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
    send(ctx, &js_template(entry.get("method")), &url, &headers, entry["reqBody"].as_str().unwrap_or_default()).map_err(
        |err| match (stopped, err.code()) {
            // Only a listener without a stored forward lands here: its own port is what refused.
            (Some(name), 4) => CliError::network(format!("{err}; the listener \"{name}\" is stopped, use --to <url>")),
            _ => err,
        },
    )
}

/// Send `method url` with these headers and a base64 body through `request.replay` and print the answer.
fn send(
    ctx: &mut Context,
    method: &str,
    url: &str,
    headers: &serde_json::Map<String, Value>,
    body: &str,
) -> Result<String> {
    let payload = json!({ "method": method, "url": url, "headers": headers, "body": body });
    let started = Instant::now();
    let mut sent = ctx.request("request.replay", payload)?;
    sent["durationMs"] = json!(started.elapsed().as_millis() as u64);
    present(ctx, &Response::from_rest(&sent))
}

const FIXTURES: [(&str, &str); 2] = [
    ("stripe.checkout.session.completed", "A paid Stripe Checkout session (checkout.session.completed event)"),
    ("github.push", "A GitHub push event with one commit"),
];

fn unix_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

/// `len` random-looking characters of `alphabet`.
// shortcut: seeded from the clock (xorshift), not a CSPRNG; fine for fake ids, use a real RNG if one is ever needed.
fn random_chars(alphabet: &str, len: usize) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |s| s.as_nanos() as u64);
    let mut x = (nanos ^ COUNTER.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed)) | 1;
    let chars: Vec<char> = alphabet.chars().collect();
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            chars[(x % chars.len() as u64) as usize]
        })
        .collect()
}

const ALNUM: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const HEX: &str = "0123456789abcdef";

/// The event of a fixture, with fresh ids and times.
fn fixture(name: &str, now: u64) -> Value {
    if name == "github.push" {
        let (before, after) = (random_chars(HEX, 40), random_chars(HEX, 40));
        let commit = json!({ "id": after, "message": "Update README", "timestamp": format_time(now),
            "url": format!("https://github.com/octo/demo/commit/{after}"),
            "author": { "name": "Octo Cat", "email": "octocat@example.com", "username": "octocat" },
            "added": [], "removed": [], "modified": ["README.md"] });
        return json!({ "ref": "refs/heads/main", "before": before, "after": after,
            "repository": { "id": 100_000_001, "name": "demo", "full_name": "octo/demo", "private": false,
                "owner": { "login": "octo", "id": 1_000_001 }, "html_url": "https://github.com/octo/demo",
                "default_branch": "main" },
            "pusher": { "name": "octocat", "email": "octocat@example.com" },
            "sender": { "login": "octocat", "id": 583_231 },
            "created": false, "deleted": false, "forced": false,
            "compare": format!("https://github.com/octo/demo/compare/{}...{}", &before[..12], &after[..12]),
            "commits": [commit.clone()], "head_commit": commit });
    }
    json!({ "id": format!("evt_{}", random_chars(ALNUM, 24)), "object": "event", "api_version": "2024-06-20",
        "created": now, "type": "checkout.session.completed", "livemode": false, "pending_webhooks": 1,
        "request": { "id": null, "idempotency_key": null },
        "data": { "object": { "id": format!("cs_test_{}", random_chars(ALNUM, 40)), "object": "checkout.session",
            "amount_subtotal": 2000, "amount_total": 2000, "currency": "usd",
            "customer": format!("cus_{}", random_chars(ALNUM, 14)),
            "customer_details": { "email": "jenny@example.com", "name": "Jenny Rosen" },
            "mode": "payment", "payment_status": "paid", "status": "complete", "metadata": {},
            "payment_intent": format!("pi_{}", random_chars(ALNUM, 24)), "client_reference_id": null,
            "created": now, "success_url": "https://example.com/success" } } })
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

/// RFC 2104 HMAC over SHA-256.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.map(|b| b ^ byte);
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(data).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where a trigger goes: `--to`, else the one running listener (or `--name`) on its own address.
fn trigger_target(ctx: &mut Context) -> Result<String> {
    if let Some(to) = ctx.args.flag("to") {
        return Ok(to.to_string());
    }
    let listed = ctx.request("webhook.list", json!({}))?;
    let all: Vec<&Value> = listed["listeners"].as_array().into_iter().flatten().collect();
    let chosen = if let Some(name) = ctx.args.flag("name") {
        all.iter()
            .find(|l| l["name"].as_str() == Some(name))
            .ok_or_else(|| CliError::not_found(format!("no webhook listener \"{name}\" is running")))?
    } else {
        match all.as_slice() {
            [] => {
                return Err(CliError::usage("no webhook listener running; use --to <url> or bifurc webhooks listen"));
            }
            [one] => one,
            _ => return Err(CliError::usage("several listeners running; pick one with --name")),
        }
    };
    Ok(format!("http://127.0.0.1:{}/", js_template(chosen.get("port"))))
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
    let rows: Vec<Value> = FIXTURES.iter().map(|(name, _)| json!({ "name": name, "id": name })).collect();
    let name = find_one(&rows, &wanted, &["name"], "fixture", "webhooks")
        .map(|row| js_template(row.get("name")))
        .map_err(|err| match err {
            CliError::Exit(NOT_FOUND, _) => {
                let names: Vec<&str> = FIXTURES.iter().map(|(name, _)| *name).collect();
                CliError::usage(format!("no fixture \"{wanted}\"; fixtures: {}", names.join(", ")))
            }
            other => other,
        })?;
    let vault = ctx.args.flag("vault").map(str::to_string);
    if vault.is_none() && ctx.args.flag("secret").is_some() {
        return Err(CliError::usage("--secret needs --vault <vault> to read the secret from"));
    }
    let now = unix_secs();
    let mut payload = fixture(&name, now);
    for patch in ctx.args.flags("patch") {
        crate::select::patch(&mut payload, patch).map_err(CliError::usage)?;
    }
    let stripe = name.starts_with("stripe.");
    let default_key = if stripe { "STRIPE_WEBHOOK_SECRET" } else { "GITHUB_WEBHOOK_SECRET" };
    let key_name = ctx.args.flag("secret").unwrap_or(default_key).to_string();
    let secret = match &vault {
        Some(vault) => {
            let workspace = workspace_id(ctx)?;
            let secrets = crate::vault::secrets_of(ctx, &workspace, vault)?;
            let found = secrets.into_iter().find(|(key, _)| *key == key_name).map(|(_, value)| value);
            Some(found.ok_or_else(|| {
                CliError::not_found(format!(
                    "no secret \"{key_name}\" in vault \"{vault}\" (pick another with --secret)"
                ))
            })?)
        }
        None => None,
    };
    let url = trigger_target(ctx)?;

    // These exact bytes are signed and sent.
    let body = serde_json::to_vec(&payload).map_err(|err| CliError::msg(err.to_string()))?;
    let mut headers = serde_json::Map::new();
    let mut put = |name: &str, value: String| headers.insert(name.to_string(), json!(value));
    if stripe {
        put("content-type", "application/json; charset=utf-8".into());
        put("user-agent", "Stripe/1.0 (+https://stripe.com/docs/webhooks)".into());
        if let Some(secret) = &secret {
            let mut signed = format!("{now}.").into_bytes();
            signed.extend(&body);
            put("stripe-signature", format!("t={now},v1={}", hex(&hmac_sha256(secret.as_bytes(), &signed))));
        }
    } else {
        let id = [8, 4, 4, 4, 12].map(|n| random_chars(HEX, n)).join("-");
        put("content-type", "application/json".into());
        put("user-agent", "GitHub-Hookshot/bifurc".into());
        put("x-github-event", "push".into());
        put("x-github-delivery", id);
        put("x-github-hook-id", random_chars("0123456789", 9));
        if let Some(secret) = &secret {
            put("x-hub-signature-256", format!("sha256={}", hex(&hmac_sha256(secret.as_bytes(), &body))));
        }
    }
    send(ctx, "POST", &url, &headers, &STANDARD.encode(&body))
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
        entity["url"] = json!(replay_target(url, Some(forward), None)?);
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
        let (out, _, _) = go(&["webhooks", "replay", "1111"], answers.clone(), vec![]);
        assert_eq!(out.unwrap_err().code(), 4);
        // A stopped listener with no stored forward: the refusal says so.
        answers.retain(|(action, _)| *action != "webhook.list");
        answers.push(("webhook.list", Ok(json!({"listeners": []}))));
        let (out, _, _) = go(&["webhooks", "replay", "1111"], answers, vec![]);
        let err = out.unwrap_err();
        assert_eq!(err.code(), 4);
        assert!(err.line().contains("\"stripe\" is stopped, use --to <url>"), "{}", err.line());
    }

    #[test]
    fn replay_uses_the_forward_stored_on_the_entry_without_the_listener() {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let mut entry = hook("mg1a1111", Some("stripe"), "webhook");
        entry["forward"] = json!("http://api.localhost/in");
        entry["reqBody"] = json!(STANDARD.encode("{}"));
        let reply = json!({"status": 200, "headers": {}, "body": ""});
        let mut answers = buffer();
        answers.extend([("capture.get", Ok(json!({"entry": entry}))), ("request.replay", Ok(reply))]);
        let (out, calls, _) = go(&["webhooks", "replay", "1111"], answers.clone(), vec![]);
        out.unwrap();
        assert!(payloads(&calls, "webhook.list").is_empty());
        assert_eq!(payloads(&calls, "request.replay")[0]["url"], "http://api.localhost/in/hooks/a?x=1");
        let (_, calls, _) = go(&["webhooks", "replay", "1111", "--to", "http://127.0.0.1:4021"], answers, vec![]);
        assert_eq!(payloads(&calls, "request.replay")[0]["url"], "http://127.0.0.1:4021/hooks/a?x=1");
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

    fn hmac_hex(key: &[u8], data: &[u8]) -> String {
        hex(&hmac_sha256(key, data))
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        assert_eq!(
            hmac_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hmac_hex(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First"),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    /// Answers for a trigger: a vault holding `secrets`, one listener, and the target's reply.
    fn triggering(listeners: Value) -> Vec<Answer> {
        vec![
            ("config.get", Ok(config())),
            ("vault.list", Ok(json!([{"id": "v1", "name": "hooks", "state": "unsealed"}]))),
            (
                "vault.secrets",
                Ok(json!({"secrets": {"STRIPE_WEBHOOK_SECRET": "whsec_abc", "GITHUB_WEBHOOK_SECRET": "gh-s3cret"}})),
            ),
            ("webhook.list", Ok(json!({ "listeners": listeners }))),
            ("request.replay", Ok(json!({"status": 200, "headers": {}, "body": ""}))),
        ]
    }

    fn one_listener() -> Value {
        json!([{"id": "k1", "port": 4020, "name": "stripe"}])
    }

    /// The method, url, headers and decoded body of the one `request.replay` call.
    fn sent(calls: &[(String, Value)]) -> (Value, Vec<u8>) {
        let call = payloads(calls, "request.replay").remove(0);
        let body = STANDARD.decode(call["body"].as_str().unwrap()).unwrap();
        (call, body)
    }

    #[test]
    fn trigger_stripe_signs_the_exact_bytes() {
        let argv = ["webhooks", "trigger", "stripe", "--vault", "hooks"];
        let (out, calls, _) = go(&argv, triggering(one_listener()), vec![]);
        assert!(out.unwrap().starts_with("200 OK"));
        let (call, body) = sent(&calls);
        assert_eq!((call["method"].as_str(), call["url"].as_str()), (Some("POST"), Some("http://127.0.0.1:4020/")));
        let event: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(event["type"], "checkout.session.completed");
        assert!(event["id"].as_str().unwrap().starts_with("evt_"));
        let header = call["headers"]["stripe-signature"].as_str().unwrap();
        let (t, v1) = header.strip_prefix("t=").unwrap().split_once(",v1=").unwrap();
        let mut signed = format!("{t}.").into_bytes();
        signed.extend(&body);
        assert_eq!(v1, hmac_hex(b"whsec_abc", &signed));
        assert_eq!(call["headers"]["content-type"], "application/json; charset=utf-8");
        // Without --vault nothing is signed.
        let (_, calls, _) = go(&["webhooks", "trigger", "stripe"], triggering(one_listener()), vec![]);
        assert!(sent(&calls).0["headers"].get("stripe-signature").is_none());
    }

    #[test]
    fn trigger_github_signs_the_body_and_names_the_event() {
        let argv = ["webhooks", "trigger", "github.push", "--vault", "hooks", "--to", "http://localhost:3000/gh"];
        let (out, calls, _) = go(&argv, triggering(json!([])), vec![]);
        out.unwrap();
        assert!(payloads(&calls, "webhook.list").is_empty());
        let (call, body) = sent(&calls);
        assert_eq!(call["url"], "http://localhost:3000/gh");
        assert_eq!(call["headers"]["x-github-event"], "push");
        assert_eq!(call["headers"]["x-hub-signature-256"], format!("sha256={}", hmac_hex(b"gh-s3cret", &body)));
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["ref"], "refs/heads/main");
    }

    #[test]
    fn trigger_finds_its_listener_or_fails_clearly() {
        let argv = ["webhooks", "trigger", "github.push"];
        let two = json!([{"port": 4020, "name": "a"}, {"port": 4021, "name": "b"}]);
        let (out, _, _) = go(&argv, triggering(json!([])), vec![]);
        assert_eq!(out.unwrap_err().code(), 2);
        let (out, _, _) = go(&argv, triggering(two.clone()), vec![]);
        assert_eq!(out.unwrap_err().code(), 2);
        let (out, calls, _) = go(&[argv[0], argv[1], argv[2], "--name", "b"], triggering(two.clone()), vec![]);
        out.unwrap();
        assert_eq!(sent(&calls).0["url"], "http://127.0.0.1:4021/");
        let (out, _, _) = go(&[argv[0], argv[1], argv[2], "--name", "zz"], triggering(two), vec![]);
        assert_eq!(out.unwrap_err().code(), 3);
    }

    #[test]
    fn trigger_patch_secret_and_fixture_errors() {
        let (out, calls, _) = go(
            &["webhooks", "trigger", "stripe", "--vault", "hooks", "--patch", ".data.object.amount_total=999"],
            triggering(one_listener()),
            vec![],
        );
        out.unwrap();
        let (call, body) = sent(&calls);
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap()["data"]["object"]["amount_total"], 999);
        let mut signed =
            format!("{}.", call["headers"]["stripe-signature"].as_str().unwrap()[2..].split(',').next().unwrap())
                .into_bytes();
        signed.extend(&body);
        assert!(call["headers"]["stripe-signature"].as_str().unwrap().ends_with(&hmac_hex(b"whsec_abc", &signed)));

        let err = |argv: &[&str]| go(argv, triggering(one_listener()), vec![]).0.unwrap_err();
        assert_eq!(err(&["webhooks", "trigger", "paypal"]).code(), 2);
        assert_eq!(err(&["webhooks", "trigger", "stripe", "--secret", "K"]).code(), 2);
        let missing = err(&["webhooks", "trigger", "stripe", "--vault", "hooks", "--secret", "NOPE"]);
        assert_eq!(missing.code(), 3);
        assert!(missing.line().contains("NOPE") && missing.line().contains("hooks"), "{}", missing.line());
    }

    #[test]
    fn fixtures_lists_both() {
        let (out, _, _) = go(&["webhooks", "fixtures"], vec![], vec![]);
        let out = out.unwrap();
        assert!(out.lines().next().unwrap().starts_with("stripe.checkout.session.completed  "), "{out}");
        assert!(out.lines().nth(1).unwrap().starts_with("github.push  "), "{out}");
    }
}
