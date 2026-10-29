//! The browser companion's way in. The extension catches a page's request before the browser encrypts it,
//! and the engine routes it exactly as the proxy port routes one: faults, the cassette being replayed, mocks,
//! then proxy rules. The answer tells the extension to hand the page a response, or to let the browser send
//! the request itself.
//!
//! The `browser.*` commands carry that over the companion session. A relayed body is pulled with
//! `browser.read`; a request the browser sent itself is reported back (`browser.open`, `browser.message`,
//! `browser.close`) and logged as if the proxy had passed it through. Each stream belongs to the session that
//! made it and goes when that session ends, when it is cancelled, or after an hour nobody asked about it.
//!
//! A WebSocket a proxy rule claims is relayed by the engine: frames from the page go out with
//! `browser.wsSend`, and the target's come back through `browser.read`.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use base64::Engine as _;
use futures::{SinkExt, StreamExt, stream::SplitSink};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::{
    io::{DuplexStream, duplex},
    sync::mpsc,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{
        Message,
        protocol::{CloseFrame, Role, frame::coding::CloseCode},
    },
};

use super::{
    handler::match_proxy_rule,
    response::{LiveLog, log_body},
    routing::{Exchange, finished_entry, handle_proxy_routing, mask_secrets},
    server::{self, Shared, base_entry},
    websocket::{logged, relay_proxied},
    wire::{Prefixed, ResponseBody, raw_header, read_head},
};
use crate::{
    kernel::{
        CommandContext, CommandRegistry,
        traffic::{Headers, RequestLogEntry, emit_log, emit_log_message, forwarded_headers},
    },
    protocol::EngineError,
    utils::{
        cache_policy::strip_cache_headers,
        decompress::strip_content_encoding,
        http::is_hop_by_hop,
        id::{mk_id, now_ms},
        replay::decode_base64_lenient,
    },
};

/// The largest answer handed over whole; a bigger one, or one of unknown length, is relayed as it arrives.
const MAX_WHOLE_BYTES: usize = 1024 * 1024;

/// The engine's end of a WebSocket it relays for the page.
type Socket = WebSocketStream<Prefixed<DuplexStream>>;

/// What the extension does with a request it caught.
pub(super) enum Decision {
    /// Nothing claimed it: the browser sends it. `base` is the log entry the extension's mirror completes.
    Continue(Box<RequestLogEntry>),
    /// A whole answer of known length, already read: a mock, a preflight, a fault, most rule answers.
    Respond { status: u16, headers: Vec<(String, String)>, body: Vec<u8> },
    /// An answer whose body is still arriving: a streaming mock, a rule's target streaming or sending a lot.
    Relay { status: u16, headers: Vec<(String, String)>, body: ResponseBody<DuplexStream> },
    /// A WebSocket a rule claimed, open to the rule's target.
    Socket { headers: Vec<(String, String)>, socket: Box<Socket> },
    /// Claimed but never answered (a dropped or reset fault, an upstream that broke off): a network error.
    Failed,
}

/// Route a request the extension caught. `None` when `url` is not an `http(s)` or `ws(s)` URL.
pub(super) async fn decide(
    shared: &Shared,
    method: &str,
    url: &str,
    headers: &Headers,
    body: Vec<u8>,
) -> Option<Decision> {
    let parsed = url::Url::parse(url).ok()?;
    // Rules are written against the `http(s)` URL a WebSocket's upgrade request has.
    let (scheme, socket) = match parsed.scheme() {
        scheme @ ("http" | "https") => (scheme, false),
        "ws" => ("http", true),
        "wss" => ("https", true),
        _ => return None,
    };
    let hostname = parsed.host_str()?.to_string();
    // `port()` is `None` for the scheme's default, which neither the URL nor `Host` spells out.
    let host = match parsed.port() {
        Some(port) => format!("{hostname}:{port}"),
        None => hostname.clone(),
    };
    let req_path = format!("{}{}", parsed.path(), parsed.query().map(|q| format!("?{q}")).unwrap_or_default());
    let url = format!("{scheme}://{host}{req_path}");
    // The page never sees `Host`; a rule's target gets the production host, as with TLS interception.
    let mut headers: Headers =
        headers.iter().filter(|(name, _)| !is_hop_by_hop(name)).map(|(n, v)| (n.to_lowercase(), v.clone())).collect();
    strip_cache_headers(&mut headers);
    headers.entry("host".to_string()).or_insert(host);
    if socket {
        // The relay answers this upgrade as the page's browser would have had it answered.
        let key = tokio_tungstenite::tungstenite::handshake::client::generate_key();
        headers.insert("sec-websocket-key".to_string(), key);
    }

    let snapshot = shared.snapshot();
    let t0 = now_ms();
    let (mut base, req_body_b64) =
        base_entry(mk_id(), t0, method, url.clone(), hostname.clone(), headers, &body, &snapshot.routing.workspace_id);
    let method = method.to_string();
    // The routing writes an HTTP/1.1 answer as it would to a socket; it runs on its own so a streamed body
    // keeps flowing after the head is read here.
    let (mut out, mut answer) = duplex(64 * 1024);
    let routed = tokio::spawn(async move {
        if crate::faults::apply(&mut out, &mut base, t0).await {
            return None;
        }
        let forwarded = forwarded_headers(&base.req_headers, &base.id);
        let exchange = Exchange {
            routing: &snapshot.routing,
            method: &method,
            url: &url,
            rule_match_target: &url,
            req_path: &req_path,
            headers: &forwarded,
            body: &body,
            req_body_b64: &req_body_b64,
            base: &base,
            t0,
        };
        if !socket {
            let claimed = handle_proxy_routing(&mut out, &exchange).await;
            return (!claimed).then_some(base);
        }
        // Mocks do not answer WebSockets, and one no rule claims is the browser's own.
        if !match_proxy_rule(&snapshot.routing.rules, &url, &snapshot.routing.mappings).matched {
            return Some(base);
        }
        relay_proxied(out, &exchange, &hostname).await;
        None
    });

    let Some((head, leftover)) = read_head(&mut answer).await else {
        return Some(match routed.await {
            Ok(Some(base)) => Decision::Continue(Box::new(base)),
            _ => Decision::Failed,
        });
    };
    let status = head.target.parse().unwrap_or(502);
    let headers = head
        .header_lines
        .iter()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_string()))
        .filter(|(name, _)| !is_hop_by_hop(name))
        .collect();
    if socket && status == 101 {
        let socket = WebSocketStream::from_raw_socket(Prefixed::new(leftover, answer), Role::Client, None).await;
        return Some(Decision::Socket { headers, socket: Box::new(socket) });
    }
    let body = ResponseBody::new(answer, leftover, &head.header_lines);
    let length = raw_header(&head.header_lines, "content-length").and_then(|v| v.parse::<usize>().ok());
    if length.is_none_or(|n| n > MAX_WHOLE_BYTES) {
        return Some(Decision::Relay { status, headers, body });
    }
    Some(match body.read_to_end().await {
        Ok(body) => Decision::Respond { status, headers, body },
        Err(_) => Decision::Failed,
    })
}

/// How long a stream nobody asked about is kept: the backstop for a tab that went away without a word.
const IDLE_TTL: Duration = Duration::from_secs(60 * 60);
/// How long `browser.read` waits for something to hand over before it answers with nothing.
const READ_WAIT: Duration = Duration::from_secs(20);

/// A piece of a relayed answer, in order.
enum Piece {
    Chunk(Vec<u8>),
    /// A WebSocket frame from the target, as the log shows it.
    Frame(Value),
    /// The answer broke off.
    Failed,
}

/// A request the browser sent itself, as far as the extension has reported it.
struct Mirror {
    base: RequestLogEntry,
    /// The status and headers, once they arrived.
    opened: Option<(u16, Headers)>,
    /// `sse` or `ws` for a stream logged as live while it is open.
    kind: Option<String>,
    seq: u64,
}

type Pieces = Arc<tokio::sync::Mutex<mpsc::Receiver<Piece>>>;

enum Kind {
    Relay(Pieces),
    /// A relayed WebSocket: what the page sends goes into `sink`.
    Socket(Pieces, Arc<tokio::sync::Mutex<SplitSink<Socket, Message>>>),
    Mirror(Box<Mirror>),
}

struct Stream {
    session: String,
    touched: Instant,
    kind: Kind,
}

static STREAMS: LazyLock<Mutex<HashMap<String, Stream>>> = LazyLock::new(Default::default);

fn session_of(ctx: &CommandContext) -> String {
    ctx.session.as_ref().map(|s| s.session_id.clone()).unwrap_or_default()
}

/// Takes every stream `gone` picks out of the table.
fn remove_where(gone: impl Fn(&Stream) -> bool) -> Vec<Stream> {
    let mut streams = STREAMS.lock();
    let ids: Vec<String> = streams.iter().filter(|(_, s)| gone(s)).map(|(id, _)| id.clone()).collect();
    ids.iter().filter_map(|id| streams.remove(id)).collect()
}

/// Puts a stream in the table, first ending the ones that went idle.
fn insert(id: String, session: String, kind: Kind) {
    remove_where(|s| s.touched.elapsed() > IDLE_TTL).into_iter().for_each(end);
    STREAMS.lock().insert(id, Stream { session, touched: Instant::now(), kind });
}

/// Runs `f` on the session's stream `id` and marks it used; `None` when the session has no such stream.
fn with_stream<T>(id: &str, session: &str, f: impl FnOnce(&mut Stream) -> T) -> Option<T> {
    let mut streams = STREAMS.lock();
    let stream = streams.get_mut(id).filter(|s| s.session == session)?;
    stream.touched = Instant::now();
    Some(f(stream))
}

fn take(id: &str, session: &str) -> Option<Stream> {
    let mut streams = STREAMS.lock();
    if streams.get(id).is_some_and(|s| s.session == session) { streams.remove(id) } else { None }
}

/// A stream that goes without its close. A relay's answer is dropped with it, which stops the routing; a
/// mirror is logged as it stands, so an entry shown as live does not stay live.
fn end(stream: Stream) {
    if let Kind::Mirror(mirror) = stream.kind {
        finish(*mirror, &[]);
    }
}

/// Ends the streams of a session that has gone.
pub(crate) fn end_session(session: &str) {
    remove_where(|s| s.session == session).into_iter().for_each(end);
}

/// Logs a request the browser sent itself as the proxy logs one it passed through. The browser hands over the
/// body decoded, so the log does not claim an encoding.
fn finish(mirror: Mirror, body: &[u8]) {
    let base = &mirror.base;
    let duration = now_ms() - base.ts;
    let mut entry = match mirror.opened {
        Some((status, headers)) => {
            let headers = strip_content_encoding(&headers);
            let mut entry =
                finished_entry(base, status, "proxy", Some(base.host.clone()), duration, headers, log_body(body));
            entry.kind = mirror.kind;
            entry
        }
        // It failed before an answer: no status, as with a dropped connection.
        None => RequestLogEntry { via: "error".to_string(), duration_ms: Some(duration), ..base.clone() },
    };
    mask_secrets(&mut entry);
    emit_log(&entry);
}

/// Feeds a relayed body into a channel, so a read that times out loses nothing. Stops when the reader goes.
fn pump(mut body: ResponseBody<DuplexStream>) -> Pieces {
    let (tx, rx) = mpsc::channel(8);
    tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                _ = tx.closed() => return,
                next = body.next() => next,
            };
            let piece = match next {
                Ok(Some(chunk)) => Piece::Chunk(chunk),
                Ok(None) => return,
                Err(_) => Piece::Failed,
            };
            let failed = matches!(piece, Piece::Failed);
            if tx.send(piece).await.is_err() || failed {
                return;
            }
        }
    });
    Arc::new(tokio::sync::Mutex::new(rx))
}

/// Feeds a relayed WebSocket's frames into a channel, up to and including its close.
fn pump_socket(socket: Socket) -> Kind {
    let (sink, mut frames) = socket.split();
    let (tx, rx) = mpsc::channel(32);
    tokio::spawn(async move {
        loop {
            let next = tokio::select! {
                _ = tx.closed() => return,
                next = frames.next() => next,
            };
            let Some(Ok(message)) = next else {
                let _ = tx.send(Piece::Failed).await;
                return;
            };
            let close = message.is_close();
            if let Some(frame) = logged(&message, "received")
                && tx.send(Piece::Frame(frame)).await.is_err()
            {
                return;
            }
            if close {
                return;
            }
        }
    });
    Kind::Socket(Arc::new(tokio::sync::Mutex::new(rx)), Arc::new(tokio::sync::Mutex::new(sink)))
}

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn headers_of(value: &Value) -> Headers {
    value
        .as_object()
        .map(|map| map.iter().map(|(k, v)| (k.to_lowercase(), v.as_str().unwrap_or_default().to_string())).collect())
        .unwrap_or_default()
}

fn str_param<'a>(params: &'a Value, name: &str) -> &'a str {
    params.get(name).and_then(Value::as_str).unwrap_or_default()
}

async fn decide_command(params: Value, ctx: CommandContext) -> crate::protocol::Result<Value> {
    let Some(shared) = server::running() else {
        return Err(EngineError::engine("the forward proxy is not running").keyed("browser.proxy_off", json!({})));
    };
    let body = decode_base64_lenient(str_param(&params, "body"));
    let headers = headers_of(&params["headers"]);
    let decision = decide(&shared, str_param(&params, "method"), str_param(&params, "url"), &headers, body)
        .await
        .ok_or_else(|| EngineError::engine("not a web address").keyed("browser.bad_url", json!({})))?;
    let session = session_of(&ctx);
    Ok(match decision {
        Decision::Continue(base) => {
            let id = base.id.clone();
            insert(
                id.clone(),
                session,
                Kind::Mirror(Box::new(Mirror { base: *base, opened: None, kind: None, seq: 0 })),
            );
            json!({ "action": "continue", "id": id })
        }
        Decision::Respond { status, headers, body } => {
            json!({ "action": "respond", "status": status, "headers": headers, "body": base64(&body) })
        }
        Decision::Relay { status, headers, body } => {
            let id = mk_id();
            insert(id.clone(), session, Kind::Relay(pump(body)));
            json!({ "action": "relay", "id": id, "status": status, "headers": headers })
        }
        Decision::Socket { headers, socket } => {
            let id = mk_id();
            insert(id.clone(), session, pump_socket(*socket));
            json!({ "action": "socket", "id": id, "headers": headers })
        }
        Decision::Failed => json!({ "action": "fail" }),
    })
}

/// What has arrived of a relayed answer or WebSocket since the last read: waits up to [`READ_WAIT`] for the
/// first piece, then takes whatever else is ready. `done` once it has ended (or there is no such stream).
async fn read_command(params: Value, ctx: CommandContext) -> crate::protocol::Result<Value> {
    let (id, session) = (str_param(&params, "id"), session_of(&ctx));
    let pieces = with_stream(id, &session, |s| match &s.kind {
        Kind::Relay(pieces) | Kind::Socket(pieces, _) => Some(pieces.clone()),
        Kind::Mirror(_) => None,
    });
    let Some(pieces) = pieces.flatten() else { return Ok(json!({ "chunks": [], "frames": [], "done": true })) };
    let mut pieces = pieces.lock().await;
    let Ok(mut next) = tokio::time::timeout(READ_WAIT, pieces.recv()).await else {
        return Ok(json!({ "chunks": [], "frames": [], "done": false }));
    };
    let (mut chunks, mut frames, mut done, mut failed) = (Vec::new(), Vec::new(), false, false);
    loop {
        match next {
            Some(Piece::Chunk(chunk)) => chunks.push(base64(&chunk)),
            Some(Piece::Frame(frame)) => frames.push(frame),
            Some(Piece::Failed) => (done, failed) = (true, true),
            None => done = true,
        }
        if done {
            break;
        }
        match pieces.try_recv() {
            Ok(piece) => next = Some(piece),
            Err(mpsc::error::TryRecvError::Empty) => break,
            Err(mpsc::error::TryRecvError::Disconnected) => next = None,
        }
    }
    drop(pieces);
    if done {
        take(id, &session);
    }
    let mut out = json!({ "chunks": chunks, "frames": frames, "done": done });
    if failed {
        out["error"] = json!("the answer broke off");
    }
    Ok(out)
}

/// The answer to a request the browser sent itself has arrived. An event stream or a WebSocket is logged now,
/// as live, and again when it ends.
fn open_command(params: &Value, ctx: &CommandContext) -> Value {
    let status = params["status"].as_u64().unwrap_or(0) as u16;
    let headers = headers_of(&params["headers"]);
    let kind = params.get("kind").and_then(Value::as_str).map(str::to_string);
    with_stream(str_param(params, "id"), &session_of(ctx), |s| {
        let Kind::Mirror(mirror) = &mut s.kind else { return };
        if mirror.opened.is_some() {
            return;
        }
        if let Some(kind) = &kind {
            let live = LiveLog { base: &mirror.base, via: "proxy", target: Some(mirror.base.host.clone()) };
            emit_log(&live.open_entry(kind, status, now_ms() - mirror.base.ts, headers.clone()));
        }
        mirror.opened = Some((status, headers));
        mirror.kind = kind;
    });
    json!({})
}

/// One event of a mirrored event stream, or one frame of a mirrored WebSocket.
fn message_command(params: &Value, ctx: &CommandContext) -> Value {
    with_stream(str_param(params, "id"), &session_of(ctx), |s| {
        if let Kind::Mirror(mirror) = &mut s.kind {
            mirror.seq += 1;
            emit_log_message(&mirror.base, mirror.seq, params["message"].clone());
        }
    });
    json!({})
}

/// A mirrored exchange ended, with the body the browser saw (the extension caps it).
fn close_command(params: &Value, ctx: &CommandContext) -> Value {
    if let Some(Stream { kind: Kind::Mirror(mirror), .. }) = take(str_param(params, "id"), &session_of(ctx)) {
        finish(*mirror, &decode_base64_lenient(str_param(params, "body")));
    }
    json!({})
}

/// The page gave up on a request, or the extension leaves it out of the capture (a script or a stylesheet a page
/// fetched): a relayed answer is dropped, which stops the routing; a mirror with no answer yet goes unlogged, one
/// that had one is logged as it stands.
fn cancel_command(params: &Value, ctx: &CommandContext) -> Value {
    match take(str_param(params, "id"), &session_of(ctx)) {
        Some(Stream { kind: Kind::Mirror(mirror), .. }) if mirror.opened.is_none() => {}
        Some(stream) => end(stream),
        None => {}
    }
    json!({})
}

/// The page's end of a relayed WebSocket: `send` (text, or base64 when `binary`) or `close`.
async fn socket_command(params: Value, ctx: CommandContext, close: bool) -> crate::protocol::Result<Value> {
    let sink = with_stream(str_param(&params, "id"), &session_of(&ctx), |s| match &s.kind {
        Kind::Socket(_, sink) => Some(sink.clone()),
        _ => None,
    });
    let Some(sink) = sink.flatten() else { return Ok(json!({ "sent": false })) };
    let message = if close {
        let code = params.get("code").and_then(Value::as_u64).map_or(CloseCode::Normal, |c| CloseCode::from(c as u16));
        Message::Close(Some(CloseFrame { code, reason: str_param(&params, "reason").to_string().into() }))
    } else if params.get("binary").and_then(Value::as_bool).unwrap_or(false) {
        Message::binary(decode_base64_lenient(str_param(&params, "data")))
    } else {
        Message::text(str_param(&params, "data").to_string())
    };
    let sent = sink.lock().await.send(message).await.is_ok();
    Ok(json!({ "sent": sent }))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("browser.decide", decide_command);
    registry.register("browser.read", read_command);
    registry.register_sync("browser.open", |params, ctx| Ok(open_command(&params, ctx)));
    registry.register_sync("browser.message", |params, ctx| Ok(message_command(&params, ctx)));
    registry.register_sync("browser.close", |params, ctx| Ok(close_command(&params, ctx)));
    registry.register_sync("browser.cancel", |params, ctx| Ok(cancel_command(&params, ctx)));
    registry.register("browser.wsSend", |params, ctx| socket_command(params, ctx, false));
    registry.register("browser.wsClose", |params, ctx| socket_command(params, ctx, true));
}
