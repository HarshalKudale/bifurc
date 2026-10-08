//! A plain HTTP exchange for the GraphQL and SOAP commands: send text, read the body as text.
//!
//! Unlike `request.replay` it does not decode the body or strip headers, and it does not verify the server's
//! certificate: the target is whatever the user typed in.

use std::sync::OnceLock;

use reqwest::{
    Client, Method,
    header::{HeaderMap, HeaderName, HeaderValue},
    redirect::Policy,
};
use serde_json::{Map, Value, json};

/// What came back: status, headers (lower-case names, repeated headers joined with `, `) and the body text.
pub struct DirectResponse {
    pub status: u16,
    pub headers: Value,
    pub body: String,
}

/// A header map as pairs, in order; non-string values are skipped.
pub fn header_pairs(headers: &Value) -> Vec<(String, String)> {
    headers
        .as_object()
        .map(|map| map.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
        .unwrap_or_default()
}

pub(crate) fn client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client);
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let built = Client::builder()
        .http1_only()
        .redirect(Policy::none())
        .pool_max_idle_per_host(0)
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|err| err.to_string())?;
    Ok(CLIENT.get_or_init(|| built))
}

/// Node's wording for the failures a user meets most, the innermost message of the chain otherwise.
pub(crate) fn describe_error(err: &reqwest::Error, host: &str, port: u16) -> String {
    let mut message = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(inner) = source {
        if let Some(io) = inner.downcast_ref::<std::io::Error>() {
            if io.kind() == std::io::ErrorKind::ConnectionRefused {
                return format!("connect ECONNREFUSED {host}:{port}");
            }
        }
        message = inner.to_string();
        if message.contains("dns error") {
            return format!("getaddrinfo ENOTFOUND {host}");
        }
        source = inner.source();
    }
    message
}

/// Send `method url` with `headers` (names compared case-insensitively, later ones replace earlier ones) and
/// the text `body`. Fails with the transport's message, or `Invalid URL`.
pub async fn exchange(
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<DirectResponse, String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid URL".to_string())?;
    let host = parsed.host_str().filter(|host| !host.is_empty()).ok_or_else(|| "Invalid URL".to_string())?;
    let https = parsed.scheme() == "https";
    let port = parsed.port().unwrap_or(if https { 443 } else { 80 });
    let mut target = format!("{}://{host}:{port}{}", if https { "https" } else { "http" }, parsed.path());
    if let Some(query) = parsed.query() {
        target.push('?');
        target.push_str(query);
    }

    let mut request_headers = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| format!("Invalid character in header name \"{name}\""))?;
        let value = HeaderValue::from_bytes(value.as_bytes())
            .map_err(|_| format!("Invalid character in header content [\"{name}\"]"))?;
        request_headers.insert(name, value);
    }

    let mut request = client()?.request(Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?, target);
    request = request.headers(request_headers);
    if let Some(body) = body {
        request = request.body(body.as_bytes().to_vec());
    }
    let response = request.send().await.map_err(|err| describe_error(&err, host, port))?;
    let status = response.status().as_u16();
    let mut out = Map::new();
    for name in response.headers().keys() {
        let values: Vec<String> = response
            .headers()
            .get_all(name)
            .iter()
            .map(|value| value.as_bytes().iter().map(|&b| b as char).collect())
            .collect();
        out.insert(name.as_str().to_string(), json!(values.join(", ")));
    }
    let raw = response.bytes().await.map_err(|err| describe_error(&err, host, port))?;
    Ok(DirectResponse { status, headers: Value::Object(out), body: String::from_utf8_lossy(&raw).into_owned() })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::rest::test_server;

    #[tokio::test]
    async fn posts_text_and_reads_text() {
        let base = test_server::start(HashMap::new()).await;
        let headers = vec![("X-Test".to_string(), "1".to_string())];
        let response = exchange("POST", &format!("{base}/a?b=1"), &headers, Some("hello")).await.unwrap();
        assert_eq!(response.status, 200);
        let echoed: Value = serde_json::from_str(&response.body).unwrap();
        assert_eq!(echoed["method"], "POST");
        assert_eq!(echoed["path"], "/a?b=1");
        assert_eq!(echoed["body"], "hello");
        assert_eq!(echoed["headers"]["x-test"], "1");
        assert_eq!(response.headers["content-type"], "application/json");
    }

    #[tokio::test]
    async fn a_bad_url_and_a_closed_port_fail_with_a_message() {
        assert_eq!(exchange("GET", "nope", &[], None).await.err().as_deref(), Some("Invalid URL"));
        let error = exchange("GET", "http://127.0.0.1:1/", &[], None).await.err().unwrap();
        assert!(!error.is_empty());
    }
}
