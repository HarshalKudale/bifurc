//! `fault.add|list|clear`: fault-injection rules for proxied traffic, so retry, timeout and error UI can be
//! checked against the real backend. Rules live in memory, engine-wide, and are lost when the engine stops (by
//! design: a fault must never outlive the session that asked for it; `bifurc status` will show them).
//!
//! A rule is `host[:port][/path-prefix]` (scheme ignored, `*.x.com` allowed, the path matches on segment
//! boundaries) plus one kind: `latencyMs` (then the request goes on), `status` (answered at once, upstream not
//! called; may follow a latency), `drop` (never answered, held open up to 60 s) or `reset` (connection closed
//! with no answer). `rate` is the percent of matching requests that are hit (default 100).
//!
//! Applied by the forward proxy (http and intercepted https) and the reverse-proxy edge, before cassette replay
//! and mocks. Not applied by the local mock server nor by `request.send`. A blind CONNECT tunnel only meets
//! host-only rules without a status: latency, drop, reset (no path or status can be seen or injected there).
//! Hits are logged with via "mock" (status) or "error" (drop, reset) and target `fault:<id>`; a latency marks the
//! entry's `fault` field.
//!
//! shortcut: reset is a plain close (FIN, no response), not an RST, as the proxies' streams are generic; upgrade
//! when a client must see ECONNRESET (set SO_LINGER 0 on the accepted TcpStream).

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use parking_lot::Mutex;
use rand::Rng;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{
    kernel::{
        CommandRegistry,
        traffic::{Headers, RequestLogEntry, emit_log},
    },
    mocks::write_response,
    protocol::{EngineError, Result},
    proxy::{finished_entry, log_body},
    utils::id::{mk_id, now_ms},
};

/// How long a dropped request is held when the client does not give up first.
const DROP_HOLD: Duration = Duration::from_secs(60);

struct Rule {
    id: String,
    target: String,
    host: String,
    port: Option<u16>,
    path: Option<String>,
    latency_ms: Option<u64>,
    status: Option<u16>,
    rate: f64,
    drop: bool,
    reset: bool,
    created_at: i64,
    hits: u64,
}

impl Rule {
    fn json(&self) -> Value {
        let mut out = json!({ "id": self.id, "target": self.target, "host": self.host, "rate": self.rate,
            "createdAt": self.created_at, "hits": self.hits });
        for (key, value) in [
            ("port", self.port.map(Value::from)),
            ("path", self.path.clone().map(Value::from)),
            ("latencyMs", self.latency_ms.map(Value::from)),
            ("status", self.status.map(Value::from)),
            ("drop", self.drop.then_some(Value::Bool(true))),
            ("reset", self.reset.then_some(Value::Bool(true))),
        ] {
            if let Some(value) = value {
                out[key] = value;
            }
        }
        out
    }

    fn matches(&self, host: &str, port: Option<u16>, path: Option<&str>) -> bool {
        let host_ok = match self.host.strip_prefix("*.") {
            Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
            None => self.host == host,
        };
        let path_ok = match (&self.path, path) {
            (None, _) => true,
            (Some(prefix), Some(path)) => {
                path == prefix || path.strip_prefix(prefix.as_str()).is_some_and(|rest| rest.starts_with('/'))
            }
            (Some(_), None) => false,
        };
        host_ok && path_ok && self.port.is_none_or(|p| Some(p) == port)
    }
}

static ANY: AtomicBool = AtomicBool::new(false);
static RULES: Mutex<Vec<Rule>> = Mutex::new(Vec::new());

// ---- commands ---------------------------------------------------------------------------------------------

/// `host`, `port`, `path` of `[scheme://]host[:port][/path][?query]`.
fn parse_target(raw: &str) -> Result<(String, Option<u16>, Option<String>)> {
    let bad = || EngineError::bad_request(format!("\"{raw}\" is not a fault target (host[:port][/path-prefix])."));
    let rest = raw.trim().split_once("://").map_or(raw.trim(), |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (authority, path) = rest.find('/').map_or((rest, ""), |at| rest.split_at(at));
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port.parse::<u16>().map_err(|_| bad())?)),
        None => (authority, None),
    };
    let host = host.to_ascii_lowercase();
    let name = host.strip_prefix("*.").unwrap_or(&host);
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || "-._[]:".contains(c)) {
        return Err(bad());
    }
    let path = path.trim_end_matches('/');
    Ok((host, port, (!path.is_empty()).then(|| path.to_string())))
}

fn number(params: &Value, key: &str, min: f64, max: f64) -> Result<Option<f64>> {
    let Some(value) = params.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    match value.as_f64() {
        Some(n) if n >= min && n <= max && n.fract() == 0.0 => Ok(Some(n)),
        _ => Err(EngineError::bad_request(format!("{key} must be a whole number from {min} to {max}."))),
    }
}

fn flag(params: &Value, key: &str) -> Result<bool> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(EngineError::bad_request(format!("{key} must be true or false."))),
    }
}

fn add(params: &Value) -> Result<Value> {
    let target = params.get("target").and_then(Value::as_str).unwrap_or("").to_string();
    let (host, port, path) = parse_target(&target)?;
    let latency_ms = number(params, "latencyMs", 1.0, 120_000.0)?.map(|n| n as u64);
    let status = number(params, "status", 100.0, 599.0)?.map(|n| n as u16);
    let rate = match params.get("rate") {
        None | Some(Value::Null) => 100.0,
        Some(v) => match v.as_f64() {
            Some(r) if r > 0.0 && r <= 100.0 => r,
            _ => return Err(EngineError::bad_request("rate must be a number above 0 and at most 100 (percent).")),
        },
    };
    let (drop, reset) = (flag(params, "drop")?, flag(params, "reset")?);
    if !drop && !reset && latency_ms.is_none() && status.is_none() {
        return Err(EngineError::bad_request("A fault needs one of latencyMs, status, drop or reset."));
    }
    if (drop || reset) && (latency_ms.is_some() || status.is_some() || (drop && reset)) {
        return Err(EngineError::bad_request("drop and reset cannot be combined with anything else."));
    }
    let rule = Rule {
        id: mk_id(),
        target,
        host,
        port,
        path,
        latency_ms,
        status,
        rate,
        drop,
        reset,
        created_at: now_ms(),
        hits: 0,
    };
    let mut shown = rule.json();
    let wildcard = rule.host.trim_start_matches("*.");
    if !crate::cassettes::is_local_host(wildcard) && !crate::cassettes::https_seen() {
        shown["warnings"] = json!([
            "https:// traffic is only tunnelled: only host-wide latency, drop and reset apply (see: bifurc tls status)"
        ]);
    }
    RULES.lock().push(rule);
    ANY.store(true, Ordering::Release);
    Ok(shown)
}

fn clear(params: &Value) -> Result<Value> {
    let id = params.get("id").and_then(Value::as_str);
    let mut rules = RULES.lock();
    let before = rules.len();
    rules.retain(|r| id.is_some_and(|id| r.id != id));
    ANY.store(!rules.is_empty(), Ordering::Release);
    let cleared = before - rules.len();
    if let Some(id) = id.filter(|_| cleared == 0) {
        return Err(EngineError::not_found(format!("No fault \"{id}\" (see: fault.list).")));
    }
    Ok(json!({ "cleared": cleared }))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("fault.add", |params, _| add(&params));
    registry.register_sync("fault.list", |_, _| {
        Ok(json!({ "faults": RULES.lock().iter().map(Rule::json).collect::<Vec<_>>() }))
    });
    registry.register_sync("fault.clear", |params, _| clear(&params));
}

// ---- applying ---------------------------------------------------------------------------------------------

/// A rule fires when the roll (0..100) is below its rate.
fn fires(rate: f64, roll: f64) -> bool {
    roll < rate
}

struct Hit {
    id: String,
    latency_ms: Option<u64>,
    status: Option<u16>,
    drop: bool,
    reset: bool,
}

/// The first matching rule, if it wins its roll, its hit counted. `tunnel`: only rules a blind tunnel can honour.
fn pick(host: &str, port: Option<u16>, path: Option<&str>, tunnel: bool) -> Option<Hit> {
    let mut rules = RULES.lock();
    let rule = rules
        .iter_mut()
        .find(|r| r.matches(host, port, path) && (!tunnel || (r.path.is_none() && r.status.is_none())))?;
    if !fires(rule.rate, rand::rng().random::<f64>() * 100.0) {
        return None;
    }
    rule.hits += 1;
    Some(Hit {
        id: rule.id.clone(),
        latency_ms: rule.latency_ms,
        status: rule.status,
        drop: rule.drop,
        reset: rule.reset,
    })
}

/// Hold the client until it closes or `DROP_HOLD` passes.
async fn hold<W: AsyncRead + Unpin>(out: &mut W) {
    let mut buf = [0u8; 1024];
    let _ =
        tokio::time::timeout(DROP_HOLD, async { while matches!(out.read(&mut buf).await, Ok(n) if n > 0) {} }).await;
}

fn failed_entry(base: &RequestLogEntry, id: &str, t0: i64) -> RequestLogEntry {
    RequestLogEntry {
        status: None,
        via: "error".to_string(),
        target: Some(format!("fault:{id}")),
        duration_ms: Some(now_ms() - t0),
        ..base.clone()
    }
}

/// Apply the fault matching `base`'s request, if any. `true` when the request is finished (answered, dropped or
/// reset): the caller returns. A latency fault sleeps, marks `base.fault`, and returns `false`.
pub async fn apply<W: AsyncRead + AsyncWrite + Unpin>(out: &mut W, base: &mut RequestLogEntry, t0: i64) -> bool {
    if !ANY.load(Ordering::Acquire) {
        return false;
    }
    let Ok(url) = url::Url::parse(&base.url) else { return false };
    let Some(host) = url.host_str() else { return false };
    let Some(hit) = pick(&host.to_ascii_lowercase(), url.port_or_known_default(), Some(url.path()), false) else {
        return false;
    };
    if let Some(ms) = hit.latency_ms {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    base.fault = Some(hit.id.clone());
    if let Some(status) = hit.status {
        let body = json!({ "error": "fault injected by bifurc", "fault": hit.id, "status": status }).to_string();
        let mut headers = Headers::new();
        headers.insert("content-type".into(), "application/json".into());
        headers.insert("x-bifurc-fault".into(), hit.id.clone());
        if matches!(status, 429 | 503) {
            headers.insert("retry-after".into(), "1".into());
        }
        emit_log(&finished_entry(
            base,
            status,
            "mock",
            Some(format!("fault:{}", hit.id)),
            now_ms() - t0,
            headers.clone(),
            log_body(body.as_bytes()),
        ));
        let _ = write_response(out, status, &headers, body.as_bytes()).await;
        return true;
    }
    if hit.drop {
        hold(out).await;
    } else if hit.reset {
        let _ = out.shutdown().await;
    } else {
        return false;
    }
    emit_log(&failed_entry(base, &hit.id, t0));
    true
}

/// The same for a CONNECT that will be tunnelled blind to `host:port`. `true` when the tunnel must not be made.
pub async fn tunnel<W: AsyncRead + AsyncWrite + Unpin>(out: &mut W, host: &str, port: u16) -> bool {
    if !ANY.load(Ordering::Acquire) {
        return false;
    }
    let Some(hit) = pick(&host.to_ascii_lowercase(), Some(port), None, true) else { return false };
    if let Some(ms) = hit.latency_ms {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
    if !hit.drop && !hit.reset {
        return false;
    }
    let t0 = now_ms();
    if hit.drop {
        hold(out).await;
    } else {
        let _ = out.shutdown().await;
    }
    let base = RequestLogEntry {
        id: mk_id(),
        ts: t0,
        method: "CONNECT".to_string(),
        url: format!("{host}:{port}"),
        host: host.to_string(),
        origin: Some("forward".to_string()),
        ..Default::default()
    };
    emit_log(&failed_entry(&base, &hit.id, t0));
    true
}

/// Test helpers for the proxies' end-to-end tests: rules are global, so each test clears only its own.
#[cfg(test)]
pub(crate) fn add_for_tests(params: &Value) -> String {
    add(params).unwrap()["id"].as_str().unwrap().to_string()
}

#[cfg(test)]
pub(crate) fn hits_for_tests(id: &str) -> u64 {
    RULES.lock().iter().find(|r| r.id == id).map_or(0, |r| r.hits)
}

#[cfg(test)]
pub(crate) fn clear_for_tests(id: &str) {
    clear(&json!({ "id": id })).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(target: &str) -> Rule {
        let (host, port, path) = parse_target(target).unwrap();
        Rule {
            id: "r".into(),
            target: target.into(),
            host,
            port,
            path,
            latency_ms: Some(1),
            status: None,
            rate: 100.0,
            drop: false,
            reset: false,
            created_at: 0,
            hits: 0,
        }
    }

    #[test]
    fn matching_host_port_wildcard_path_and_scheme() {
        let r = rule("https://API.localhost:8080/orders/");
        assert_eq!((r.host.as_str(), r.port, r.path.as_deref()), ("api.localhost", Some(8080), Some("/orders")));
        assert!(r.matches("api.localhost", Some(8080), Some("/orders")));
        assert!(r.matches("api.localhost", Some(8080), Some("/orders/1")));
        assert!(!r.matches("api.localhost", Some(8080), Some("/ordersx")));
        assert!(!r.matches("api.localhost", Some(81), Some("/orders")));
        assert!(!r.matches("api.localhost", Some(8080), None));
        let r = rule("api.localhost");
        assert!(r.matches("api.localhost", Some(1), Some("/any")) && r.matches("api.localhost", None, None));
        assert!(!r.matches("xapi.localhost", None, None));
        let r = rule("*.x.com");
        assert!(r.matches("a.x.com", Some(443), Some("/")) && r.matches("a.b.x.com", None, None));
        assert!(!r.matches("x.com", None, None) && !r.matches("ax.com", None, None));
    }

    #[test]
    fn validation_errors_are_clear() {
        let bad = |p: Value| add(&p).unwrap_err().to_string();
        assert!(bad(json!({ "target": "a.com" })).contains("needs one of"));
        assert!(bad(json!({ "target": "", "status": 500 })).contains("not a fault target"));
        assert!(bad(json!({ "target": "a.com:x", "status": 500 })).contains("not a fault target"));
        assert!(bad(json!({ "target": "a.com", "latencyMs": 0 })).contains("latencyMs"));
        assert!(bad(json!({ "target": "a.com", "latencyMs": 120001 })).contains("latencyMs"));
        assert!(bad(json!({ "target": "a.com", "status": 99 })).contains("status"));
        assert!(bad(json!({ "target": "a.com", "status": 600 })).contains("status"));
        assert!(bad(json!({ "target": "a.com", "status": 500, "rate": 0 })).contains("rate"));
        assert!(bad(json!({ "target": "a.com", "status": 500, "rate": 101 })).contains("rate"));
        assert!(bad(json!({ "target": "a.com", "status": 500, "rate": "30%" })).contains("rate"));
        assert!(bad(json!({ "target": "a.com", "drop": true, "status": 500 })).contains("cannot be combined"));
        assert!(bad(json!({ "target": "a.com", "drop": true, "reset": true })).contains("cannot be combined"));
        assert!(bad(json!({ "target": "a.com", "reset": "yes" })).contains("true or false"));
    }

    #[test]
    fn rate_is_a_percent_roll() {
        assert!(fires(100.0, 99.99) && fires(30.0, 29.9) && !fires(30.0, 30.0) && !fires(0.5, 0.5));
    }

    #[test]
    fn add_list_hits_and_clear_by_id() {
        let a = add(&json!({ "target": "unit-add.localhost/p", "latencyMs": 5, "status": 503, "rate": 50 })).unwrap();
        assert_eq!((a["rate"].clone(), a["status"].clone(), a["hits"].clone()), (json!(50.0), json!(503), json!(0)));
        let id = a["id"].as_str().unwrap().to_string();
        let b = add(&json!({ "target": "unit-add.localhost", "reset": true })).unwrap();
        assert_eq!(b["rate"], 100.0);
        // Rate 100 always fires, and the hit is counted on the first matching rule only.
        let hit = pick("unit-add.localhost", None, Some("/q"), false).unwrap();
        assert_eq!(hit.id, b["id"].as_str().unwrap());
        assert!(RULES.lock().iter().any(|r| r.id == hit.id && r.hits == 1));
        // A tunnel skips path and status rules.
        assert!(pick("unit-add.localhost", Some(443), None, true).is_some_and(|h| h.reset));
        assert_eq!(clear(&json!({ "id": id })).unwrap()["cleared"], 1);
        assert!(clear(&json!({ "id": id })).is_err());
        assert_eq!(clear(&json!({ "id": b["id"] })).unwrap()["cleared"], 1);
    }
}
