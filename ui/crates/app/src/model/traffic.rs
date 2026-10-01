//! Proxied traffic the engine has captured, and the live entries that extend it.

use serde_json::Value;

/// The engine keeps a bounded history; the view mirrors the bound so a long session stays bounded here too.
pub const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, PartialEq)]
pub struct TrafficEntry {
    pub id: String,
    pub ts: u64,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub duration_ms: Option<u64>,
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
        })
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

    #[test]
    fn a_list_and_a_live_batch_read_to_the_same_entry() {
        let from_list = parse_list(&json!({"entries": [
            {"id": "a", "ts": 5, "method": "GET", "url": "http://x/", "status": 200, "durationMs": 3},
        ]}));
        let from_event = parse_event(&json!({"entries": [
            {"id": "a", "timestamp": 5, "method": "GET", "url": "http://x/", "status": 200, "durationMs": 3},
        ]}));
        assert_eq!(from_list, from_event);
    }

    #[test]
    fn a_pending_request_has_no_status_yet() {
        let e = TrafficEntry::from_value(&json!({"id": "a", "ts": 1, "method": "GET", "url": "u", "status": null}))
            .unwrap();
        assert_eq!(e.status, None);
    }

    #[test]
    fn live_entries_go_to_the_front_and_an_update_replaces_its_row() {
        let entry = |id: &str, status: Option<u16>| TrafficEntry {
            id: id.into(),
            ts: 0,
            method: "GET".into(),
            url: "u".into(),
            status,
            duration_ms: None,
        };
        let mut entries = vec![entry("old", Some(200))];
        merge_live(&mut entries, vec![entry("new", None)]);
        merge_live(&mut entries, vec![entry("new", Some(404))]);
        assert_eq!(entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["new", "old"]);
        assert_eq!(entries[0].status, Some(404));
    }

    #[test]
    fn the_history_is_bounded() {
        let mut entries = Vec::new();
        let batch = (0..MAX_ENTRIES + 10)
            .map(|n| TrafficEntry {
                id: n.to_string(),
                ts: 0,
                method: String::new(),
                url: String::new(),
                status: None,
                duration_ms: None,
            })
            .collect();
        merge_live(&mut entries, batch);
        assert_eq!(entries.len(), MAX_ENTRIES);
    }

    #[test]
    fn the_clock_reads_hours_minutes_seconds_of_the_day() {
        assert_eq!(clock_utc(86_400_000 + (3 * 3600 + 4 * 60 + 5) * 1000 + 678), "03:04:05");
        assert_eq!(clock_utc(0), "00:00:00");
    }
}
