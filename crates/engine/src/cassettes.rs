//! `cassette.recordStart|recordStop|recordStatus`: record the real upstream answers that pass through the proxies
//! into `cassettes/<name>.json` in the workspace (a folder of its own, not an entity kind: the app tree, config.get
//! and sync never list it). One recording at a time. Every kept exchange rewrites the file (temp + rename), so a
//! crash or a stopped engine leaves a valid cassette; the commit happens at stop.
//!
//! Secrets are masked at record time with the literal [`MASK`]. Masking is deterministic and loses the value, so a
//! replayer (D5b) that matches on method + path + query + normalised body MUST treat `••••` as a wildcard.
//! The `masked` list of each interaction names every place it replaced, e.g. `response.body.data[0].client_secret`.

use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::{
    kernel::{CommandRegistry, ListenerId, bus, traffic::MAX_LOGGED_BODY_BYTES},
    protocol::{EngineError, Result},
    utils::{
        id::now_ms,
        json::{array_of, str_of},
    },
    workspace::{
        contracts::plain,
        fs::ws_dir,
        git_store::{CommitMutation, commit_mutation},
        settings::load_settings,
    },
};

pub mod replay;

pub const MASK: &str = "••••";
const DIR: &str = "cassettes";

struct Recording {
    name: String,
    workspace_id: String,
    hosts: Vec<String>,
    started_at: i64,
    path: PathBuf,
    cassette: Value,
    masked: usize,
    listener: ListenerId,
}

impl Recording {
    fn interactions(&self) -> usize {
        self.cassette["interactions"].as_array().map_or(0, Vec::len)
    }
}

static RECORDING: Mutex<Option<Recording>> = Mutex::new(None);

// ---- masking ----------------------------------------------------------------------------------------------

/// Whether a header, query parameter or body key holds a credential (same list as the CLI's `is_sensitive_name`).
fn sensitive(name: &str) -> bool {
    let name = name.to_ascii_lowercase().replace('_', "-");
    ["authorization", "proxy-authorization", "cookie", "set-cookie", "x-api-key", "x-auth-token"]
        .contains(&name.as_str())
        || ["token", "secret", "api-key", "apikey", "session", "password", "private-key"]
            .iter()
            .any(|w| name.contains(w))
}

fn mask_headers(headers: &mut Map<String, Value>, place: &str, masked: &mut Vec<String>) {
    for (name, value) in headers.iter_mut() {
        if sensitive(name) {
            *value = json!(MASK);
            masked.push(format!("{place}.headers.{name}"));
        }
    }
}

/// The URL with sensitive query values masked.
fn mask_url(url: &str, masked: &mut Vec<String>) -> String {
    let Ok(mut parsed) = url::Url::parse(url) else { return url.to_string() };
    let pairs: Vec<(String, String)> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    if !pairs.iter().any(|(k, _)| sensitive(k)) {
        return url.to_string();
    }
    let mut query = parsed.query_pairs_mut();
    query.clear();
    for (key, value) in &pairs {
        if sensitive(key) {
            masked.push(format!("request.query.{key}"));
            query.append_pair(key, MASK);
        } else {
            query.append_pair(key, value);
        }
    }
    drop(query);
    parsed.to_string()
}

fn mask_json(value: &mut Value, path: &str, place: &str, masked: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            // Stripe card: `number` and `cvc` are secrets only next to `exp_month`.
            let card = map.contains_key("exp_month");
            for (key, child) in map.iter_mut() {
                let here = if path.is_empty() { key.clone() } else { format!("{path}.{key}") };
                // Only strings: `usage.total_tokens` (OpenAI) is a count, not a secret, and must keep its type.
                if child.is_string() && (sensitive(key) || (card && matches!(key.as_str(), "number" | "cvc"))) {
                    *child = json!(MASK);
                    masked.push(format!("{place}.body.{here}"));
                } else {
                    mask_json(child, &here, place, masked);
                }
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter_mut().enumerate() {
                mask_json(child, &format!("{path}[{i}]"), place, masked);
            }
        }
        _ => {}
    }
}

/// A text body with its secrets masked: JSON by shape, form-urlencoded by content type; anything else untouched.
fn mask_body(text: &str, content_type: &str, place: &str, masked: &mut Vec<String>) -> String {
    let before = masked.len();
    if matches!(text.trim_start().chars().next(), Some('{' | '['))
        && let Ok(mut parsed) = serde_json::from_str::<Value>(text)
    {
        mask_json(&mut parsed, "", place, masked);
        return if masked.len() > before { parsed.to_string() } else { text.to_string() };
    }
    if content_type.to_ascii_lowercase().contains("x-www-form-urlencoded") {
        let mut out = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in url::form_urlencoded::parse(text.as_bytes()) {
            if sensitive(&key) {
                masked.push(format!("{place}.body.{key}"));
                out.append_pair(&key, MASK);
            } else {
                out.append_pair(&key, &value);
            }
        }
        return if masked.len() > before { out.finish() } else { text.to_string() };
    }
    text.to_string()
}

// ---- entries ----------------------------------------------------------------------------------------------

/// Whether `host` (any case, port ignored) is one of `patterns`: exact, or `*.example.com` for a subdomain.
fn host_matches(host: &str, patterns: &[String]) -> bool {
    let host = host.rsplit_once(':').filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit())).map_or(host, |(h, _)| h);
    let host = host.to_ascii_lowercase();
    patterns.iter().any(|p| match p.strip_prefix("*.") {
        Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
        None => *p == host,
    })
}

/// Proxy traffic for this workspace and these hosts that really went upstream (not a mock, mapping or error).
fn wanted(entry: &Value, workspace_id: &str, hosts: &[String]) -> bool {
    matches!(str_of(entry, "origin"), "forward" | "reverse")
        && matches!(str_of(entry, "via"), "proxy" | "rule" | "rfc6761")
        && entry.get("status").is_some_and(Value::is_number)
        && entry.get("workspaceId").and_then(Value::as_str).is_none_or(|w| w == workspace_id)
        && host_matches(str_of(entry, "host"), hosts)
}

fn clean_headers(value: &Value) -> Map<String, Value> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
        .filter(|(k, _)| {
            !matches!(k.as_str(), "connection" | "keep-alive" | "transfer-encoding" | "content-length")
                && (!k.starts_with("proxy-") || k == "proxy-authorization")
        })
        .collect()
}

/// `body` (text, or null) plus `bodyBase64` when the bytes are not UTF-8.
fn put_body(side: &mut Value, b64: &str, content_type: &str, place: &str, masked: &mut Vec<String>) {
    let bytes = STANDARD.decode(b64).unwrap_or_default();
    if place == "response" && bytes.len() >= MAX_LOGGED_BODY_BYTES {
        // A body cut at the capture cap cannot be masked reliably (the JSON no longer parses) nor replayed whole.
        side["body"] = Value::Null;
        side["truncated"] = json!(true);
        return;
    }
    match String::from_utf8(bytes) {
        Ok(text) if !text.is_empty() => side["body"] = json!(mask_body(&text, content_type, place, masked)),
        Ok(_) => side["body"] = Value::Null,
        Err(_) => {
            // shortcut: binary bodies are not masked, a secret inside one is kept; upgrade when a binary API matters.
            side["body"] = Value::Null;
            side["bodyBase64"] = json!(b64);
        }
    }
}

/// One `log.entry` as a cassette interaction, secrets masked.
fn interaction(entry: &Value) -> Value {
    let mut masked = Vec::new();
    let mut req_headers = clean_headers(&entry["reqHeaders"]);
    let mut res_headers = clean_headers(&entry["resHeaders"]);
    // The proxy logs the response body decompressed.
    res_headers.shift_remove("content-encoding");
    mask_headers(&mut req_headers, "request", &mut masked);
    mask_headers(&mut res_headers, "response", &mut masked);
    let ctype = |h: &Map<String, Value>| h.get("content-type").and_then(Value::as_str).unwrap_or_default().to_string();
    let (req_type, res_type) = (ctype(&req_headers), ctype(&res_headers));
    let url = mask_url(str_of(entry, "url"), &mut masked);
    let mut request = json!({ "method": str_of(entry, "method"), "url": url, "headers": req_headers });
    let mut response = json!({ "status": entry["status"], "headers": res_headers });
    put_body(&mut request, str_of(entry, "reqBody"), &req_type, "request", &mut masked);
    put_body(&mut response, str_of(entry, "resBody"), &res_type, "response", &mut masked);
    json!({ "request": request, "response": response, "recordedAt": now_ms(), "masked": masked })
}

fn on_entry(entry: &Value) {
    let mut guard = RECORDING.lock();
    let Some(rec) = guard.as_mut() else { return };
    if !wanted(entry, &rec.workspace_id, &rec.hosts) {
        return;
    }
    let item = interaction(entry);
    rec.masked += array_of(&item, "masked").len();
    if let Some(list) = rec.cassette["interactions"].as_array_mut() {
        list.push(item);
    }
    // shortcut: the whole file is rewritten per interaction (O(n^2)); upgrade to append/segment past a few hundred.
    let _ = write_atomic(&rec.path, &rec.cassette);
}

fn write_atomic(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(value)?)?;
    std::fs::rename(&tmp, path)
}

// ---- commands ---------------------------------------------------------------------------------------------

/// The name of the cassette being recorded, if any.
fn recording_name() -> Option<String> {
    RECORDING.lock().as_ref().map(|r| r.name.clone())
}

/// Whether the proxy intercepts HTTPS (TLS toggle on and a root certificate loaded).
pub(crate) fn https_seen() -> bool {
    load_settings().tls_enabled() && crate::certificates::is_ca_loaded()
}

/// `localhost`, `*.localhost` or an IP address: reached over plain http by the local tools.
pub(crate) fn is_local_host(h: &str) -> bool {
    h == "localhost" || h.ends_with(".localhost") || h.parse::<std::net::IpAddr>().is_ok()
}

/// What the user must know before recording or replaying `hosts`: the proxy intercepts HTTPS (and so sees paths and
/// bodies) only with the TLS toggle on and a root certificate loaded, for every host alike; otherwise it only
/// tunnels. Plain http:// is always seen.
fn https_warnings(hosts: &[String]) -> Vec<String> {
    if https_seen() || hosts.iter().all(|h| is_local_host(h)) {
        return Vec::new();
    }
    vec!["HTTPS interception is off, so https:// traffic is only tunnelled, neither recorded nor replayed (see: bifurc tls status)"
        .to_string()]
}

fn start(params: &Value) -> Result<Value> {
    let ws = str_of(params, "workspaceId");
    let name = str_of(params, "name");
    if !plain(ws) || !ws_dir(ws).is_dir() {
        return Err(EngineError::not_found(format!("No workspace \"{ws}\".")));
    }
    if !plain(name) {
        return Err(EngineError::bad_request(format!("\"{name}\" is not a cassette name (letters, digits, - and _).")));
    }
    let hosts: Vec<String> = array_of(params, "hosts")
        .iter()
        .filter_map(|h| h.as_str())
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    if hosts.is_empty() || hosts.iter().any(|h| h.contains(['/', ' ', ':'])) {
        return Err(EngineError::bad_request(
            "hosts are names like api.stripe.com or *.example.com, without scheme or port.",
        ));
    }
    if let Some(replaying) = replay::active_name() {
        return Err(EngineError::conflict(format!("replaying \"{replaying}\"; bifurc replay stop before recording")));
    }
    let mut guard = RECORDING.lock();
    if let Some(active) = guard.as_ref() {
        return Err(EngineError::conflict(format!("already recording \"{}\"; bifurc record stop", active.name)));
    }
    let path = ws_dir(ws).join(DIR).join(format!("{name}.json"));
    let replaced = path.exists();
    let started_at = now_ms();
    let cassette = json!({ "version": 1, "name": name, "hosts": hosts, "recordedAt": started_at, "interactions": [] });
    write_atomic(&path, &cassette)?;
    let warnings = https_warnings(&hosts);
    *guard = Some(Recording {
        name: name.to_string(),
        workspace_id: ws.to_string(),
        hosts: hosts.clone(),
        started_at,
        path,
        cassette,
        masked: 0,
        listener: bus().on("log.entry", on_entry),
    });
    Ok(
        json!({ "name": name, "hosts": hosts, "path": format!("{DIR}/{name}.json"), "replaced": replaced, "warnings": warnings }),
    )
}

pub fn register(registry: &mut CommandRegistry) {
    replay::register(registry);
    registry.register_sync("cassette.recordStart", |params, _| start(&params));

    registry.register_sync("cassette.recordStatus", |_, _| {
        Ok(json!({ "recording": RECORDING.lock().as_ref().map(|r| json!({
            "name": r.name, "workspaceId": r.workspace_id, "hosts": r.hosts, "startedAt": r.started_at,
            "interactions": r.interactions(),
        })) }))
    });

    registry.register("cassette.recordStop", |_, _| async move {
        let rec = RECORDING.lock().take().ok_or_else(|| EngineError::not_found("Nothing is recording."))?;
        bus().off(rec.listener);
        let path = format!("{DIR}/{}.json", rec.name);
        // Best effort, as contracts: a workspace without a repository keeps the file and has no audit entry.
        let _ = commit_mutation(CommitMutation {
            action: "create",
            entity: "cassette",
            entity_id: &rec.name,
            entity_name: &rec.name,
            workspace_id: &rec.workspace_id,
            rel_path: &path,
            actor: None,
            changed_fields: &[],
        })
        .await;
        Ok(json!({ "name": rec.name, "path": path, "interactions": rec.interactions(), "masked": rec.masked }))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{kernel::CommandContext, testing::scratch_data_root};

    fn entry(host: &str, origin: &str, via: &str) -> Value {
        json!({ "method": "POST", "url": format!("https://{host}/v1/charges?api_key=k&page=2"), "host": host,
                "reqHeaders": { "authorization": "Bearer x", "content-length": "9", "connection": "close", "accept": "*/*" },
                "reqBody": STANDARD.encode(r#"{"amount":5,"card":{"number":"4242","exp_month":1,"cvc":"123"}}"#),
                "status": 200, "via": via, "origin": origin, "workspaceId": "ws1",
                "resHeaders": { "set-cookie": "a=b", "content-type": "application/json", "content-encoding": "gzip" },
                "resBody": STANDARD.encode(r#"{"data":[{"client_secret":"s","id":"c1"}]}"#) })
    }

    #[test]
    fn masks_headers_query_json_form_and_leaves_other_bodies() {
        let item = interaction(&entry("api.stripe.com", "forward", "proxy"));
        let masked: Vec<&str> = item["masked"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(
            masked,
            [
                "request.headers.authorization",
                "response.headers.set-cookie",
                "request.query.api_key",
                "request.body.card.number",
                "request.body.card.cvc",
                "response.body.data[0].client_secret"
            ]
        );
        let req = &item["request"];
        assert_eq!(req["headers"], json!({ "authorization": MASK, "accept": "*/*" }));
        assert!(req["url"].as_str().unwrap().contains("page=2"));
        assert!(!req["url"].as_str().unwrap().contains("api_key=k"));
        let body: Value = serde_json::from_str(req["body"].as_str().unwrap()).unwrap();
        assert_eq!(body["card"], json!({ "number": MASK, "exp_month": 1, "cvc": MASK }));
        assert_eq!(body["amount"], 5);
        let res: Value = serde_json::from_str(item["response"]["body"].as_str().unwrap()).unwrap();
        assert_eq!(res["data"][0], json!({ "client_secret": MASK, "id": "c1" }));
        assert!(item["response"]["headers"].get("content-encoding").is_none());

        // `number` outside a card is not a secret.
        let mut m = Vec::new();
        mask_json(&mut json!({"number": 1}), "", "request", &mut m);
        assert!(m.is_empty());
        // Counts under token-ish keys keep their type; nested objects are walked.
        let mut usage = json!({"usage": {"total_tokens": 30, "tokens": {"access_token": "t"}}});
        mask_json(&mut usage, "", "response", &mut m);
        assert_eq!(usage, json!({"usage": {"total_tokens": 30, "tokens": {"access_token": MASK}}}));
        assert_eq!(m, ["response.body.usage.tokens.access_token"]);
        m.clear();
        let form = mask_body("a=1&password=p", "application/x-www-form-urlencoded", "request", &mut m);
        assert!(form.starts_with("a=1&password=") && !form.contains("=p"), "{form}");
        assert_eq!(m, ["request.body.password"]);
        // Not JSON or a form: untouched, even when it looks like a secret; unchanged JSON keeps its bytes.
        assert_eq!(mask_body("token=abc plain", "text/plain", "request", &mut Vec::new()), "token=abc plain");
        assert_eq!(mask_body("{\"a\": 1}", "application/json", "request", &mut Vec::new()), "{\"a\": 1}");
    }

    #[test]
    fn bodies_are_text_or_base64() {
        let mut side = json!({});
        put_body(&mut side, &STANDARD.encode([0xff, 0xfe]), "", "response", &mut Vec::new());
        assert_eq!((side["body"].clone(), side["bodyBase64"].clone()), (Value::Null, json!("//4=")));
        let mut side = json!({});
        put_body(&mut side, "", "", "request", &mut Vec::new());
        assert!(side["body"].is_null() && side.get("bodyBase64").is_none());
        let mut side = json!({});
        put_body(&mut side, &STANDARD.encode(vec![b'a'; MAX_LOGGED_BODY_BYTES]), "", "response", &mut Vec::new());
        assert_eq!((side["truncated"].clone(), side["body"].clone()), (json!(true), Value::Null));
    }

    #[test]
    fn hosts_match_exactly_without_port_and_by_wildcard() {
        let h = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(host_matches("API.Stripe.com:443", &h(&["api.stripe.com"])));
        assert!(!host_matches("evil-api.stripe.com", &h(&["api.stripe.com"])));
        assert!(host_matches("a.example.com", &h(&["*.example.com"])));
        assert!(!host_matches("example.com", &h(&["*.example.com"])));
        assert!(!host_matches("xexample.com", &h(&["*.example.com"])));
    }

    #[test]
    fn only_proxied_upstream_traffic_of_the_workspace_is_kept() {
        let hosts = vec!["api.stripe.com".to_string()];
        assert!(wanted(&entry("api.stripe.com", "forward", "proxy"), "ws1", &hosts));
        assert!(wanted(&entry("api.stripe.com", "reverse", "proxy"), "ws1", &hosts));
        for (origin, via) in [("server", "mock"), ("webhook", "proxy"), ("forward", "mock"), ("forward", "error")] {
            assert!(!wanted(&entry("api.stripe.com", origin, via), "ws1", &hosts), "{origin} {via}");
        }
        assert!(!wanted(&entry("api.stripe.com", "forward", "proxy"), "other", &hosts));
        assert!(!wanted(&entry("api.openai.com", "forward", "proxy"), "ws1", &hosts));
    }

    #[tokio::test]
    async fn start_record_stop_writes_a_masked_cassette() {
        let _root = scratch_data_root();
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        let registry = Arc::new(registry);
        let call = |name: &'static str, payload: Value| {
            let registry = registry.clone();
            async move { registry.invoke(name, payload, CommandContext::internal(registry.clone())).await }
        };
        assert!(call("cassette.recordStop", json!({})).await.is_err());
        assert_eq!(call("cassette.recordStatus", json!({})).await.unwrap()["recording"], Value::Null);
        let start = json!({ "workspaceId": "ws1", "name": "checkout", "hosts": ["api.stripe.com"] });
        let started = call("cassette.recordStart", start.clone()).await.unwrap();
        assert_eq!(started["path"], "cassettes/checkout.json");
        assert_eq!(started["replaced"], false);
        let busy = call("cassette.recordStart", json!({ "workspaceId": "ws1", "name": "x", "hosts": ["a.com"] })).await;
        assert!(busy.unwrap_err().to_string().contains("already recording"));
        bus().emit("log.entry", entry("api.stripe.com", "forward", "proxy"));
        bus().emit("log.entry", entry("api.stripe.com", "server", "mock"));
        bus().emit("log.entry", entry("api.stripe.com", "forward", "proxy"));
        let status = call("cassette.recordStatus", json!({})).await.unwrap();
        assert_eq!(status["recording"]["interactions"], 2);
        let stopped = call("cassette.recordStop", json!({})).await.unwrap();
        assert_eq!((stopped["interactions"].clone(), stopped["masked"].clone()), (json!(2), json!(12)));
        let file: Value =
            serde_json::from_str(&std::fs::read_to_string(ws_dir("ws1").join(DIR).join("checkout.json")).unwrap())
                .unwrap();
        assert_eq!(file["version"], 1);
        assert_eq!(file["interactions"].as_array().unwrap().len(), 2);
        assert_eq!(file["interactions"][0]["request"]["headers"]["authorization"], MASK);
        // A second recording of the same name replaces it.
        assert_eq!(call("cassette.recordStart", start).await.unwrap()["replaced"], true);
        call("cassette.recordStop", json!({})).await.unwrap();
    }
}
