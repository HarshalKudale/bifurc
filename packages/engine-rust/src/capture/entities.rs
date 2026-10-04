//! Capture to entity construction: turning a captured exchange into a saved request or a mock.
//!
//! Both functions take the captured entry as JSON (the wire form of a `RequestLogEntry`) and return the
//! entity body, ready for `entity.create`.

use serde_json::{Map, Value, json};

use crate::utils::{content_type::is_binary_content_type, curl::is_skipped_header, replay::decode_base64_lenient};

fn is_base64_text(stripped: &str) -> bool {
    let body = stripped.trim_end_matches('=');
    stripped.len() - body.len() <= 2
        && stripped.len() % 4 == 0
        && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
}

fn without_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Base64 to text. Input that is not valid base64 comes back as it was, so a body that was never base64
/// shows as-is instead of vanishing.
fn b64_to_text(b64: &str) -> String {
    if b64.is_empty() {
        return String::new();
    }
    let stripped = without_whitespace(b64);
    if !is_base64_text(&stripped) {
        return b64.to_string();
    }
    String::from_utf8_lossy(&decode_base64_lenient(&stripped)).into_owned()
}

/// Whether the base64 is valid and its bytes are UTF-8. Invalid base64 counts as text.
fn is_utf8_base64(b64: &str) -> bool {
    let stripped = without_whitespace(b64);
    if !is_base64_text(&stripped) {
        return true;
    }
    std::str::from_utf8(&decode_base64_lenient(&stripped)).is_ok()
}

/// Case-insensitive header lookup; `""` when absent.
fn header_of<'a>(headers: Option<&'a Value>, name: &str) -> &'a str {
    headers
        .and_then(Value::as_object)
        .and_then(|headers| headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or("")
}

fn object_or_empty(value: Option<&Value>) -> Value {
    match value {
        Some(value) if !value.is_null() => value.clone(),
        _ => Value::Object(Map::new()),
    }
}

fn string_or_empty(value: Option<&Value>) -> Value {
    match value {
        Some(value) if !value.is_null() => value.clone(),
        _ => json!(""),
    }
}

/// A captured request as a saved request: hop-by-hop headers dropped, the body decoded to text. A binary,
/// multipart or non-UTF-8 body is kept as the captured base64 with `bodyEncoding: "base64"`. `name` is blank;
/// the caller labels it.
pub fn captured_to_request(entry: &Value) -> Value {
    let req_headers = entry.get("reqHeaders").filter(|v| !v.is_null());
    let req_body = entry.get("reqBody").and_then(Value::as_str).unwrap_or("");
    let mut headers = Map::new();
    for (name, value) in req_headers.and_then(Value::as_object).into_iter().flatten() {
        if !is_skipped_header(&name.to_lowercase()) {
            headers.insert(name.clone(), value.clone());
        }
    }
    let mut out = Map::new();
    out.insert("name".into(), json!(""));
    if let Some(method) = entry.get("method") {
        out.insert("method".into(), method.clone());
    }
    if let Some(url) = entry.get("url") {
        out.insert("url".into(), url.clone());
    }
    out.insert("headers".into(), Value::Object(headers));
    let content_type = header_of(req_headers, "content-type").to_lowercase();
    if !req_body.is_empty()
        && (is_binary_content_type(&content_type)
            || content_type.starts_with("multipart/")
            || !is_utf8_base64(req_body))
    {
        out.insert("body".into(), json!(req_body));
        out.insert("bodyEncoding".into(), json!("base64"));
    } else {
        out.insert("body".into(), json!(b64_to_text(req_body)));
    }
    Value::Object(out)
}

/// A captured exchange as a mock. The captured request headers are stored raw, hop-by-hop ones included, so
/// the mock replays the capture byte for byte; the mock editor filters them for display.
pub fn captured_to_mock(entry: &Value) -> Value {
    let res_headers = entry.get("resHeaders").filter(|v| !v.is_null());
    let res_body = entry.get("resBody").and_then(Value::as_str).unwrap_or("");
    let res_content_type = header_of(res_headers, "content-type").to_lowercase();
    let binary = is_binary_content_type(&res_content_type);
    let response_body = if binary {
        res_body.to_string()
    } else {
        let text = b64_to_text(res_body);
        if text.is_empty() { "{}".to_string() } else { text }
    };
    let mut out = Map::new();
    out.insert("name".into(), json!(""));
    if let Some(method) = entry.get("method") {
        out.insert("method".into(), method.clone());
    }
    if let Some(url) = entry.get("url") {
        out.insert("urlPattern".into(), url.clone());
    }
    out.insert("useRegex".into(), json!(false));
    out.insert("capturedHeaders".into(), object_or_empty(entry.get("reqHeaders")));
    out.insert("capturedBody".into(), string_or_empty(entry.get("reqBody")));
    let status = entry.get("resStatus").filter(|v| !v.is_null()).cloned().unwrap_or(json!(200));
    out.insert("responseStatus".into(), status);
    out.insert("responseStatusMocked".into(), json!(true));
    out.insert("responseHeaders".into(), object_or_empty(entry.get("resHeaders")));
    out.insert("mockedResponseHeaders".into(), json!([]));
    out.insert("responseBody".into(), json!(response_body));
    out.insert("responseBodyMocked".into(), json!(true));
    if binary {
        out.insert("responseBodyEncoding".into(), json!("base64"));
    }
    out.insert("responseDelayMocked".into(), json!(true));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;

    use super::*;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    fn entry() -> Value {
        json!({
            "id": "a", "method": "POST", "url": "http://h/p", "resStatus": 201,
            "reqHeaders": { "Host": "h", "Content-Type": "application/json", "X-A": "1", "content-length": "3" },
            "reqBody": b64(b"{\"a\":1}"),
            "resHeaders": { "content-type": "application/json" },
            "resBody": b64(b"{\"ok\":true}"),
        })
    }

    #[test]
    fn request_drops_hop_by_hop_headers_and_decodes_the_body() {
        let request = captured_to_request(&entry());
        assert_eq!(
            request,
            json!({
                "name": "", "method": "POST", "url": "http://h/p",
                "headers": { "Content-Type": "application/json", "X-A": "1" },
                "body": "{\"a\":1}",
            })
        );
    }

    #[test]
    fn request_keeps_binary_multipart_and_non_utf8_bodies_as_base64() {
        for (ct, body) in [
            ("image/png", b64(b"abc")),
            ("multipart/form-data; boundary=x", b64(b"abc")),
            ("application/x-custom", b64(&[0xff, 0xfe, 0x00])),
        ] {
            let mut e = entry();
            e["reqHeaders"] = json!({ "content-type": ct });
            e["reqBody"] = json!(body);
            let request = captured_to_request(&e);
            assert_eq!(request["bodyEncoding"], "base64", "{ct}");
            assert_eq!(request["body"], json!(body), "{ct}");
        }
    }

    #[test]
    fn a_body_that_is_not_base64_shows_as_is() {
        let mut e = entry();
        e["reqBody"] = json!("plain text, not base64");
        assert_eq!(captured_to_request(&e)["body"], "plain text, not base64");
        assert!(captured_to_request(&e).get("bodyEncoding").is_none());
    }

    #[test]
    fn mock_stores_the_capture_raw() {
        let mock = captured_to_mock(&entry());
        assert_eq!(mock["urlPattern"], "http://h/p");
        assert_eq!(mock["useRegex"], false);
        assert_eq!(mock["capturedHeaders"]["Host"], "h");
        assert_eq!(mock["capturedBody"], json!(b64(b"{\"a\":1}")));
        assert_eq!(mock["responseStatus"], 201);
        assert_eq!(mock["responseBody"], "{\"ok\":true}");
        assert!(mock.get("responseBodyEncoding").is_none());
        assert_eq!(mock["mockedResponseHeaders"], json!([]));
        let keys: Vec<&str> = mock.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "name",
                "method",
                "urlPattern",
                "useRegex",
                "capturedHeaders",
                "capturedBody",
                "responseStatus",
                "responseStatusMocked",
                "responseHeaders",
                "mockedResponseHeaders",
                "responseBody",
                "responseBodyMocked",
                "responseDelayMocked"
            ]
        );
    }

    #[test]
    fn mock_of_a_binary_response_keeps_base64_and_an_empty_body_becomes_braces() {
        let mut e = entry();
        e["resHeaders"] = json!({ "Content-Type": "image/png" });
        e["resBody"] = json!(b64(&[0x89, 0x50]));
        let mock = captured_to_mock(&e);
        assert_eq!(mock["responseBodyEncoding"], "base64");
        assert_eq!(mock["responseBody"], json!(b64(&[0x89, 0x50])));

        let bare = captured_to_mock(&json!({ "method": "GET", "url": "http://h/" }));
        assert_eq!(bare["responseBody"], "{}");
        assert_eq!(bare["responseStatus"], 200);
        assert_eq!(bare["capturedHeaders"], json!({}));
        assert_eq!(bare["capturedBody"], "");
    }
}
