//! Proxied traffic the engine has captured, and the live entries that extend it.
//!
//! Two shapes: the light `TrafficEntry` the list shows (it arrives with `capture.list` and live), and the
//! full `Exchange` with headers and bodies that `capture.get` returns for one id.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

use crate::model::{
    http::split_query,
    media::{self, BodyClass},
};

/// The engine keeps the last 200 exchanges; the view mirrors the bound so a long session stays bounded here
/// too.
pub const MAX_ENTRIES: usize = 200;

/// The engine keeps this many bytes of a response body; a body of exactly this size was probably cut.
pub const RESPONSE_BODY_LIMIT: usize = 512 * 1024;

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
    /// The workspace the exchange belongs to, when the engine says.
    pub workspace_id: Option<String>,
    /// The response's `Content-Type`; empty when there is none yet.
    pub content_type: String,
    /// The response body's size in bytes.
    pub size: Option<u64>,
    /// `forward` or `reverse`: the listener that took the request. Entries recorded before the engine said so have none.
    pub origin: Option<String>,
}

/// What a response is, for the type filter of the list: the groups of a browser's Network tab, told apart by
/// the response's content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceType {
    Fetch,
    Doc,
    Css,
    Js,
    Font,
    Img,
    Media,
    Other,
}

impl ResourceType {
    pub const ALL: [Self; 8] =
        [Self::Fetch, Self::Doc, Self::Css, Self::Js, Self::Font, Self::Img, Self::Media, Self::Other];

    pub fn label(self) -> &'static str {
        match self {
            Self::Fetch => "Fetch/XHR",
            Self::Doc => "Doc",
            Self::Css => "CSS",
            Self::Js => "JS",
            Self::Font => "Font",
            Self::Img => "Img",
            Self::Media => "Media",
            Self::Other => "Other",
        }
    }

    pub fn of(content_type: &str) -> Self {
        let kind = media::essence(content_type);
        if kind == "text/html" || kind == "application/xhtml+xml" {
            Self::Doc
        } else if kind == "text/css" {
            Self::Css
        } else if kind.contains("javascript") || kind.contains("ecmascript") {
            Self::Js
        } else if kind.starts_with("font/") || kind.contains("font-") || kind.ends_with("fontobject") {
            Self::Font
        } else if kind.starts_with("image/") {
            Self::Img
        } else if kind.starts_with("audio/") || kind.starts_with("video/") || kind.contains("mpegurl") {
            Self::Media
        } else if kind.contains("json")
            || kind.contains("xml")
            || kind == "text/plain"
            || kind == "application/x-www-form-urlencoded"
            || kind == "application/graphql"
            || kind == "application/x-ndjson"
            || kind == "text/event-stream"
        {
            Self::Fetch
        } else {
            Self::Other
        }
    }
}

impl TrafficEntry {
    /// Served by the reverse proxy (a mapped domain) rather than the forward proxy.
    pub fn is_local(&self) -> bool {
        match self.origin.as_deref() {
            Some(origin) => origin == "reverse",
            None => matches!(self.via.as_str(), "rfc6761" | "mapping"),
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        ResourceType::of(&self.content_type)
    }

    /// Reads one entry of `capture.list` (older engines name the time `ts`) or of `event.log.entry`
    /// (`timestamp`).
    pub fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            id: value.get("id")?.as_str()?.to_owned(),
            ts: value.get("timestamp").or_else(|| value.get("ts")).and_then(Value::as_u64).unwrap_or(0),
            method: text("method").unwrap_or_default(),
            url: text("url").unwrap_or_default(),
            status: value.get("status").and_then(Value::as_u64).and_then(|s| u16::try_from(s).ok()),
            duration_ms: value.get("durationMs").and_then(Value::as_u64),
            via: text("via").unwrap_or_default(),
            workspace_id: text("workspaceId"),
            content_type: text("contentType").unwrap_or_default(),
            size: value.get("size").and_then(Value::as_u64),
            origin: text("origin"),
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

    /// What the exchange is called in the list: see [`url_name`].
    pub fn name(&self) -> String {
        url_name(&self.url)
    }

    /// The server the exchange went to: see [`url_host`].
    pub fn host(&self) -> String {
        url_host(&self.url)
    }

    pub fn type_label(&self) -> String {
        media::short_label(&self.content_type)
    }

    pub fn matches(&self, needle: &str) -> bool {
        needle.is_empty()
            || self.url.to_lowercase().contains(needle)
            || self.method.to_lowercase().contains(needle)
            || self.status.is_some_and(|status| status.to_string().contains(needle))
            || self.via_label().to_lowercase().contains(needle)
            || self.type_label().contains(needle)
    }
}

/// The part of a URL after the scheme, without a query or fragment: `host[:port][/path]`.
fn without_scheme_and_query(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['?', '#']).next().unwrap_or_default()
}

/// The host (and port) of a URL, without any credentials. Empty when the text has none.
pub fn url_host(url: &str) -> String {
    let rest = without_scheme_and_query(url);
    let authority = rest.split('/').next().unwrap_or_default();
    authority.rsplit('@').next().unwrap_or_default().to_owned()
}

/// What a URL is called in a list: the last segment of its path, or its host when the path is only `/`.
/// The query string is dropped.
pub fn url_name(url: &str) -> String {
    let rest = without_scheme_and_query(url);
    let path = rest.split_once('/').map_or("", |(_, path)| path);
    match path.rsplit('/').find(|segment| !segment.is_empty()) {
        Some(segment) => segment.to_owned(),
        None => url_host(url),
    }
}

/// The class a body is shown as, or `None` when there is no body. Bytes that are not valid UTF-8 are never
/// text, whatever the content type claims.
pub fn body_class(content_type: &str, bytes: &[u8]) -> Option<BodyClass> {
    if bytes.is_empty() {
        return None;
    }
    match media::classify(content_type) {
        BodyClass::Text if std::str::from_utf8(bytes).is_err() => Some(BodyClass::Binary),
        class => Some(class),
    }
}

/// A body as the engine sends it (base64). Text that is not base64 after all is kept as it arrived.
pub fn decode_body(encoded: &str) -> Vec<u8> {
    let trimmed = encoded.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    STANDARD.decode(trimmed).unwrap_or_else(|_| encoded.as_bytes().to_vec())
}

fn read_headers(value: Option<&Value>) -> Vec<(String, String)> {
    value
        .and_then(Value::as_object)
        .map(|map| {
            map.iter().map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned())).collect()
        })
        .unwrap_or_default()
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> &'a str {
    headers.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map_or("", |(_, value)| value.as_str())
}

/// One exchange in full, as `capture.get` returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct Exchange {
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub via: String,
    pub target: Option<String>,
    pub duration_ms: Option<u64>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub request_body: Vec<u8>,
    pub response_body: Vec<u8>,
}

impl Exchange {
    /// Reads the `entry` of a `capture.get` answer. `None` when it is not an object.
    pub fn from_value(value: &Value) -> Option<Self> {
        value.as_object()?;
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let body = |key: &str| decode_body(value.get(key).and_then(Value::as_str).unwrap_or_default());
        Some(Self {
            method: text("method").unwrap_or_default(),
            url: text("url").unwrap_or_default(),
            status: value
                .get("status")
                .and_then(Value::as_u64)
                .or_else(|| value.get("resStatus").and_then(Value::as_u64))
                .and_then(|s| u16::try_from(s).ok()),
            via: text("via").unwrap_or_default(),
            target: text("target").filter(|target| !target.is_empty()),
            duration_ms: value.get("durationMs").and_then(Value::as_u64),
            request_headers: read_headers(value.get("reqHeaders")),
            response_headers: read_headers(value.get("resHeaders")),
            request_body: body("reqBody"),
            response_body: body("resBody"),
        })
    }

    /// The URL's query as name/value pairs, as written (not decoded).
    pub fn query(&self) -> Vec<(String, String)> {
        split_query(&self.url).1
    }

    pub fn request_content_type(&self) -> &str {
        find_header(&self.request_headers, "content-type")
    }

    pub fn response_content_type(&self) -> &str {
        find_header(&self.response_headers, "content-type")
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

/// Drops the entries that belong to another workspace than `active`. An entry that names none is kept.
pub fn only_workspace(entries: Vec<TrafficEntry>, active: &str) -> Vec<TrafficEntry> {
    entries.into_iter().filter(|entry| entry.workspace_id.as_deref().is_none_or(|id| id == active)).collect()
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

/// The current time in epoch milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0))
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
        assert_eq!(from_list, from_event);
    }

    #[test]
    fn the_light_entry_carries_type_size_and_workspace() {
        let e = TrafficEntry::from_value(&json!({
            "id": "a", "timestamp": 1, "workspaceId": "w1", "contentType": "application/json; charset=utf-8",
            "size": 42, "via": "proxy",
        }))
        .unwrap();
        assert_eq!(e.workspace_id.as_deref(), Some("w1"));
        assert_eq!(e.type_label(), "json");
        assert_eq!(e.size, Some(42));
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
    fn entries_of_another_workspace_are_dropped() {
        let named =
            |id: &str, ws: Option<&str>| TrafficEntry::from_value(&json!({ "id": id, "workspaceId": ws })).unwrap();
        let kept = only_workspace(vec![named("a", Some("w1")), named("b", Some("w2")), named("c", None)], "w1");
        assert_eq!(kept.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["a", "c"]);
    }

    #[test]
    fn a_name_is_the_last_path_segment_or_the_host() {
        assert_eq!(url_name("http://api.localhost:3000/v1/users?page=2"), "users");
        assert_eq!(url_name("https://x.test/files/report.pdf#top"), "report.pdf");
        assert_eq!(url_name("https://x.test/users/"), "users");
        assert_eq!(url_name("https://x.test/"), "x.test");
        assert_eq!(url_name("https://x.test?a=1"), "x.test");
        assert_eq!(url_name("/relative/path"), "path");
        assert_eq!(url_name(""), "");
    }

    #[test]
    fn a_host_has_no_scheme_path_or_credentials() {
        assert_eq!(url_host("http://api.localhost:3000/v1/users?x=1"), "api.localhost:3000");
        assert_eq!(url_host("https://user:pw@x.test/a"), "x.test");
        assert_eq!(url_host("x.test/a"), "x.test");
        assert_eq!(url_host("https://x.test?q=/a"), "x.test");
    }

    #[test]
    fn an_exchange_reads_headers_bodies_and_status() {
        let e = Exchange::from_value(&json!({
            "id": "a", "method": "POST", "url": "http://x/u?a=1&b=", "status": null, "resStatus": 201,
            "reqBody": STANDARD.encode("{\"a\":1}"),
            "resBody": STANDARD.encode([0xffu8, 0xfe, 0x00]),
            "reqHeaders": { "Content-Type": "application/json", "Accept": "*/*" },
            "resHeaders": { "content-type": "image/png" },
            "target": "",
        }))
        .unwrap();
        assert_eq!(e.status, Some(201));
        assert_eq!(e.request_body, b"{\"a\":1}");
        assert_eq!(e.response_body, [0xff, 0xfe, 0x00]);
        assert_eq!(e.request_content_type(), "application/json");
        assert_eq!(e.response_content_type(), "image/png");
        assert_eq!(e.target, None);
        assert_eq!(e.query(), [("a".to_owned(), "1".to_owned()), ("b".to_owned(), String::new())]);
        assert_eq!(e.request_headers.len(), 2);
        assert!(Exchange::from_value(&Value::Null).is_none());
    }

    #[test]
    fn a_body_that_is_not_base64_is_kept_as_it_arrived() {
        assert_eq!(decode_body("not base64!"), b"not base64!");
        assert_eq!(decode_body(""), Vec::<u8>::new());
    }

    #[test]
    fn bytes_that_are_not_utf8_are_never_text() {
        assert_eq!(body_class("application/json", b"{}"), Some(BodyClass::Text));
        assert_eq!(body_class("application/json", &[0xff, 0xfe]), Some(BodyClass::Binary));
        assert_eq!(body_class("image/png", &[0x89, b'P']), Some(BodyClass::Image));
        assert_eq!(body_class("text/plain", b""), None);
    }

    #[test]
    fn a_filter_matches_the_url_the_method_the_status_the_route_and_the_type() {
        let e = TrafficEntry::from_value(&json!({
            "id": "a", "method": "POST", "url": "http://api.localhost/users", "status": 201, "via": "mock",
            "contentType": "application/json",
        }))
        .unwrap();
        assert!(e.matches("users") && e.matches("post") && e.matches("201") && e.matches("mock") && e.matches("json"));
        assert!(!e.matches("orders"));
        assert!(e.matches(""));
    }

    #[test]
    fn a_response_is_grouped_by_its_content_type() {
        let of = ResourceType::of;
        assert_eq!(of("application/json; charset=utf-8"), ResourceType::Fetch);
        assert_eq!(of("text/html"), ResourceType::Doc);
        assert_eq!(of("text/css"), ResourceType::Css);
        assert_eq!(of("application/javascript"), ResourceType::Js);
        assert_eq!(of("font/woff2"), ResourceType::Font);
        assert_eq!(of("application/x-font-ttf"), ResourceType::Font);
        assert_eq!(of("image/svg+xml"), ResourceType::Img);
        assert_eq!(of("video/mp4"), ResourceType::Media);
        assert_eq!(of("application/pdf"), ResourceType::Other);
        assert_eq!(of(""), ResourceType::Other);
    }

    #[test]
    fn local_traffic_is_what_the_reverse_proxy_served() {
        let local = |value: Value| TrafficEntry::from_value(&value).unwrap().is_local();
        assert!(local(json!({ "id": "a", "origin": "reverse", "via": "mock" })));
        assert!(!local(json!({ "id": "a", "origin": "forward", "via": "mock" })));
        assert!(local(json!({ "id": "a", "via": "mapping" })));
        assert!(!local(json!({ "id": "a", "via": "proxy" })));
    }

    #[test]
    fn the_clock_reads_hours_minutes_seconds_of_the_day() {
        assert_eq!(clock_utc(86_400_000 + (3 * 3600 + 4 * 60 + 5) * 1000 + 678), "03:04:05");
        assert_eq!(clock_utc(0), "00:00:00");
    }
}
