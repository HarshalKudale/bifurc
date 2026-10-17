//! The edge end to end on loopback: a real listener, a canned upstream, and a hand-built routing config.

use std::{sync::Arc, time::Duration};

use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use super::*;
use crate::kernel::bus;

async fn canned_upstream(response: String) -> (u16, Arc<Mutex<Vec<String>>>) {
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
                while !received.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(n) if n > 0 => received.extend_from_slice(&buf[..n]),
                        _ => return,
                    }
                }
                log.lock().push(String::from_utf8_lossy(&received).into_owned());
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (port, seen)
}

fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port()
}

async fn ask(port: u16, host: &str, path: &str) -> String {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes()).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), socket.read_to_end(&mut out)).await.unwrap().unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

fn edge_with(cfg: Option<Value>) -> (Arc<dyn Listener>, u16) {
    let edge = create_edge_router(Arc::new(move || cfg.clone()));
    let port = futures::executor::block_on(free_port());
    edge.start(port);
    assert!(edge.error().is_none());
    (edge, port)
}

fn mock(extra: Value) -> Value {
    let mut mock = json!({
        "id": "m1", "enabled": true, "method": "GET", "urlPattern": "http://app.localhost/mocked", "useRegex": false,
        "responseStatus": 201, "responseHeaders": {}, "responseBody": "mocked", "responseBodyEncoding": "utf8",
        "responseDelay": 0,
    });
    for (key, value) in extra.as_object().unwrap() {
        mock[key] = value.clone();
    }
    mock
}

fn log_entries() -> (Arc<Mutex<Vec<Value>>>, crate::kernel::ListenerId) {
    let entries = Arc::new(Mutex::new(Vec::new()));
    let sink = entries.clone();
    (entries, bus().on("log.entry", move |entry| sink.lock().push(entry.clone())))
}

async fn find(entries: &Mutex<Vec<Value>>, url: &str) -> Value {
    for _ in 0..200 {
        if let Some(found) = entries.lock().iter().find(|e| e["url"] == url) {
            return found.clone();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("no log entry for {url}");
}

#[tokio::test]
async fn the_home_page_lists_the_mappings() {
    let cfg = json!({ "activeWorkspaceId": "ws1", "mocks": [], "proxyRules": [], "mappings": [
        { "id": "a", "domain": "app.localhost", "target": "localhost:3000", "enabled": true },
    ] });
    let _root = crate::testing::scratch_data_root();
    let (edge, port) = edge_with(Some(cfg));
    let page = ask(port, "localhost", "/").await;
    assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
    assert!(page.contains("Local proxy is active") && page.contains("app.localhost"));
    assert!(page.contains(&format!("<div class=\"badge\">Port {port}</div>")));
    edge.stop();
}

#[tokio::test]
async fn an_edge_without_a_workspace_answers_503() {
    let _root = crate::testing::scratch_data_root();
    let (edge, port) = edge_with(None);
    let page = ask(port, "localhost", "/").await;
    assert!(page.starts_with("HTTP/1.1 503 Service Unavailable\r\n"), "{page}");
    assert!(page.contains("No workspace is being served"));
    edge.stop();
}

#[tokio::test]
async fn a_mapped_domain_is_proxied_to_its_target() {
    let (entries, listener) = log_entries();
    let (target, seen) = canned_upstream(ok("from target")).await;
    let cfg = json!({ "activeWorkspaceId": "ws1", "mocks": [], "proxyRules": [], "mappings": [
        { "id": "a", "domain": "Shop.Example", "target": format!("127.0.0.1:{target}"), "enabled": true },
        { "id": "b", "domain": "off.example", "target": "127.0.0.1:1", "enabled": false },
    ] });
    let _root = crate::testing::scratch_data_root();
    crate::capture::refresh_capture_limit();
    let (edge, port) = edge_with(Some(cfg));

    let answer = ask(port, "shop.example", "/cart?x=1").await;
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n") && answer.ends_with("from target"), "{answer}");
    let sent = seen.lock()[0].to_lowercase();
    assert!(sent.starts_with("get /cart?x=1 http/1.1") && sent.contains("host: shop.example"));

    let entry = find(&entries, "http://shop.example/cart?x=1").await;
    assert_eq!(entry["via"], "mapping");
    assert_eq!(entry["origin"], "reverse");
    assert_eq!(entry["workspaceId"], "ws1");
    assert_eq!(entry["target"], format!("127.0.0.1:{target}"));
    // The upstream sees the capture id; the capture shows the request as the client sent it.
    assert!(sent.contains(&format!("x-bifurc-trace: {}", entry["id"].as_str().unwrap())), "{sent}");
    assert!(!entry["reqHeaders"].as_object().unwrap().contains_key("x-bifurc-trace"));

    // A disabled mapping is not a mapping, and neither is an unknown host: they get the home page.
    assert!(ask(port, "off.example", "/").await.contains("Local proxy is active"));
    bus().off(listener);
    edge.stop();
}

#[tokio::test]
async fn a_localhost_subdomain_is_rfc6761_and_an_unmapped_one_is_404() {
    let (entries, listener) = log_entries();
    let (target, _) = canned_upstream(ok("dev server")).await;
    let cfg = json!({ "activeWorkspaceId": "ws1", "mocks": [], "proxyRules": [], "mappings": [
        { "id": "a", "domain": "app.localhost", "target": format!("127.0.0.1:{target}"), "enabled": true },
    ] });
    let _root = crate::testing::scratch_data_root();
    let (edge, port) = edge_with(Some(cfg));

    assert!(ask(port, "app.localhost", "/").await.ends_with("dev server"));
    assert_eq!(find(&entries, "http://app.localhost/").await["via"], "rfc6761");

    let missing = ask(port, "ghost.localhost", "/x").await;
    assert!(missing.starts_with("HTTP/1.1 404 Not Found\r\n") && missing.contains("<h1>ghost.localhost</h1>"));
    let entry = find(&entries, "http://ghost.localhost/x").await;
    assert_eq!((entry["via"].as_str(), entry["status"].as_u64()), (Some("error"), Some(404)));
    bus().off(listener);
    edge.stop();
}

#[tokio::test]
async fn a_dead_target_is_a_502() {
    let dead = free_port().await;
    let cfg = json!({ "activeWorkspaceId": "ws1", "mocks": [], "proxyRules": [], "mappings": [
        { "id": "a", "domain": "app.localhost", "target": format!("127.0.0.1:{dead}"), "enabled": true },
    ] });
    let _root = crate::testing::scratch_data_root();
    let (edge, port) = edge_with(Some(cfg));
    let answer = ask(port, "app.localhost", "/").await;
    assert!(answer.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{answer}");
    assert!(answer.contains(&format!("<code>127.0.0.1:{dead}</code>")));
    edge.stop();
}

#[tokio::test]
async fn mocks_take_precedence_and_partial_ones_need_a_mapping() {
    let (entries, listener) = log_entries();
    let (target, _) = canned_upstream(ok("real")).await;
    let cfg = json!({
        "activeWorkspaceId": "ws1", "proxyRules": [],
        "mappings": [{ "id": "a", "domain": "app.localhost", "target": format!("127.0.0.1:{target}"), "enabled": true }],
        "mocks": [
            mock(json!({})),
            mock(json!({ "id": "partial", "urlPattern": "http://app.localhost/partial", "responseBodyMocked": false,
                         "responseStatus": 202 })),
            mock(json!({ "id": "orphan", "urlPattern": "http://free.localhost/partial", "responseBodyMocked": false })),
        ],
    });
    let _root = crate::testing::scratch_data_root();
    let (edge, port) = edge_with(Some(cfg));

    let full = ask(port, "app.localhost", "/mocked").await;
    assert!(full.starts_with("HTTP/1.1 201 Created\r\n") && full.ends_with("mocked"), "{full}");
    let entry = find(&entries, "http://app.localhost/mocked").await;
    assert_eq!((entry["via"].as_str(), entry["target"].as_str()), (Some("mock"), Some("mock:m1")));

    let partial = ask(port, "app.localhost", "/partial").await;
    assert!(partial.starts_with("HTTP/1.1 202 Unknown\r\n") && partial.ends_with("real"), "{partial}");

    let orphan = ask(port, "free.localhost", "/partial").await;
    assert!(orphan.starts_with("HTTP/1.1 502 Bad Gateway\r\n"), "{orphan}");
    assert!(orphan.contains("No downstream mapping exists for <code>free.localhost</code>"));
    bus().off(listener);
    edge.stop();
}

#[tokio::test]
async fn a_request_body_is_forwarded() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = upstream.local_addr().unwrap().port();
    let received = tokio::spawn(async move {
        let (mut socket, _) = upstream.accept().await.unwrap();
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        while !String::from_utf8_lossy(&data).ends_with("payload") {
            let n = socket.read(&mut buf).await.unwrap();
            data.extend_from_slice(&buf[..n]);
        }
        socket.write_all(ok("done").as_bytes()).await.unwrap();
        let _ = socket.shutdown().await;
        String::from_utf8_lossy(&data).to_lowercase()
    });
    let cfg = json!({ "activeWorkspaceId": "ws1", "mocks": [], "proxyRules": [], "mappings": [
        { "id": "a", "domain": "app.localhost", "target": format!("127.0.0.1:{target}"), "enabled": true },
    ] });
    let _root = crate::testing::scratch_data_root();
    let (edge, port) = edge_with(Some(cfg));
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket
        .write_all(b"POST /submit HTTP/1.1\r\nHost: app.localhost\r\nTransfer-Encoding: chunked\r\n\r\n7\r\npayload\r\n0\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    socket.read_to_string(&mut out).await.unwrap();
    assert!(out.ends_with("done"));
    let sent = received.await.unwrap();
    assert!(sent.starts_with("post /submit http/1.1") && sent.contains("content-length: 7"), "{sent}");
    edge.stop();
}

#[tokio::test]
async fn restarting_moves_the_edge_to_another_port() {
    let _root = crate::testing::scratch_data_root();
    let (edge, port) =
        edge_with(Some(json!({ "activeWorkspaceId": "ws1", "mocks": [], "proxyRules": [], "mappings": [] })));
    assert!(edge.is_running() && edge.port() == port);
    let other = free_port().await;
    edge.start(other);
    assert_eq!((edge.is_running(), edge.port()), (true, other));
    assert!(ask(other, "localhost", "/").await.contains(&format!("Port {other}")));
    edge.stop();
    assert!(!edge.is_running());
}

async fn tls_ask(client: Arc<rustls::ClientConfig>, port: u16, host: &str, path: &str) -> String {
    let socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).unwrap();
    let mut tls = tokio_rustls::TlsConnector::from(client).connect(name, socket).await.unwrap();
    tls.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), tls.read_to_end(&mut out)).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn with_https_on_the_one_port_serves_tls_and_redirects_plain_http() {
    let root = crate::testing::scratch_data_root();
    let client = crate::testing::enable_https(&root);
    let (upstream, seen) = canned_upstream(ok("secure hello")).await;
    let cfg = json!({
        "activeWorkspaceId": "ws1", "mocks": [], "proxyRules": [],
        "mappings": [{ "id": "a", "enabled": true, "domain": "app.localhost", "target": format!("127.0.0.1:{upstream}") }],
    });
    let (edge, port) = edge_with(Some(cfg));

    let https_port = port;
    let redirect = ask(port, "app.localhost", "/x?y=1").await;
    assert!(redirect.starts_with("HTTP/1.1 308"), "{redirect}");
    assert!(redirect.contains(&format!("Location: https://app.localhost:{https_port}/x?y=1\r\n")), "{redirect}");

    let answer = tls_ask(client.clone(), port, "app.localhost", "/x").await;
    assert!(answer.starts_with("HTTP/1.1 200") && answer.ends_with("secure hello"), "{answer}");
    assert!(seen.lock()[0].to_lowercase().starts_with("get /x http/1.1"));

    let home = tls_ask(client, port, "localhost", "/").await;
    assert!(home.contains(&format!("https://app.localhost:{https_port}/")), "{home}");
    edge.stop();
}
