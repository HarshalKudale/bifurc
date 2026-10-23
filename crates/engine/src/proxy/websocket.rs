//! A WebSocket through the proxies. The client's upgrade is answered once the upstream accepted its own, then
//! every message is relayed both ways and captured as it passes (`log.message`). The exchange is logged as live
//! when the socket opens and again when either side closes it.

use std::time::Instant;

use base64::Engine as _;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::{
    WebSocketStream, connect_async,
    tungstenite::{
        Error as WsError, Message,
        client::IntoClientRequest,
        handshake::derive_accept_key,
        http::{HeaderName, HeaderValue},
        protocol::Role,
    },
};

use super::{
    handler::{http_url, match_proxy_rule},
    response::{LiveLog, collect_response_headers, log_body, send_html},
    routing::{Exchange, error_entry, mask_secrets},
    wire::raw_header,
};
use crate::kernel::traffic::{Headers, emit_log, emit_log_message};

/// Whether a request asks to become a WebSocket.
pub fn is_websocket_upgrade(header_lines: &[String]) -> bool {
    raw_header(header_lines, "upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// Handshake headers each side gets from the relay itself, never copied across. Extensions are dropped: the
/// relay speaks plain frames to both sides, so neither negotiates compression it would not get.
const OWN_HEADERS: [&str; 4] = ["sec-websocket-key", "sec-websocket-version", "sec-websocket-extensions", "sec-websocket-accept"];

const RULE_WITHOUT_TARGET: &str = "<h1>502 Bad Gateway</h1><p>Proxy rule matched but the target is not configured.</p>";

/// `http://` becomes `ws://`, `https://` becomes `wss://`.
fn ws_scheme(url: &str) -> String {
    url.replacen("http", "ws", 1)
}

/// A WebSocket upgrade that reached the forward proxy: to the target of the proxy rule it matches, else where it
/// was headed. Mocks do not answer WebSockets.
pub async fn relay_proxied<S: AsyncRead + AsyncWrite + Unpin>(client: S, ex: &Exchange<'_>, host: &str) {
    let rule = match_proxy_rule(&ex.routing.rules, ex.rule_match_target, &ex.routing.mappings);
    if !rule.matched {
        let live = LiveLog { base: ex.base, via: "proxy", target: Some(host.to_string()) };
        return relay_websocket(client, &ws_scheme(ex.url), ex.headers, live).await;
    }
    let Some(target) = rule.target.filter(|t| !t.is_empty()) else {
        let mut client = client;
        send_html(&mut client, 502, RULE_WITHOUT_TARGET).await;
        emit_log(&error_entry(ex.base, 502, crate::utils::id::now_ms() - ex.t0));
        return;
    };
    let url = ws_scheme(&http_url(&target, ex.req_path));
    relay_websocket(client, &url, ex.headers, LiveLog { base: ex.base, via: "rule", target: Some(target) }).await;
}

/// A WebSocket to a mapping's target (`host:port`).
pub async fn relay_mapped<S: AsyncRead + AsyncWrite + Unpin>(
    client: S,
    target: &str,
    path: &str,
    headers: &Headers,
    live: LiveLog<'_>,
) {
    relay_websocket(client, &ws_scheme(&http_url(target, path)), headers, live).await;
}

/// Open `url` (`ws://` or `wss://`) with the client's upgrade `headers`, answer the client with the upstream's
/// handshake, and relay until either side closes. An upstream that refuses the upgrade has its answer passed on.
async fn relay_websocket<S: AsyncRead + AsyncWrite + Unpin>(
    mut client: S,
    url: &str,
    headers: &Headers,
    live: LiveLog<'_>,
) {
    let t0 = Instant::now();
    let elapsed = || t0.elapsed().as_millis() as i64;
    let Some(key) = headers.get("sec-websocket-key").cloned() else {
        send_html(&mut client, 400, "<h1>400 Bad Request</h1><p>A WebSocket upgrade needs a Sec-WebSocket-Key.</p>")
            .await;
        emit_log(&error_entry(live.base, 400, elapsed()));
        return;
    };
    let Ok(mut request) = url.into_client_request() else {
        send_html(&mut client, 502, &format!("<h1>502 Bad Gateway</h1><p>Not a WebSocket URL: <code>{url}</code>.</p>"))
            .await;
        emit_log(&error_entry(live.base, 502, elapsed()));
        return;
    };
    for (name, value) in headers {
        if OWN_HEADERS.contains(&name.as_str()) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
            request.headers_mut().insert(name, value);
        }
    }

    // shortcut: wss:// upstreams are verified against the system roots, unlike the HTTP passthrough; add a
    // no-verify rustls connector when self-signed WebSocket servers behind the forward proxy come up.
    let (upstream, response) = match connect_async(request).await {
        Ok(connected) => connected,
        Err(WsError::Http(refused)) => {
            let status = refused.status().as_u16();
            let (res_headers, lines) = collect_response_headers(refused.headers());
            let body = refused.body().clone().unwrap_or_default();
            let mut head = format!("HTTP/1.1 {status} {}\r\n", refused.status().canonical_reason().unwrap_or(""));
            for (name, value) in &lines {
                head.push_str(&format!("{name}: {}\r\n", String::from_utf8_lossy(value)));
            }
            head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", body.len()));
            let _ = client.write_all(head.as_bytes()).await;
            let _ = client.write_all(&body).await;
            let _ = client.shutdown().await;
            let mut entry = live.open_entry("ws", status, elapsed(), res_headers);
            entry.live = false;
            entry.res_body = log_body(&body);
            emit_log(&entry);
            return;
        }
        Err(_) => {
            send_html(&mut client, 502, &format!("<h1>502 Bad Gateway</h1><p>Could not connect to <code>{url}</code>.</p>"))
                .await;
            emit_log(&error_entry(live.base, 502, elapsed()));
            return;
        }
    };

    let (res_headers, lines) = collect_response_headers(response.headers());
    let mut head = format!(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {}\r\n",
        derive_accept_key(key.as_bytes())
    );
    for (name, value) in lines.iter().filter(|(name, _)| !OWN_HEADERS.contains(&name.as_str())) {
        head.push_str(&format!("{name}: {}\r\n", String::from_utf8_lossy(value)));
    }
    head.push_str("\r\n");
    if client.write_all(head.as_bytes()).await.is_err() || client.flush().await.is_err() {
        return;
    }
    let client = WebSocketStream::from_raw_socket(client, Role::Server, None).await;
    let mut entry = live.open_entry("ws", 101, elapsed(), res_headers);
    emit_log(&entry);

    let (mut to_upstream, mut from_upstream) = upstream.split();
    let (mut to_client, mut from_client) = client.split();
    let mut seq = 0u64;
    loop {
        // Each side's own pings are answered by its own socket; only messages and closes cross.
        let (message, sent) = tokio::select! {
            message = from_client.next() => (message, true),
            message = from_upstream.next() => (message, false),
        };
        let Some(Ok(message)) = message else { break };
        if let Some(logged) = logged(&message, if sent { "sent" } else { "received" }) {
            seq += 1;
            emit_log_message(&entry, seq, logged);
        }
        let close = message.is_close();
        let forwarded = if sent { to_upstream.send(message).await } else { to_client.send(message).await };
        if forwarded.is_err() || close {
            break;
        }
    }
    let _ = to_upstream.close().await;
    let _ = to_client.close().await;

    entry.live = false;
    entry.duration_ms = Some(elapsed());
    mask_secrets(&mut entry);
    emit_log(&entry);
}

/// What the log keeps of a message: text as is, binary as base64, a close with its code. Pings and pongs are not
/// logged.
fn logged(message: &Message, direction: &str) -> Option<Value> {
    Some(match message {
        Message::Text(text) => json!({ "direction": direction, "data": text.as_str() }),
        Message::Binary(bytes) => json!({
            "direction": direction, "data": base64::engine::general_purpose::STANDARD.encode(bytes), "binary": true
        }),
        Message::Close(Some(frame)) => {
            json!({ "direction": direction, "data": frame.reason.as_str(), "close": u16::from(frame.code) })
        }
        Message::Close(None) => json!({ "direction": direction, "data": "", "close": 1005 }),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => return None,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{sync::Arc, time::Duration};

    use futures::{SinkExt, StreamExt};
    use parking_lot::Mutex;
    use serde_json::Value;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    use crate::kernel::{ListenerId, bus};

    /// A WebSocket server that answers each text message with it in upper case.
    pub async fn upper_echo() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let Ok(mut socket) = accept_async(socket).await else { return };
                    while let Some(Ok(message)) = socket.next().await {
                        match message {
                            Message::Text(text) => {
                                let _ = socket.send(Message::text(text.as_str().to_uppercase())).await;
                            }
                            Message::Close(_) => break,
                            _ => {}
                        }
                    }
                });
            }
        });
        port
    }

    /// Collects the payloads of one bus event.
    pub struct Collected {
        pub items: Arc<Mutex<Vec<Value>>>,
        id: ListenerId,
    }

    impl Collected {
        pub fn on(event: &'static str) -> Self {
            let items = Arc::new(Mutex::new(Vec::new()));
            let sink = items.clone();
            let id = bus().on(event, move |payload| sink.lock().push(payload.clone()));
            Self { items, id }
        }

        /// Waits until some payload passes `wanted`, and returns it.
        pub async fn wait(&self, wanted: impl Fn(&Value) -> bool) -> Value {
            for _ in 0..300 {
                if let Some(found) = self.items.lock().iter().find(|item| wanted(item)) {
                    return found.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("not seen: {:?}", self.items.lock());
        }
    }

    impl Drop for Collected {
        fn drop(&mut self) {
            bus().off(self.id);
        }
    }
}
