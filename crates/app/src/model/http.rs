//! Editable request fields and the response that comes back, as the UI holds them.
//!
//! Headers are edited as text, one `Name: value` per line, and stored by the engine as a map. These
//! functions are the only translation between the two.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value};

use crate::i18n::{t, t_with};

pub const HTTP_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// The engine's header map as ordered pairs, for a name/value table.
pub fn header_pairs(headers: &Value) -> Vec<(String, String)> {
    headers
        .as_object()
        .map(|map| {
            map.iter().map(|(name, value)| (name.clone(), value.as_str().unwrap_or_default().to_owned())).collect()
        })
        .unwrap_or_default()
}

/// Pairs back to the engine's header map. A later pair with the same name replaces the earlier one, as a
/// map requires.
pub fn header_map(pairs: &[(String, String)]) -> Map<String, Value> {
    pairs.iter().map(|(name, value)| (name.clone(), Value::String(value.clone()))).collect()
}

/// Splits a URL into everything but its query, and the query's pairs. The text is not decoded: what the
/// user typed (including `{{variables}}`) is what the table shows.
pub fn split_query(url: &str) -> (String, Vec<(String, String)>) {
    let (before_fragment, fragment) = match url.find('#') {
        Some(ix) => (&url[..ix], &url[ix..]),
        None => (url, ""),
    };
    let Some(ix) = before_fragment.find('?') else { return (url.to_owned(), Vec::new()) };
    let pairs = before_fragment[ix + 1..]
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| match part.split_once('=') {
            Some((name, value)) => (name.to_owned(), value.to_owned()),
            None => (part.to_owned(), String::new()),
        })
        .collect();
    (format!("{}{fragment}", &before_fragment[..ix]), pairs)
}

/// The inverse of [`split_query`]: `base` with `pairs` as its query.
pub fn join_query(base: &str, pairs: &[(String, String)]) -> String {
    let (path, fragment) = match base.find('#') {
        Some(ix) => (&base[..ix], &base[ix..]),
        None => (base, ""),
    };
    if pairs.is_empty() {
        return base.to_owned();
    }
    let query: Vec<String> = pairs
        .iter()
        .map(|(name, value)| if value.is_empty() { name.clone() } else { format!("{name}={value}") })
        .collect();
    format!("{path}?{}{fragment}", query.join("&"))
}

fn form_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            ' ' => out.push('+'),
            '%' => out.push_str("%25"),
            '&' => out.push_str("%26"),
            '=' => out.push_str("%3D"),
            '+' => out.push_str("%2B"),
            '#' => out.push_str("%23"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            other => out.push(other),
        }
    }
    out
}

fn form_unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut ix = 0;
    while ix < bytes.len() {
        match bytes[ix] {
            b'+' => out.push(b' '),
            b'%' if bytes.get(ix + 1..ix + 3).is_some_and(|hex| hex.iter().all(u8::is_ascii_hexdigit)) => {
                let hex = std::str::from_utf8(&bytes[ix + 1..ix + 3]).unwrap_or("00");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                ix += 2;
            }
            other => out.push(other),
        }
        ix += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Name/value pairs as an `application/x-www-form-urlencoded` body. Only the characters that would change
/// the body's structure are escaped, so `{{variables}}` stay readable for the engine to substitute.
pub fn encode_form(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", form_escape(name), form_escape(value)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn decode_form(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            (form_unescape(name), form_unescape(value))
        })
        .collect()
}

/// How a request body is edited. The saved request stores only the body text and its headers, so the mode
/// is read from, and written to, the `Content-Type` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMode {
    None,
    Json,
    Text,
    Xml,
    Html,
    Form,
    Multipart,
    File,
}

impl BodyMode {
    pub const ALL: [BodyMode; 8] = [
        BodyMode::None,
        BodyMode::Json,
        BodyMode::Text,
        BodyMode::Xml,
        BodyMode::Html,
        BodyMode::Form,
        BodyMode::Multipart,
        BodyMode::File,
    ];

    pub fn label(self) -> &'static str {
        t(match self {
            BodyMode::None => "model.http.body_mode.none",
            BodyMode::Json => "model.http.body_mode.json",
            BodyMode::Text => "model.http.body_mode.text",
            BodyMode::Xml => "model.http.body_mode.xml",
            BodyMode::Html => "model.http.body_mode.html",
            BodyMode::Form => "model.http.body_mode.form",
            BodyMode::Multipart => "model.http.body_mode.multipart",
            BodyMode::File => "model.http.body_mode.file",
        })
    }

    /// The `Content-Type` this mode sends, or `None` for a request without a body.
    pub fn content_type(self) -> Option<&'static str> {
        match self {
            BodyMode::None => None,
            BodyMode::Json => Some("application/json"),
            BodyMode::Text => Some("text/plain"),
            BodyMode::Xml => Some("application/xml"),
            BodyMode::Html => Some("text/html"),
            BodyMode::Form => Some("application/x-www-form-urlencoded"),
            BodyMode::Multipart => Some("multipart/form-data"),
            // Set from the chosen file, by the session.
            BodyMode::File => None,
        }
    }

    /// The mode a stored request opens in.
    pub fn detect(content_type: Option<&str>, body: &str) -> Self {
        let Some(content_type) = content_type.map(str::to_ascii_lowercase) else {
            let trimmed = body.trim_start();
            return if trimmed.is_empty() {
                BodyMode::None
            } else if trimmed.starts_with(['{', '[']) {
                BodyMode::Json
            } else if trimmed.starts_with('<') {
                BodyMode::Xml
            } else {
                BodyMode::Text
            };
        };
        if content_type.contains("json") {
            BodyMode::Json
        } else if content_type.contains("x-www-form-urlencoded") {
            BodyMode::Form
        } else if content_type.contains("html") {
            BodyMode::Html
        } else if content_type.contains("xml") {
            BodyMode::Xml
        } else {
            BodyMode::Text
        }
    }
}

/// Re-indents JSON text. `None` if it is not JSON, which leaves the caller's text alone.
pub fn pretty_json(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    serde_json::to_string_pretty(&value).ok()
}

/// What one request overrides of the app-wide request defaults. `None` falls back to the default.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RequestSettings {
    pub verify_tls: Option<bool>,
    pub follow_redirects: Option<bool>,
    pub timeout_ms: Option<u64>,
}

impl RequestSettings {
    pub fn from_entity(entity: &Map<String, Value>) -> Self {
        let settings = entity.get("settings");
        let flag = |key| settings.and_then(|s| s.get(key)).and_then(Value::as_bool);
        Self {
            verify_tls: flag("verifyTls"),
            follow_redirects: flag("followRedirects"),
            timeout_ms: settings.and_then(|s| s.get("timeoutMs")).and_then(Value::as_u64),
        }
    }

    /// Only the overridden keys; `None` when nothing is, so an untouched request is stored as it was.
    pub fn to_json(&self) -> Option<Value> {
        let mut settings = Map::new();
        if let Some(on) = self.verify_tls {
            settings.insert("verifyTls".into(), on.into());
        }
        if let Some(on) = self.follow_redirects {
            settings.insert("followRedirects".into(), on.into());
        }
        if let Some(ms) = self.timeout_ms {
            settings.insert("timeoutMs".into(), ms.into());
        }
        (!settings.is_empty()).then(|| Value::Object(settings))
    }

    /// Writes `settings` into the entity, or removes it when nothing is overridden.
    pub fn write_to(&self, entity: &mut Map<String, Value>) {
        match self.to_json() {
            Some(settings) => entity.insert("settings".into(), settings),
            None => entity.remove("settings"),
        };
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestOutcome {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

/// How a send command returns its response body. `request.send` returns the bytes the upstream sent,
/// base64-encoded, so that binary bodies survive JSON; the other protocols return text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyEncoding {
    Base64,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// The body as text. For a body that is not valid UTF-8 this is empty and `is_binary` is set.
    pub body: String,
    pub is_binary: bool,
    /// The body's size in bytes, as received.
    pub size_bytes: usize,
    pub duration_ms: u64,
    pub tests: Vec<TestOutcome>,
    pub script_errors: Vec<String>,
}

impl HttpResponse {
    /// Reads the result of `request.send` (and the shape its siblings share).
    pub fn from_value(result: &Value, encoding: BodyEncoding) -> Option<Self> {
        let status = u16::try_from(result.get("status")?.as_u64()?).ok()?;
        let headers = result
            .get("headers")
            .and_then(Value::as_object)
            .map(|map| map.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_owned())).collect())
            .unwrap_or_default();
        let tests = result
            .get("tests")
            .and_then(Value::as_array)
            .map(|tests| {
                tests
                    .iter()
                    .map(|t| TestOutcome {
                        name: t.get("name").and_then(Value::as_str).unwrap_or_default().to_owned(),
                        passed: t.get("passed").and_then(Value::as_bool).unwrap_or(false),
                        error: t.get("error").and_then(Value::as_str).map(str::to_owned),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let script_errors = ["preScriptError", "postScriptError"]
            .into_iter()
            .filter_map(|key| result.get(key).and_then(Value::as_str).map(str::to_owned))
            .collect();
        let raw = result.get("body").and_then(Value::as_str).unwrap_or_default();
        let bytes = match encoding {
            BodyEncoding::Text => raw.as_bytes().to_vec(),
            BodyEncoding::Base64 => STANDARD.decode(raw.trim()).unwrap_or_else(|_| raw.as_bytes().to_vec()),
        };
        let size_bytes = bytes.len();
        let (body, is_binary) = match String::from_utf8(bytes) {
            Ok(text) => (text, false),
            Err(_) => (String::new(), true),
        };
        Some(Self {
            status,
            headers,
            body,
            is_binary,
            size_bytes,
            duration_ms: result.get("durationMs").and_then(Value::as_u64).unwrap_or(0),
            tests,
            script_errors,
        })
    }

    pub fn is_success(&self) -> bool {
        (200..400).contains(&self.status)
    }

    pub fn content_type(&self) -> Option<&str> {
        self.headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-type")).map(|(_, value)| value.as_str())
    }

    /// The body exactly as it was received (empty for a body that is not text).
    pub fn display_body_raw(&self) -> String {
        self.body.clone()
    }

    pub fn body_size_label(&self) -> String {
        let bytes = self.size_bytes;
        if bytes < 1024 {
            t_with("model.size.bytes", &[("n", &bytes)])
        } else if bytes < 1024 * 1024 {
            t_with("model.size.kilobytes", &[("n", &format!("{:.1}", bytes as f64 / 1024.0))])
        } else {
            t_with("model.size.megabytes", &[("n", &format!("{:.1}", bytes as f64 / (1024.0 * 1024.0)))])
        }
    }
}

/// A short reason phrase for the status line. Only the common ones; the number is the authority.
pub fn status_text(status: u16) -> &'static str {
    match status {
        200 => t("model.http.status.200"),
        201 => t("model.http.status.201"),
        202 => t("model.http.status.202"),
        204 => t("model.http.status.204"),
        301 => t("model.http.status.301"),
        302 => t("model.http.status.302"),
        304 => t("model.http.status.304"),
        400 => t("model.http.status.400"),
        401 => t("model.http.status.401"),
        403 => t("model.http.status.403"),
        404 => t("model.http.status.404"),
        405 => t("model.http.status.405"),
        409 => t("model.http.status.409"),
        422 => t("model.http.status.422"),
        429 => t("model.http.status.429"),
        500 => t("model.http.status.500"),
        502 => t("model.http.status.502"),
        503 => t("model.http.status.503"),
        504 => t("model.http.status.504"),
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_settings_write_only_the_overridden_keys_and_none_when_empty() {
        let mut entity = json!({ "url": "u" }).as_object().unwrap().clone();
        RequestSettings::default().write_to(&mut entity);
        assert_eq!(entity, json!({ "url": "u" }).as_object().unwrap().clone());
        let settings = RequestSettings { verify_tls: Some(false), timeout_ms: Some(0), ..Default::default() };
        settings.write_to(&mut entity);
        assert_eq!(entity["settings"], json!({ "verifyTls": false, "timeoutMs": 0 }));
        assert_eq!(RequestSettings::from_entity(&entity), settings);
        RequestSettings::default().write_to(&mut entity);
        assert!(!entity.contains_key("settings"));
    }

    fn encoded(text: &str) -> String {
        STANDARD.encode(text)
    }

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    #[test]
    fn a_query_splits_into_pairs_and_joins_back_unchanged() {
        let url = "{{HOST}}/users?active=1&q={{term}}&flag#top";
        let (base, query) = split_query(url);
        assert_eq!(base, "{{HOST}}/users#top");
        assert_eq!(query, pairs(&[("active", "1"), ("q", "{{term}}"), ("flag", "")]));
        assert_eq!(join_query(&base, &query), url);
    }

    #[test]
    fn a_url_without_a_query_is_only_a_base() {
        assert_eq!(split_query("http://x/y"), ("http://x/y".to_owned(), Vec::new()));
        assert_eq!(join_query("http://x/y", &[]), "http://x/y");
    }

    #[test]
    fn form_bodies_escape_structure_and_leave_variables_readable() {
        let fields = pairs(&[("name", "a b&c=d"), ("token", "{{token}}"), ("pct", "100%")]);
        let body = encode_form(&fields);
        assert_eq!(body, "name=a+b%26c%3Dd&token={{token}}&pct=100%25");
        assert_eq!(decode_form(&body), fields);
    }

    #[test]
    fn the_body_mode_follows_the_content_type_and_falls_back_to_the_body() {
        assert_eq!(BodyMode::detect(Some("application/json; charset=utf-8"), ""), BodyMode::Json);
        assert_eq!(BodyMode::detect(Some("application/x-www-form-urlencoded"), "a=1"), BodyMode::Form);
        assert_eq!(BodyMode::detect(Some("application/soap+xml"), ""), BodyMode::Xml);
        assert_eq!(BodyMode::detect(None, ""), BodyMode::None);
        assert_eq!(BodyMode::detect(None, " [1]"), BodyMode::Json);
        assert_eq!(BodyMode::detect(None, "hello"), BodyMode::Text);
    }

    #[test]
    fn every_mode_has_a_label_and_the_file_modes_come_last() {
        assert_eq!(BodyMode::ALL.len(), 8);
        assert_eq!(BodyMode::ALL[6], BodyMode::Multipart);
        assert_eq!(BodyMode::ALL[7], BodyMode::File);
        assert_eq!(BodyMode::Multipart.label(), "Multipart");
        assert_eq!(BodyMode::File.label(), "File");
        assert_eq!(BodyMode::Multipart.content_type(), Some("multipart/form-data"));
        assert_eq!(BodyMode::File.content_type(), None);
    }

    #[test]
    fn header_pairs_keep_the_maps_order() {
        let map = header_map(&pairs(&[("B", "2"), ("A", "1")]));
        assert_eq!(header_pairs(&Value::Object(map)), pairs(&[("B", "2"), ("A", "1")]));
    }

    #[test]
    fn the_engines_send_result_becomes_a_response() {
        let r = HttpResponse::from_value(
            &json!({
                "status": 201, "headers": {"Content-Type": "application/json"}, "body": encoded("{\"a\":1}"),
                "durationMs": 42,
                "tests": [{"name": "ok", "passed": true, "durationMs": 1}, {"name": "bad", "passed": false, "error": "nope", "durationMs": 1}],
                "preScriptError": "boom",
            }),
            BodyEncoding::Base64,
        )
        .unwrap();
        assert_eq!(r.status, 201);
        assert!(r.is_success());
        assert_eq!(r.duration_ms, 42);
        assert_eq!(r.tests.len(), 2);
        assert_eq!(r.tests[1].error.as_deref(), Some("nope"));
        assert_eq!(r.script_errors, ["boom"]);
    }

    #[test]
    fn a_result_without_a_status_is_not_a_response() {
        assert!(HttpResponse::from_value(&json!({"headers": {}}), BodyEncoding::Text).is_none());
    }

    #[test]
    fn sizes_are_the_bytes_received_and_read_in_the_unit_a_person_would_pick() {
        let body = STANDARD.encode("x".repeat(2048));
        let r = HttpResponse::from_value(&json!({"status": 200, "headers": {}, "body": body}), BodyEncoding::Base64)
            .unwrap();
        assert_eq!(r.size_bytes, 2048);
        assert_eq!(r.body_size_label(), "2.0 KB");
        let small =
            HttpResponse::from_value(&json!({"status": 200, "headers": {}, "body": "0123456789"}), BodyEncoding::Text)
                .unwrap();
        assert_eq!(small.body_size_label(), "10 B");
    }
}
