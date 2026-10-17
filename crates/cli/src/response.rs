//! What a one-shot response prints: a summary line and only what was asked for (`--body`, `--headers`,
//! `--select`, `--out`, `--raw`, `--quiet`, `--json`, `--expect-status`, `--expect`). `requests send` builds a
//! [`Response`] from the engine's result of each type; `bifurc curl` and `capture show` build theirs the same way.

use std::io::IsTerminal;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::{
    commands::Context,
    error::{CliError, Result},
    format::{js_template, truthy},
    select,
};

/// How much of the body a failed or error response shows when nothing asks for more.
const FAILURE_PREVIEW: usize = 512;

/// A test of a saved request and whether it passed.
pub struct Test {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

/// One response, whatever sent it.
pub struct Response {
    /// The HTTP status; the gRPC status code for gRPC.
    pub status: u16,
    pub status_text: String,
    /// gRPC only: how many messages the body holds (a gRPC summary counts them instead of sizing the body).
    pub messages: Option<usize>,
    pub duration_ms: u64,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub content_type: String,
    pub tests: Vec<Test>,
    /// A script error or a failed call: why the command fails whatever the status.
    pub failure: Option<String>,
}

const GRPC_CODES: [&str; 17] = [
    "OK",
    "CANCELLED",
    "UNKNOWN",
    "INVALID_ARGUMENT",
    "DEADLINE_EXCEEDED",
    "NOT_FOUND",
    "ALREADY_EXISTS",
    "PERMISSION_DENIED",
    "RESOURCE_EXHAUSTED",
    "FAILED_PRECONDITION",
    "ABORTED",
    "OUT_OF_RANGE",
    "UNIMPLEMENTED",
    "INTERNAL",
    "UNAVAILABLE",
    "DATA_LOSS",
    "UNAUTHENTICATED",
];

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
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

fn pairs(object: Option<&Value>) -> Vec<(String, String)> {
    object.and_then(Value::as_object).into_iter().flatten().map(|(k, v)| (k.clone(), js_template(Some(v)))).collect()
}

fn millis(result: &Value) -> u64 {
    result.get("durationMs").and_then(Value::as_f64).map_or(0, |ms| ms.round() as u64)
}

impl Response {
    /// The HTTP result of the engine, whose body is `body` already decoded.
    fn http(result: &Value, body: Vec<u8>) -> Self {
        let status = result.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
        let headers = pairs(result.get("headers"));
        let content_type =
            headers.iter().find(|(name, _)| name.eq_ignore_ascii_case("content-type")).map(|(_, v)| v.clone());
        Self {
            status,
            status_text: reason(status).to_string(),
            messages: None,
            duration_ms: millis(result),
            headers,
            body,
            content_type: content_type.unwrap_or_default(),
            tests: Vec::new(),
            failure: None,
        }
    }

    /// `request.send`: a base64 body, tests and scripts.
    pub fn from_rest(result: &Value) -> Self {
        let body = result.get("body").and_then(Value::as_str).unwrap_or_default();
        let decoded =
            base64::engine::general_purpose::STANDARD_NO_PAD.decode(body.trim_end_matches('=')).unwrap_or_default();
        let mut response = Self::http(result, decoded);
        response.tests = result
            .get("tests")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|test| Test {
                name: js_template(test.get("name")),
                passed: test.get("passed").is_some_and(truthy),
                error: test.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).map(str::to_string),
            })
            .collect();
        response.failure = [("preScriptError", "pre-script"), ("postScriptError", "post-script")].into_iter().find_map(
            |(key, label)| {
                let message = result.get(key).and_then(Value::as_str).filter(|m| !m.is_empty())?;
                Some(format!("{label} error: {message}"))
            },
        );
        response
    }

    /// `graphql.send` and `soap.send`: the body is text already.
    pub fn from_text(result: &Value) -> Self {
        Self::http(result, result.get("body").and_then(Value::as_str).unwrap_or_default().as_bytes().to_vec())
    }

    /// `grpc.send`: the body is the JSON of the response message, or of the array of them.
    pub fn from_grpc(result: &Value) -> Self {
        let ok = result.get("ok").and_then(Value::as_bool) != Some(false);
        let mut code = result.get("status").and_then(Value::as_u64).unwrap_or(0) as u16;
        if !ok && code == 0 {
            code = 2;
        }
        let mut messages: Vec<Value> = result
            .get("responses")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|message| match message {
                Value::String(text) => serde_json::from_str(text).unwrap_or_else(|_| message.clone()),
                other => other.clone(),
            })
            .collect();
        let count = messages.len();
        let body = match count {
            0 => String::new(),
            1 => messages.remove(0).to_string(),
            _ => Value::Array(messages).to_string(),
        };
        let said = |key: &str| result.get(key).and_then(Value::as_str).filter(|m| !m.is_empty());
        Self {
            status: code,
            status_text: GRPC_CODES.get(code as usize).copied().unwrap_or_default().to_string(),
            messages: Some(count),
            duration_ms: millis(result),
            headers: pairs(result.get("metadata")),
            body: body.into_bytes(),
            content_type: "application/json".to_string(),
            tests: Vec::new(),
            failure: (!ok)
                .then(|| said("error").or(said("statusMessage")).unwrap_or("the gRPC call failed").to_string()),
        }
    }
}

/// `512B`, `4.2KB`, `1.3MB`.
pub fn size(bytes: usize) -> String {
    match bytes {
        0..1024 => format!("{bytes}B"),
        1024..1_048_576 => format!("{:.1}KB", bytes as f64 / 1024.0),
        _ => format!("{:.1}MB", bytes as f64 / 1_048_576.0),
    }
}

/// Whether a header of this name holds a secret that is not printed.
fn sensitive(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["authorization", "proxy-authorization", "cookie", "set-cookie", "x-api-key", "x-auth-token"]
        .contains(&name.as_str())
        || ["token", "secret", "api-key", "apikey", "session"].iter().any(|word| name.contains(word))
}

/// `value`, or `••••(<length>)` when a header of this name holds a secret.
pub fn mask_header(name: &str, value: &str) -> String {
    if sensitive(name) { format!("••••({})", value.chars().count()) } else { value.to_string() }
}

/// The short name of a body's type: json, html, xml, text, js, css, form, binary or the subtype; with no
/// content type it is guessed from the body ("" for an empty one).
fn short_type(content_type: &str, body: &[u8]) -> String {
    let essence = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let binary_type = ["image/", "audio/", "video/", "font/"].iter().any(|prefix| essence.starts_with(prefix))
        || matches!(essence.as_str(), "application/octet-stream" | "application/pdf" | "application/zip");
    if binary_type || std::str::from_utf8(body).is_err() {
        return "binary".into();
    }
    let has = |word: &str| essence.contains(word);
    match essence.as_str() {
        "" if body.is_empty() => "",
        "" if serde_json::from_slice::<Value>(body).is_ok() => "json",
        "" | "text/plain" => "text",
        _ if has("json") => "json",
        _ if has("html") => "html",
        _ if has("xml") => "xml",
        _ if has("javascript") || has("ecmascript") => "js",
        _ if essence.ends_with("/css") => "css",
        "application/x-www-form-urlencoded" => "form",
        _ if essence.starts_with("multipart/") => "form",
        other => other.rsplit('/').next().unwrap_or_default(),
    }
    .to_string()
}

/// The first `limit` bytes of `text` (all of it when `limit` is 0), cut at a character, and how many bytes are left.
fn preview(text: &str, limit: usize) -> (&str, usize) {
    if limit == 0 || text.len() <= limit {
        return (text, 0);
    }
    let cut = (0..=limit).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
    (&text[..cut], text.len() - cut)
}

/// Whether an `--expect` is about the status, which a body that is not JSON can still answer.
fn about_status(text: &str) -> bool {
    text.trim().strip_prefix("status").is_some_and(|rest| rest.starts_with(char::is_whitespace))
}

/// The number `--<flag>` holds, when it is given.
fn number<T: std::str::FromStr>(ctx: &Context, flag: &str) -> Result<Option<T>> {
    ctx.args
        .flag(flag)
        .map(|raw| raw.parse().map_err(|_| CliError::usage(format!("--{flag} must be a whole number, not \"{raw}\""))))
        .transpose()
}

/// Print `response` as the output options of the command say. `--raw` hands the body bytes to `ctx.raw` for the
/// caller to write as they are; a failure (a test, a script, an expectation) is a [`CliError::Failed`] holding
/// the same output, so that the exit code is the only difference.
pub fn present(ctx: &mut Context, response: &Response) -> Result<String> {
    let args = ctx.args;
    let kind = short_type(&response.content_type, &response.body);
    let text = if kind == "binary" { None } else { std::str::from_utf8(&response.body).ok() };
    let parsed: Option<Value> = text.and_then(|text| serde_json::from_str(text).ok());

    let selected = match args.flag("select") {
        Some(path) => {
            let body = parsed
                .as_ref()
                .ok_or_else(|| CliError::msg(format!("--select needs a JSON body, but this response is {kind}")))?;
            Some(select::select(body, path).map_err(CliError::msg)?)
        }
        None => None,
    };

    let mut expects: Vec<String> = args.flags("expect").to_vec();
    if let Some(status) = number::<u16>(ctx, "expect-status")? {
        expects.insert(0, format!("status == {status}"));
    }
    let null = Value::Null;
    let mut checks = Vec::new();
    for expect in expects {
        let wanted = select::parse(&expect).map_err(CliError::usage)?;
        let result = match &parsed {
            None if !about_status(&expect) => Err(format!("expect {expect}, but the body is not JSON")),
            body => wanted.check(body.as_ref().unwrap_or(&null), response.status),
        };
        checks.push((expect, result));
    }

    let failed_test = response.tests.iter().find(|test| !test.passed).map(|test| match &test.error {
        Some(error) => format!("test \"{}\": {error}", test.name),
        None => format!("test \"{}\"", test.name),
    });
    let why = response
        .failure
        .clone()
        .or(failed_test)
        .or_else(|| checks.iter().find_map(|(_, result)| result.as_ref().err().cloned()));

    let saved = match args.flag("out") {
        Some(path) => {
            std::fs::write(path, &response.body).map_err(|e| CliError::msg(format!("cannot write {path}: {e}")))?;
            Some(path)
        }
        None => None,
    };
    let finish = |out: String| match &why {
        Some(_) => Err(CliError::Failed(out)),
        None => Ok(out),
    };
    if args.has("quiet") {
        return finish(String::new());
    }
    if args.has("raw") {
        ctx.raw = Some(response.body.clone());
        // The bytes are the output; the reason goes to stderr.
        return match &why {
            Some(why) => Err(CliError::msg(format!("✗ {why}"))),
            None => Ok(String::new()),
        };
    }

    let errored = why.is_some() || (response.messages.is_none() && response.status >= 400);
    let limit = if args.has("body") {
        Some(number::<usize>(ctx, "max-body")?.unwrap_or(2048))
    } else {
        errored.then_some(FAILURE_PREVIEW)
    };
    let shown = match (limit, text) {
        (Some(limit), Some(text)) if !text.is_empty() => Some(preview(text, limit)),
        _ => None,
    };
    let reveal = args.has("reveal") && std::io::stdout().is_terminal();
    let headers: Option<Vec<(&str, String)>> = args.has("headers").then(|| {
        let shown = |name: &str, value: &str| if reveal { value.to_string() } else { mask_header(name, value) };
        response.headers.iter().map(|(name, value)| (name.as_str(), shown(name, value))).collect()
    });

    if ctx.json {
        let mut out = json!({
            "schemaVersion": 1, "status": response.status, "statusText": response.status_text,
            "durationMs": response.duration_ms, "size": response.body.len(),
            "contentType": if response.content_type.is_empty() { &kind } else { &response.content_type },
            "tests": response.tests.iter()
                .map(|t| json!({ "name": t.name, "passed": t.passed, "error": t.error }))
                .collect::<Vec<_>>(),
            "body": shown.map(|(shown, _)| shown), "bodyTruncated": shown.is_some_and(|(_, left)| left > 0),
            "error": why,
        });
        if let Some(headers) = headers {
            out["headers"] = Value::Object(headers.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect());
        }
        if let Some(values) = selected {
            out["selected"] = json!(values);
        }
        if !checks.is_empty() {
            out["expectations"] = checks
                .iter()
                .map(|(text, result)| json!({ "text": text, "passed": result.is_ok(), "error": result.as_ref().err() }))
                .collect();
        }
        if let Some(path) = saved {
            out["saved"] = json!(path);
        }
        return finish(crate::format::json(&out));
    }

    let mut lines = Vec::new();
    match &selected {
        Some(values) if !errored => lines.push(select::render(values)),
        _ => {
            let head = match (response.messages, response.status) {
                (Some(_), 0) => format!("gRPC {}", response.status_text),
                (Some(_), code) => format!("gRPC {code} {}", response.status_text),
                (None, status) => format!("{status} {}", response.status_text).trim().to_string(),
            };
            let mut parts = vec![head, format!("{}ms", response.duration_ms)];
            match response.messages {
                Some(0) => {}
                Some(n) => parts.push(format!("{n} message{}", if n == 1 { "" } else { "s" })),
                None => parts.push(format!("{} {kind}", size(response.body.len())).trim_end().to_string()),
            }
            if !response.tests.is_empty() {
                let passed = response.tests.iter().filter(|test| test.passed).count();
                let mark = if passed == response.tests.len() { "✓" } else { "✗" };
                parts.push(format!("tests {passed}/{} {mark}", response.tests.len()));
            }
            lines.push(parts.join(" · "));
            if let Some(why) = &why {
                lines.push(format!("✗ {why}"));
            }
        }
    }
    for (name, value) in headers.into_iter().flatten() {
        lines.push(format!("{name}: {value}"));
    }
    match shown {
        Some((shown, left)) => {
            // The line break that ends most bodies would print as an empty line.
            lines.push(shown.trim_end_matches(['\r', '\n']).to_string());
            if left > 0 {
                lines.push(format!("… {} truncated — use --out or --select", size(left)));
            }
        }
        None if args.has("body") && !response.body.is_empty() => {
            lines.push(format!("binary body not shown ({}) — use --out", size(response.body.len())));
        }
        None => {}
    }
    if let Some(path) = saved {
        lines.push(format!("saved {path} ({})", size(response.body.len())));
    }
    finish(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_one_decimal_from_a_kilobyte() {
        assert_eq!([512, 1024, 4300, 1_363_149].map(size), ["512B", "1.0KB", "4.2KB", "1.3MB"].map(String::from));
    }

    #[test]
    fn the_short_type_comes_from_the_content_type_else_the_body() {
        for (content_type, body, expected) in [
            ("application/problem+json; charset=utf-8", &b"{}"[..], "json"),
            ("text/html", b"<p>", "html"),
            ("application/atom+xml", b"<a/>", "xml"),
            ("text/plain", b"x", "text"),
            ("application/javascript", b"x", "js"),
            ("text/css", b"x", "css"),
            ("multipart/form-data; boundary=x", b"x", "form"),
            ("text/csv", b"a", "csv"),
            ("", b"{\"a\":1}", "json"),
            ("", b"hello", "text"),
            ("", b"", ""),
            ("image/png", b"x", "binary"),
            ("text/plain", &[0xff, 0xfe][..], "binary"),
        ] {
            assert_eq!(short_type(content_type, body), expected, "{content_type}");
        }
    }

    #[test]
    fn secrets_are_masked_by_name() {
        assert_eq!(mask_header("Authorization", "Bearer abc"), "••••(10)");
        assert_eq!(mask_header("X-Session-Id", "ab"), "••••(2)");
        assert_eq!(mask_header("Content-Type", "a/b"), "a/b");
    }
}
