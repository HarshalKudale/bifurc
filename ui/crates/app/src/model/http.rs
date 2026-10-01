//! Editable request fields and the response that comes back, as the UI holds them.
//!
//! Headers are edited as text, one `Name: value` per line, and stored by the engine as a map. These
//! functions are the only translation between the two.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map, Value};

pub const HTTP_METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

/// `Name: value` lines to the engine's header map. Blank lines are skipped; a line with no colon is a header
/// with an empty value (the user has typed a name and not yet the rest) rather than being dropped.
pub fn parse_headers(text: &str) -> Map<String, Value> {
    let mut headers = Map::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').map_or((line, ""), |(n, v)| (n.trim(), v.trim()));
        if !name.is_empty() {
            headers.insert(name.to_owned(), Value::String(value.to_owned()));
        }
    }
    headers
}

pub fn format_headers(headers: &Value) -> String {
    headers
        .as_object()
        .map(|map| {
            map.iter()
                .map(|(name, value)| format!("{name}: {}", value.as_str().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
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

    /// The body as it should be displayed: JSON is re-indented, everything else is shown as received.
    pub fn display_body(&self) -> String {
        if self.is_binary {
            return format!("Binary body, {}. It can't be shown as text.", self.body_size_label());
        }
        let looks_json =
            self.content_type().is_some_and(|ct| ct.contains("json")) || self.body.trim_start().starts_with(['{', '[']);
        if looks_json
            && let Ok(value) = serde_json::from_str::<Value>(&self.body)
            && let Ok(pretty) = serde_json::to_string_pretty(&value)
        {
            return pretty;
        }
        self.body.clone()
    }

    pub fn body_size_label(&self) -> String {
        let bytes = self.size_bytes;
        if bytes < 1024 {
            format!("{bytes} B")
        } else if bytes < 1024 * 1024 {
            format!("{:.1} KB", bytes as f64 / 1024.0)
        } else {
            format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
        }
    }
}

/// A short reason phrase for the status line. Only the common ones; the number is the authority.
pub fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
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
        504 => "Gateway Timeout",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn headers_round_trip_through_text_in_order() {
        let map = parse_headers("Accept: application/json\nX-Trace:  abc \n\n");
        assert_eq!(map["Accept"], "application/json");
        assert_eq!(map["X-Trace"], "abc");
        assert_eq!(format_headers(&Value::Object(map)), "Accept: application/json\nX-Trace: abc");
    }

    #[test]
    fn a_value_may_itself_contain_colons() {
        assert_eq!(parse_headers("Referer: https://a.test:8080/x")["Referer"], "https://a.test:8080/x");
    }

    #[test]
    fn a_line_with_only_a_name_is_kept_with_an_empty_value() {
        assert_eq!(parse_headers("Authorization")["Authorization"], "");
    }

    fn encoded(text: &str) -> String {
        STANDARD.encode(text)
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
        assert_eq!(
            r.display_body(),
            "{
  \"a\": 1
}"
        );
    }

    #[test]
    fn a_result_without_a_status_is_not_a_response() {
        assert!(HttpResponse::from_value(&json!({"headers": {}}), BodyEncoding::Text).is_none());
    }

    #[test]
    fn non_json_bodies_are_shown_as_received() {
        let r = HttpResponse::from_value(
            &json!({"status": 200, "headers": {}, "body": encoded("<p>hi</p>")}),
            BodyEncoding::Base64,
        )
        .unwrap();
        assert_eq!(r.display_body(), "<p>hi</p>");
    }

    #[test]
    fn a_json_content_type_with_an_invalid_body_is_left_alone() {
        let r = HttpResponse::from_value(
            &json!({"status": 200, "headers": {"content-type": "application/json"}, "body": "{oops"}),
            BodyEncoding::Text,
        )
        .unwrap();
        assert_eq!(r.display_body(), "{oops");
    }

    #[test]
    fn a_body_that_is_not_text_is_reported_rather_than_mangled() {
        let png_header = STANDARD.encode([0x89u8, b'P', b'N', b'G', 0xFF, 0xFE]);
        let r =
            HttpResponse::from_value(&json!({"status": 200, "headers": {}, "body": png_header}), BodyEncoding::Base64)
                .unwrap();
        assert!(r.is_binary);
        assert_eq!(r.size_bytes, 6);
        assert!(r.display_body().starts_with("Binary body, 6 B"));
    }

    #[test]
    fn a_body_that_is_not_valid_base64_is_shown_as_it_arrived() {
        let r = HttpResponse::from_value(
            &json!({"status": 200, "headers": {}, "body": "not base64!"}),
            BodyEncoding::Base64,
        )
        .unwrap();
        assert_eq!(r.display_body(), "not base64!");
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
