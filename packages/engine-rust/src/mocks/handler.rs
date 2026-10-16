//! Mock matching and responding: which mock answers a request, the response it describes (whole, or merged
//! with the real server's answer), and writing that response to a socket.

use std::{collections::HashSet, time::Duration};

use serde_json::Value;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::{
    env::resolve_vars,
    kernel::traffic::Headers,
    utils::{
        content_type::is_text_content_type,
        http::HOP_BY_HOP,
        json::{is_truthy, opt_str, str_of, string_map},
        replay::decode_base64_lenient,
    },
};

/// A response a mock produced, ready to write.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedMockResponse {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub delay_ms: u64,
}

/// What the real server answered, buffered and decompressed.
#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamResponse {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
    pub duration_ms: i64,
}

/// Whether `value[key]` is on: absent or null means `default`, anything else is JavaScript truthiness.
fn flag(mock: &Value, key: &str, default: bool) -> bool {
    match mock.get(key) {
        None | Some(Value::Null) => default,
        Some(value) => is_truthy(value),
    }
}

fn response_status(mock: &Value) -> u16 {
    mock.get("responseStatus")
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        .and_then(|n| u16::try_from(n).ok())
        .unwrap_or(200)
}

/// The mock's configured delay in milliseconds; zero when absent or not positive.
fn configured_delay(mock: &Value) -> u64 {
    match mock.get("responseDelay").and_then(Value::as_f64) {
        Some(ms) if ms > 0.0 => ms as u64,
        _ => 0,
    }
}

fn is_base64(mock: &Value) -> bool {
    opt_str(mock, "responseBodyEncoding") == Some("base64")
}

/// The first enabled mock that answers `method url`. A `*` method matches any method; the URL pattern is
/// resolved against `env`, then compared exactly, or tested as a JavaScript regular expression.
pub fn match_mock<'a>(mocks: &'a [Value], method: &str, url: &str, env: Option<&Value>) -> Option<&'a Value> {
    for mock in mocks {
        if !flag(mock, "enabled", false) {
            continue;
        }
        let mock_method = str_of(mock, "method");
        if mock_method != "*" && !mock_method.eq_ignore_ascii_case(method) {
            continue;
        }
        let pattern = resolve_vars(str_of(mock, "urlPattern"), env);
        let hit = if flag(mock, "useRegex", false) {
crate::utils::regex::is_match(&pattern, url)
        } else {
            url == pattern
        };
        if hit {
            return Some(mock);
        }
    }
    None
}

fn skipped_response_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    HOP_BY_HOP.contains(&lower.as_str()) || lower == "content-length" || lower == "content-encoding"
}

fn resolve_mock_body(mock: &Value, env: Option<&Value>) -> Vec<u8> {
    let body = str_of(mock, "responseBody");
    if is_base64(mock) { decode_base64_lenient(body) } else { resolve_vars(body, env).into_bytes() }
}

fn resolve_mock_headers(mock: &Value, env: Option<&Value>) -> Headers {
    let mut headers = Headers::new();
    for (name, value) in string_map(mock, "responseHeaders") {
        if !skipped_response_header(&name) {
            let value = if is_base64(mock) { value } else { resolve_vars(&value, env) };
            headers.insert(name, value);
        }
    }
    headers
}

/// `headers` with a `content-type` (the default when none is set, any case) and the body's `content-length`.
fn ensure_response_headers(headers: &Headers, body: &[u8], default_content_type: &str) -> Headers {
    let mut next = headers.clone();
    if !next.keys().any(|key| key.eq_ignore_ascii_case("content-type")) {
        next.insert("content-type".to_string(), default_content_type.to_string());
    }
    next.insert("content-length".to_string(), body.len().to_string());
    next.insert("connection".to_string(), "close".to_string());
    next
}

fn default_content_type(mock: &Value) -> &'static str {
    if is_base64(mock) { "application/octet-stream" } else { "application/json" }
}

fn header_value(headers: &Headers, name: &str) -> String {
    headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, v)| v.clone()).unwrap_or_default()
}

fn mock_header_value(mock: &Value, name: &str) -> String {
    string_map(mock, "responseHeaders")
        .into_iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
        .unwrap_or_default()
}

/// A mock only replaces text. A body stored as bytes under a data type (an image, a PDF) is not text, so the
/// mock is not answered on its own: the real response goes through as it is.
fn mock_body_is_text(mock: &Value) -> bool {
    let content_type = mock_header_value(mock, "content-type");
    if content_type.is_empty() { !is_base64(mock) } else { is_text_content_type(&content_type) }
}

/// Whether the mock answers in pieces over time.
pub fn is_streaming_mock(mock: &Value) -> bool {
    opt_str(mock, "streamingMode").is_some_and(|mode| !mode.is_empty() && mode != "none")
}

/// Whether the mock is the whole response (nothing is fetched from the real server).
pub fn is_fully_mocked(mock: &Value) -> bool {
    if is_streaming_mock(mock) {
        return true;
    }
    // Nothing real to merge with (the endpoint does not exist yet): the mock is the whole response.
    if flag(mock, "standalone", false) {
        return true;
    }
    if !mock_body_is_text(mock) {
        return false;
    }
    let mocked_keys: HashSet<String> = mock
        .get("mockedResponseHeaders")
        .and_then(Value::as_array)
        .map(|keys| keys.iter().filter_map(Value::as_str).map(str::to_lowercase).collect())
        .unwrap_or_default();
    // The response's Content-Type is always the real server's, so it is never a header the mock has to set.
    let all_headers_mocked = resolve_mock_headers(mock, None)
        .keys()
        .filter(|key| !key.trim().is_empty() && !key.eq_ignore_ascii_case("content-type"))
        .all(|key| mocked_keys.contains(&key.to_lowercase()));
    flag(mock, "responseStatusMocked", true)
        && flag(mock, "responseBodyMocked", true)
        && flag(mock, "responseDelayMocked", true)
        && all_headers_mocked
}

/// The response of a fully mocked request.
pub fn resolve_mock_only_response(mock: &Value, env: Option<&Value>) -> ResolvedMockResponse {
    let body = resolve_mock_body(mock, env);
    let headers = ensure_response_headers(&resolve_mock_headers(mock, env), &body, default_content_type(mock));
    ResolvedMockResponse { status: response_status(mock), headers, body, delay_ms: configured_delay(mock) }
}

/// A partial mock laid over what the real server answered: the parts the mock marks as mocked replace the
/// server's, everything else is the server's.
pub fn merge_mock_with_upstream(
    mock: &Value,
    upstream: &UpstreamResponse,
    env: Option<&Value>,
) -> ResolvedMockResponse {
    let mock_headers = resolve_mock_headers(mock, env);
    // Data (an image, a PDF) cannot be mocked: the body the server sent is the body the caller gets. Status,
    // the other headers and the delay can still be.
    let upstream_is_text = is_text_content_type(&header_value(&upstream.headers, "content-type"));
    let body = if flag(mock, "responseBodyMocked", true) && mock_body_is_text(mock) && upstream_is_text {
        resolve_mock_body(mock, env)
    } else {
        upstream.body.clone()
    };
    let mocked_keys: HashSet<String> = mock
        .get("mockedResponseHeaders")
        .and_then(Value::as_array)
        .map(|keys| keys.iter().filter_map(Value::as_str).map(str::to_lowercase).collect())
        .unwrap_or_default();
    let mut headers = upstream.headers.clone();
    for (key, value) in mock_headers {
        // Content-Type is locked: whatever type the server answered with is the one the caller gets.
        if key.eq_ignore_ascii_case("content-type") {
            continue;
        }
        if mocked_keys.contains(&key.to_lowercase()) {
            headers.insert(key, value);
        }
    }
    ResolvedMockResponse {
        status: if flag(mock, "responseStatusMocked", true) { response_status(mock) } else { upstream.status },
        headers: ensure_response_headers(&headers, &body, default_content_type(mock)),
        body,
        delay_ms: if flag(mock, "responseDelayMocked", true) { configured_delay(mock) } else { 0 },
    }
}

/// The reason phrase of the statuses a mock commonly answers with; `Unknown` for the rest.
pub fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// Write a complete response (a `set-cookie` value holding several cookies, newline separated, becomes one
/// header line each) and close the stream.
pub async fn write_response<W: AsyncWrite + Unpin>(
    out: &mut W,
    status: u16,
    headers: &Headers,
    body: &[u8],
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", status_text(status));
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("set-cookie") {
            for cookie in value.split('\n').filter(|c| !c.is_empty()) {
                head.push_str(&format!("{name}: {cookie}\r\n"));
            }
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes()).await?;
    out.write_all(body).await?;
    out.shutdown().await
}

/// Answer with a fully mocked response, after its delay (streaming mocks stream).
pub async fn serve_mock<W: AsyncWrite + Unpin>(out: &mut W, mock: &Value, env: Option<&Value>) -> std::io::Result<()> {
    let delay = configured_delay(mock);
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
    if is_streaming_mock(mock) {
        return serve_streaming_mock(out, mock, env).await;
    }
    let resolved = resolve_mock_only_response(mock, env);
    write_response(out, resolved.status, &resolved.headers, &resolved.body).await
}

/// Write an already resolved response after its delay.
pub async fn serve_resolved_response<W: AsyncWrite + Unpin>(
    out: &mut W,
    response: &ResolvedMockResponse,
) -> std::io::Result<()> {
    if response.delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(response.delay_ms)).await;
    }
    write_response(out, response.status, &response.headers, &response.body).await
}

fn set_default(headers: &mut Headers, name: &str, fallback: &str) {
    if headers.get(name).is_none_or(|v| v.is_empty()) {
        headers.insert(name.to_string(), fallback.to_string());
    }
}

async fn write_chunk<W: AsyncWrite + Unpin>(out: &mut W, data: &[u8]) -> std::io::Result<()> {
    out.write_all(format!("{:x}\r\n", data.len()).as_bytes()).await?;
    out.write_all(data).await?;
    out.write_all(b"\r\n").await
}

/// Answer with the mock's body in pieces, `streamingChunkDelay` milliseconds apart: server-sent events
/// (split on blank lines) or chunks split on `streamingChunkSeparator`.
pub async fn serve_streaming_mock<W: AsyncWrite + Unpin>(
    out: &mut W,
    mock: &Value,
    env: Option<&Value>,
) -> std::io::Result<()> {
    let chunk_delay = Duration::from_millis(match mock.get("streamingChunkDelay").and_then(Value::as_f64) {
        Some(ms) if ms > 0.0 => ms as u64,
        Some(_) => 0,
        None => 100,
    });
    let body = str_of(mock, "responseBody");
    let body = if is_base64(mock) { body.to_string() } else { resolve_vars(body, env) };

    let mut headers = Headers::new();
    for (name, value) in string_map(mock, "responseHeaders") {
        if !skipped_response_header(&name) {
            headers.insert(name, resolve_vars(&value, env));
        }
    }

    let sse = opt_str(mock, "streamingMode") == Some("sse");
    let separator = opt_str(mock, "streamingChunkSeparator").unwrap_or("\n\n").to_string();
    if sse {
        set_default(&mut headers, "content-type", "text/event-stream");
        headers.insert("cache-control".to_string(), "no-cache".to_string());
        headers.insert("connection".to_string(), "keep-alive".to_string());
        headers.insert("transfer-encoding".to_string(), "chunked".to_string());
    } else {
        headers.insert("transfer-encoding".to_string(), "chunked".to_string());
        headers.insert("connection".to_string(), "keep-alive".to_string());
        set_default(&mut headers, "content-type", "application/json");
    }

    let status = response_status(mock);
    let mut head = format!("HTTP/1.1 {status} {}\r\n", status_text(status));
    for (name, value) in &headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    out.write_all(head.as_bytes()).await?;

    let pieces: Vec<&str> =
        body.split(if sse { "\n\n" } else { separator.as_str() }).filter(|p| !p.is_empty()).collect();
    for (index, piece) in pieces.iter().enumerate() {
        let data = if sse {
            format!("{piece}\n\n")
        } else if index < pieces.len() - 1 {
            format!("{piece}{separator}")
        } else {
            piece.to_string()
        };
        write_chunk(out, data.as_bytes()).await?;
        out.flush().await?;
        tokio::time::sleep(chunk_delay).await;
    }
    out.write_all(b"0\r\n\r\n").await?;
    out.shutdown().await
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::AsyncReadExt;

    use super::*;

    fn mock(extra: Value) -> Value {
        let mut base = json!({
            "id": "m1", "method": "GET", "urlPattern": "http://a.test/x", "useRegex": false, "enabled": true,
            "responseStatus": 201, "responseHeaders": { "x-one": "1" }, "responseBody": "{\"a\":1}",
            "responseBodyEncoding": "utf8", "responseDelay": 0,
        });
        for (key, value) in extra.as_object().unwrap() {
            base[key] = value.clone();
        }
        base
    }

    fn env() -> Value {
        json!({ "variables": [{ "key": "host", "value": "a.test" }, { "key": "who", "value": "me" }] })
    }

    #[test]
    fn matching_by_method_exact_url_and_regex() {
        let mocks = vec![
            mock(json!({ "id": "off", "enabled": false })),
            mock(json!({ "id": "post", "method": "POST" })),
            mock(json!({ "id": "bad", "useRegex": true, "urlPattern": "(" })),
            mock(json!({ "id": "re", "useRegex": true, "urlPattern": "^http://a\\.test/y/\\d+$", "method": "*" })),
            mock(json!({ "id": "var", "urlPattern": "http://{{host}}/x" })),
        ];
        let id = |m: Option<&Value>| m.map(|m| str_of(m, "id").to_string());
        let e = env();
        assert_eq!(id(match_mock(&mocks, "get", "http://a.test/x", Some(&e))), Some("var".into()));
        assert_eq!(id(match_mock(&mocks, "GET", "http://a.test/x", None)), None);
        assert_eq!(id(match_mock(&mocks, "PUT", "http://a.test/y/12", None)), Some("re".into()));
        assert_eq!(id(match_mock(&mocks, "PUT", "http://a.test/y/ab", None)), None);
        assert_eq!(id(match_mock(&mocks, "POST", "http://a.test/x", None)), Some("post".into()));
    }

    #[test]
    fn fully_mocked_rules() {
        assert!(is_fully_mocked(&mock(json!({ "streamingMode": "sse" }))));
        assert!(!is_fully_mocked(&mock(json!({ "streamingMode": "none" }))));
        assert!(is_fully_mocked(&mock(json!({ "responseStatusMocked": false, "standalone": true }))));
        assert!(!is_fully_mocked(&mock(json!({ "responseStatusMocked": false }))));
        // A header the mock sets but does not mark as mocked makes it partial; Content-Type never counts.
        assert!(!is_fully_mocked(&mock(json!({ "mockedResponseHeaders": [] }))));
        assert!(is_fully_mocked(&mock(json!({ "mockedResponseHeaders": ["X-One"] }))));
        assert!(is_fully_mocked(&mock(json!({ "responseHeaders": { "Content-Type": "application/json" } }))));
        // A binary body is never answered alone.
        let image = json!({ "responseBodyEncoding": "base64", "responseHeaders": { "content-type": "image/png" } });
        assert!(!is_fully_mocked(&mock(image)));
    }

    #[test]
    fn mock_only_response() {
        let m = mock(json!({
            "responseBody": "hi {{who}}", "responseDelay": 25,
            "responseHeaders": { "x-who": "{{who}}", "Content-Length": "9", "connection": "keep-alive" },
        }));
        let r = resolve_mock_only_response(&m, Some(&env()));
        assert_eq!(r.status, 201);
        assert_eq!(r.body, b"hi me");
        assert_eq!(r.delay_ms, 25);
        let keys: Vec<&str> = r.headers.keys().map(String::as_str).collect();
        assert_eq!(keys, ["x-who", "content-type", "content-length", "connection"]);
        assert_eq!(r.headers["x-who"], "me");
        assert_eq!(r.headers["content-type"], "application/json");
        assert_eq!(r.headers["content-length"], "5");
    }

    #[test]
    fn base64_mock_is_not_resolved_and_defaults_to_octet_stream() {
        let m = mock(json!({
            "responseBodyEncoding": "base64", "responseBody": "aGk=", "responseHeaders": { "x-a": "{{who}}" },
        }));
        let r = resolve_mock_only_response(&m, Some(&env()));
        assert_eq!(r.body, b"hi");
        assert_eq!(r.headers["x-a"], "{{who}}");
        assert_eq!(r.headers["content-type"], "application/octet-stream");
    }

    fn upstream(content_type: &str) -> UpstreamResponse {
        let mut headers = Headers::new();
        headers.insert("content-type".into(), content_type.into());
        headers.insert("x-one".into(), "real".into());
        headers.insert("x-two".into(), "real".into());
        UpstreamResponse { status: 200, headers, body: b"real body".to_vec(), duration_ms: 3 }
    }

    #[test]
    fn merge_replaces_only_what_is_marked_mocked() {
        let m = mock(json!({
            "responseStatusMocked": false, "responseBodyMocked": true, "responseDelayMocked": false,
            "responseDelay": 50, "mockedResponseHeaders": ["x-one"],
            "responseHeaders": { "x-one": "mock", "x-two": "mock", "Content-Type": "text/plain" },
        }));
        let r = merge_mock_with_upstream(&m, &upstream("application/json"), None);
        assert_eq!(r.status, 200);
        assert_eq!(r.delay_ms, 0);
        assert_eq!(r.body, b"{\"a\":1}");
        assert_eq!(r.headers["x-one"], "mock");
        assert_eq!(r.headers["x-two"], "real");
        assert_eq!(r.headers["content-type"], "application/json");
        assert_eq!(r.headers["content-length"], "7");
        assert_eq!(r.headers["connection"], "close");
    }

    #[test]
    fn merge_keeps_the_servers_data_body() {
        let m = mock(json!({}));
        let r = merge_mock_with_upstream(&m, &upstream("image/png"), None);
        assert_eq!(r.body, b"real body");
        assert_eq!(r.status, 201);
    }

    #[tokio::test]
    async fn write_response_splits_cookies_and_closes() {
        let mut headers = Headers::new();
        headers.insert("set-cookie".into(), "a=1\nb=2".into());
        headers.insert("content-length".into(), "2".into());
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_response(&mut a, 418, &headers, b"ok").await.unwrap();
        let mut text = String::new();
        b.read_to_string(&mut text).await.unwrap();
        assert_eq!(text, "HTTP/1.1 418 Unknown\r\nset-cookie: a=1\r\nset-cookie: b=2\r\ncontent-length: 2\r\n\r\nok");
    }

    #[tokio::test]
    async fn sse_mock_streams_events_as_chunks() {
        let m = mock(json!({
            "streamingMode": "sse", "streamingChunkDelay": 1, "responseBody": "data: 1\n\ndata: 2\n\n",
            "responseHeaders": {},
        }));
        let (mut a, mut b) = tokio::io::duplex(4096);
        serve_mock(&mut a, &m, None).await.unwrap();
        let mut text = String::new();
        b.read_to_string(&mut text).await.unwrap();
        assert!(text.starts_with("HTTP/1.1 201 Created\r\ncontent-type: text/event-stream\r\n"));
        assert!(text.contains("transfer-encoding: chunked\r\n"));
        assert!(text.ends_with("\r\n\r\n9\r\ndata: 1\n\n\r\n9\r\ndata: 2\n\n\r\n0\r\n\r\n"));
    }

    #[tokio::test]
    async fn chunked_mock_splits_on_the_separator() {
        let m = mock(json!({
            "streamingMode": "chunked", "streamingChunkDelay": 0, "streamingChunkSeparator": "|",
            "responseBody": "a|b|c", "responseHeaders": {},
        }));
        let (mut a, mut b) = tokio::io::duplex(4096);
        serve_mock(&mut a, &m, None).await.unwrap();
        let mut text = String::new();
        b.read_to_string(&mut text).await.unwrap();
        assert!(text.contains("content-type: application/json\r\n"));
        assert!(text.ends_with("\r\n\r\n2\r\na|\r\n2\r\nb|\r\n1\r\nc\r\n0\r\n\r\n"));
    }
}
