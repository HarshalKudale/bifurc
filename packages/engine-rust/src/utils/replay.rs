//! `replay_request`: send a request to its target and return the response, for `request.replay` and for
//! the proxies' own request path.
//!
//! Every request gets a fresh connection (`connection: close`, no pooling) so a server that answers
//! `keep-alive` cannot hand a half-closed socket to the next request to the same origin.

use std::sync::OnceLock;

use base64::Engine as _;
use reqwest::{
    Client, Method,
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};

use crate::{
    kernel::telemetry::Headers,
    utils::{
        decompress::{decompress_body, strip_content_encoding},
        http::is_hop_by_hop,
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

fn client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let built = Client::builder()
        .http1_only()
        .redirect(Policy::none())
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|err| err.to_string())?;
    Ok(CLIENT.get_or_init(|| built))
}

/// The innermost message of an error chain, which is the one that names the cause.
fn root_message(err: &dyn std::error::Error) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(inner) = source {
        message = inner.to_string();
        source = inner.source();
    }
    message
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
/// not followed. There is no timeout.
pub async fn replay_request(
    method: &str,
    url: &str,
    headers: &Headers,
    body_base64: &str,
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
    request_headers.insert(HeaderName::from_static("connection"), HeaderValue::from_static("close"));

    let method = Method::from_bytes(method.as_bytes())
        .map_err(|_| format!("Method must be a valid HTTP token [\"{method}\"]"))?;
    let mut request = client()?.request(method, target).headers(request_headers);
    if !body.is_empty() {
        request = request.body(body);
    }

    let response = request.send().await.map_err(|err| root_message(&err))?;
    let status = response.status().as_u16();
    let mut headers = response_headers(response.headers());
    let raw = response.bytes().await.map_err(|err| root_message(&err))?;

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
}
