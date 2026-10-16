//! Response history: the last few responses of a saved request, which the engine keeps on this device
//! (`response.history`). Not the git history of the request itself.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value, json};

use super::http::BodyEncoding;

#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEntry {
    /// When it arrived, in milliseconds since the epoch.
    pub at: i64,
    pub duration_ms: u64,
    /// The HTTP status, or the gRPC status code; 0 for a send that got no answer.
    pub status: i64,
    pub status_text: String,
    pub headers: Map<String, Value>,
    /// The body as text, or as base64 when `base64` is set.
    pub body: String,
    pub base64: bool,
    pub truncated: bool,
    pub error: Option<String>,
}

/// Reads the result of `response.history`, newest first. Entries that are not objects are skipped.
pub fn parse(result: &Value) -> Vec<HistoryEntry> {
    let text = |entry: &Value, key: &str| entry.get(key).and_then(Value::as_str).map(str::to_owned);
    result
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry.is_object())
        .map(|entry| HistoryEntry {
            at: entry.get("at").and_then(Value::as_i64).unwrap_or(0),
            duration_ms: entry.get("durationMs").and_then(Value::as_u64).unwrap_or(0),
            status: entry.get("status").and_then(Value::as_i64).unwrap_or(0),
            status_text: text(entry, "statusText").unwrap_or_default(),
            headers: entry.get("headers").and_then(Value::as_object).cloned().unwrap_or_default(),
            body: text(entry, "body").unwrap_or_default(),
            base64: entry.get("encoding").and_then(Value::as_str) == Some("base64"),
            truncated: entry.get("truncated").and_then(Value::as_bool).unwrap_or(false),
            error: text(entry, "error"),
        })
        .collect()
}

impl HistoryEntry {
    /// A send that got no answer: there is nothing but its error to show.
    pub fn is_unanswered(&self) -> bool {
        self.status == 0 && self.error.is_some()
    }

    /// The entry as the result of the send that made it, in the shape the response viewer reads.
    pub fn as_http_result(&self, encoding: BodyEncoding) -> Value {
        let body = match (encoding, self.base64) {
            (BodyEncoding::Base64, false) => STANDARD.encode(&self.body),
            (BodyEncoding::Text, true) => String::from_utf8_lossy(&STANDARD.decode(&self.body).unwrap_or_default()).into_owned(),
            _ => self.body.clone(),
        };
        json!({ "status": self.status, "headers": self.headers, "body": body, "durationMs": self.duration_ms })
    }

    /// The entry as the result of a gRPC call: its body holds the messages as a JSON array of strings.
    pub fn as_grpc_result(&self) -> Value {
        let responses = serde_json::from_str::<Value>(&self.body).ok().filter(Value::is_array).unwrap_or_else(|| json!([]));
        json!({
            "status": self.status,
            "statusMessage": self.status_text,
            "metadata": self.headers,
            "responses": responses,
            "durationMs": self.duration_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<HistoryEntry> {
        parse(&json!({ "entries": [
            { "at": 1000, "durationMs": 12, "status": 200, "headers": { "a": "b" }, "body": "hi", "size": 2,
              "truncated": false, "request": { "method": "GET", "url": "u" } },
            { "at": 900, "status": 0, "error": "refused", "body": "" },
            { "at": 800, "status": 200, "body": "AP8=", "encoding": "base64", "truncated": true },
            "junk",
        ] }))
    }

    #[test]
    fn entries_are_read_newest_first_and_junk_is_skipped() {
        let all = entries();
        assert_eq!(all.len(), 3);
        assert_eq!((all[0].at, all[0].duration_ms, all[0].status), (1000, 12, 200));
        assert_eq!(all[0].headers["a"], "b");
        assert!(all[1].is_unanswered() && !all[0].is_unanswered());
        assert!(all[2].base64 && all[2].truncated);
        assert!(parse(&json!({})).is_empty());
    }

    #[test]
    fn a_body_is_given_in_the_encoding_the_viewer_expects() {
        let all = entries();
        assert_eq!(all[0].as_http_result(BodyEncoding::Base64)["body"], "aGk=");
        assert_eq!(all[0].as_http_result(BodyEncoding::Text)["body"], "hi");
        assert_eq!(all[2].as_http_result(BodyEncoding::Base64)["body"], "AP8=");
        assert_eq!(all[0].as_http_result(BodyEncoding::Text)["status"], 200);
    }

    #[test]
    fn a_grpc_entry_gives_its_messages_back() {
        let entry = parse(&json!({ "entries": [
            { "status": 0, "statusText": "", "headers": { "m": "1" }, "body": "[\n  \"{}\"\n]", "durationMs": 4 },
        ] }))
        .remove(0);
        let result = entry.as_grpc_result();
        assert_eq!(result["responses"], json!(["{}"]));
        assert_eq!((result["metadata"]["m"].clone(), result["durationMs"].clone()), (json!("1"), json!(4)));
    }
}
