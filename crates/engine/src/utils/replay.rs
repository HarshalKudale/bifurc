//! `replay_request`: send a request to its target and return the response, for `request.replay` and for
//! the proxies' own request path.
//!
//! `replay_request` (the proxies' own path) gives every request a fresh connection (`connection: close`,
//! no pooling) so a server that answers `keep-alive` cannot hand a half-closed socket to the next request to
//! the same origin. `replay_streamed` (what the user sends) pools connections.

use std::time::Duration;

use base64::Engine as _;
use reqwest::{
    Method,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::{
    kernel::{bus, traffic::Headers},
    sse::parser::Parser,
    utils::{
        content_type::is_text_content_type,
        decompress::{decompress_body, strip_content_encoding},
        http::{Cancellable, ClientOptions, RequestSettings, client as http_client, is_hop_by_hop, root_message},
        id::now_ms,
    },
};

/// The answer to a replayed request. `body` is base64 of the decompressed bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayResponse {
    pub status: u16,
    pub headers: Headers,
    pub body: String,
}

/// Response headers Node keeps only the first occurrence of when a server sends them twice.
const FIRST_WINS: [&str; 18] = [
    "age",
    "authorization",
    "content-length",
    "content-type",
    "etag",
    "expires",
    "from",
    "host",
    "if-modified-since",
    "if-unmodified-since",
    "last-modified",
    "location",
    "max-forwards",
    "proxy-authorization",
    "referer",
    "retry-after",
    "server",
    "user-agent",
];

/// `Buffer.from(text, "base64")`: both alphabets are accepted, characters outside them are skipped and
/// decoding stops at the first `=`. Never fails.
pub fn decode_base64_lenient(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut bits: u32 = 0;
    let mut count = 0;
    for c in text.chars() {
        let value = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' | '-' => 62,
            '/' | '_' => 63,
            '=' => break,
            _ => continue,
        };
        bits = (bits << 6) | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    out
}

/// Plain wording for a refused connection or an unknown host, the innermost message of the chain otherwise.
fn connect_message(err: &reqwest::Error, host: &str, port: u16) -> String {
    super::http::describe_send_error(err, Some(host), Some(port))
}

/// A header value's bytes as text, one char per byte (Node decodes header values as latin1).
fn latin1(value: &HeaderValue) -> String {
    value.as_bytes().iter().map(|&b| b as char).collect()
}

fn response_headers(map: &HeaderMap) -> Headers {
    let mut out = Headers::new();
    for name in map.keys() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        let mut values = map.get_all(name).iter().map(latin1);
        let joined = if FIRST_WINS.contains(&name.as_str()) {
            values.next().unwrap_or_default()
        } else {
            values.collect::<Vec<_>>().join(", ")
        };
        out.insert(name.as_str().to_string(), joined);
    }
    out
}

/// Send `method url` with `headers` and the base64 `body_base64`; the response body comes back decompressed
/// and base64 encoded, without its `content-encoding` header and without hop-by-hop headers. Redirects are
/// not followed, certificates are verified and there is no timeout.
pub async fn replay_request(
    method: &str,
    url: &str,
    headers: &Headers,
    body_base64: &str,
) -> Result<ReplayResponse, String> {
    let client = http_client(ClientOptions { pooled: false, ..ClientOptions::sending(true, false) })?;
    send(client, false, &RequestSettings::default(), method, url, headers, body_base64, None).await
}

/// How long text that arrived waits for more before it is reported, so a fast stream is a few events a second
/// rather than one for every chunk.
const STREAM_FLUSH: Duration = Duration::from_millis(50);

/// Whether a response is one the server may send in pieces over time: text with no `content-length`.
fn arrives_in_pieces(headers: &HeaderMap) -> bool {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_ascii_lowercase();
    // shortcut: a compressed body is not reported while it arrives, decode it per chunk when one needs to be.
    !headers.contains_key("content-length")
        && matches!(text("content-encoding").as_str(), "" | "identity")
        && is_text_content_type(&text("content-type"))
}

/// How much of `bytes` is whole characters: all of it, unless it ends inside one.
fn whole_chars(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Err(err) if err.error_len().is_none() => err.valid_up_to(),
        _ => bytes.len(),
    }
}

/// What turns a body that is arriving into the messages of `request.stream` events.
struct Reporter<'a> {
    stream_id: &'a str,
    /// The status and headers, until the first event has carried them.
    head: Option<(u16, &'a Headers)>,
    /// Set for a `text/event-stream` body: each of its events is a message. Any other body is a message
    /// for the text that arrived since the last one.
    events: Option<Parser>,
    /// How much of a plain body has become messages.
    reported: usize,
    pending: Vec<Value>,
    count: u64,
}

impl Reporter<'_> {
    fn message(&mut self, data: String) -> Value {
        self.count += 1;
        json!({ "id": self.count.to_string(), "ts": now_ms(), "data": data })
    }

    /// Take the bytes that were just added to the body.
    fn take(&mut self, chunk: &[u8]) {
        let events = self.events.as_mut().map(|parser| parser.feed(chunk)).unwrap_or_default();
        for event in events {
            let mut message = self.message(event.data);
            message["event"] = json!(event.event);
            if let Some(id) = event.id {
                message["eventId"] = json!(id);
            }
            self.pending.push(message);
        }
    }

    /// Emit the messages there are. `raw` is the body so far; at its `end` the last characters are final.
    fn flush(&mut self, raw: &[u8], end: bool) {
        if self.events.is_none() {
            let rest = &raw[self.reported..];
            let whole = if end { rest.len() } else { whole_chars(rest) };
            if whole > 0 {
                let message = self.message(String::from_utf8_lossy(&rest[..whole]).into_owned());
                self.pending.push(message);
                self.reported += whole;
            }
        }
        if self.pending.is_empty() {
            return;
        }
        let mut payload = json!({ "streamId": self.stream_id, "messages": std::mem::take(&mut self.pending) });
        if let Some((status, headers)) = self.head.take() {
            payload["status"] = json!(status);
            payload["headers"] = json!(headers);
        }
        bus().emit("request.stream", payload);
    }
}

/// Read a body to its end, reporting it as it arrives.
async fn read_reporting(
    mut response: reqwest::Response,
    stream_id: &str,
    status: u16,
    headers: &Headers,
) -> Result<Vec<u8>, String> {
    let is_events = headers.get("content-type").is_some_and(|t| t.to_ascii_lowercase().contains("text/event-stream"));
    let mut reporter = Reporter {
        stream_id,
        head: Some((status, headers)),
        events: is_events.then(Parser::new),
        reported: 0,
        pending: Vec::new(),
        count: 0,
    };
    let mut raw: Vec<u8> = Vec::new();
    let mut due: Option<Instant> = None;
    loop {
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|err| root_message(&err))?,
            _ = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                due = None;
                reporter.flush(&raw, false);
                continue;
            }
        };
        let Some(chunk) = chunk else { break };
        raw.extend_from_slice(&chunk);
        reporter.take(&chunk);
        due.get_or_insert_with(|| Instant::now() + STREAM_FLUSH);
    }
    reporter.flush(&raw, true);
    Ok(raw)
}

/// `replay_request` for a caller that shows the response while it arrives. With a `stream_id`, a text body
/// the server sends without a `content-length` is also reported in `request.stream` events as it is read:
/// `{streamId, messages}`, the first one with the `status` and `headers` too. A message is `{id, ts, data}`:
/// one event of a `text/event-stream` body (with its `event` name and `eventId`), or the text that arrived
/// since the last message of any other body. The result is the whole response either way.
///
/// `settings` pick the client (certificate checks, redirects) and the time limit: it covers connecting and
/// waiting for the headers, and for a body that is not reported while it arrives, reading it. A body that is
/// reported while it arrives is not cut by it. With a `stream_id` the send can be stopped by `request.cancel`,
/// which fails it with [`CANCELLED`](crate::utils::http::CANCELLED).
pub async fn replay_streamed(
    method: &str,
    url: &str,
    headers: &Headers,
    body_base64: &str,
    stream_id: Option<&str>,
    settings: &RequestSettings,
) -> Result<ReplayResponse, String> {
    let client = settings.client()?;
    let cancellable = stream_id.map(Cancellable::register);
    let sending = send(client, true, settings, method, url, headers, body_base64, stream_id);
    match &cancellable {
        Some(cancellable) => cancellable.run(sending).await,
        None => sending.await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn send(
    client: reqwest::Client,
    pooled: bool,
    settings: &RequestSettings,
    method: &str,
    url: &str,
    headers: &Headers,
    body_base64: &str,
    stream_id: Option<&str>,
) -> Result<ReplayResponse, String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid URL".to_string())?;
    let host = parsed.host_str().filter(|host| !host.is_empty()).ok_or_else(|| "Invalid URL".to_string())?;
    let https = parsed.scheme() == "https";
    let port = parsed.port().unwrap_or(if https { 443 } else { 80 });
    let mut target = format!("{}://{host}:{port}{}", if https { "https" } else { "http" }, parsed.path());
    if let Some(query) = parsed.query() {
        target.push('?');
        target.push_str(query);
    }

    let body = if body_base64.is_empty() { Vec::new() } else { decode_base64_lenient(body_base64) };

    let mut request_headers = HeaderMap::new();
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("connection") {
            continue;
        }
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("Invalid character in header name \"{name}\""))?;
        let value = HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| format!("Invalid character in header content [\"{name}\"]"))?;
        request_headers.append(name, value);
    }
    if !pooled {
        request_headers.insert(HeaderName::from_static("connection"), HeaderValue::from_static("close"));
    }
    crate::cookies::attach(parsed.as_str(), &mut request_headers);

    let method = Method::from_bytes(method.as_bytes())
        .map_err(|_| format!("Method must be a valid HTTP token [\"{method}\"]"))?;
    let mut request = client.request(method, target).headers(request_headers);
    if !body.is_empty() {
        request = request.body(body);
    }

    let deadline = settings.deadline();
    let response = deadline.run(request.send()).await?.map_err(|err| connect_message(&err, host, port))?;
    let status = response.status().as_u16();
    crate::cookies::collect(parsed.as_str(), response.headers());
    let mut headers = response_headers(response.headers());
    let raw = match stream_id.filter(|_| arrives_in_pieces(response.headers())) {
        Some(id) => read_reporting(response, id, status, &headers).await?,
        None => deadline.run(response.bytes()).await?.map_err(|err| root_message(&err))?.to_vec(),
    };

    let encoding = headers.get("content-encoding").cloned().unwrap_or_default();
    let decompressed = decompress_body(&raw, &encoding);
    if !encoding.is_empty() {
        headers = strip_content_encoding(&headers);
    }
    Ok(ReplayResponse { status, headers, body: base64::engine::general_purpose::STANDARD.encode(decompressed) })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::{Compression, write::GzEncoder};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;

    /// Serve one connection: record the raw request, answer with `response`.
    async fn serve_once(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                received.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&received).to_string();
                if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if rest.len() >= length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            socket.write_all(&response).await.unwrap();
            let _ = socket.shutdown().await;
            String::from_utf8_lossy(&received).to_string()
        });
        (format!("http://{addr}"), task)
    }

    fn http_response(head: &str, body: &[u8]) -> Vec<u8> {
        let mut out =
            format!("HTTP/1.1 {head}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len()).into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[tokio::test]
    async fn sends_the_request_and_returns_status_headers_and_body() {
        let response = http_response("201 Created\r\nX-Multi: a\r\nX-Multi: b\r\nx-Custom: Yes", b"hello");
        let (base, server) = serve_once(response).await;
        let mut headers = Headers::new();
        headers.insert("X-Test".into(), "1".into());
        let body = base64::engine::general_purpose::STANDARD.encode("payload");
        let result = replay_request("POST", &format!("{base}/path?q=1#frag"), &headers, &body).await.unwrap();
        assert_eq!(result.status, 201);
        assert_eq!(result.headers.get("x-multi").unwrap(), "a, b");
        assert_eq!(result.headers.get("x-custom").unwrap(), "Yes");
        assert!(!result.headers.contains_key("connection"));
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&result.body).unwrap(), b"hello");

        let request = server.await.unwrap().to_lowercase();
        assert!(request.starts_with("post /path?q=1 http/1.1\r\n"), "{request}");
        assert!(request.contains("x-test: 1\r\n"));
        assert!(request.contains("connection: close\r\n"));
        assert!(request.contains("content-length: 7\r\n"));
        assert!(request.ends_with("payload"));
    }

    #[tokio::test]
    async fn a_gzip_body_is_decoded_and_content_encoding_dropped() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"squeezed text").unwrap();
        let gz = encoder.finish().unwrap();
        let response = http_response("200 OK\r\ncontent-encoding: gzip\r\nContent-Type: text/plain", &gz);
        let (base, _server) = serve_once(response).await;
        let result = replay_request("GET", &format!("{base}/"), &Headers::new(), "").await.unwrap();
        assert_eq!(result.status, 200);
        assert!(!result.headers.contains_key("content-encoding"));
        assert_eq!(result.headers.get("content-type").unwrap(), "text/plain");
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&result.body).unwrap(), b"squeezed text");
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_get_sends_no_content_length() {
        let response = http_response("302 Found\r\nLocation: /elsewhere", b"");
        let (base, server) = serve_once(response).await;
        let result = replay_request("GET", &format!("{base}/start"), &Headers::new(), "").await.unwrap();
        assert_eq!(result.status, 302);
        assert_eq!(result.headers.get("location").unwrap(), "/elsewhere");
        assert_eq!(result.body, "");
        assert!(!server.await.unwrap().to_lowercase().contains("content-length"));
    }

    #[tokio::test]
    async fn a_body_sent_in_pieces_is_reported_as_it_arrives_and_returned_whole() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (go, wait) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let head = b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\n\r\n";
            socket.write_all(head).await.unwrap();
            // The first piece ends inside the two bytes of "é".
            socket.write_all(b"6\r\nhello\xc3\r\n").await.unwrap();
            socket.flush().await.unwrap();
            // The rest is held back until the first piece has been reported.
            wait.await.unwrap();
            socket.write_all(b"7\r\n\xa9 world\r\n0\r\n\r\n").await.unwrap();
            let _ = socket.shutdown().await;
        });

        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let (sink, go) = (seen.clone(), parking_lot::Mutex::new(Some(go)));
        let listening = bus().on("request.stream", move |payload| {
            if payload["streamId"] == "s-pieces" {
                sink.lock().push(payload.clone());
                if let Some(go) = go.lock().take() {
                    let _ = go.send(());
                }
            }
        });
        let result = replay_streamed(
            "GET",
            &format!("http://{addr}/"),
            &Headers::new(),
            "",
            Some("s-pieces"),
            &RequestSettings::default(),
        )
        .await
        .unwrap();
        bus().off(listening);

        let seen = seen.lock();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(seen[0]["status"], 200);
        assert_eq!(seen[0]["headers"]["content-type"], "text/plain");
        assert_eq!(seen[0]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(seen[0]["messages"][0]["id"], "1");
        assert_eq!(seen[0]["messages"][0]["data"], "hello");
        assert!(seen[1].get("status").is_none());
        assert_eq!(seen[1]["messages"][0]["id"], "2");
        assert_eq!(seen[1]["messages"][0]["data"], "é world");
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&result.body).unwrap(), "helloé world".as_bytes());
    }

    #[tokio::test]
    async fn an_event_stream_is_reported_as_its_events() {
        let response = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n\
            30\r\nevent: delta\nid: 7\ndata: {\"n\":1}\n\ndata: [DONE]\n\n\r\n0\r\n\r\n";
        let (base, _server) = serve_once(response.to_vec()).await;
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let listening = bus().on("request.stream", move |payload| {
            if payload["streamId"] == "s-events" {
                sink.lock().extend(payload["messages"].as_array().unwrap().clone());
            }
        });
        let result = replay_streamed(
            "GET",
            &format!("{base}/"),
            &Headers::new(),
            "",
            Some("s-events"),
            &RequestSettings::default(),
        )
        .await
        .unwrap();
        bus().off(listening);

        let seen = seen.lock();
        assert_eq!(seen.len(), 2, "{seen:?}");
        assert_eq!(
            (&seen[0]["event"], &seen[0]["eventId"], &seen[0]["data"]),
            (&json!("delta"), &json!("7"), &json!("{\"n\":1}"))
        );
        assert_eq!((&seen[1]["event"], &seen[1]["data"]), (&json!("message"), &json!("[DONE]")));
        assert!(
            String::from_utf8(base64::engine::general_purpose::STANDARD.decode(&result.body).unwrap())
                .unwrap()
                .ends_with("[DONE]\n\n")
        );
    }

    #[tokio::test]
    async fn a_body_of_a_known_length_is_not_reported() {
        let (base, _server) = serve_once(http_response("200 OK\r\nContent-Type: text/plain", b"hello")).await;
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(0));
        let sink = seen.clone();
        let listening = bus().on("request.stream", move |payload| {
            if payload["streamId"] == "s-whole" {
                *sink.lock() += 1;
            }
        });
        let result = replay_streamed(
            "GET",
            &format!("{base}/"),
            &Headers::new(),
            "",
            Some("s-whole"),
            &RequestSettings::default(),
        )
        .await
        .unwrap();
        bus().off(listening);
        assert_eq!(*seen.lock(), 0);
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&result.body).unwrap(), b"hello");
    }

    #[test]
    fn lenient_base64_matches_node() {
        assert_eq!(decode_base64_lenient("aGVsbG8="), b"hello");
        assert_eq!(decode_base64_lenient("aGVsbG8"), b"hello");
        assert_eq!(decode_base64_lenient("aGVs\nbG8="), b"hello");
        assert_eq!(decode_base64_lenient("-_-_"), [0xfb, 0xff, 0xbf]);
        assert_eq!(decode_base64_lenient("!!"), b"");
    }

    #[tokio::test]
    async fn an_invalid_url_is_an_error() {
        let err = replay_request("GET", "not a url", &Headers::new(), "").await.unwrap_err();
        assert_eq!(err, "Invalid URL");
    }

    #[tokio::test]
    async fn a_refused_connection_is_an_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = replay_request("GET", &format!("http://127.0.0.1:{port}/"), &Headers::new(), "").await.unwrap_err();
        assert!(!err.is_empty());
    }

    /// A server that accepts connections and never answers.
    async fn silent_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                held.push(socket);
            }
        });
        format!("http://{addr}/")
    }

    fn settings(timeout_ms: u64, follow_redirects: bool) -> RequestSettings {
        RequestSettings { timeout_ms, follow_redirects, ..RequestSettings::default() }
    }

    #[tokio::test]
    async fn a_send_that_gets_no_answer_times_out() {
        let url = silent_server().await;
        let err = replay_streamed("GET", &url, &Headers::new(), "", None, &settings(200, false)).await.unwrap_err();
        assert_eq!(err, "Timed out after 200 ms");
        assert!(crate::utils::http::is_timeout(&err));
    }

    #[tokio::test]
    async fn the_time_limit_does_not_cut_a_body_that_is_reported_while_it_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await.unwrap();
            let head = b"HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ntransfer-encoding: chunked\r\n\r\n";
            socket.write_all(head).await.unwrap();
            socket.write_all(b"2\r\nhi\r\n").await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
            socket.write_all(b"1\r\n!\r\n0\r\n\r\n").await.unwrap();
            let _ = socket.shutdown().await;
        });
        let result = replay_streamed(
            "GET",
            &format!("http://{addr}/"),
            &Headers::new(),
            "",
            Some("s-limit"),
            &settings(200, false),
        )
        .await
        .unwrap();
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&result.body).unwrap(), b"hi!");
    }

    #[tokio::test]
    async fn a_cancelled_send_fails_as_cancelled() {
        let url = silent_server().await;
        let sending = tokio::spawn(async move {
            replay_streamed("GET", &url, &Headers::new(), "", Some("s-cancel"), &RequestSettings::default()).await
        });
        while !crate::utils::http::cancel("s-cancel") {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(sending.await.unwrap().unwrap_err(), crate::utils::http::CANCELLED);
        assert!(!crate::utils::http::cancel("s-cancel"));
    }

    #[tokio::test]
    async fn redirects_are_followed_only_when_asked() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    let first = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_string();
                    let reply = if first.contains("/start") {
                        "HTTP/1.1 302 Found\r\nlocation: /end\r\ncontent-length: 0\r\n\r\n"
                    } else {
                        "HTTP/1.1 200 OK\r\ncontent-length: 4\r\n\r\ndone"
                    };
                    socket.write_all(reply.as_bytes()).await.unwrap();
                    let _ = socket.shutdown().await;
                });
            }
        });
        let url = format!("http://{addr}/start");
        let kept = replay_streamed("GET", &url, &Headers::new(), "", None, &settings(0, false)).await.unwrap();
        assert_eq!(kept.status, 302);
        let followed = replay_streamed("GET", &url, &Headers::new(), "", None, &settings(0, true)).await.unwrap();
        assert_eq!(followed.status, 200);
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(&followed.body).unwrap(), b"done");
    }

    #[tokio::test]
    async fn certificates_are_checked_unless_the_settings_say_not_to() {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let key = rustls::pki_types::PrivateKeyDer::try_from(certified.signing_key.serialize_der()).unwrap();
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], key)
            .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(socket).await else { return };
                    let mut buf = [0u8; 4096];
                    let _ = tls.read(&mut buf).await;
                    let _ = tls.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok").await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        let url = format!("https://localhost:{port}/");
        let verified = replay_streamed("GET", &url, &Headers::new(), "", None, &RequestSettings::default()).await;
        assert!(verified.is_err(), "a self-signed certificate must be refused by default");
        let lax = RequestSettings { verify_tls: false, ..RequestSettings::default() };
        let result = replay_streamed("GET", &url, &Headers::new(), "", None, &lax).await.unwrap();
        assert_eq!(result.status, 200);
    }
}
