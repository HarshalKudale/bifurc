//! Proxied traffic the engine has captured, and the live entries that extend it.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

/// The engine keeps the last 200 exchanges; the view mirrors the bound so a long session stays bounded here
/// too.
pub const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, PartialEq)]
pub struct TrafficEntry {
    pub id: String,
    pub ts: u64,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
    /// How the proxy handled it: `rfc6761`, `mapping`, `proxy`, `rule`, `mock` or `error`.
    pub via: String,
    /// The entry as the engine sent it. The detail pane reads headers and bodies from it, and the engine's
    /// own commands (`capture.buildRequest`, `capture.buildMock`) take it back whole.
    pub raw: Value,
}

/// A captured body: text when it is text, otherwise only its size.
#[derive(Debug, Clone, PartialEq)]
pub enum CapturedBody {
    Empty,
    Text(String),
    Binary { bytes: usize },
}

impl TrafficEntry {
    /// Reads one entry of `capture.list` (which names the time `ts`) or of `event.log.entry` (`timestamp`).
    pub fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            ts: value.get("ts").or_else(|| value.get("timestamp")).and_then(Value::as_u64).unwrap_or(0),
            method: value.get("method").and_then(Value::as_str).unwrap_or_default().to_owned(),
            url: value.get("url").and_then(Value::as_str).unwrap_or_default().to_owned(),
            status: value.get("status").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()),
            duration_ms: value.get("durationMs").and_then(Value::as_u64),
            via: value.get("via").and_then(Value::as_str).unwrap_or_default().to_owned(),
            raw: value.clone(),
        })
    }

    /// A word for how the proxy handled the exchange.
    pub fn via_label(&self) -> &'static str {
        match self.via.as_str() {
            "rfc6761" => "Localhost",
            "mapping" => "Mapping",
            "proxy" => "Proxy",
            "rule" => "Rule",
            "mock" => "Mock",
            "error" => "Error",
            _ => "",
        }
    }

    /// The request's (`reqHeaders`) or the response's (`resHeaders`) headers, in the order received.
    pub fn headers(&self, key: &str) -> Vec<(String, String)> {
        self.raw
            .get(key)
            .and_then(Value::as_object)
            .map(|map| {
                map.iter().map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned())).collect()
            })
            .unwrap_or_default()
    }

    /// The request's (`reqBody`) or the response's (`resBody`) body. The engine sends both base64-encoded.
    pub fn body(&self, key: &str) -> CapturedBody {
        let encoded = self.raw.get(key).and_then(Value::as_str).unwrap_or_default();
        if encoded.is_empty() {
            return CapturedBody::Empty;
        }
        match STANDARD.decode(encoded.trim()) {
            Ok(bytes) if bytes.is_empty() => CapturedBody::Empty,
            Ok(bytes) => {
                let size = bytes.len();
                String::from_utf8(bytes).map(CapturedBody::Text).unwrap_or(CapturedBody::Binary { bytes: size })
            }
            // Not base64 after all: show it as it arrived rather than nothing.
            Err(_) => CapturedBody::Text(encoded.to_owned()),
        }
    }

    pub fn matches(&self, needle: &str) -> bool {
        needle.is_empty()
            || self.url.to_lowercase().contains(needle)
            || self.method.to_lowercase().contains(needle)
            || self.status.is_some_and(|status| status.to_string().contains(needle))
            || self.via_label().to_lowercase().contains(needle)
    }
}

/// Entries of a `capture.list` result, newest first as the engine returns them.
pub fn parse_list(result: &Value) -> Vec<TrafficEntry> {
    result
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(TrafficEntry::from_value).collect())
        .unwrap_or_default()
}

/// Entries of an `event.log.entry` payload: a batch (`{entries: [...]}`), or a single entry.
pub fn parse_event(payload: &Value) -> Vec<TrafficEntry> {
    match payload.get("entries").and_then(Value::as_array) {
        Some(entries) => entries.iter().filter_map(TrafficEntry::from_value).collect(),
        None => TrafficEntry::from_value(payload).into_iter().collect(),
    }
}

/// Puts `incoming` (oldest first, as batches arrive) at the front of `entries`, newest first. An entry the
/// list already holds replaces its old row, which is how a request's status arrives after it was logged.
pub fn merge_live(entries: &mut Vec<TrafficEntry>, incoming: Vec<TrafficEntry>) {
    for entry in incoming {
        if let Some(position) = entries.iter().position(|e| e.id == entry.id) {
            entries[position] = entry;
        } else {
            entries.insert(0, entry);
        }
    }
    entries.truncate(MAX_ENTRIES);
}

/// `HH:MM:SS` of an epoch-millisecond timestamp, in UTC. The standard library has no time zone, so the
/// views that show this say UTC rather than guessing a zone.
pub fn clock_utc(epoch_ms: u64) -> String {
    let seconds = epoch_ms / 1000 % 86_400;
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, status: Option<u16>) -> TrafficEntry {
        TrafficEntry::from_value(&json!({ "id": id, "ts": 0, "method": "GET", "url": "http://x/u", "status": status }))
            .unwrap()
    }

    #[test]
    fn a_list_and_a_live_batch_read_to_the_same_row() {
        let from_list = parse_list(&json!({"entries": [
            {"id": "a", "ts": 5, "method": "GET", "url": "http://x/", "status": 200, "durationMs": 3},
        ]}));
        let from_event = parse_event(&json!({"entries": [
            {"id": "a", "timestamp": 5, "method": "GET", "url": "http://x/", "status": 200, "durationMs": 3},
        ]}));
        let row = |e: &TrafficEntry| (e.id.clone(), e.ts, e.method.clone(), e.url.clone(), e.status, e.duration_ms);
        assert_eq!(row(&from_list[0]), row(&from_event[0]));
    }

    #[test]
    fn a_pending_request_has_no_status_yet() {
        assert_eq!(entry("a", None).status, None);
    }

    #[test]
    fn live_entries_go_to_the_front_and_an_update_replaces_its_row() {
        let mut entries = vec![entry("old", Some(200))];
        merge_live(&mut entries, vec![entry("new", None)]);
        merge_live(&mut entries, vec![entry("new", Some(404))]);
        assert_eq!(entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["new", "old"]);
        assert_eq!(entries[0].status, Some(404));
    }

    #[test]
    fn the_history_is_bounded() {
        let mut entries = Vec::new();
        let batch = (0..MAX_ENTRIES + 10).map(|n| entry(&n.to_string(), None)).collect();
        merge_live(&mut entries, batch);
        assert_eq!(entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn bodies_are_decoded_and_binary_ones_are_only_measured() {
        let e = TrafficEntry::from_value(&json!({
            "id": "a",
            "reqBody": STANDARD.encode("{\"a\":1}"),
            "resBody": STANDARD.encode([0xffu8, 0xfe, 0x00]),
            "reqHeaders": { "Accept": "*/*" },
        }))
        .unwrap();
        assert_eq!(e.body("reqBody"), CapturedBody::Text("{\"a\":1}".into()));
        assert_eq!(e.body("resBody"), CapturedBody::Binary { bytes: 3 });
        assert_eq!(e.body("missing"), CapturedBody::Empty);
        assert_eq!(e.headers("reqHeaders"), [("Accept".to_owned(), "*/*".to_owned())]);
    }

    #[test]
    fn a_filter_matches_the_url_the_method_the_status_and_the_route() {
        let e = TrafficEntry::from_value(
            &json!({ "id": "a", "method": "POST", "url": "http://api.localhost/users", "status": 201, "via": "mock" }),
        )
        .unwrap();
        assert!(e.matches("users") && e.matches("post") && e.matches("201") && e.matches("mock"));
        assert!(!e.matches("orders"));
        assert!(e.matches(""));
    }

    #[test]
    fn the_clock_reads_hours_minutes_seconds_of_the_day() {
        assert_eq!(clock_utc(86_400_000 + (3 * 3600 + 4 * 60 + 5) * 1000 + 678), "03:04:05");
        assert_eq!(clock_utc(0), "00:00:00");
    }
}
