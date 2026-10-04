//! A request's body fields: resolving `{{variables}}` in them and turning them into wire bytes.
//!
//! A request carries `body`, `bodyEncoding` (`utf8` | `base64`) and `bodyParts` (multipart fields:
//! `{name, kind: "text" | "file", value, fileName?, contentType?}`), all as JSON.

use base64::Engine as _;
use rand::Rng;
use serde_json::{Map, Value, json};

use crate::{
    env::resolve_vars,
    kernel::telemetry::Headers,
    utils::{
        json::{is_truthy, opt_str},
        replay::decode_base64_lenient,
    },
};

/// Resolve `{{variables}}` in the body fields: a text body, part names, text-part values and file names.
/// Base64 data (a base64 body, a file part's bytes) is never touched. The result holds `body`, and
/// `bodyEncoding` / `bodyParts` when the request had them.
pub fn resolve_request_body(request: &Value, env: Option<&Value>) -> Value {
    let body = opt_str(request, "body").unwrap_or("");
    let mut out = Map::new();
    let body =
        if opt_str(request, "bodyEncoding") == Some("base64") { body.to_string() } else { resolve_vars(body, env) };
    out.insert("body".into(), json!(body));
    if let Some(encoding) = request.get("bodyEncoding").filter(|v| is_truthy(v)) {
        out.insert("bodyEncoding".into(), encoding.clone());
    }
    if let Some(parts) = request.get("bodyParts").and_then(Value::as_array) {
        let resolved = parts
            .iter()
            .map(|part| {
                let mut part = part.as_object().cloned().unwrap_or_default();
                let name = resolve_vars(part.get("name").and_then(Value::as_str).unwrap_or(""), env);
                part.insert("name".into(), json!(name));
                if part.get("kind").and_then(Value::as_str) == Some("text") {
                    let value = resolve_vars(part.get("value").and_then(Value::as_str).unwrap_or(""), env);
                    part.insert("value".into(), json!(value));
                }
                if let Some(file_name) = part.get("fileName").filter(|v| !v.is_null()) {
                    let file_name = resolve_vars(file_name.as_str().unwrap_or(""), env);
                    part.insert("fileName".into(), json!(file_name));
                }
                Value::Object(part)
            })
            .collect();
        out.insert("bodyParts".into(), Value::Array(resolved));
    }
    Value::Object(out)
}

fn quote_param(text: &str) -> String {
    text.replace(['\r', '\n'], "").replace('"', "%22")
}

/// A multipart payload with the given boundary.
pub fn build_multipart_with_boundary(boundary: &str, parts: &[Value]) -> Vec<u8> {
    let mut payload = Vec::new();
    for part in parts {
        let name = opt_str(part, "name").unwrap_or("");
        let value = opt_str(part, "value").unwrap_or("");
        let is_file = opt_str(part, "kind") == Some("file");
        let mut head = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"", quote_param(name));
        if is_file {
            let content_type =
                opt_str(part, "contentType").filter(|ct| !ct.is_empty()).unwrap_or("application/octet-stream");
            head.push_str(&format!(
                "; filename=\"{}\"\r\nContent-Type: {content_type}",
                quote_param(opt_str(part, "fileName").unwrap_or(""))
            ));
        }
        head.push_str("\r\n\r\n");
        payload.extend_from_slice(head.as_bytes());
        if is_file {
            payload.extend_from_slice(&decode_base64_lenient(value));
        } else {
            payload.extend_from_slice(value.as_bytes());
        }
        payload.extend_from_slice(b"\r\n");
    }
    payload.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    payload
}

/// A multipart payload with a fresh boundary, and the `content-type` header value that names it.
pub fn build_multipart(parts: &[Value]) -> (Vec<u8>, String) {
    let mut rng = rand::rng();
    let suffix: String = (0..12).map(|_| format!("{:02x}", rng.random::<u8>())).collect();
    let boundary = format!("----bifurc{suffix}");
    let payload = build_multipart_with_boundary(&boundary, parts);
    (payload, format!("multipart/form-data; boundary={boundary}"))
}

/// What goes on the wire: the body as base64 (what `request.replay` takes) and the headers to send.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedBody {
    pub body_base64: String,
    pub headers: Headers,
}

fn request_headers(request: &Value) -> Headers {
    request
        .get("headers")
        .and_then(Value::as_object)
        .map(|headers| headers.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

/// The one place a request's body becomes wire bytes: `bodyParts` (multipart) win, then a base64 body, then
/// a text body (a blank text body sends nothing). For multipart the `content-type` header is replaced.
/// Variables must already be resolved.
pub fn encode_request_body(request: &Value) -> EncodedBody {
    let mut headers = request_headers(request);
    let parts = request.get("bodyParts").and_then(Value::as_array).filter(|parts| !parts.is_empty());
    if let Some(parts) = parts {
        let (payload, content_type) = build_multipart(parts);
        headers.retain(|name, _| !name.eq_ignore_ascii_case("content-type"));
        headers.insert("content-type".to_string(), content_type);
        return EncodedBody { body_base64: base64::engine::general_purpose::STANDARD.encode(payload), headers };
    }
    let body = opt_str(request, "body").unwrap_or("");
    if opt_str(request, "bodyEncoding") == Some("base64") {
        return EncodedBody { body_base64: body.to_string(), headers };
    }
    if body.trim().is_empty() {
        return EncodedBody { body_base64: String::new(), headers };
    }
    EncodedBody { body_base64: base64::engine::general_purpose::STANDARD.encode(body.as_bytes()), headers }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Value {
        json!({ "variables": [
            { "key": "host", "value": "example.com" },
            { "key": "file", "value": "a.txt" },
            { "key": "field", "value": "avatar" },
        ] })
    }

    #[test]
    fn variables_resolve_in_text_but_never_in_base64_data() {
        let request = json!({ "body": "https://{{host}}/x", "bodyParts": [
            { "name": "{{field}}", "kind": "text", "value": "{{host}}" },
            { "name": "{{field}}", "kind": "file", "value": "e3t7aG9zdH19fQ==", "fileName": "{{file}}", "contentType": "image/png" },
        ] });
        let out = resolve_request_body(&request, Some(&env()));
        assert_eq!(out["body"], "https://example.com/x");
        assert!(out.get("bodyEncoding").is_none());
        assert_eq!(out["bodyParts"][0], json!({ "name": "avatar", "kind": "text", "value": "example.com" }));
        assert_eq!(out["bodyParts"][1]["value"], "e3t7aG9zdH19fQ==");
        assert_eq!(out["bodyParts"][1]["fileName"], "a.txt");
        assert_eq!(out["bodyParts"][1]["contentType"], "image/png");

        let base64_body = json!({ "body": "{{host}}", "bodyEncoding": "base64" });
        let out = resolve_request_body(&base64_body, Some(&env()));
        assert_eq!(out, json!({ "body": "{{host}}", "bodyEncoding": "base64" }));
    }

    #[test]
    fn multipart_is_built_with_text_and_file_parts() {
        let parts = vec![
            json!({ "name": "a\"b\n", "kind": "text", "value": "hello" }),
            json!({ "name": "f", "kind": "file", "value": "AQID", "fileName": "x\".bin" }),
            json!({ "name": "g", "kind": "file", "value": "", "contentType": "text/plain" }),
        ];
        let payload = build_multipart_with_boundary("B", &parts);
        let mut expected = Vec::new();
        expected.extend_from_slice(
            b"--B\r\nContent-Disposition: form-data; name=\"a%22b\"\r\n\r\nhello\r\n\
              --B\r\nContent-Disposition: form-data; name=\"f\"; filename=\"x%22.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n",
        );
        expected.extend_from_slice(&[1, 2, 3]);
        expected.extend_from_slice(
            b"\r\n--B\r\nContent-Disposition: form-data; name=\"g\"; filename=\"\"\r\nContent-Type: text/plain\r\n\r\n\r\n--B--\r\n",
        );
        assert_eq!(payload, expected);
    }

    #[test]
    fn encoding_a_multipart_request_replaces_the_content_type() {
        let request = json!({
            "headers": { "Content-Type": "application/json", "X-A": "1" },
            "bodyParts": [{ "name": "a", "kind": "text", "value": "v" }],
            "body": "ignored",
        });
        let encoded = encode_request_body(&request);
        let keys: Vec<&str> = encoded.headers.keys().map(String::as_str).collect();
        assert_eq!(keys, ["X-A", "content-type"]);
        let content_type = &encoded.headers["content-type"];
        let boundary = content_type.strip_prefix("multipart/form-data; boundary=").unwrap();
        assert!(boundary.starts_with("----bifurc") && boundary.len() == "----bifurc".len() + 24);
        let payload = base64::engine::general_purpose::STANDARD.decode(&encoded.body_base64).unwrap();
        let text = String::from_utf8(payload).unwrap();
        assert!(text.starts_with(&format!("--{boundary}\r\n")));
        assert!(text.ends_with(&format!("--{boundary}--\r\n")));
    }

    #[test]
    fn plain_bodies() {
        let text = encode_request_body(&json!({ "headers": { "a": "b" }, "body": "hi" }));
        assert_eq!(text.body_base64, "aGk=");
        assert_eq!(text.headers["a"], "b");

        let blank = encode_request_body(&json!({ "headers": {}, "body": "  \n " }));
        assert_eq!(blank.body_base64, "");
        let missing = encode_request_body(&json!({}));
        assert_eq!(missing.body_base64, "");

        let raw = encode_request_body(&json!({ "headers": {}, "body": "AQID", "bodyEncoding": "base64" }));
        assert_eq!(raw.body_base64, "AQID");
        let empty_parts = encode_request_body(&json!({ "headers": {}, "body": "x", "bodyParts": [] }));
        assert_eq!(empty_parts.body_base64, "eA==");
    }
}
