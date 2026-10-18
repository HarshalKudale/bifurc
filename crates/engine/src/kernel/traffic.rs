//! Captured traffic: what the proxies report for every exchange they handle.

use base64::Engine as _;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::bus::bus;

/// Header names to values, in the order they were set.
pub type Headers = IndexMap<String, String>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RequestLogEntry {
    pub id: String,
    pub ts: i64,
    pub method: String,
    pub url: String,
    pub host: String,
    pub req_headers: Headers,
    /// base64
    pub req_body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// `forward` | `reverse` | `server` (the local mock server)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The trace-id of the request's own `traceparent`, so log lines that carry it match this exchange.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// A webhook listener's forward URL when the exchange was received, so a replay still has it after the listener stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forward: Option<String>,
    pub status: Option<u16>,
    /// `rfc6761` | `mapping` | `proxy` | `rule` | `mock` | `error`
    pub via: String,
    pub target: Option<String>,
    pub duration_ms: Option<i64>,
    pub res_headers: Headers,
    /// base64, at most the first 512 KiB of the body
    pub res_body: String,
    pub res_status: Option<u16>,
}

/// How much of a response body a log entry keeps.
pub const MAX_LOGGED_BODY_BYTES: usize = 512 * 1024;

/// Report a finished exchange (bus event `log.entry`).
pub fn emit_log(entry: &RequestLogEntry) {
    bus().emit("log.entry", serde_json::to_value(entry).unwrap_or(Value::Null));
}

/// Report a piece of a streamed response body (bus event `log.chunk`).
pub fn emit_log_chunk(log_id: &str, chunk: &[u8], done: bool) {
    let chunk = base64::engine::general_purpose::STANDARD.encode(chunk);
    bus().emit("log.chunk", json!({ "logId": log_id, "chunk": chunk, "done": done }));
}

/// Report a listener failure (bus event `server.error`).
pub fn emit_server_error(message: &str) {
    bus().emit("server.error", Value::String(message.to_string()));
}

/// The trace-id of a W3C `traceparent` header (`00-<32 hex>-<16 hex>-<2 hex>`), if the request has a valid one.
pub fn traceparent_trace_id(headers: &Headers) -> Option<String> {
    let value = headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("traceparent"))?.1;
    let id = value.trim().split('-').nth(1)?;
    let valid = id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) && id.bytes().any(|b| b != b'0');
    valid.then(|| id.to_ascii_lowercase())
}

/// The headers to forward upstream: the client's own plus `<trace header>: <capture id>`. The log entry keeps
/// the request as the client sent it.
pub fn forwarded_headers(headers: &Headers, capture_id: &str) -> Headers {
    let mut forwarded = headers.clone();
    if let Some(name) = crate::capture::trace_header() {
        forwarded.retain(|key, _| !key.eq_ignore_ascii_case(&name));
        forwarded.insert(name, capture_id.to_string());
    }
    forwarded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Headers {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn traceparent_yields_its_trace_id() {
        let ok = headers(&[("traceparent", "00-4BF92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01")]);
        assert_eq!(traceparent_trace_id(&ok).as_deref(), Some("4bf92f3577b34da6a3ce929d0e0e4736"));
        assert_eq!(traceparent_trace_id(&headers(&[("traceparent", "garbage")])), None);
        let zero = headers(&[("traceparent", "00-00000000000000000000000000000000-00f067aa0ba902b7-01")]);
        assert_eq!(traceparent_trace_id(&zero), None);
    }

    #[test]
    fn forwarded_headers_follow_the_setting() {
        let _root = crate::testing::scratch_data_root();
        crate::capture::refresh_capture_limit();
        let sent = headers(&[("accept", "*/*")]);
        let forwarded = forwarded_headers(&sent, "abc");
        assert_eq!(forwarded.get("x-bifurc-trace").map(String::as_str), Some("abc"));
        assert!(!sent.contains_key("x-bifurc-trace"));

        let mut settings = crate::workspace::settings::load_settings();
        settings.set("traceHeader", serde_json::json!("X-Req-Id"));
        crate::workspace::settings::save_settings(&settings).unwrap();
        crate::capture::refresh_capture_limit();
        assert_eq!(forwarded_headers(&sent, "abc").get("x-req-id").map(String::as_str), Some("abc"));

        settings.set("traceHeader", serde_json::json!(""));
        crate::workspace::settings::save_settings(&settings).unwrap();
        crate::capture::refresh_capture_limit();
        assert_eq!(forwarded_headers(&sent, "abc"), sent);
    }
}
