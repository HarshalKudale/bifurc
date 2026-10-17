//! The local mock server (`mocks.serve`): the workspace's enabled mocks over plain HTTP on a local port, for
//! clients that ignore proxy settings. A request has no meaningful host here, so the host part of a mock's
//! pattern is ignored (see [`match_local`]). Mocks are read again for every request, so a scenario switch
//! takes effect at once.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
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

use super::{is_streaming_mock, resolve_mock_only_response, serve_mock, serve_resolved_response, write_response};
use crate::{
    env::resolve_vars_in,
    kernel::{
        CommandContext, CommandRegistry,
        traffic::{Headers, RequestLogEntry, emit_log},
    },
    protocol::{EngineError, Result},
    proxy::{
        finished_entry, forward_streaming, log_body, log_outcome,
        wire::{Duplicates, parse_header_lines, read_body, read_head},
    },
    utils::{
        id::{mk_id, now_ms},
        json::{bool_of, opt_str, str_of},
    },
    workspace::{
        config::load_workspace_dispatch_slice,
        dispatch::{load_enabled_sets, workspace_cfg},
        models::{Environment, Mock, Routing},
    },
};

const DEFAULT_PORT: u16 = 4010;

struct Running {
    port: u16,
    workspace_id: String,
    fallthrough: Option<String>,
    /// The session that started it; the server ends with it. `None`: it runs until stopped.
    owner: Option<String>,
    hits: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

static RUNNING: Mutex<Vec<Running>> = Mutex::new(Vec::new());

struct Served {
    workspace_id: String,
    fallthrough: Option<String>,
    hits: Arc<AtomicU64>,
}

/// A pattern's path and query: a URL without its scheme and host, anything else as it is.
fn path_of(pattern: &str) -> &str {
    match pattern.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/", |at| &rest[at..]),
        None => pattern,
    }
}

/// `scheme://host:port` of a URL pattern.
fn origin_of(pattern: &str) -> Option<&str> {
    let (scheme, rest) = pattern.split_once("://")?;
    Some(&pattern[..scheme.len() + 3 + rest.find('/').unwrap_or(rest.len())])
}

/// The first enabled mock that answers `method target` (path and query). A plain pattern matches when its path
/// and query equal the target's; a regex pattern is tried against `http://<host><target>` and against each
/// origin the workspace's plain patterns name, followed by the target.
fn match_local<'a>(
    mocks: &'a [Mock],
    method: &str,
    host: &str,
    target: &str,
    env: Option<&Environment>,
) -> Option<&'a Mock> {
    let patterns: Vec<String> = mocks.iter().map(|m| resolve_vars_in(&m.url_pattern, env)).collect();
    let mut origins = vec![format!("http://{host}")];
    for (mock, pattern) in mocks.iter().zip(&patterns) {
        if let Some(origin) = origin_of(pattern).filter(|_| mock.enabled && !mock.use_regex)
            && !origins.iter().any(|seen| seen == origin)
        {
            origins.push(origin.to_string());
        }
    }
    mocks.iter().zip(&patterns).find_map(|(mock, pattern)| {
        let hit = mock.enabled
            && (mock.method == "*" || mock.method.eq_ignore_ascii_case(method))
            && if mock.use_regex {
                origins.iter().any(|origin| crate::utils::regex::is_match(pattern, &format!("{origin}{target}")))
            } else {
                path_of(pattern) == target
            };
        hit.then_some(mock)
    })
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = std::mem::replace(&mut row[0], i + 1);
        for (j, cb) in b.iter().enumerate() {
            let next = (diagonal + usize::from(ca != *cb)).min(row[j] + 1).min(row[j + 1] + 1);
            diagonal = std::mem::replace(&mut row[j + 1], next);
        }
    }
    row[b.len()]
}

/// Up to three enabled mocks whose path is nearest to the requested one.
fn closest(mocks: &[Mock], names: &HashMap<String, String>, env: Option<&Environment>, target: &str) -> Vec<Value> {
    let wanted = target.split('?').next().unwrap_or(target);
    let mut near: Vec<(usize, Value)> = mocks
        .iter()
        .filter(|m| m.enabled)
        .map(|m| {
            let pattern = resolve_vars_in(&m.url_pattern, env);
            let shown = if m.use_regex { pattern } else { path_of(&pattern).to_string() };
            let distance = edit_distance(wanted, shown.split('?').next().unwrap_or(&shown));
            let name = names.get(&m.id).cloned().unwrap_or_default();
            (distance, json!({ "name": name, "method": m.method, "pattern": shown }))
        })
        .collect();
    near.sort_by_key(|(distance, _)| *distance);
    near.into_iter().take(3).map(|(_, mock)| mock).collect()
}

/// The workspace's enabled mocks as they are right now, with their names.
fn snapshot(workspace_id: &str) -> (Routing, HashMap<String, String>) {
    let slice = load_workspace_dispatch_slice(workspace_id);
    let names = slice.mocks.iter().map(|m| (str_of(m, "id").to_string(), str_of(m, "name").to_string())).collect();
    let dispatch = workspace_cfg(&json!({}), &[], &load_enabled_sets(workspace_id), &slice);
    (Routing::from_cfg(&dispatch), names)
}

async fn handle(served: Arc<Served>, bound: SocketAddr, mut socket: TcpStream) {
    let Some((head, leftover)) = read_head(&mut socket).await else { return };
    let Ok(body) = read_body(&mut socket, leftover, &head.header_lines).await else { return };
    served.hits.fetch_add(1, Ordering::Relaxed);
    let t0 = now_ms();
    let headers = parse_header_lines(&head.header_lines, Duplicates::LastWins);
    let host = headers.get("host").cloned().unwrap_or_else(|| bound.to_string());
    let target = match url::Url::parse(&head.target) {
        Ok(u) => format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default()),
        Err(_) => head.target.clone(),
    };
    let target = if target.is_empty() { "/".to_string() } else { target };
    let base = RequestLogEntry {
        id: mk_id(),
        ts: t0,
        method: head.method.clone(),
        url: format!("http://{host}{target}"),
        host: host.split(':').next().unwrap_or_default().to_string(),
        req_headers: headers.clone(),
        req_body: {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&body)
        },
        workspace_id: Some(served.workspace_id.clone()),
        origin: Some("server".to_string()),
        ..Default::default()
    };

    let (routing, names) = snapshot(&served.workspace_id);
    let env = routing.env.as_ref();
    if let Some(mock) = match_local(&routing.mocks, &head.method, &host, &target, env) {
        let resolved = resolve_mock_only_response(mock, env);
        emit_log(&finished_entry(
            &base,
            resolved.status,
            "mock",
            Some(format!("mock:{}", mock.id)),
            now_ms() - t0 + resolved.delay_ms as i64,
            resolved.headers.clone(),
            log_body(&resolved.body),
        ));
        let _ = if is_streaming_mock(mock) {
            serve_mock(&mut socket, mock, env).await
        } else {
            serve_resolved_response(&mut socket, &resolved).await
        };
    } else if let Some(origin) = &served.fallthrough {
        let mut forwarded = headers;
        forwarded.shift_remove("host");
        let url = format!("{}{target}", origin.trim_end_matches('/'));
        if let Some(outcome) =
            forward_streaming(&mut socket, &head.method, &url, origin, &forwarded, &body, Some(&base.id)).await
        {
            log_outcome(&base, "proxy", Some(origin.clone()), outcome);
        }
    } else {
        let answer = json!({
            "error": format!("no mock matches {} {target}", head.method),
            "closest": closest(&routing.mocks, &names, env, &target),
        });
        let body = answer.to_string().into_bytes();
        let mut res_headers = Headers::new();
        res_headers.insert("content-type".into(), "application/json".into());
        res_headers.insert("content-length".into(), body.len().to_string());
        res_headers.insert("connection".into(), "close".into());
        emit_log(&finished_entry(&base, 404, "error", None, now_ms() - t0, res_headers.clone(), log_body(&body)));
        let _ = write_response(&mut socket, 404, &res_headers, &body).await;
    }
}

fn serve(params: &Value, ctx: &CommandContext) -> Result<Value> {
    let workspace_id = str_of(params, "workspaceId").to_string();
    let host = opt_str(params, "host").unwrap_or("127.0.0.1");
    let addr: IpAddr = host
        .parse()
        .map_err(|_| EngineError::bad_request(format!("\"{host}\" is not an IP address (try 127.0.0.1 or 0.0.0.0)")))?;
    let port = params.get("port").and_then(Value::as_u64).map_or(DEFAULT_PORT, |p| p as u16);
    let fallthrough = opt_str(params, "fallthrough").map(str::to_string);
    if fallthrough.as_deref().is_some_and(|url| !url.starts_with("http://") && !url.starts_with("https://")) {
        return Err(EngineError::bad_request("--fallthrough is a base URL, like http://localhost:3000"));
    }
    let listener = std::net::TcpListener::bind((addr, port))
        .and_then(|l| l.set_nonblocking(true).map(|()| l))
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::AddrInUse => EngineError::conflict(format!("Port {port} is already in use"))
                .keyed("server.port_in_use", json!({ "port": port })),
            _ => EngineError::engine(format!("cannot listen on {host}:{port}: {err}")),
        })?;
    let bound = listener.local_addr().map_err(|err| EngineError::engine(err.to_string()))?;
    let listener = TcpListener::from_std(listener).map_err(|err| EngineError::engine(err.to_string()))?;
    let served =
        Arc::new(Served { workspace_id: workspace_id.clone(), fallthrough: fallthrough.clone(), hits: Arc::default() });
    let hits = served.hits.clone();
    let task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((socket, _)) => {
                    let _ = socket.set_nodelay(true);
                    tokio::spawn(handle(served.clone(), bound, socket));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    });
    let owner = (!bool_of(params, "detach", false)).then(|| ctx.session.as_ref().map(|s| s.session_id.clone()));
    let mocks = snapshot(&workspace_id).0.mocks.iter().filter(|m| m.enabled).count();
    RUNNING.lock().push(Running { port: bound.port(), workspace_id, fallthrough, owner: owner.flatten(), hits, task });
    Ok(json!({ "port": bound.port(), "url": format!("http://{host}:{}", bound.port()), "mocks": mocks }))
}

fn stop(params: &Value) -> Result<Value> {
    let wanted = params.get("port").and_then(Value::as_u64);
    let mut running = RUNNING.lock();
    let (gone, kept): (Vec<Running>, Vec<Running>) =
        std::mem::take(&mut *running).into_iter().partition(|s| wanted.is_none_or(|port| u64::from(s.port) == port));
    *running = kept;
    if gone.is_empty() {
        return Err(EngineError::not_found(match wanted {
            Some(port) => format!("no mock server is running on port {port}"),
            None => "no mock server is running".to_string(),
        }));
    }
    let ports: Vec<u16> = gone.iter().map(|s| s.port).collect();
    gone.iter().for_each(|s| s.task.abort());
    Ok(json!({ "stopped": ports }))
}

/// Stop the servers a session started without `detach`; called when it ends.
pub fn stop_owned_by(session_id: &str) {
    RUNNING.lock().retain(|server| {
        let owned = server.owner.as_deref() == Some(session_id);
        if owned {
            server.task.abort();
        }
        !owned
    });
}

fn status() -> Value {
    let servers: Vec<Value> = RUNNING
        .lock()
        .iter()
        .map(|s| {
            json!({ "port": s.port, "workspaceId": s.workspace_id, "fallthrough": s.fallthrough,
                    "detached": s.owner.is_none(), "hits": s.hits.load(Ordering::Relaxed) })
        })
        .collect();
    json!({ "servers": servers })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("mocks.serve", |params, ctx| serve(&params, ctx));
    registry.register_sync("mocks.serveStop", |params, _| stop(&params));
    registry.register_sync("mocks.serveStatus", |_, _| Ok(status()));
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::{
        kernel::{SessionIdentity, bus},
        rest::test_server,
        testing::scratch_data_root,
        workspace::fs::{sync_enabled_set, write_entity},
    };

    fn put(id: &str, name: &str, pattern: &str, extra: Value) {
        let mut mock = json!({
            "id": id, "name": name, "workspaceId": "ws1", "method": "GET", "urlPattern": pattern, "useRegex": false,
            "responseStatus": 201, "responseHeaders": {}, "responseBody": format!("body of {id}"),
            "responseBodyEncoding": "utf8", "responseDelay": 0, "createdAt": 0,
        });
        for (key, value) in extra.as_object().unwrap() {
            mock[key] = value.clone();
        }
        write_entity("ws1", "mocks", id, &mock, None).unwrap();
        sync_enabled_set("ws1", "mocks", id, true).unwrap();
    }

    fn registry() -> Arc<CommandRegistry> {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        Arc::new(registry)
    }

    async fn call(registry: &Arc<CommandRegistry>, action: &str, payload: Value) -> Result<Value> {
        registry.invoke(action, payload, CommandContext::internal(registry.clone())).await
    }

    async fn start(registry: &Arc<CommandRegistry>, extra: Value) -> u16 {
        let mut payload = json!({ "workspaceId": "ws1", "port": 0, "detach": true });
        for (key, value) in extra.as_object().unwrap() {
            payload[key] = value.clone();
        }
        call(registry, "mocks.serve", payload).await.unwrap()["port"].as_u64().unwrap() as u16
    }

    async fn get(port: u16, path: &str) -> String {
        let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut out)).await.unwrap().unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    /// The log entries the servers emit.
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
    fn paths_origins_and_distances() {
        assert_eq!(path_of("http://a.test/orders?x=1"), "/orders?x=1");
        assert_eq!((path_of("/orders"), path_of("http://a.test")), ("/orders", "/"));
        assert_eq!(origin_of("https://a.test:8080/x"), Some("https://a.test:8080"));
        assert_eq!(origin_of("/x"), None);
        assert_eq!(edit_distance("/orderz", "/orders"), 1);
        assert_eq!((edit_distance("", "ab"), edit_distance("abc", "abc")), (2, 0));
    }

    #[tokio::test]
    async fn serves_mocks_ignoring_the_host_and_follows_scenario_switches() {
        let _root = scratch_data_root();
        let logs = Logs::new();
        put("m1", "Get orders", "http://api.localhost/orders", json!({}));
        put("re", "Items", "^http://api\\.localhost/items/\\d+$", json!({ "useRegex": true, "method": "*" }));
        put("m2", "Get orders · empty", "http://api.localhost/orders", json!({ "responseBody": "[]" }));
        sync_enabled_set("ws1", "mocks", "m2", false).unwrap();
        let registry = registry();
        let port = start(&registry, json!({})).await;

        let ok = get(port, "/orders").await;
        assert!(ok.starts_with("HTTP/1.1 201 "), "{ok}");
        assert!(ok.ends_with("body of m1"));
        // A regex mock is tried against the origins the plain patterns name.
        assert!(get(port, "/items/7").await.starts_with("HTTP/1.1 201 "));
        assert!(get(port, "/items/x").await.starts_with("HTTP/1.1 404 "));
        // The query is part of what a plain pattern matches.
        assert!(get(port, "/orders?x=1").await.starts_with("HTTP/1.1 404 "));

        let entry = logs.of(port, "/orders").await;
        assert_eq!(entry["origin"], "server");
        assert_eq!((entry["via"].as_str(), entry["target"].as_str()), (Some("mock"), Some("mock:m1")));
        assert_eq!(entry["workspaceId"], "ws1");

        // Switching the scenario is seen by the next request.
        sync_enabled_set("ws1", "mocks", "m1", false).unwrap();
        sync_enabled_set("ws1", "mocks", "m2", true).unwrap();
        assert!(get(port, "/orders").await.ends_with("[]"));
        call(&registry, "mocks.serveStop", json!({ "port": port })).await.unwrap();
    }

    #[tokio::test]
    async fn an_unmatched_request_is_a_404_naming_the_closest_mocks() {
        let _root = scratch_data_root();
        let logs = Logs::new();
        put("m1", "Get orders", "http://api.localhost/orders", json!({}));
        put("m3", "Get cart", "http://api.localhost/cart", json!({}));
        let registry = registry();
        let port = start(&registry, json!({})).await;

        let answer = get(port, "/orderz").await;
        assert!(answer.starts_with("HTTP/1.1 404 "), "{answer}");
        assert!(answer.contains("content-type: application/json"));
        let body: Value = serde_json::from_str(answer.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["error"], "no mock matches GET /orderz");
        assert_eq!(body["closest"][0], json!({ "name": "Get orders", "method": "GET", "pattern": "/orders" }));
        assert_eq!(body["closest"].as_array().unwrap().len(), 2);
        let entry = logs.of(port, "/orderz").await;
        assert_eq!(
            (entry["via"].as_str(), entry["origin"].as_str(), entry["status"].as_u64()),
            (Some("error"), Some("server"), Some(404))
        );
        call(&registry, "mocks.serveStop", json!({})).await.unwrap();
    }

    #[tokio::test]
    async fn an_unmatched_request_goes_to_the_fallthrough_base() {
        let _root = scratch_data_root();
        let logs = Logs::new();
        put("m1", "Get orders", "http://api.localhost/orders", json!({}));
        let canned = HashMap::from([("/real".to_string(), ("text/plain".to_string(), "from upstream".to_string()))]);
        let upstream = test_server::start(canned).await;
        let registry = registry();
        let port = start(&registry, json!({ "fallthrough": upstream })).await;

        let answer = get(port, "/real?a=1").await;
        assert!(answer.starts_with("HTTP/1.1 200 ") && answer.ends_with("from upstream"), "{answer}");
        assert!(get(port, "/orders").await.ends_with("body of m1"));
        let entry = logs.of(port, "/real?a=1").await;
        assert_eq!((entry["via"].as_str(), entry["origin"].as_str()), (Some("proxy"), Some("server")));
        call(&registry, "mocks.serveStop", json!({})).await.unwrap();
    }

    #[tokio::test]
    async fn servers_are_listed_stopped_and_owned_by_their_session() {
        let _root = scratch_data_root();
        put("m1", "Get orders", "http://api.localhost/orders", json!({}));
        let registry = registry();
        let session = SessionIdentity { session_id: "s-owner".into(), ..Default::default() };
        let ctx = CommandContext { registry: registry.clone(), session: Some(session) };
        let owned = registry.invoke("mocks.serve", json!({ "workspaceId": "ws1", "port": 0 }), ctx).await.unwrap();
        assert_eq!(owned["mocks"], 1);
        let owned_port = owned["port"].as_u64().unwrap() as u16;
        let detached = start(&registry, json!({})).await;
        get(owned_port, "/orders").await;

        let listed = call(&registry, "mocks.serveStatus", json!({})).await.unwrap();
        let row = listed["servers"].as_array().unwrap().iter().find(|s| s["port"] == owned_port).unwrap().clone();
        assert_eq!((&row["detached"], &row["hits"], &row["workspaceId"]), (&json!(false), &json!(1), &json!("ws1")));

        // A busy port is a conflict.
        let busy = call(&registry, "mocks.serve", json!({ "workspaceId": "ws1", "port": owned_port })).await;
        assert_eq!(busy.unwrap_err().code, crate::protocol::ErrorCode::Conflict);

        // Its session ends: its server stops, the detached one stays.
        stop_owned_by("s-owner");
        let listed = call(&registry, "mocks.serveStatus", json!({})).await.unwrap();
        assert!(listed["servers"].as_array().unwrap().iter().all(|s| s["port"] != owned_port));
        assert!(listed["servers"].as_array().unwrap().iter().any(|s| s["port"] == detached && s["detached"] == true));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(TcpStream::connect(("127.0.0.1", owned_port)).await.is_err());

        call(&registry, "mocks.serveStop", json!({ "port": detached })).await.unwrap();
        let none = call(&registry, "mocks.serveStop", json!({ "port": detached })).await;
        assert_eq!(none.unwrap_err().code, crate::protocol::ErrorCode::NotFound);
    }
}
