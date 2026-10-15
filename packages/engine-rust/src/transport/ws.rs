//! The WebSocket transport, on loopback only: what the browser extension and a web UI connect to. One JSON
//! object per text message in each direction (WebSocket already frames messages, so there is no length
//! prefix); the session behind each connection is the same one the socket transport serves.
//!
//! A browser is a local process, and a WebSocket handshake is not covered by the same-origin policy, so the
//! `Origin` of an upgrade is checked: pages served from this machine are accepted, and so are clients that
//! send no `Origin` at all.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc::unbounded_channel, watch},
    task::JoinHandle,
    time::{Instant, timeout},
};
use tokio_tungstenite::{
    WebSocketStream, accept_hdr_async_with_config,
    tungstenite::{
        Message,
        handshake::server::{ErrorResponse, Request, Response},
        http,
        protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
    },
};

use super::session::{Outbound, Session, SessionOptions};
use crate::{
    protocol::{EngineError, Result, capability},
    utils::net::{DEFAULT_HOST_DOMAIN, is_allowed_origin},
};

/// The largest message accepted from a client.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// How long an upgrade request has to arrive in full.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a closing connection waits for the peer to acknowledge the close.
const CLOSE_GRACE: Duration = Duration::from_secs(1);

const BIND_HOST: &str = "127.0.0.1";

pub struct WsServer {
    /// `ws://127.0.0.1:<port>`
    pub url: String,
    pub port: u16,
    closed: watch::Sender<bool>,
    accept: Mutex<Option<JoinHandle<()>>>,
}

impl WsServer {
    /// Stop listening and drop every connection.
    pub async fn close(&self) {
        let _ = self.closed.send(true);
        if let Some(accept) = self.accept.lock().take() {
            accept.abort();
        }
    }
}

/// Serve sessions over WebSocket on loopback: what the browser extension and a web UI connect to. Port `0`
/// takes an ephemeral port; `url` and `port` report the one bound.
pub async fn create_ws_server(port: u16, mut opts: SessionOptions) -> Result<Arc<WsServer>> {
    if !opts.capabilities.contains(&capability::WS) {
        opts.capabilities.push(capability::WS);
    }
    let listener = TcpListener::bind((BIND_HOST, port))
        .await
        .map_err(|e| EngineError::engine(format!("Could not bind {BIND_HOST}:{port}: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| EngineError::engine(format!("The listener for {BIND_HOST} has no address after binding: {e}")))?
        .port();
    let (closed, closed_rx) = watch::channel(false);
    let accept = tokio::spawn(async move {
        while let Ok((stream, remote)) = listener.accept().await {
            let mut opts = opts.clone();
            opts.peer = Some(remote.ip().to_string());
            tokio::spawn(serve_connection(stream, opts, closed_rx.clone()));
        }
    });
    Ok(Arc::new(WsServer { url: format!("ws://{BIND_HOST}:{port}"), port, closed, accept: Mutex::new(Some(accept)) }))
}

/// Whether the request waiting on `stream` asks for a WebSocket upgrade. Leaves the bytes unread.
async fn asks_for_upgrade(stream: &TcpStream) -> bool {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let mut buffer = vec![0u8; 16 * 1024];
    let mut seen = 0;
    loop {
        let Ok(Ok(n)) = timeout(deadline.saturating_duration_since(Instant::now()), stream.peek(&mut buffer)).await
        else {
            return false;
        };
        let head = String::from_utf8_lossy(&buffer[..n]).to_lowercase();
        if let Some(end) = head.find("\r\n\r\n") {
            return head[..end].lines().any(|line| {
                line.split_once(':').is_some_and(|(name, value)| {
                    name.trim() == "upgrade" && value.split(',').any(|v| v.trim() == "websocket")
                })
            });
        }
        if n == 0 || n == buffer.len() {
            return false;
        }
        if n == seen {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        seen = n;
    }
}

/// A non-upgrade request gets an answer rather than a hang.
async fn refuse_plain_http(mut stream: TcpStream) {
    let body = "This port speaks WebSocket, not HTTP.\n";
    let response = format!(
        "HTTP/1.1 426 Upgrade Required\r\ncontent-type: text/plain; charset=utf-8\r\nupgrade: websocket\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut sink = [0u8; 4096];
    let _ = timeout(Duration::from_millis(100), stream.read(&mut sink)).await;
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn check_origin(request: &Request, response: Response) -> std::result::Result<Response, ErrorResponse> {
    let origin = request.headers().get("origin").and_then(|value| value.to_str().ok());
    if is_allowed_origin(origin, DEFAULT_HOST_DOMAIN) {
        return Ok(response);
    }
    let shown = serde_json::to_string(&origin).unwrap_or_default();
    let body = format!(
        "Refusing a WebSocket from origin {shown}.\nThis engine accepts pages served from {DEFAULT_HOST_DOMAIN} (or a \
         subdomain of it) and clients that send no Origin at all.\n"
    );
    let mut refusal = http::Response::new(Some(body));
    *refusal.status_mut() = http::StatusCode::FORBIDDEN;
    refusal.headers_mut().insert("content-type", http::HeaderValue::from_static("text/plain; charset=utf-8"));
    refusal.headers_mut().insert("connection", http::HeaderValue::from_static("close"));
    Err(refusal)
}

fn close_message(reason: &str) -> Message {
    let mut end = reason.len().min(120);
    while !reason.is_char_boundary(end) {
        end -= 1;
    }
    Message::Close(Some(CloseFrame { code: CloseCode::Policy, reason: reason[..end].into() }))
}

async fn serve_connection(stream: TcpStream, opts: SessionOptions, closed: watch::Receiver<bool>) {
    if !asks_for_upgrade(&stream).await {
        refuse_plain_http(stream).await;
        return;
    }
    let config =
        WebSocketConfig::default().max_message_size(Some(MAX_PAYLOAD_BYTES)).max_frame_size(Some(MAX_PAYLOAD_BYTES));
    let Ok(Ok(socket)) =
        timeout(REQUEST_TIMEOUT, accept_hdr_async_with_config(stream, check_origin, Some(config))).await
    else {
        return;
    };
    serve_socket(socket, opts, closed).await;
}

/// Drive one upgraded connection: text messages in become session frames, session output goes back out as text
/// messages, and an `UNAUTHORIZED` / `UNSUPPORTED` reply is followed by a policy close.
async fn serve_socket<S>(mut socket: WebSocketStream<S>, opts: SessionOptions, mut closed: watch::Receiver<bool>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (tx, mut rx) = unbounded_channel();
    let session = Session::new(opts, tx);
    // What to say when the connection is ended by the engine rather than by the peer.
    let mut ending: Option<Message> = None;
    loop {
        tokio::select! {
            message = socket.next() => {
                let bytes = match message {
                    Some(Ok(Message::Text(text))) => text.as_bytes().to_vec(),
                    Some(Ok(Message::Binary(bytes))) => bytes.to_vec(),
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                match serde_json::from_slice::<Value>(&bytes) {
                    Ok(frame) => session.handle_frame(frame),
                    Err(_) => {
                        ending = Some(close_message("frames must be JSON"));
                        break;
                    }
                }
            }
            outbound = rx.recv() => {
                match outbound {
                    Some(Outbound::Frame(frame)) => {
                        if send_json(&mut socket, &frame).await.is_err() {
                            break;
                        }
                    }
                    Some(Outbound::FrameThenClose(frame)) => {
                        let reason = frame["error"]["message"].as_str().unwrap_or("session ended").to_string();
                        if send_json(&mut socket, &frame).await.is_ok() {
                            ending = Some(close_message(&reason));
                        }
                        break;
                    }
                    Some(Outbound::Close) => {
                        ending = Some(close_message("frames must be RpcRequest envelopes"));
                        break;
                    }
                    None => break,
                }
            }
            _ = closed.changed() => break,
        }
    }
    session.close();
    if let Some(message) = ending {
        if socket.send(message).await.is_ok() {
            // Let the peer see the close before the connection goes away.
            let _ = timeout(CLOSE_GRACE, async { while let Some(Ok(_)) = socket.next().await {} }).await;
        }
    }
}

async fn send_json<S>(socket: &mut WebSocketStream<S>, frame: &Value) -> std::result::Result<(), ()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let text = serde_json::to_string(frame).map_err(|_| ())?;
    socket.send(Message::text(text)).await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::{
        kernel::{CommandRegistry, EngineEventBus, Scope},
        transport::{event_log::EventLog, scopes::companion_scopes},
    };
    use serde_json::json;
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{client::IntoClientRequest, error::Error as WsError},
    };

    type Client = WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

    async fn server() -> (Arc<WsServer>, &'static EngineEventBus) {
        let bus: &'static EngineEventBus = Box::leak(Box::new(EngineEventBus::new()));
        let mut registry = CommandRegistry::new();
        registry.register_sync("proxy.status", |_, _| Ok(json!({ "running": false })));
        registry.register_sync("ws.clearMessages", |_, _| Ok(json!({ "cleared": true })));
        registry.register_sync("engine.shutdown", |_, _| Ok(json!({ "ok": true })));
        let opts = SessionOptions {
            registry: Arc::new(registry),
            log: EventLog::new(bus),
            engine_version: "1.2.3".into(),
            host_domain: Some("localhost".into()),
            capabilities: Vec::new(),
            scopes_for: Some(Arc::new(|hello: &Value| -> HashSet<Scope> {
                if hello["clientName"] == "bifurc-extension" {
                    companion_scopes()
                } else {
                    crate::transport::scopes::shell_scopes()
                }
            })),
            on_auth_failure: None,
            on_session_start: None,
            on_session_end: None,
            refuse_command: None,
            peer: None,
            allowed_events: None,
        };
        (create_ws_server(0, opts).await.unwrap(), bus)
    }

    async fn connect(server: &WsServer) -> Client {
        connect_async(server.url.as_str()).await.unwrap().0
    }

    async fn call(client: &mut Client, frame: Value) {
        client.send(Message::text(frame.to_string())).await.unwrap();
    }

    async fn next_frame(client: &mut Client) -> Value {
        loop {
            match client.next().await.expect("a message").expect("no error") {
                Message::Text(text) => return serde_json::from_str(text.as_str()).unwrap(),
                Message::Close(frame) => panic!("closed: {frame:?}"),
                _ => {}
            }
        }
    }

    /// The close code the server ends the connection with.
    async fn close_code(client: &mut Client) -> u16 {
        loop {
            match client.next().await {
                Some(Ok(Message::Close(Some(frame)))) => return frame.code.into(),
                Some(Ok(Message::Close(None))) | None | Some(Err(_)) => panic!("closed without a code"),
                Some(Ok(_)) => {}
            }
        }
    }

    fn hello(client_name: &str) -> Value {
        json!({ "id": "h", "action": "hello",
            "payload": { "protocolVersion": "1.0.0", "clientName": client_name, "clientVersion": "1" } })
    }

    #[tokio::test]
    async fn binds_loopback_and_reports_the_real_port() {
        let (server, _) = server().await;
        assert_ne!(server.port, 0);
        assert_eq!(server.url, format!("ws://127.0.0.1:{}", server.port));
        server.close().await;
    }

    #[tokio::test]
    async fn hello_commands_and_events() {
        let (server, bus) = server().await;
        let mut client = connect(&server).await;
        call(&mut client, hello("web-ui")).await;
        let reply = next_frame(&mut client).await;
        assert_eq!(reply["data"]["capabilities"], json!(["ws", "events.replay"]));
        assert_eq!(reply["data"]["hostDomain"], "localhost");
        assert_eq!(reply["data"]["engineVersion"], "1.2.3");

        call(&mut client, json!({ "id": "1", "action": "proxy.status", "payload": {} })).await;
        assert_eq!(next_frame(&mut client).await, json!({ "id": "1", "ok": true, "data": { "running": false } }));

        call(&mut client, json!({ "id": "2", "action": "subscribe", "payload": { "events": ["event.ws.status"] } }))
            .await;
        assert_eq!(next_frame(&mut client).await["ok"], true);
        bus.emit("ws.status", json!({ "socketId": "s" }));
        assert_eq!(
            next_frame(&mut client).await,
            json!({ "event": "event.ws.status", "seq": 1, "payload": { "socketId": "s" } })
        );
        server.close().await;
    }

    #[tokio::test]
    async fn a_refused_origin_gets_403_and_an_allowed_one_connects() {
        let (server, _) = server().await;
        let mut request = server.url.as_str().into_client_request().unwrap();
        request.headers_mut().insert("origin", "https://evil.example".parse().unwrap());
        match connect_async(request).await {
            Err(WsError::Http(response)) => assert_eq!(response.status(), 403),
            other => panic!("expected a 403, got {:?}", other.map(|_| ())),
        }

        let mut request = server.url.as_str().into_client_request().unwrap();
        request.headers_mut().insert("origin", "http://app.localhost:3000".parse().unwrap());
        assert!(connect_async(request).await.is_ok());
        server.close().await;
    }

    #[tokio::test]
    async fn plain_http_gets_426() {
        let (server, _) = server().await;
        let mut stream = TcpStream::connect((BIND_HOST, server.port)).await.unwrap();
        stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 426"), "{response}");
        server.close().await;
    }

    #[tokio::test]
    async fn a_command_before_hello_is_refused_and_closes_with_policy() {
        let (server, _) = server().await;
        let mut client = connect(&server).await;
        call(&mut client, json!({ "id": "1", "action": "proxy.status", "payload": {} })).await;
        assert_eq!(next_frame(&mut client).await["error"]["code"], "UNAUTHORIZED");
        assert_eq!(close_code(&mut client).await, 1008);
        server.close().await;
    }

    #[tokio::test]
    async fn frames_that_are_not_requests_close_with_policy() {
        let (server, _) = server().await;
        let mut client = connect(&server).await;
        client.send(Message::text("not json")).await.unwrap();
        assert_eq!(close_code(&mut client).await, 1008);

        let mut client = connect(&server).await;
        call(&mut client, json!({ "action": "proxy.status" })).await;
        assert_eq!(close_code(&mut client).await, 1008);
        server.close().await;
    }

    #[tokio::test]
    async fn an_incompatible_protocol_version_is_unsupported_and_closes() {
        let (server, _) = server().await;
        let mut client = connect(&server).await;
        let mut frame = hello("cli");
        frame["payload"]["protocolVersion"] = json!("9.0.0");
        call(&mut client, frame).await;
        assert_eq!(next_frame(&mut client).await["error"]["code"], "UNSUPPORTED");
        assert_eq!(close_code(&mut client).await, 1008);
        server.close().await;
    }

    #[tokio::test]
    async fn the_extension_gets_read_and_write_but_not_admin() {
        let (server, _) = server().await;
        let mut client = connect(&server).await;
        call(&mut client, hello("bifurc-extension")).await;
        assert_eq!(next_frame(&mut client).await["ok"], true);

        let clear = json!({ "workspaceId": "w", "socketId": "s" });
        call(&mut client, json!({ "id": "1", "action": "ws.clearMessages", "payload": clear })).await;
        assert_eq!(next_frame(&mut client).await["ok"], true);

        call(&mut client, json!({ "id": "2", "action": "engine.shutdown", "payload": {} })).await;
        let reply = next_frame(&mut client).await;
        assert_eq!(reply["error"]["code"], "FORBIDDEN");
        assert_eq!(reply["error"]["details"]["required"], "admin");

        let mut shell = connect(&server).await;
        call(&mut shell, hello("desktop-ui")).await;
        next_frame(&mut shell).await;
        call(&mut shell, json!({ "id": "3", "action": "engine.shutdown", "payload": {} })).await;
        assert_eq!(next_frame(&mut shell).await["ok"], true);
        server.close().await;
    }

    #[tokio::test]
    async fn closing_the_server_drops_connections() {
        let (server, _) = server().await;
        let mut client = connect(&server).await;
        call(&mut client, hello("cli")).await;
        next_frame(&mut client).await;
        server.close().await;
        loop {
            match timeout(Duration::from_secs(5), client.next()).await.unwrap() {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                Some(Ok(_)) => {}
            }
        }
    }
}
