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
    pub status: Option<u16>,
    /// `rfc6761` | `mapping` | `proxy` | `rule` | `mock` | `error`
    pub via: String,
    pub target: Option<String>,
    pub duration_ms: Option<i64>,
    pub req_headers: Headers,
    /// base64
    pub req_body: String,
    pub res_headers: Headers,
    /// base64, at most the first 512 KiB of the body
    pub res_body: String,
    pub res_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// `forward` | `reverse`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
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
