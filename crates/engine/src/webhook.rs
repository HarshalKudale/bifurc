//! The webhook receiver (`webhook.listen`): a plain-HTTP listener on 127.0.0.1 that captures every inbound request
//! (any method, any path) with its exact headers and raw body, and answers 200 or, with a forward URL, passes the
//! request on and returns the upstream's answer. Entries carry origin `webhook` and, when the listener is
//! named, target `webhook:<name>` (no stored-shape change: both fields exist on every capture entry).

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

use crate::{
    kernel::{
        CommandContext, CommandRegistry, bus,
        traffic::{Headers, RequestLogEntry, emit_log},
    },
    mocks::write_response,
    protocol::{EngineError, ErrorCode, Result},
    proxy::{
        finished_entry, forward_streaming, log_body, log_outcome,
        wire::{Duplicates, parse_header_lines, read_body, read_head},
    },
    utils::{
        id::{mk_id, now_ms},
        json::{bool_of, opt_str, str_of},
    },
};

struct Running {
    id: String,
    port: u16,
    name: Option<String>,
    forward: Option<String>,
    workspace_id: String,
    /// The session that started it; the listener ends with it. `None`: it runs until stopped.
    owner: Option<String>,
    hits: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

struct Listening {
    workspace_id: String,
    name: Option<String>,
    forward: Option<url::Url>,
    hits: Arc<AtomicU64>,
}

/// The forward URL with the request's path appended to its own path, and the request's query.
fn forward_url(base: &url::Url, target: &str) -> String {
    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    };
    let mut url = base.clone();
    url.set_path(&format!("{}{}", base.path().trim_end_matches('/'), path));
    url.set_query(query);
    url.to_string()
}

async fn handle(listening: Arc<Listening>, bound: SocketAddr, mut socket: TcpStream) {
    let Some((head, leftover)) = read_head(&mut socket).await else { return };
    let Ok(body) = read_body(&mut socket, leftover, &head.header_lines).await else { return };
    listening.hits.fetch_add(1, Ordering::Relaxed);
    let t0 = now_ms();
    // Joined, so a header sent twice keeps both values for the replay.
    let headers = parse_header_lines(&head.header_lines, Duplicates::Joined);
    let host = headers.get("host").cloned().unwrap_or_else(|| bound.to_string());
    let target = match url::Url::parse(&head.target) {
        Ok(u) => format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default()),
        Err(_) => head.target.clone(),
    };
    let target = if target.is_empty() { "/".to_string() } else { target };
    let tag = listening.name.as_ref().map(|name| format!("webhook:{name}"));
    let base = RequestLogEntry {
        id: mk_id(),
        ts: t0,
        method: head.method.clone(),
        url: format!("http://{host}{target}"),
        host: host.split(':').next().unwrap_or_default().to_string(),
        req_headers: headers.clone(),
        // The raw bytes, base64: a signature over the body still verifies when the entry is replayed.
        req_body: {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&body)
        },
        workspace_id: Some(listening.workspace_id.clone()),
        origin: Some("webhook".to_string()),
        forward: listening.forward.as_ref().map(|u| u.to_string()),
        ..Default::default()
    };

    if let Some(forward) = &listening.forward {
        let mut forwarded = headers;
        forwarded.shift_remove("host");
        let url = forward_url(forward, &target);
        if let Some(outcome) =
            forward_streaming(&mut socket, &head.method, &url, forward.as_str(), &forwarded, &body, Some(&base.id))
                .await
        {
            // forward_streaming's own 502 (nothing answered) has no headers; an upstream's 502 always has some.
            let via = if outcome.status == 502 && outcome.res_headers.is_empty() { "error" } else { "proxy" };
            log_outcome(&base, via, tag, outcome);
        }
    } else {
        let answer = br#"{"ok":true}"#;
        let mut res_headers = Headers::new();
        res_headers.insert("content-type".into(), "application/json".into());
        res_headers.insert("content-length".into(), answer.len().to_string());
        res_headers.insert("connection".into(), "close".into());
        emit_log(&finished_entry(&base, 200, "webhook", tag, now_ms() - t0, res_headers.clone(), log_body(answer)));
        let _ = write_response(&mut socket, 200, &res_headers, answer).await;
    }
}

fn listen(params: &Value, ctx: &CommandContext) -> Result<Value> {
    let workspace_id = str_of(params, "workspaceId").to_string();
    let port = params.get("port").and_then(Value::as_u64).map_or(0, |p| p as u16);
    let name = opt_str(params, "name").map(str::to_string);
    let forward = opt_str(params, "forward").map(str::to_string);
    let parsed = forward
        .as_deref()
        .map(|raw| {
            url::Url::parse(raw)
                .ok()
                .filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
                .ok_or_else(|| {
                    EngineError::bad_request("--forward is a URL, like http://api.localhost/webhooks/stripe")
                })
        })
        .transpose()?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))
        .and_then(|l| l.set_nonblocking(true).map(|()| l))
        .map_err(|err| match err.kind() {
        std::io::ErrorKind::AddrInUse => EngineError::conflict(format!("Port {port} is already in use"))
            .keyed("server.port_in_use", json!({ "port": port })),
        _ => EngineError::engine(format!("cannot listen on 127.0.0.1:{port}: {err}")),
    })?;
    let bound = listener.local_addr().map_err(|err| EngineError::engine(err.to_string()))?;
    let listener = TcpListener::from_std(listener).map_err(|err| EngineError::engine(err.to_string()))?;
    let listening = Arc::new(Listening {
        workspace_id: workspace_id.clone(),
        name: name.clone(),
        forward: parsed,
        hits: Arc::default(),
    });
    let hits = listening.hits.clone();
    let task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((socket, _)) => {
                    let _ = socket.set_nodelay(true);
                    tokio::spawn(handle(listening.clone(), bound, socket));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    });
    let owner = (!bool_of(params, "detach", false)).then(|| ctx.session.as_ref().map(|s| s.session_id.clone()));
    let id = mk_id();
    let started =
        Running { id: id.clone(), port: bound.port(), name, forward, workspace_id, owner: owner.flatten(), hits, task };
    RUNNING.lock().push(started);
    announce();
    Ok(json!({ "id": id, "port": bound.port(), "url": format!("http://127.0.0.1:{}", bound.port()) }))
}

fn stop(params: &Value) -> Result<Value> {
    let wanted = opt_str(params, "id");
    let mut running = RUNNING.lock();
    let (gone, kept): (Vec<Running>, Vec<Running>) =
        std::mem::take(&mut *running).into_iter().partition(|l| wanted.is_none_or(|id| l.id == id));
    *running = kept;
    if gone.is_empty() {
        return Err(EngineError::not_found(match wanted {
            Some(id) => format!("no webhook listener \"{id}\" is running"),
            None => "no webhook listener is running".to_string(),
        }));
    }
    gone.iter().for_each(|l| l.task.abort());
    let stopped: Vec<Value> = gone.iter().map(|l| json!({ "id": l.id, "port": l.port, "name": l.name })).collect();
    Ok(json!({ "stopped": stopped }))
}

/// Stop the listeners a session started without `detach`; called when it ends.
pub fn stop_owned_by(session_id: &str) {
    let removed = {
        let mut running = RUNNING.lock();
        let before = running.len();
        running.retain(|l| {
            let owned = l.owner.as_deref() == Some(session_id);
            if owned {
                l.task.abort();
            }
            !owned
        });
        before - running.len()
    };
    if removed > 0 {
        announce();
    }
}

/// Tell clients the listeners changed; the payload is `webhook.list`'s, sent after the lock is released.
fn announce() {
    bus().emit("webhook.changed", list());
}

/// `scheme://authority` of a URL, and what follows it.
fn split_origin(url: &str) -> (&str, &str) {
    let from = url.find("://").map_or(0, |at| at + 3);
    let end = url[from..].find(['/', '?', '#']).map_or(url.len(), |at| from + at);
    url.split_at(end)
}

/// Where a webhook is sent again: `to` (host only: the entry's path and query go on it; with a path of its
/// own: used whole), else the listener's forward URL, else the URL it came in on.
fn replay_target(entry_url: &str, forward: Option<&str>, to: Option<&str>) -> Result<String> {
    let path = match split_origin(entry_url).1 {
        "" => "/",
        path => path,
    };
    if let Some(to) = to {
        let (origin, rest) = split_origin(to);
        if !to.contains("://") || origin.ends_with("://") {
            return Err(EngineError::bad_request(format!("--to is a URL like http://127.0.0.1:4021, not \"{to}\"")));
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

/// `webhook.replay`: the captured request again, with its method, headers and raw body bytes, through
/// `request.replay` so the workspace policy still guards it.
async fn replay(params: Value, ctx: CommandContext) -> Result<Value> {
    let (workspace_id, id) = (str_of(&params, "workspaceId"), str_of(&params, "id"));
    let entry = crate::capture::get_capture(workspace_id, id)
        .filter(|entry| entry["origin"] == "webhook")
        .ok_or_else(|| EngineError::not_found(format!("no webhook \"{id}\" (see: bifurc webhooks list)")))?;
    let to = opt_str(&params, "to");
    // The entry's own forward (new entries); older entries fall back to the running listener's.
    let mut forward = entry["forward"].as_str().map(str::to_string);
    let mut stopped = None;
    if to.is_none()
        && forward.is_none()
        && let Some(name) = entry["target"].as_str().and_then(|t| t.strip_prefix("webhook:"))
    {
        let running = RUNNING.lock();
        let listener = running.iter().find(|l| l.name.as_deref() == Some(name));
        forward = listener.and_then(|l| l.forward.clone());
        if listener.is_none() {
            stopped = Some(name.to_string());
        }
    }
    let as_text = |value: &Value| value.as_str().map_or_else(|| value.to_string(), str::to_string);
    let url = replay_target(entry["url"].as_str().unwrap_or_default(), forward.as_deref(), to)?;
    // The body was read whole (no chunking), and the HTTP client sets its own host and length.
    let headers: serde_json::Map<String, Value> = entry["reqHeaders"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, _)| !["host", "content-length", "transfer-encoding"].contains(&name.to_lowercase().as_str()))
        .map(|(name, value)| (name.clone(), json!(as_text(value))))
        .collect();
    let payload = json!({ "method": as_text(&entry["method"]), "url": url, "headers": headers,
                          "body": entry["reqBody"].as_str().unwrap_or_default() });
    ctx.registry.invoke("request.replay", payload, ctx.clone()).await.map_err(|mut err| {
        // Only a listener without a stored forward lands here: its own port is what refused.
        if let (Some(name), ErrorCode::EngineError | ErrorCode::UpstreamFailed | ErrorCode::Timeout) =
            (stopped, err.code)
        {
            err.message = format!("{}; the listener \"{name}\" is stopped, use --to <url>", err.message);
        }
        err
    })
}

fn list() -> Value {
    let listeners: Vec<Value> = RUNNING
        .lock()
        .iter()
        .map(|l| {
            json!({ "id": l.id, "port": l.port, "name": l.name, "forward": l.forward, "workspaceId": l.workspace_id,
                    "detached": l.owner.is_none(), "hits": l.hits.load(Ordering::Relaxed) })
        })
        .collect();
    json!({ "listeners": listeners })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("webhook.listen", |params, ctx| listen(&params, ctx));
    registry.register_sync("webhook.stop", |params, _| stop(&params).inspect(|_| announce()));
    registry.register("webhook.replay", replay);
    registry.register_sync("webhook.list", |_, _| Ok(list()));
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use base64::Engine as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::{
        kernel::{SessionIdentity, bus},
        rest::test_server,
    };

    fn registry() -> Arc<CommandRegistry> {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        crate::rest::commands::register(&mut registry);
        Arc::new(registry)
    }

    async fn call(registry: &Arc<CommandRegistry>, action: &str, payload: Value) -> Result<Value> {
        registry.invoke(action, payload, CommandContext::internal(registry.clone())).await
    }

    async fn start(registry: &Arc<CommandRegistry>, extra: Value) -> Value {
        let mut payload = json!({ "workspaceId": "ws1", "detach": true });
        for (key, value) in extra.as_object().unwrap() {
            payload[key] = value.clone();
        }
        call(registry, "webhook.listen", payload).await.unwrap()
    }

    /// Send a raw request, return everything the listener answered.
    async fn send(port: u16, method: &str, path: &str, extra_headers: &str, body: &[u8]) -> Vec<u8> {
        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{extra_headers}Content-Length: {}\r\n\r\n",
            body.len()
        );
        socket.write_all(&[head.as_bytes(), body].concat()).await.unwrap();
        let mut out = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut out)).await.unwrap().unwrap();
        out
    }

    /// The log entries the listeners emit.
    struct Logs(Arc<Mutex<Vec<Value>>>, crate::kernel::ListenerId);

    impl Logs {
        fn new() -> Self {
            let entries = Arc::new(Mutex::new(Vec::new()));
            let sink = entries.clone();
            Self(entries, bus().on("log.entry", move |entry| sink.lock().push(entry.clone())))
        }

        async fn of(&self, port: u16, path: &str) -> Value {
            let url = format!("http://127.0.0.1:{port}{path}");
            for _ in 0..200 {
                if let Some(found) = self.0.lock().iter().find(|e| e["url"] == url) {
                    return found.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("no log entry for {url}");
        }
    }

    impl Drop for Logs {
        fn drop(&mut self) {
            bus().off(self.1);
        }
    }

    #[test]
    fn the_request_path_and_query_are_appended_to_the_forward_url() {
        let base = url::Url::parse("http://api.localhost/webhooks/stripe/").unwrap();
        assert_eq!(forward_url(&base, "/a/b?x=1"), "http://api.localhost/webhooks/stripe/a/b?x=1");
        let bare = url::Url::parse("http://api.localhost").unwrap();
        assert_eq!(forward_url(&bare, "/"), "http://api.localhost/");
    }

    #[tokio::test]
    async fn a_request_is_captured_with_exact_headers_and_raw_body_and_answered_200() {
        let logs = Logs::new();
        let registry = registry();
        let started = start(&registry, json!({ "port": 0, "name": "stripe" })).await;
        let port = started["port"].as_u64().unwrap() as u16;
        assert_eq!(started["url"], format!("http://127.0.0.1:{port}"));

        let body: &[u8] = &[0xff, 0xfe, b'{', 0x00, 0x80, b'}'];
        let sig = "Stripe-Signature: t=1,v1=abc\r\nX-Dup: a\r\nX-Dup: b\r\n";
        let answer = send(port, "PUT", "/hooks/x?y=1", sig, body).await;
        let answer = String::from_utf8_lossy(&answer).into_owned();
        assert!(answer.starts_with("HTTP/1.1 200 ") && answer.ends_with(r#"{"ok":true}"#), "{answer}");

        let entry = logs.of(port, "/hooks/x?y=1").await;
        assert_eq!(
            (entry["origin"].as_str(), entry["target"].as_str(), entry["method"].as_str(), entry["status"].as_u64()),
            (Some("webhook"), Some("webhook:stripe"), Some("PUT"), Some(200))
        );
        let stored = base64::engine::general_purpose::STANDARD.decode(entry["reqBody"].as_str().unwrap()).unwrap();
        assert_eq!(stored, body);
        assert_eq!(entry["reqHeaders"]["stripe-signature"], "t=1,v1=abc");
        assert_eq!(entry["reqHeaders"]["x-dup"], "a, b");
        call(&registry, "webhook.stop", json!({ "id": started["id"] })).await.unwrap();
    }

    #[tokio::test]
    async fn a_forwarded_request_returns_the_upstream_answer_and_records_it() {
        let logs = Logs::new();
        let upstream = test_server::start(HashMap::new()).await;
        let registry = registry();
        let started = start(&registry, json!({ "forward": format!("{upstream}/webhooks/stripe") })).await;
        let port = started["port"].as_u64().unwrap() as u16;

        let answer = send(port, "POST", "/a?b=2", "X-Sig: s1\r\n", br#"{"id":"evt_1"}"#).await;
        let answer = String::from_utf8_lossy(&answer).into_owned();
        assert!(answer.starts_with("HTTP/1.1 200 "), "{answer}");
        let echoed: Value = serde_json::from_str(answer.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            (echoed["method"].as_str(), echoed["path"].as_str()),
            (Some("POST"), Some("/webhooks/stripe/a?b=2"))
        );
        assert_eq!(
            (echoed["headers"]["x-sig"].as_str(), echoed["body"].as_str()),
            (Some("s1"), Some(r#"{"id":"evt_1"}"#))
        );

        let entry = logs.of(port, "/a?b=2").await;
        assert_eq!(
            (entry["via"].as_str(), entry["origin"].as_str(), entry["status"].as_u64()),
            (Some("proxy"), Some("webhook"), Some(200))
        );
        assert!(!entry["resBody"].as_str().unwrap().is_empty());
        assert_eq!(entry["forward"], format!("{upstream}/webhooks/stripe"));
        call(&registry, "webhook.stop", json!({ "id": started["id"] })).await.unwrap();
    }

    #[test]
    fn an_entry_stored_without_a_forward_still_loads() {
        let old = json!({ "id": "a", "ts": 1, "method": "POST", "url": "http://h/", "host": "h", "reqHeaders": {},
                          "reqBody": "", "status": 200, "via": "webhook", "target": "webhook:x", "durationMs": 1,
                          "resHeaders": {}, "resBody": "", "resStatus": 200 });
        let entry: RequestLogEntry = serde_json::from_value(old).unwrap();
        assert_eq!(entry.forward, None);
        assert!(serde_json::to_value(&entry).unwrap().get("forward").is_none());
    }

    #[tokio::test]
    async fn a_refused_forward_is_a_502_and_an_error_entry() {
        let logs = Logs::new();
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let registry = registry();
        let started = start(&registry, json!({ "forward": format!("http://127.0.0.1:{free}/hook") })).await;
        let port = started["port"].as_u64().unwrap() as u16;

        let answer = send(port, "POST", "/p", "", b"{}").await;
        assert!(String::from_utf8_lossy(&answer).starts_with("HTTP/1.1 502 "));
        let entry = logs.of(port, "/p").await;
        assert_eq!((entry["via"].as_str(), entry["status"].as_u64()), (Some("error"), Some(502)));
        call(&registry, "webhook.stop", json!({ "id": started["id"] })).await.unwrap();
    }

    #[tokio::test]
    async fn listeners_are_listed_stopped_and_owned_by_their_session() {
        let registry = registry();
        let session = SessionIdentity { session_id: "s-hook".into(), ..Default::default() };
        let ctx = CommandContext { registry: registry.clone(), session: Some(session) };
        let owned = registry.invoke("webhook.listen", json!({ "workspaceId": "ws1" }), ctx).await.unwrap();
        let owned_port = owned["port"].as_u64().unwrap() as u16;
        let detached = start(&registry, json!({ "name": "d" })).await;
        send(owned_port, "GET", "/", "", b"").await;

        let listed = call(&registry, "webhook.list", json!({})).await.unwrap();
        let row = listed["listeners"].as_array().unwrap().iter().find(|l| l["id"] == owned["id"]).unwrap().clone();
        assert_eq!((&row["detached"], &row["hits"]), (&json!(false), &json!(1)));

        let busy = call(&registry, "webhook.listen", json!({ "workspaceId": "ws1", "port": owned_port })).await;
        assert_eq!(busy.unwrap_err().code, crate::protocol::ErrorCode::Conflict);
        let bad = call(&registry, "webhook.listen", json!({ "workspaceId": "ws1", "forward": "nope" })).await;
        assert_eq!(bad.unwrap_err().code, crate::protocol::ErrorCode::BadRequest);

        stop_owned_by("s-hook");
        let listed = call(&registry, "webhook.list", json!({})).await.unwrap();
        assert!(listed["listeners"].as_array().unwrap().iter().all(|l| l["id"] != owned["id"]));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(("127.0.0.1", owned_port)).await.is_err());

        call(&registry, "webhook.stop", json!({ "id": detached["id"] })).await.unwrap();
        let none = call(&registry, "webhook.stop", json!({ "id": detached["id"] })).await;
        assert_eq!(none.unwrap_err().code, crate::protocol::ErrorCode::NotFound);
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
        assert_eq!(replay_target(came, None, Some("localhost:3000")).unwrap_err().code, ErrorCode::BadRequest);
    }

    /// A one-shot HTTP server: the whole request it receives (head and body bytes) comes out of the channel.
    async fn catcher() -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                got.extend_from_slice(&buf[..n]);
                if n == 0 {
                    break;
                }
                let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") else { continue };
                let head = String::from_utf8_lossy(&got[..end]).to_lowercase();
                let want: usize = head
                    .split("content-length: ")
                    .nth(1)
                    .and_then(|v| v.split("\r\n").next()?.parse().ok())
                    .unwrap_or(0);
                if got.len() >= end + 4 + want {
                    break;
                }
            }
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok").await;
            let _ = tx.send(got);
        });
        (port, rx)
    }

    fn stored(id: &str, name: &str, forward: Option<&str>, body: &[u8]) -> Value {
        let mut entry = json!({ "id": id, "ts": 1, "workspaceId": "ws1", "method": "POST", "origin": "webhook",
            "url": "http://127.0.0.1:4020/hooks/a?x=1", "host": "127.0.0.1", "target": format!("webhook:{name}"),
            "reqHeaders": { "Host": "127.0.0.1:4020", "Content-Length": "7", "Transfer-Encoding": "chunked",
                            "stripe-signature": "t=1,v1=ab", "x-dup": "a, b" },
            "reqBody": base64::engine::general_purpose::STANDARD.encode(body), "status": 200, "via": "webhook" });
        if let Some(forward) = forward {
            entry["forward"] = json!(forward);
        }
        crate::capture::record_capture(&entry);
        entry
    }

    #[tokio::test]
    async fn replay_sends_the_raw_body_without_the_hop_headers_to_the_target_in_order() {
        let _root = crate::testing::scratch_data_root();
        let registry = registry();
        let raw: &[u8] = &[0xff, 0xfe, 0x00, 0x80, b'{', 0xc3, 0xa9];
        let (port, got) = catcher().await;
        stored("rp-to", "gone", Some("http://127.0.0.1:1/never"), raw);
        // `to` beats the stored forward.
        let to = format!("http://127.0.0.1:{port}");
        let sent =
            call(&registry, "webhook.replay", json!({ "workspaceId": "ws1", "id": "rp-to", "to": to })).await.unwrap();
        assert_eq!(sent["status"], 200);
        let got = got.await.unwrap();
        let end = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&got[..end]).to_lowercase();
        assert!(head.starts_with("post /hooks/a?x=1 "), "{head}");
        assert!(head.contains("stripe-signature: t=1,v1=ab") && head.contains("x-dup: a, b"), "{head}");
        assert!(!head.contains("transfer-encoding") && !head.contains("127.0.0.1:4020"), "{head}");
        assert!(head.contains("content-length: 7"), "{head}");
        assert_eq!(&got[end + 4..], raw);

        // The entry's own forward is next.
        let (port, got) = catcher().await;
        stored("rp-fwd", "gone", Some(&format!("http://127.0.0.1:{port}/in/")), raw);
        call(&registry, "webhook.replay", json!({ "workspaceId": "ws1", "id": "rp-fwd" })).await.unwrap();
        assert!(String::from_utf8_lossy(&got.await.unwrap()).starts_with("POST /in/hooks/a?x=1 "));
    }

    #[tokio::test]
    async fn replay_falls_back_to_the_running_listener_and_names_a_stopped_one() {
        let _root = crate::testing::scratch_data_root();
        let registry = registry();
        let (port, got) = catcher().await;
        let started =
            start(&registry, json!({ "name": "rp-live", "forward": format!("http://127.0.0.1:{port}/l") })).await;
        stored("rp-old", "rp-live", None, b"{}");
        call(&registry, "webhook.replay", json!({ "workspaceId": "ws1", "id": "rp-old" })).await.unwrap();
        assert!(String::from_utf8_lossy(&got.await.unwrap()).starts_with("POST /l/hooks/a?x=1 "));
        call(&registry, "webhook.stop", json!({ "id": started["id"] })).await.unwrap();

        // Stopped, no forward stored: the original address refuses, and the error says why.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut entry = stored("rp-stopped", "rp-gone", None, b"{}");
        entry["url"] = json!(format!("http://127.0.0.1:{free}/x"));
        crate::capture::record_capture(&entry);
        let err =
            call(&registry, "webhook.replay", json!({ "workspaceId": "ws1", "id": "rp-stopped" })).await.unwrap_err();
        assert!(err.message.ends_with("the listener \"rp-gone\" is stopped, use --to <url>"), "{}", err.message);
        let missing = call(&registry, "webhook.replay", json!({ "workspaceId": "ws1", "id": "nope" })).await;
        assert_eq!(missing.unwrap_err().code, ErrorCode::NotFound);
        let bad = call(&registry, "webhook.replay", json!({ "workspaceId": "ws1", "id": "rp-old", "to": "x" }));
        assert_eq!(bad.await.unwrap_err().code, ErrorCode::BadRequest);
    }

    #[tokio::test]
    async fn listen_stop_and_a_closing_session_announce_the_listeners() {
        let seen = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = seen.clone();
        let listener = bus().on("webhook.changed", move |list| sink.lock().push(list.clone()));
        let registry = registry();
        let session = SessionIdentity { session_id: "s-announce".into(), ..Default::default() };
        let ctx = CommandContext { registry: registry.clone(), session: Some(session) };
        let owned = registry.invoke("webhook.listen", json!({ "workspaceId": "ws1" }), ctx).await.unwrap();
        let detached = start(&registry, json!({})).await;
        // Other tests announce too: look for our own ids.
        let listed = |wanted: &Value, present: bool| {
            seen.lock().iter().any(|list| {
                list["listeners"].as_array().unwrap().iter().any(|l| l["id"] == *wanted) == present
                    && list["listeners"].as_array().unwrap().iter().any(|l| l["id"] == owned["id"])
            })
        };
        assert!(listed(&detached["id"], true));
        call(&registry, "webhook.stop", json!({ "id": detached["id"] })).await.unwrap();
        assert!(listed(&detached["id"], false));
        stop_owned_by("s-announce");
        assert!(
            seen.lock().iter().any(|list| list["listeners"].as_array().unwrap().iter().all(|l| l["id"] != owned["id"])
                && list["listeners"].as_array().unwrap().iter().all(|l| l["id"] != detached["id"]))
        );
        bus().off(listener);
    }
}
