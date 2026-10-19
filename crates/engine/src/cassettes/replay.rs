//! `cassette.replayStart|replayStop|replayStatus|list`: serve a recorded cassette instead of the real upstream.
//!
//! The hook is [`serve`] (called first by the forward proxy and by the local mock server); with no replay active
//! it is one lock and an `is_none`. A request matches a recorded one by method, path, query and a normalised body
//! (see [`matches`]); headers are never compared. Replayed answers are logged with via "mock" and target
//! `cassette:<name>`, so the recorder (proxy|rule|rfc6761 only) never records them again.
//!
//! Choices: while recording, replay cannot start and the other way round (CONFLICT; hosts may overlap, so the
//! simplest correct rule is to refuse). A recorded response whose body was truncated at the capture cap is served
//! with an empty body and the header `x-bifurc-cassette: truncated`.

use std::cmp::Reverse;

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::AsyncWrite;

use super::{DIR, MASK, host_matches, https_warnings};
use crate::{
    kernel::traffic::{Headers, RequestLogEntry, emit_log},
    mocks::write_response,
    protocol::{EngineError, Result},
    proxy::{finished_entry, log_body},
    utils::{
        id::now_ms,
        json::{array_of, bool_of, str_of},
    },
    workspace::{contracts::plain, fs::ws_dir},
};

/// Keys whose values change on every call (compared case-insensitively, `-` and `_` alike).
const VOLATILE: [&str; 10] = [
    "idempotency_key",
    "request_id",
    "nonce",
    "timestamp",
    "ts",
    "created",
    "created_at",
    "updated_at",
    "date",
    "expires_at",
];
const MAX_UNMATCHED: usize = 20;

// ---- matching ---------------------------------------------------------------------------------------------

fn volatile(key: &str) -> bool {
    VOLATILE.contains(&key.to_ascii_lowercase().replace('-', "_").as_str())
}

/// `2026-10-18T10:24:16.123+05:30`, `2026-10-18 10:24Z`: an ISO 8601 / RFC 3339 date-time.
// shortcut: seconds and zone are only checked by their characters, so `2026-10-18T10:24:99` passes; upgrade when a
// body legitimately carries near-timestamps that must differ.
fn is_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    s.is_ascii()
        && b.len() >= 16
        && b[..16].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => matches!(c, b'T' | b't' | b' '),
            13 => *c == b':',
            _ => c.is_ascii_digit(),
        })
        && b[16..].iter().all(|c| c.is_ascii_digit() || b":.+-Zz".contains(c))
}

/// Same multiset of pairs, volatile keys left out; a recorded `••••` value matches any value of its key.
fn pairs_match(recorded: &[(String, String)], incoming: &[(String, String)]) -> bool {
    let recorded: Vec<_> = recorded.iter().filter(|(k, _)| !volatile(k)).collect();
    let mut left: Vec<_> = incoming.iter().filter(|(k, _)| !volatile(k)).collect();
    if recorded.len() != left.len() {
        return false;
    }
    // Exact pairs first, so a wildcard does not take the pair an exact one needs.
    let mut wild = Vec::new();
    for pair in recorded {
        if pair.1 == MASK {
            wild.push(pair);
        } else if let Some(at) = left.iter().position(|p| *p == pair) {
            left.swap_remove(at);
        } else {
            return false;
        }
    }
    wild.into_iter().all(|(key, _)| match left.iter().position(|(k, _)| k == key) {
        Some(at) => {
            left.swap_remove(at);
            true
        }
        None => false,
    })
}

fn json_matches(recorded: &Value, incoming: &Value) -> bool {
    match (recorded, incoming) {
        (Value::String(r), _) if r == MASK => true,
        (Value::String(r), Value::String(i)) => r == i || (is_timestamp(r) && is_timestamp(i)),
        (Value::Object(r), Value::Object(i)) => {
            let r: Vec<_> = r.iter().filter(|(k, _)| !volatile(k)).collect();
            let i: Vec<_> = i.iter().filter(|(k, _)| !volatile(k)).collect();
            r.len() == i.len() && r.iter().all(|(k, v)| i.iter().any(|(ik, iv)| ik == k && json_matches(v, iv)))
        }
        (Value::Array(r), Value::Array(i)) => r.len() == i.len() && r.iter().zip(i).all(|(r, i)| json_matches(r, i)),
        (r, i) => r == i,
    }
}

struct Incoming<'a> {
    method: &'a str,
    /// Percent-decoded.
    path: String,
    query: Vec<(String, String)>,
    form: bool,
    body: &'a [u8],
}

impl<'a> Incoming<'a> {
    fn new(method: &'a str, target: &str, content_type: &str, body: &'a [u8]) -> Self {
        let (path, query) = split_target(target);
        Self { method, path, query, form: content_type.to_ascii_lowercase().contains("x-www-form-urlencoded"), body }
    }
}

/// Decoded path and decoded query pairs of `/path?query`.
fn split_target(target: &str) -> (String, Vec<(String, String)>) {
    match url::Url::parse(&format!("http://h{target}")) {
        Ok(u) => (
            percent_encoding::percent_decode_str(u.path()).decode_utf8_lossy().into_owned(),
            u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect(),
        ),
        Err(_) => (target.to_string(), Vec::new()),
    }
}

fn form_pairs(body: &[u8]) -> Vec<(String, String)> {
    url::form_urlencoded::parse(body).map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

struct Recorded {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    body: Vec<u8>,
    status: u16,
    headers: Headers,
    res_body: Vec<u8>,
    truncated: bool,
    /// `METHOD /path?query` as recorded.
    shown: String,
}

fn body_bytes(side: &Value) -> Vec<u8> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    match (side.get("body").and_then(Value::as_str), side.get("bodyBase64").and_then(Value::as_str)) {
        (Some(text), _) => text.as_bytes().to_vec(),
        (None, Some(b64)) => STANDARD.decode(b64).unwrap_or_default(),
        _ => Vec::new(),
    }
}

impl Recorded {
    fn parse(item: &Value) -> Option<Self> {
        let (req, res) = (&item["request"], &item["response"]);
        let method = str_of(req, "method").to_ascii_uppercase();
        let url = url::Url::parse(str_of(req, "url")).ok()?;
        let target = format!("{}{}", url.path(), url.query().map(|q| format!("?{q}")).unwrap_or_default());
        let (path, query) = split_target(&target);
        let headers = res["headers"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.to_ascii_lowercase(), v.as_str()?.to_string())))
            .filter(|(k, v)| {
                v != MASK
                    && !matches!(
                        k.as_str(),
                        "connection"
                            | "keep-alive"
                            | "transfer-encoding"
                            | "content-length"
                            | "upgrade"
                            | "te"
                            | "trailer"
                    )
                    && !k.starts_with("proxy-")
            })
            .collect();
        Some(Self {
            shown: format!("{method} {target}"),
            method,
            path,
            query,
            body: body_bytes(req),
            status: res["status"].as_u64()? as u16,
            headers,
            res_body: body_bytes(res),
            truncated: bool_of(res, "truncated", false),
        })
    }
}

/// Whether the recorded request answers the incoming one: method, path, query, then body.
fn matches(recorded: &Recorded, incoming: &Incoming) -> bool {
    recorded.method.eq_ignore_ascii_case(incoming.method)
        && recorded.path == incoming.path
        && pairs_match(&recorded.query, &incoming.query)
        && body_matches(&recorded.body, incoming)
}

fn body_matches(recorded: &[u8], incoming: &Incoming) -> bool {
    if recorded.is_empty() || incoming.body.is_empty() {
        return recorded.is_empty() && incoming.body.is_empty();
    }
    if let (Ok(r), Ok(i)) = (serde_json::from_slice::<Value>(recorded), serde_json::from_slice::<Value>(incoming.body))
    {
        return json_matches(&r, &i);
    }
    if incoming.form {
        return pairs_match(&form_pairs(recorded), &form_pairs(incoming.body));
    }
    recorded == incoming.body
}

// ---- state ------------------------------------------------------------------------------------------------

/// What to answer the client with.
pub struct Reply {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    /// The log entry's `via`: "mock" for a recorded answer, "error" for an unmatched request.
    pub via: &'static str,
    pub target: Option<String>,
}

struct Replay {
    name: String,
    workspace_id: String,
    hosts: Vec<String>,
    passthrough: bool,
    items: Vec<Recorded>,
    used: Vec<u32>,
    served: u64,
    passed: u64,
    unmatched_count: u64,
    unmatched: Vec<Value>,
}

static REPLAY: Mutex<Option<Replay>> = Mutex::new(None);

/// The name of the cassette being replayed, if any.
pub(super) fn active_name() -> Option<String> {
    REPLAY.lock().as_ref().map(|r| r.name.clone())
}

impl Replay {
    fn new(name: &str, workspace_id: &str, file: &Value, passthrough: bool) -> Self {
        let items: Vec<Recorded> = array_of(file, "interactions").iter().filter_map(Recorded::parse).collect();
        Self {
            name: name.to_string(),
            workspace_id: workspace_id.to_string(),
            hosts: array_of(file, "hosts").iter().filter_map(|h| h.as_str().map(str::to_string)).collect(),
            passthrough,
            used: vec![0; items.len()],
            items,
            served: 0,
            passed: 0,
            unmatched_count: 0,
            unmatched: Vec::new(),
        }
    }

    /// The recorded request to answer with: the first match not yet used, else the last match (polling).
    fn pick(&mut self, incoming: &Incoming) -> Option<usize> {
        let hits: Vec<usize> = (0..self.items.len()).filter(|&i| matches(&self.items[i], incoming)).collect();
        let at = hits.iter().copied().find(|&i| self.used[i] == 0).or(hits.last().copied())?;
        self.used[at] += 1;
        Some(at)
    }

    /// Up to three recorded requests nearest to the incoming one: same method first, then the longest common path.
    fn closest(&self, incoming: &Incoming) -> Vec<String> {
        let common = |r: &Recorded| r.path.chars().zip(incoming.path.chars()).take_while(|(a, b)| a == b).count();
        let mut near: Vec<&Recorded> = self.items.iter().collect();
        near.sort_by_key(|r| (!r.method.eq_ignore_ascii_case(incoming.method), Reverse(common(r))));
        let mut shown: Vec<String> = Vec::new();
        for r in near {
            if shown.len() < 3 && !shown.contains(&r.shown) {
                shown.push(r.shown.clone());
            }
        }
        shown
    }

    /// `Some` answer (recorded, or the 404 of an unmatched request); `None`: pass through to the upstream.
    /// `scoped`: the request is known to be for the cassette's hosts (the proxy); otherwise (the mock server)
    /// a miss is not the cassette's to fail and goes on to the mocks.
    fn decide(&mut self, incoming: &Incoming, target: &str, url: &str, scoped: bool) -> Option<Reply> {
        if let Some(at) = self.pick(incoming) {
            self.served += 1;
            let r = &self.items[at];
            let mut headers = r.headers.clone();
            if r.truncated {
                headers.insert("x-bifurc-cassette".into(), "truncated".into());
            }
            let body = if r.truncated { Vec::new() } else { r.res_body.clone() };
            headers.insert("content-length".into(), body.len().to_string());
            headers.insert("connection".into(), "close".into());
            return Some(Reply {
                status: r.status,
                headers,
                body,
                via: "mock",
                target: Some(format!("cassette:{}", self.name)),
            });
        }
        if !scoped {
            return None;
        }
        if self.passthrough {
            self.passed += 1;
            return None;
        }
        let closest = self.closest(incoming);
        self.unmatched_count += 1;
        let seen = |u: &Value| str_of(u, "method") == incoming.method && str_of(u, "url") == url;
        if self.unmatched.len() < MAX_UNMATCHED && !self.unmatched.iter().any(seen) {
            self.unmatched.push(json!({ "method": incoming.method, "url": url, "closest": closest.first() }));
        }
        let body = json!({
            "error": format!("no recorded interaction in cassette \"{}\" matches {} {target}", self.name, incoming.method),
            "closest": closest,
        })
        .to_string()
        .into_bytes();
        let headers: Headers = [
            ("content-type", "application/json".to_string()),
            ("content-length", body.len().to_string()),
            ("connection", "close".to_string()),
            ("x-bifurc-cassette", "unmatched".to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        Some(Reply { status: 404, headers, body, via: "error", target: None })
    }
}

/// The answer for a request when a cassette is replayed for its workspace (and host, when `host` is given), else `None`.
pub fn lookup(
    workspace_id: Option<&str>,
    host: Option<&str>,
    method: &str,
    target: &str,
    content_type: &str,
    body: &[u8],
    url: &str,
) -> Option<Reply> {
    let mut guard = REPLAY.lock();
    let replay = guard.as_mut()?;
    if workspace_id.is_some_and(|w| w != replay.workspace_id) || host.is_some_and(|h| !host_matches(h, &replay.hosts)) {
        return None;
    }
    replay.decide(&Incoming::new(method, target, content_type, body), target, url, host.is_some())
}

/// Answer `base`'s request from the cassette being replayed, if one applies: logs it and writes the response.
/// `true` when it answered. `host_scoped` is false for the mock server, whose host means nothing.
pub async fn serve<W: AsyncWrite + Unpin>(
    out: &mut W,
    base: &RequestLogEntry,
    host_scoped: bool,
    target: &str,
    body: &[u8],
    t0: i64,
) -> bool {
    if REPLAY.lock().is_none() {
        return false;
    }
    let content_type = base.req_headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type"));
    let reply = lookup(
        base.workspace_id.as_deref(),
        host_scoped.then_some(base.host.as_str()),
        &base.method,
        target,
        content_type.map_or("", |(_, v)| v.as_str()),
        body,
        &base.url,
    );
    let Some(reply) = reply else { return false };
    emit_log(&finished_entry(
        base,
        reply.status,
        reply.via,
        reply.target.clone(),
        now_ms() - t0,
        reply.headers.clone(),
        log_body(&reply.body),
    ));
    let _ = write_response(out, reply.status, &reply.headers, &reply.body).await;
    true
}

// ---- commands ---------------------------------------------------------------------------------------------

fn cassette_path(ws: &str, name: &str) -> Result<std::path::PathBuf> {
    if !plain(ws) || !ws_dir(ws).is_dir() {
        return Err(EngineError::not_found(format!("No workspace \"{ws}\".")));
    }
    if !plain(name) {
        return Err(EngineError::bad_request(format!("\"{name}\" is not a cassette name (letters, digits, - and _).")));
    }
    Ok(ws_dir(ws).join(DIR).join(format!("{name}.json")))
}

fn start(params: &Value) -> Result<Value> {
    let (ws, name) = (str_of(params, "workspaceId"), str_of(params, "name"));
    let path = cassette_path(ws, name)?;
    let text = std::fs::read_to_string(&path)
        .map_err(|_| EngineError::not_found(format!("No cassette \"{name}\" (see: bifurc record list).")))?;
    let file: Value = serde_json::from_str(&text)
        .map_err(|err| EngineError::bad_request(format!("cassette \"{name}\" is not valid JSON: {err}")))?;
    if let Some(recording) = super::recording_name() {
        return Err(EngineError::conflict(format!("recording \"{recording}\"; bifurc record stop before replaying")));
    }
    let mut guard = REPLAY.lock();
    if let Some(active) = guard.as_ref() {
        return Err(EngineError::conflict(format!("already replaying \"{}\"; bifurc replay stop", active.name)));
    }
    let replay = Replay::new(name, ws, &file, bool_of(params, "passthrough", false));
    let answer = json!({
        "name": name, "hosts": replay.hosts, "interactions": replay.items.len(), "passthrough": replay.passthrough,
        "warnings": https_warnings(&replay.hosts),
    });
    *guard = Some(replay);
    Ok(answer)
}

fn list(params: &Value) -> Result<Value> {
    let ws = str_of(params, "workspaceId");
    cassette_path(ws, "x")?;
    let mut cassettes = Vec::new();
    for entry in std::fs::read_dir(ws_dir(ws).join(DIR)).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Some(file) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        else {
            continue;
        };
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        cassettes.push(json!({
            "name": name, "hosts": file["hosts"], "interactions": array_of(&file, "interactions").len(),
            "recordedAt": file["recordedAt"],
        }));
    }
    cassettes.sort_by(|a, b| str_of(a, "name").cmp(str_of(b, "name")));
    Ok(json!({ "cassettes": cassettes }))
}

pub fn register(registry: &mut crate::kernel::CommandRegistry) {
    registry.register_sync("cassette.replayStart", |params, _| start(&params));
    registry.register_sync("cassette.list", |params, _| list(&params));
    registry.register_sync("cassette.replayStatus", |_, _| {
        Ok(json!({ "replaying": REPLAY.lock().as_ref().map(|r| json!({
            "name": r.name, "workspaceId": r.workspace_id, "hosts": r.hosts, "served": r.served,
            "unmatchedCount": r.unmatched_count, "passthrough": r.passthrough,
        })) }))
    });
    registry.register_sync("cassette.replayStop", |_, _| {
        let r = REPLAY.lock().take().ok_or_else(|| EngineError::not_found("Nothing is replaying."))?;
        Ok(json!({
            "name": r.name, "served": r.served, "passedThrough": r.passed, "unmatched": r.unmatched,
            "unmatchedCount": r.unmatched_count,
        }))
    });
}

#[cfg(test)]
mod tests;
