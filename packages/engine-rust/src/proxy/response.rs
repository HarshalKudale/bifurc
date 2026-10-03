//! The forward half's upstream requests and response handling: passthrough that streams the answer to the
//! caller, the buffered fetch that runs a rule's scripts, and the header and body plumbing both share.

use std::{sync::OnceLock, time::Instant};

use base64::Engine as _;
use reqwest::{
    Client, Method, Response,
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::{
    kernel::telemetry::{Headers, MAX_LOGGED_BODY_BYTES, emit_log_chunk},
    mocks::UpstreamResponse,
    scripting::execute_response_script,
    utils::{
        decompress::{decompress_body, strip_content_encoding},
        http::is_hop_by_hop,
        json::str_of,
        send_html::html_response,
    },
};

/// The buffered answer of the real server (what mocks merge with).
pub type BufferedProxyResponse = UpstreamResponse;

/// How an upstream exchange ended, for the caller to log. The body is base64 of the first
/// `MAX_LOGGED_BODY_BYTES` of the decompressed response.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: u16,
    pub duration_ms: i64,
    pub res_headers: Headers,
    pub res_body: String,
    /// The request headers as the rule's script left them, secrets still written as `{{vault.secret}}`, when
    /// they differ from what the client sent. This is what the log shows.
    pub req_headers: Option<Headers>,
}

impl Outcome {
    /// The outcome of an exchange that failed before an answer.
    pub fn bad_gateway(duration_ms: i64) -> Self {
        Self { status: 502, duration_ms, res_headers: Headers::new(), res_body: String::new(), req_headers: None }
    }
}

/// Response headers a server may repeat that Node keeps only the first value of.
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

/// The client for every upstream: HTTP/1.1, no redirects, no pooling, no system proxy, and certificates are
/// not verified (the proxy shows what the server sends, including a server with a self-signed certificate).
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
        .no_proxy()
        .danger_accept_invalid_certs(true)
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

/// A response's headers without the hop-by-hop ones: the record that is logged (`set-cookie` values joined
/// with newlines, repeats joined with `, `) and the lines to write back, one per cookie.
pub fn collect_response_headers(map: &HeaderMap) -> (Headers, Vec<(String, Vec<u8>)>) {
    let mut record = Headers::new();
    let mut lines = Vec::new();
    for name in map.keys() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        let values: Vec<&HeaderValue> = map.get_all(name).iter().collect();
        if name.as_str() == "set-cookie" {
            for value in &values {
                lines.push((name.as_str().to_string(), value.as_bytes().to_vec()));
            }
            record.insert(name.as_str().to_string(), values.iter().map(|v| latin1(v)).collect::<Vec<_>>().join("\n"));
            continue;
        }
        if FIRST_WINS.contains(&name.as_str()) {
            lines.push((name.as_str().to_string(), values[0].as_bytes().to_vec()));
            record.insert(name.as_str().to_string(), latin1(values[0]));
            continue;
        }
        let joined: Vec<u8> = values.iter().map(|v| v.as_bytes()).collect::<Vec<_>>().join(&b", "[..]);
        lines.push((name.as_str().to_string(), joined));
        record.insert(name.as_str().to_string(), values.iter().map(|v| latin1(v)).collect::<Vec<_>>().join(", "));
    }
    (record, lines)
}

/// A request for `url` as the proxy sends it: the caller's headers (minus the ones that describe the hop),
/// `connection: close`, and the body's own length.
fn build_request(method: &str, url: &str, headers: &Headers, body: &[u8]) -> Result<reqwest::RequestBuilder, String> {
    let method = Method::from_bytes(method.as_bytes())
        .map_err(|_| format!("Method must be a valid HTTP token [\"{method}\"]"))?;
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if matches!(lower.as_str(), "connection" | "content-length" | "transfer-encoding" | "expect") {
            continue;
        }
        let (Ok(name), Ok(value)) =
            (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_bytes(value.as_bytes()))
        else {
            continue;
        };
        map.append(name, value);
    }
    map.insert(HeaderName::from_static("connection"), HeaderValue::from_static("close"));
    let mut request = client()?.request(method, url).headers(map);
    if !body.is_empty() {
        request = request.header("content-length", body.len().to_string()).body(body.to_vec());
    }
    Ok(request)
}

pub async fn send_request(method: &str, url: &str, headers: &Headers, body: &[u8]) -> Result<Response, String> {
    build_request(method, url, headers, body)?.send().await.map_err(|err| root_message(&err))
}

fn elapsed_ms(since: Instant) -> i64 {
    since.elapsed().as_millis() as i64
}

/// The first `MAX_LOGGED_BODY_BYTES` of `body`, base64.
pub fn log_body(body: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(&body[..body.len().min(MAX_LOGGED_BODY_BYTES)])
}

/// Write a small HTML error page and close.
pub async fn send_html<W: AsyncWrite + Unpin>(out: &mut W, status: u16, html: &str) {
    let _ = out.write_all(&html_response(status, html)).await;
    let _ = out.shutdown().await;
}

fn status_line(res: &Response) -> String {
    let status = res.status();
    format!("HTTP/1.1 {} {}\r\n", status.as_u16(), status.canonical_reason().unwrap_or(""))
}

/// Stream an upstream response to `out` as it arrives, and log it when it ends. `None` when the caller or the
/// upstream went away mid-response (nothing is logged for an exchange that did not finish).
pub async fn handle_proxy_response<W: AsyncWrite + Unpin>(
    mut res: Response,
    out: &mut W,
    t0: Instant,
    log_id: Option<&str>,
) -> Option<Outcome> {
    let status = res.status().as_u16();
    let (res_headers, lines) = collect_response_headers(res.headers());
    let mut head = status_line(&res).into_bytes();
    for (name, value) in &lines {
        head.extend_from_slice(name.as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(value);
        head.extend_from_slice(b"\r\n");
    }
    head.extend_from_slice(b"connection: close\r\n\r\n");

    let streaming = log_id.is_some()
        && (res_headers.get("content-type").is_some_and(|v| v.contains("text/event-stream"))
            || res_headers.get("transfer-encoding").is_some_and(|v| v.contains("chunked")));
    out.write_all(&head).await.ok()?;
    out.flush().await.ok()?;

    let mut full: Vec<u8> = Vec::new();
    loop {
        match res.chunk().await {
            Ok(Some(chunk)) => {
                full.extend_from_slice(&chunk);
                if let Some(id) = log_id.filter(|_| streaming) {
                    emit_log_chunk(id, &chunk, false);
                }
                out.write_all(&chunk).await.ok()?;
                out.flush().await.ok()?;
            }
            Ok(None) => break,
            Err(_) => {
                let _ = out.shutdown().await;
                return None;
            }
        }
    }
    let _ = out.shutdown().await;
    if let Some(id) = log_id.filter(|_| streaming) {
        emit_log_chunk(id, &[], true);
    }
    let encoding = res_headers.get("content-encoding").cloned().unwrap_or_default();
    let decompressed = decompress_body(&full, &encoding);
    let logged_headers = if encoding.is_empty() { res_headers } else { strip_content_encoding(&res_headers) };
    Some(Outcome {
        status,
        duration_ms: elapsed_ms(t0),
        res_headers: logged_headers,
        res_body: log_body(&decompressed),
        req_headers: None,
    })
}

/// Send the request to `url` and stream the answer to `out`. A failure to connect becomes a 502 page naming
/// `label`.
pub async fn forward_streaming<W: AsyncWrite + Unpin>(
    out: &mut W,
    method: &str,
    url: &str,
    label: &str,
    headers: &Headers,
    body: &[u8],
    log_id: Option<&str>,
) -> Option<Outcome> {
    let t0 = Instant::now();
    match send_request(method, url, headers, body).await {
        Ok(res) => handle_proxy_response(res, out, t0, log_id).await,
        Err(_) => {
            send_html(out, 502, &format!("<h1>502 Bad Gateway</h1><p>Could not connect to <code>{label}</code>.</p>"))
                .await;
            Some(Outcome::bad_gateway(elapsed_ms(t0)))
        }
    }
}

/// Forward a proxied `http://` request to the host its target names.
pub async fn passthrough_to_upstream<W: AsyncWrite + Unpin>(
    out: &mut W,
    method: &str,
    raw_target: &str,
    path: &str,
    headers: &Headers,
    body: &[u8],
    log_id: Option<&str>,
) -> Option<Outcome> {
    let t0 = Instant::now();
    let full_url = if raw_target.starts_with("http") {
        raw_target.to_string()
    } else {
        format!("http://{}{path}", headers.get("host").map(String::as_str).unwrap_or(""))
    };
    let parsed = url::Url::parse(&full_url).ok().and_then(|u| {
        let host = u.host_str().filter(|h| !h.is_empty())?.to_string();
        Some((host, u.port().unwrap_or(80)))
    });
    let Some((hostname, port)) = parsed else {
        send_html(out, 502, "<h1>502 Bad Gateway</h1><p>Could not parse upstream URL.</p>").await;
        return Some(Outcome::bad_gateway(elapsed_ms(t0)));
    };
    let path = if path.is_empty() { "/" } else { path };
    let label = format!("{hostname}:{port}");
    forward_streaming(out, method, &format!("http://{hostname}:{port}{path}"), &label, headers, body, log_id).await
}

/// Forward a request of an intercepted TLS connection to the real `hostname:port` over HTTPS.
pub async fn passthrough_to_upstream_https<W: AsyncWrite + Unpin>(
    out: &mut W,
    method: &str,
    hostname: &str,
    port: u16,
    path: &str,
    headers: &Headers,
    body: &[u8],
    log_id: Option<&str>,
) -> Option<Outcome> {
    let path = if path.is_empty() { "/" } else { path };
    let label = format!("{hostname}:{port}");
    forward_streaming(out, method, &format!("https://{hostname}:{port}{path}"), &label, headers, body, log_id).await
}

fn has_script(rule: Option<&Value>, key: &str) -> bool {
    rule.is_some_and(|rule| !str_of(rule, key).trim().is_empty())
}

/// Fetch the whole answer of a rule's target, running the rule's request script before and its response
/// script after (a script that fails leaves the message as it was). `target` is `host:port`, or a URL, in which
/// case the URL's own path is requested instead of `path`.
pub async fn fetch_upstream_response(
    method: &str,
    target: &str,
    path: &str,
    req_headers: &Headers,
    req_body: &[u8],
    rule: Option<&Value>,
    ws_id: &str,
) -> Result<BufferedProxyResponse, String> {
    let (mut headers, mut body) = (req_headers.clone(), req_body.to_vec());
    if has_script(rule, "requestScript") {
        let scripted =
            super::handler::run_request_script(rule.unwrap_or(&Value::Null), ws_id, req_headers, req_body).await;
        (headers, body) = (scripted.headers, scripted.body);
    }

    let t0 = Instant::now();
    let is_url = target.starts_with("http://") || target.starts_with("https://");
    let target_url = url::Url::parse(&if is_url { target.to_string() } else { format!("http://{target}") })
        .map_err(|err| err.to_string())?;
    let host = target_url.host_str().filter(|h| !h.is_empty()).ok_or_else(|| "Invalid URL".to_string())?;
    let https = target_url.scheme() == "https";
    let port = target_url.port().unwrap_or(if https { 443 } else { 80 });
    let request_path = if is_url {
        let mut path = target_url.path().to_string();
        if let Some(query) = target_url.query() {
            path.push('?');
            path.push_str(query);
        }
        path
    } else if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };
    let url = format!("{}://{host}:{port}{request_path}", if https { "https" } else { "http" });

    let res = send_request(method, &url, &headers, &body).await?;
    let status = res.status().as_u16();
    let (raw_headers, _) = collect_response_headers(res.headers());
    let raw = res.bytes().await.map_err(|err| root_message(&err))?;
    let encoding = raw_headers.get("content-encoding").cloned().unwrap_or_default();
    let decompressed = decompress_body(&raw, &encoding);
    let decompressed_headers = if encoding.is_empty() { raw_headers } else { strip_content_encoding(&raw_headers) };

    if !has_script(rule, "responseScript") {
        return Ok(UpstreamResponse {
            status,
            headers: decompressed_headers,
            body: decompressed,
            duration_ms: elapsed_ms(t0),
        });
    }
    let script = str_of(rule.unwrap_or(&Value::Null), "responseScript");
    let scripted =
        execute_response_script(script, &decompressed_headers, &String::from_utf8_lossy(&decompressed)).await;
    let failed = scripted.error.is_some();
    Ok(UpstreamResponse {
        status,
        headers: if failed { decompressed_headers } else { scripted.headers },
        body: if failed { decompressed } else { scripted.body.into_bytes() },
        duration_ms: elapsed_ms(t0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_headers_keep_cookies_apart_and_join_repeats() {
        let mut map = HeaderMap::new();
        map.append("set-cookie", HeaderValue::from_static("a=1"));
        map.append("set-cookie", HeaderValue::from_static("b=2"));
        map.append("x-multi", HeaderValue::from_static("a"));
        map.append("x-multi", HeaderValue::from_static("b"));
        map.append("content-type", HeaderValue::from_static("text/plain"));
        map.append("content-type", HeaderValue::from_static("ignored"));
        map.append("connection", HeaderValue::from_static("keep-alive"));
        map.append("transfer-encoding", HeaderValue::from_static("chunked"));
        let (record, lines) = collect_response_headers(&map);
        assert_eq!(record["set-cookie"], "a=1\nb=2");
        assert_eq!(record["x-multi"], "a, b");
        assert_eq!(record["content-type"], "text/plain");
        assert!(!record.contains_key("connection") && !record.contains_key("transfer-encoding"));
        let cookie_lines = lines.iter().filter(|(name, _)| name == "set-cookie").count();
        assert_eq!(cookie_lines, 2);
    }

    #[test]
    fn logged_body_is_capped() {
        let big = vec![b'a'; MAX_LOGGED_BODY_BYTES + 10];
        let decoded = base64::engine::general_purpose::STANDARD.decode(log_body(&big)).unwrap();
        assert_eq!(decoded.len(), MAX_LOGGED_BODY_BYTES);
    }
}
