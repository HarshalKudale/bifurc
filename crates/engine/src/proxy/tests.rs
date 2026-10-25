//! The forward proxy end to end on loopback: a real listener, a canned upstream, entities in a scratch store.

use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use super::*;
use crate::{
    kernel::{CommandContext, bus},
    runtime::instances::{bind_forward_proxy, forward_proxy},
    testing::scratch_data_root,
    workspace::fs::{sync_enabled_set, write_entity, write_flat_entity},
};

/// A server that answers every connection with `response` and records each raw request.
async fn canned_upstream(response: Vec<u8>) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let (response, log) = (response.clone(), log.clone());
            tokio::spawn(async move {
                let mut received = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let Ok(n) = socket.read(&mut buf).await else { return };
                    if n == 0 {
                        break;
                    }
                    received.extend_from_slice(&buf[..n]);
                    if received.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                log.lock().push(String::from_utf8_lossy(&received).into_owned());
                let _ = socket.write_all(&response).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (port, seen)
}

fn http_ok(extra_headers: &str, body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\n{extra_headers}content-length: {}\r\nconnection: close\r\n\r\n{body}", body.len())
        .into_bytes()
}

async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
}

async fn through(port: u16, request: &str) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut out)).await.unwrap().unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

/// Collects the log entries the proxy emits.
struct Logs {
    entries: Arc<Mutex<Vec<Value>>>,
    id: crate::kernel::ListenerId,
}

impl Logs {
    fn capture() -> Self {
        let entries = Arc::new(Mutex::new(Vec::new()));
        let sink = entries.clone();
        let id = bus().on("log.entry", move |entry| sink.lock().push(entry.clone()));
        Self { entries, id }
    }

    async fn find(&self, url: &str) -> Value {
        for _ in 0..200 {
            if let Some(found) = self.entries.lock().iter().find(|e| e["url"] == url) {
                return found.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no log entry for {url}: {:?}", self.entries.lock());
    }
}

impl Drop for Logs {
    fn drop(&mut self) {
        bus().off(self.id);
    }
}

fn start_proxy(port: u16) -> Arc<dyn Listener> {
    let proxy = create_proxy_server();
    proxy.start(port);
    assert!(proxy.error().is_none());
    proxy
}

fn put(kind: &str, entity: Value) {
    let id = entity["id"].as_str().unwrap().to_string();
    if kind == "mocks" {
        write_entity("ws1", kind, &id, &entity, None).unwrap();
    } else {
        write_flat_entity("ws1", kind, &id, &entity).unwrap();
    }
    sync_enabled_set("ws1", kind, &id, true).unwrap();
}

fn mock_entity(id: &str, url: &str, extra: Value) -> Value {
    let mut mock = json!({
        "id": id, "workspaceId": "ws1", "method": "GET", "urlPattern": url, "useRegex": false,
        "responseStatus": 201, "responseHeaders": { "x-mock": "yes" }, "responseBody": "mocked body",
        "responseBodyEncoding": "utf8", "responseDelay": 0, "mockedResponseHeaders": ["x-mock"], "createdAt": 0,
    });
    for (key, value) in extra.as_object().unwrap() {
        mock[key] = value.clone();
    }
    mock
}

#[tokio::test]
async fn passthrough_relays_the_answer_and_logs_it() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let (upstream, seen) = canned_upstream(http_ok("x-up: 1\r\nset-cookie: a=1\r\nset-cookie: b=2\r\n", "hello")).await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let url = format!("http://127.0.0.1:{upstream}/hello?x=1");
    let request = format!(
        "GET {url} HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\nProxy-Connection: keep-alive\r\nX-Client: c\r\n\r\n"
    );
    let answer = through(proxy_port, &request).await;
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert!(
        answer.contains("x-up: 1\r\n")
            && answer.contains("set-cookie: a=1\r\n")
            && answer.contains("set-cookie: b=2\r\n")
    );
    assert!(answer.contains("connection: close\r\n"));
    assert!(answer.ends_with("\r\n\r\nhello"));

    let sent = seen.lock()[0].to_lowercase();
    assert!(sent.starts_with("get /hello?x=1 http/1.1"), "{sent}");
    assert!(sent.contains("x-client: c") && !sent.contains("proxy-connection"));

    let entry = logs.find(&url).await;
    assert_eq!(entry["via"], "proxy");
    assert_eq!(entry["status"], 200);
    assert_eq!(entry["origin"], "forward");
    assert_eq!(entry["workspaceId"], "ws1");
    assert_eq!(entry["target"], "127.0.0.1");
    assert_eq!(entry["resHeaders"]["set-cookie"], "a=1\nb=2");
    assert_eq!(entry["resBody"], "aGVsbG8=");
    proxy.stop();
}

#[tokio::test]
async fn a_post_body_reaches_the_upstream_in_full() {
    let _root = scratch_data_root();
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let received = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        while !String::from_utf8_lossy(&data).ends_with("0123456789") {
            let n = socket.read(&mut buf).await.unwrap();
            data.extend_from_slice(&buf[..n]);
        }
        socket.write_all(&http_ok("", "done")).await.unwrap();
        let _ = socket.shutdown().await;
        String::from_utf8_lossy(&data).into_owned()
    });
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let mut socket = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let head = format!(
        "POST http://127.0.0.1:{upstream_port}/p HTTP/1.1\r\nHost: 127.0.0.1:{upstream_port}\r\nContent-Length: 10\r\n\r\n01234"
    );
    socket.write_all(head.as_bytes()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    socket.write_all(b"56789").await.unwrap();
    let mut out = String::new();
    socket.read_to_string(&mut out).await.unwrap();
    assert!(out.ends_with("done"));
    assert!(received.await.unwrap().to_lowercase().contains("content-length: 10"));
    proxy.stop();
}

#[tokio::test]
async fn a_mock_answers_without_the_upstream() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let url = "http://mocked.test/thing";
    put("mocks", mock_entity("m1", url, json!({})));
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let answer = through(proxy_port, &format!("GET {url} HTTP/1.1\r\nHost: mocked.test\r\n\r\n")).await;
    assert!(answer.starts_with("HTTP/1.1 201 Created\r\n"), "{answer}");
    assert!(answer.contains("x-mock: yes\r\n") && answer.ends_with("\r\n\r\nmocked body"));

    let entry = logs.find(url).await;
    assert_eq!((entry["via"].as_str(), entry["target"].as_str()), (Some("mock"), Some("mock:m1")));
    assert_eq!(entry["status"], 201);
    proxy.stop();
}

#[tokio::test]
async fn a_partial_mock_is_laid_over_the_real_answer() {
    let _root = scratch_data_root();
    let (upstream, _) = canned_upstream(http_ok("content-type: text/plain\r\nx-real: r\r\n", "real body")).await;
    let url = format!("http://127.0.0.1:{upstream}/partial");
    put(
        "mocks",
        mock_entity("m2", &url, json!({ "responseBodyMocked": false, "responseHeaders": {}, "responseStatus": 418 })),
    );
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let answer = through(proxy_port, &format!("GET {url} HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\n\r\n")).await;
    assert!(answer.starts_with("HTTP/1.1 418 Unknown\r\n"), "{answer}");
    assert!(answer.contains("x-real: r\r\n") && answer.ends_with("\r\n\r\nreal body"));
    proxy.stop();
}

#[tokio::test]
async fn a_rule_forwards_to_its_target_and_runs_its_scripts() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let (target, seen) = canned_upstream(http_ok("content-type: text/plain\r\n", "abc")).await;
    put(
        "rules",
        json!({
            "id": "r1", "workspaceId": "ws1", "name": "r", "pattern": "^http://ruled\\.test/", "useRegex": true,
            "targetType": "external", "targetExternal": format!("127.0.0.1:{target}"),
            "requestScript": "bc.request.headers['x-added'] = 'req';",
            "responseScript": "bc.response.headers['x-seen'] = '1'; bc.response.body = bc.response.body.toUpperCase();",
            "createdAt": 0,
        }),
    );
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let url = "http://ruled.test/path?q=1";
    let answer = through(proxy_port, &format!("GET {url} HTTP/1.1\r\nHost: ruled.test\r\n\r\n")).await;
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert!(answer.contains("x-seen: 1\r\n") && answer.ends_with("\r\n\r\nABC"));
    assert_eq!(answer.matches("content-length").count(), 1);
    let sent = seen.lock()[0].to_lowercase();
    assert!(
        sent.starts_with("get /path?q=1 http/1.1")
            && sent.contains("x-added: req")
            && sent.contains("host: ruled.test")
    );

    let entry = logs.find(url).await;
    assert_eq!(entry["via"], "rule");
    assert_eq!(entry["target"], format!("127.0.0.1:{target}"));
    proxy.stop();
}

#[tokio::test]
async fn a_rule_pointing_at_a_mapping_uses_its_target() {
    let _root = scratch_data_root();
    let (target, _) = canned_upstream(http_ok("", "via mapping")).await;
    put(
        "mappings",
        json!({ "id": "map1", "workspaceId": "ws1", "domain": "app.localhost", "target": format!("127.0.0.1:{target}"), "createdAt": 0 }),
    );
    put(
        "rules",
        json!({
            "id": "r2", "workspaceId": "ws1", "name": "r", "pattern": "http://m.test/", "useRegex": false,
            "targetType": "mapping", "targetMappingId": "map1", "requestScript": "", "responseScript": "", "createdAt": 0,
        }),
    );
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let answer = through(proxy_port, "GET http://m.test/ HTTP/1.1\r\nHost: m.test\r\n\r\n").await;
    assert!(answer.ends_with("via mapping"), "{answer}");
    proxy.stop();
}

#[tokio::test]
async fn an_unreachable_upstream_is_a_502() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let dead = free_port().await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let url = format!("http://127.0.0.1:{dead}/");
    let answer = through(proxy_port, &format!("GET {url} HTTP/1.1\r\nHost: 127.0.0.1:{dead}\r\n\r\n")).await;
    assert!(answer.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{answer}");
    assert!(answer.contains(&format!("<code>127.0.0.1:{dead}</code>")));
    assert_eq!(logs.find(&url).await["status"], 502);
    proxy.stop();
}

#[tokio::test]
async fn a_relative_request_is_refused_toward_the_edge() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let answer = through(proxy_port, "GET /home HTTP/1.1\r\nHost: app.localhost\r\n\r\n").await;
    assert!(answer.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{answer}");
    assert!(answer.contains("forward-proxy"));
    let entry = logs.find("http://app.localhost/home").await;
    assert_eq!((entry["via"].as_str(), entry["status"].as_u64()), (Some("error"), Some(400)));
    proxy.stop();
}

#[tokio::test]
async fn connect_without_interception_is_a_tunnel() {
    let _root = scratch_data_root();
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let echo_port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = echo.accept().await.unwrap();
        let mut buf = [0u8; 5];
        socket.read_exact(&mut buf).await.unwrap();
        socket.write_all(&buf.map(|b| b.to_ascii_uppercase())).await.unwrap();
    });
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let mut socket = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    socket.write_all(format!("CONNECT 127.0.0.1:{echo_port} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).await.unwrap();
    let mut head = vec![0u8; "HTTP/1.1 200 Connection Established\r\n\r\n".len()];
    socket.read_exact(&mut head).await.unwrap();
    assert_eq!(head, b"HTTP/1.1 200 Connection Established\r\n\r\n");
    socket.write_all(b"hello").await.unwrap();
    let mut echoed = [0u8; 5];
    socket.read_exact(&mut echoed).await.unwrap();
    assert_eq!(&echoed, b"HELLO");
    proxy.stop();
}

#[tokio::test]
async fn connect_to_a_closed_port_is_a_502() {
    let _root = scratch_data_root();
    let dead = free_port().await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let answer = through(proxy_port, &format!("CONNECT 127.0.0.1:{dead} HTTP/1.1\r\nHost: x\r\n\r\n")).await;
    assert_eq!(answer, "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
    proxy.stop();
}

#[tokio::test]
async fn an_event_stream_is_relayed_and_reported_in_chunks() {
    let _root = scratch_data_root();
    let chunks = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = chunks.clone();
    let listener = bus().on("log.chunk", move |chunk| sink.lock().push(chunk.clone()));
    let (upstream, _) = canned_upstream(http_ok("content-type: text/event-stream\r\n", "data: 1\n\n")).await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let url = format!("http://127.0.0.1:{upstream}/events");
    let answer = through(proxy_port, &format!("GET {url} HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\n\r\n")).await;
    assert!(answer.ends_with("data: 1\n\n"));
    for _ in 0..100 {
        if chunks.lock().iter().any(|c| c["done"] == true) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    bus().off(listener);
    let seen = chunks.lock();
    assert!(seen.iter().any(|c| c["done"] == false && c["chunk"] == "ZGF0YTogMQoK"), "{seen:?}");
    assert!(seen.iter().any(|c| c["done"] == true));
    proxy.stop();
}

#[tokio::test]
async fn dispatch_config_serves_what_is_switched_on() {
    let _root = scratch_data_root();
    put("mocks", mock_entity("on", "http://a.test/", json!({})));
    write_entity("ws1", "mocks", "off", &mock_entity("off", "http://b.test/", json!({})), None).unwrap();
    let proxy = create_proxy_server();
    let cfg = proxy.dispatch_config().unwrap();
    let ids: Vec<&str> = cfg["mocks"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["on"]);
    assert_eq!(cfg["activeWorkspaceId"], "ws1");
    put("mocks", mock_entity("later", "http://c.test/", json!({})));
    proxy.reload_config();
    assert_eq!(proxy.dispatch_config().unwrap()["mocks"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn the_server_commands_control_the_bound_proxy() {
    let _root = scratch_data_root();
    let mut registry = crate::kernel::CommandRegistry::new();
    register(&mut registry);
    let registry = Arc::new(registry);
    let ctx = CommandContext::internal(registry.clone());
    let call = |action: &'static str, payload: Value| {
        let (registry, ctx) = (registry.clone(), ctx.clone());
        async move { registry.invoke(action, payload, ctx).await.unwrap() }
    };

    let port = free_port().await;
    let mut settings = crate::workspace::settings::load_settings();
    settings.set("forwardProxyPort", json!(port));
    crate::workspace::settings::save_settings(&settings).unwrap();

    bind_forward_proxy(None);
    assert_eq!(call("proxy.status", json!({})).await, json!({ "running": false }));
    assert_eq!(
        call("server.status", json!({ "workspaceId": "ws1" })).await,
        json!({ "running": false, "port": 0, "error": null, "https": false, "workspaceId": "ws1" })
    );

    assert_eq!(call("server.start", json!({})).await, json!({ "ok": true }));
    assert_eq!(call("proxy.status", json!({})).await, json!({ "running": true }));
    assert_eq!(
        call("server.status", json!({ "target": "forward" })).await,
        json!({ "running": true, "port": port, "error": null, "https": false })
    );

    assert_eq!(call("server.restart", json!({})).await, json!({ "ok": true }));
    for _ in 0..100 {
        if forward_proxy().unwrap().is_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(call("proxy.status", json!({})).await, json!({ "running": true }));
    let answer = through(port, "GET /x HTTP/1.1\r\nHost: h\r\n\r\n").await;
    assert!(answer.starts_with("HTTP/1.1 400"));

    assert_eq!(call("server.stop", json!({})).await, json!({ "ok": true }));
    assert_eq!(call("proxy.status", json!({})).await, json!({ "running": false }));
    assert_eq!(
        call("server.status", json!({ "target": "reverse" })).await,
        json!({
            "running": false, "port": 0, "error": "There is no reverse proxy in this engine.",
            "errorKey": "server.no_reverse", "errorParams": {}, "https": false,
        })
    );
    bind_forward_proxy(None);
}

#[tokio::test]
async fn an_intercepted_https_request_is_routed_like_any_other() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let url = "https://secure.test/resource?a=1";
    put("mocks", mock_entity("tls1", url, json!({})));
    let (server_config, client_config) = tls_intercept::tests::self_signed_pair();
    let (mut client_end, server_end) = tokio::io::duplex(64 * 1024);
    let shared = Arc::new(server::Shared::default());
    let serving = shared.clone();
    let server = tokio::spawn(async move {
        let (mut tls, head, body) = tls_intercept::intercept_tls_with(server_end, server_config).await.unwrap();
        server::dispatch_https(&serving, &mut tls, &head, &body, "secure.test", 443).await;
    });

    let mut established = vec![0u8; tls_intercept::tests::ESTABLISHED.len()];
    client_end.read_exact(&mut established).await.unwrap();
    let connector = tokio_rustls::TlsConnector::from(client_config);
    let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let mut tls = connector.connect(name, client_end).await.unwrap();
    tls.write_all(b"GET /resource?a=1 HTTP/1.1\r\nHost: secure.test\r\n\r\n").await.unwrap();
    let mut answer = String::new();
    let _ = tls.read_to_string(&mut answer).await;
    assert!(answer.starts_with("HTTP/1.1 201 Created\r\n") && answer.ends_with("mocked body"), "{answer}");
    server.await.unwrap();

    let entry = logs.find(url).await;
    assert_eq!((entry["via"].as_str(), entry["host"].as_str()), (Some("mock"), Some("secure.test")));
}

#[tokio::test]
async fn a_rule_script_can_inject_a_secret_that_the_log_shows_unresolved() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let mut vaults = crate::kernel::CommandRegistry::new();
    crate::vault::register(&mut vaults);
    let vaults = Arc::new(vaults);
    let call = |action: &'static str, payload: Value| {
        let (vaults, ctx) = (vaults.clone(), CommandContext::internal(vaults.clone()));
        async move { vaults.invoke(action, payload, ctx).await.unwrap() }
    };
    let created =
        call("vault.create", json!({ "workspaceId": "ws1", "name": "keys", "passphrase": "correct horse battery" }))
            .await;
    let id = created["id"].as_str().unwrap().to_string();
    call("vault.setSecret", json!({ "workspaceId": "ws1", "id": id, "name": "TOKEN", "value": "s3cret-token-value" }))
        .await;

    let (target, seen) = canned_upstream(http_ok("content-type: text/plain\r\n", "echo: s3cret-token-value")).await;
    put(
        "rules",
        json!({
            "id": "r2", "workspaceId": "ws1", "name": "r", "pattern": "^http://inject\\.test/", "useRegex": true,
            "targetType": "external", "targetExternal": format!("127.0.0.1:{target}"),
            "requestScript": "bc.request.headers['authorization'] = 'Bearer {{keys.TOKEN}}';",
            "responseScript": "", "createdAt": 0,
        }),
    );
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let url = "http://inject.test/x";
    // The client asks for a reference of its own; only the rule's author may.
    let request = format!("GET {url} HTTP/1.1\r\nHost: inject.test\r\nx-evil: {{{{keys.TOKEN}}}}\r\n\r\n");
    through(proxy_port, &request).await;
    let sent = seen.lock()[0].to_lowercase();
    assert!(sent.contains("authorization: bearer s3cret-token-value"), "{sent}");
    assert!(sent.contains("x-evil: {{keys.token}}"), "a client's reference is not resolved: {sent}");

    let entry = logs.find(url).await;
    assert_eq!(entry["reqHeaders"]["authorization"], "Bearer {{keys.TOKEN}}");
    assert!(!entry.to_string().contains("s3cret-token-value"), "no value in the log: {entry}");
    assert!(entry["resBody"].as_str().is_some_and(|b| !b.is_empty()));
    proxy.stop();
}

#[tokio::test]
async fn with_https_on_the_one_port_serves_tls_clients_and_plain_ones() {
    let root = scratch_data_root();
    let client = crate::testing::enable_https(&root);
    let (upstream, _) = canned_upstream(http_ok("", "hello")).await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let request = format!("GET http://127.0.0.1:{upstream}/hello HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\n\r\n");

    // A client with an HTTP proxy setting.
    let plain = through(proxy_port, &request).await;
    assert!(plain.starts_with("HTTP/1.1 200 OK\r\n") && plain.ends_with("hello"), "{plain}");

    // A client with an HTTPS proxy setting: TLS to the proxy itself, no SNI when it connects by IP.
    let socket = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap();
    let mut tls = tokio_rustls::TlsConnector::from(client).connect(name, socket).await.unwrap();
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tls.read_to_end(&mut out).await;
    let secure = String::from_utf8_lossy(&out).into_owned();
    assert!(secure.starts_with("HTTP/1.1 200 OK\r\n") && secure.ends_with("hello"), "{secure}");
    proxy.stop();
}

// ---- faults -----------------------------------------------------------------------------------------------

fn fault(params: Value) -> String {
    crate::faults::add_for_tests(&params)
}

/// The request line through the proxy to `127.0.0.1:{upstream}{path}`.
fn get(upstream: u16, path: &str) -> String {
    format!("GET http://127.0.0.1:{upstream}{path} HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\n\r\n")
}

#[tokio::test]
async fn a_status_fault_answers_without_the_upstream() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let (upstream, seen) = canned_upstream(http_ok("", "real")).await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let id = fault(json!({ "target": format!("127.0.0.1:{upstream}/orders"), "status": 429 }));

    let answer = through(proxy_port, &get(upstream, "/orders/1?x=1")).await;
    assert!(answer.starts_with("HTTP/1.1 429 "), "{answer}");
    let lower = answer.to_lowercase();
    assert!(lower.contains("retry-after: 1") && lower.contains(&format!("x-bifurc-fault: {id}")));
    assert!(lower.contains("content-type: application/json"));
    assert!(answer.contains(&format!("{{\"error\":\"fault injected by bifurc\",\"fault\":\"{id}\",\"status\":429}}")));
    assert!(seen.lock().is_empty(), "the upstream was contacted");
    let entry = logs.find(&format!("http://127.0.0.1:{upstream}/orders/1?x=1")).await;
    assert_eq!((entry["via"].clone(), entry["status"].clone()), (json!("mock"), json!(429)));
    assert_eq!(entry["target"], format!("fault:{id}"));

    // Another path is untouched; the hits are counted; a cleared rule stops applying.
    assert!(through(proxy_port, &get(upstream, "/ordersx")).await.ends_with("real"));
    assert_eq!(seen.lock().len(), 1);
    assert_eq!(crate::faults::hits_for_tests(&id), 1);
    crate::faults::clear_for_tests(&id);
    assert!(through(proxy_port, &get(upstream, "/orders")).await.ends_with("real"));
    proxy.stop();
}

#[tokio::test]
async fn a_latency_fault_delays_and_still_reaches_the_upstream() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let (upstream, _seen) = canned_upstream(http_ok("", "real")).await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let id = fault(json!({ "target": format!("127.0.0.1:{upstream}"), "latencyMs": 300 }));

    let started = std::time::Instant::now();
    let answer = through(proxy_port, &get(upstream, "/slow")).await;
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(answer.starts_with("HTTP/1.1 200 OK") && answer.ends_with("real"), "{answer}");
    let entry = logs.find(&format!("http://127.0.0.1:{upstream}/slow")).await;
    assert_eq!((entry["via"].clone(), entry["fault"].clone()), (json!("proxy"), json!(id)));
    assert!(entry["durationMs"].as_i64().unwrap() >= 300);
    crate::faults::clear_for_tests(&id);
    proxy.stop();
}

#[tokio::test]
async fn a_reset_fault_closes_without_an_answer_and_a_drop_holds_the_client() {
    let _root = scratch_data_root();
    let logs = Logs::capture();
    let (upstream, seen) = canned_upstream(http_ok("", "real")).await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let reset = fault(json!({ "target": format!("127.0.0.1:{upstream}/reset"), "reset": true }));
    let drop = fault(json!({ "target": format!("127.0.0.1:{upstream}/drop"), "drop": true }));

    assert_eq!(through(proxy_port, &get(upstream, "/reset")).await, "");
    let entry = logs.find(&format!("http://127.0.0.1:{upstream}/reset")).await;
    assert_eq!((entry["via"].clone(), entry["status"].clone()), (json!("error"), Value::Null));
    assert_eq!(entry["target"], format!("fault:{reset}"));

    let mut socket = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    socket.write_all(get(upstream, "/drop").as_bytes()).await.unwrap();
    let mut buf = [0u8; 16];
    assert!(tokio::time::timeout(Duration::from_millis(500), socket.read(&mut buf)).await.is_err(), "answered");
    drop_socket(socket);
    let entry = logs.find(&format!("http://127.0.0.1:{upstream}/drop")).await;
    assert_eq!(entry["target"], format!("fault:{drop}"));
    assert!(seen.lock().is_empty());
    crate::faults::clear_for_tests(&reset);
    crate::faults::clear_for_tests(&drop);
    proxy.stop();
}

fn drop_socket(socket: TcpStream) {
    drop(socket);
}

#[tokio::test]
async fn a_blind_tunnel_meets_only_host_rules_without_a_status() {
    let _root = scratch_data_root();
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let status = fault(json!({ "target": "tunnel-status.test", "status": 500 }));
    let reset = fault(json!({ "target": "tunnel-reset.test:443", "reset": true }));
    // A status rule is skipped: the tunnel is attempted (the host does not resolve: 502 from the tunnel).
    let answer =
        through(proxy_port, "CONNECT tunnel-status.test:443 HTTP/1.1\r\nHost: tunnel-status.test:443\r\n\r\n").await;
    assert!(answer.starts_with("HTTP/1.1 502"), "{answer}");
    let answer =
        through(proxy_port, "CONNECT tunnel-reset.test:443 HTTP/1.1\r\nHost: tunnel-reset.test:443\r\n\r\n").await;
    assert_eq!(answer, "");
    crate::faults::clear_for_tests(&status);
    crate::faults::clear_for_tests(&reset);
    proxy.stop();
}

/// Sends a text message over `socket`, and returns the text that comes back.
async fn echo<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    text: &str,
) -> String {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    socket.send(Message::text(text)).await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_secs(10), socket.next()).await.unwrap() {
            Some(Ok(Message::Text(reply))) => return reply.as_str().to_string(),
            Some(Ok(_)) => continue,
            other => panic!("no reply: {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_websocket_through_the_proxy_is_relayed_and_its_messages_logged() {
    use websocket::test_support::{Collected, upper_echo};
    let _root = scratch_data_root();
    let entries = Collected::on("log.entry");
    let messages = Collected::on("log.message");
    let upstream = upper_echo().await;
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);

    let url = format!("http://127.0.0.1:{upstream}/chat");
    let mut socket = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
    let upgrade = format!(
        "GET {url} HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    socket.write_all(upgrade.as_bytes()).await.unwrap();
    let (head, leftover) = wire::read_head(&mut socket).await.unwrap();
    assert_eq!(head.target, "101", "{head:?}");
    assert!(
        head.header_lines.iter().any(|l| l.eq_ignore_ascii_case("sec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")),
        "{head:?}"
    );
    assert!(leftover.is_empty());
    let mut socket = tokio_tungstenite::WebSocketStream::from_raw_socket(
        socket,
        tokio_tungstenite::tungstenite::protocol::Role::Client,
        None,
    )
    .await;

    assert_eq!(echo(&mut socket, "hello").await, "HELLO");
    let open = entries.wait(|e| e["url"] == url.as_str() && e["live"] == true).await;
    assert_eq!(
        (open["kind"].as_str(), open["status"].as_u64(), open["via"].as_str()),
        (Some("ws"), Some(101), Some("proxy"))
    );
    let sent = messages.wait(|m| m["message"]["direction"] == "sent").await;
    assert_eq!((sent["logId"].as_str(), sent["message"]["data"].as_str()), (open["id"].as_str(), Some("hello")));
    let received = messages.wait(|m| m["message"]["direction"] == "received").await;
    assert_eq!(received["message"]["data"], "HELLO");

    socket.close(None).await.unwrap();
    let closed = entries.wait(|e| e["url"] == url.as_str() && e.get("live").is_none()).await;
    assert_eq!((closed["id"].as_str(), closed["kind"].as_str()), (open["id"].as_str(), Some("ws")));
    proxy.stop();
}

#[tokio::test]
async fn a_plain_websocket_in_an_intercepted_tunnel_is_relayed() {
    use websocket::test_support::{Collected, upper_echo};
    let _root = scratch_data_root();
    let entries = Collected::on("log.entry");
    let upstream = upper_echo().await;
    let (server_config, _) = tls_intercept::tests::self_signed_pair();
    let (mut client_end, server_end) = tokio::io::duplex(64 * 1024);
    let shared = Arc::new(server::Shared::default());
    tokio::spawn(async move {
        let (mut tunnelled, head, body) = tls_intercept::intercept_tls_with(server_end, server_config).await.unwrap();
        let secure = tunnelled.is_tls();
        server::dispatch_tunnelled(&shared, &mut tunnelled, &head, &body, "127.0.0.1", upstream, secure).await;
    });

    let mut established = vec![0u8; tls_intercept::tests::ESTABLISHED.len()];
    client_end.read_exact(&mut established).await.unwrap();
    let url = format!("ws://127.0.0.1:{upstream}/feed");
    let (mut socket, response) = tokio_tungstenite::client_async(url.as_str(), client_end).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);
    assert_eq!(echo(&mut socket, "tick").await, "TICK");
    let logged = format!("http://127.0.0.1:{upstream}/feed");
    let open = entries.wait(|e| e["url"] == logged.as_str()).await;
    assert_eq!((open["kind"].as_str(), open["live"].as_bool()), (Some("ws"), Some(true)));
}

#[tokio::test]
async fn an_event_stream_is_logged_live_with_its_events_before_it_ends() {
    use websocket::test_support::Collected;
    let _root = scratch_data_root();
    let entries = Collected::on("log.entry");
    let messages = Collected::on("log.message");
    let (release, held) = tokio::sync::oneshot::channel::<()>();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
        socket.write_all(head.as_bytes()).await.unwrap();
        socket.write_all(b"event: tick\nid: 7\ndata: {\"n\":1}\n\n").await.unwrap();
        let _ = held.await;
        let _ = socket.shutdown().await;
    });
    let proxy_port = free_port().await;
    let proxy = start_proxy(proxy_port);
    let url = format!("http://127.0.0.1:{upstream}/events");
    let request = format!("GET {url} HTTP/1.1\r\nHost: 127.0.0.1:{upstream}\r\n\r\n");
    let reading = tokio::spawn(async move { through(proxy_port, &request).await });

    let open = entries.wait(|e| e["url"] == url.as_str()).await;
    assert_eq!(
        (open["kind"].as_str(), open["live"].as_bool(), open["status"].as_u64()),
        (Some("sse"), Some(true), Some(200))
    );
    let event = messages.wait(|m| m["logId"] == open["id"]).await;
    assert_eq!(event["message"]["data"], "{\"n\":1}");
    assert_eq!((event["message"]["event"].as_str(), event["message"]["eventId"].as_str()), (Some("tick"), Some("7")));
    assert_eq!(event["message"]["direction"], "received");

    release.send(()).unwrap();
    assert!(reading.await.unwrap().ends_with("data: {\"n\":1}\n\n"));
    let closed = entries.wait(|e| e["url"] == url.as_str() && e.get("live").is_none()).await;
    assert_eq!(closed["kind"], "sse");
    proxy.stop();
}
