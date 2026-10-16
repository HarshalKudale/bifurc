//! What the forward proxy does with a request once it knows where it goes: tunnel a CONNECT, forward to a
//! rule's target (with or without the rule's scripts), and decide which rule a URL matches.

use std::time::Instant;

use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt, copy_bidirectional},
    net::TcpStream,
};

use super::response::{Outcome, collect_response_headers, forward_streaming, log_body, send_html};
use crate::{
    kernel::telemetry::Headers,
    scripting::{execute_request_script, execute_response_script},
    utils::{
        decompress::{decompress_body, strip_content_encoding},
        json::{is_truthy, str_of},
    },
};

/// Connect to `host:port` and copy bytes both ways until either side closes. `preamble` is written to the
/// caller first once the connection is made (the answer to a CONNECT); when the connection fails the caller
/// gets a 502, a bare one if a preamble was expected.
pub async fn tcp_tunnel<S: AsyncRead + AsyncWrite + Unpin>(
    mut client: S,
    host: &str,
    port: u16,
    initial_data: &[u8],
    preamble: Option<&str>,
) {
    let mut upstream = match TcpStream::connect((host, port)).await {
        Ok(upstream) => upstream,
        Err(_) => {
            if preamble.is_some() {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n").await;
            } else {
                send_html(
                    &mut client,
                    502,
                    &format!("<h1>502 Bad Gateway</h1><p>Could not connect to <code>{host}:{port}</code>.</p>"),
                )
                .await;
            }
            let _ = client.shutdown().await;
            return;
        }
    };
    if let Some(preamble) = preamble {
        if client.write_all(preamble.as_bytes()).await.is_err() {
            return;
        }
    }
    if !initial_data.is_empty() && upstream.write_all(initial_data).await.is_err() {
        return;
    }
    let _ = copy_bidirectional(&mut client, &mut upstream).await;
}

/// Split a rule target `host:port` at its last colon; the port is 80 without one.
fn split_target(target: &str) -> (String, Option<u16>) {
    match target.rfind(':') {
        None => (target.to_string(), Some(80)),
        Some(colon) => (target[..colon].to_string(), target[colon + 1..].parse().ok()),
    }
}

fn http_url(target: &str, path: &str) -> String {
    let (hostname, port) = split_target(target);
    let path = if path.is_empty() { "/" } else { path };
    match port {
        Some(port) => format!("http://{hostname}:{port}{path}"),
        None => format!("http://{hostname}:NaN{path}"),
    }
}

/// Forward to a rule's target (`host:port`) over plain HTTP and stream the answer back.
pub async fn proxy_to_upstream<W: AsyncWrite + Unpin>(
    out: &mut W,
    method: &str,
    target: &str,
    path: &str,
    headers: &Headers,
    body: &[u8],
    log_id: Option<&str>,
) -> Option<Outcome> {
    forward_streaming(out, method, &http_url(target, path), target, headers, body, log_id).await
}

/// A request after a rule's request script, ready to send.
pub(super) struct ScriptedRequest {
    /// What goes on the wire: the vault references the script's author wrote are replaced with their values.
    pub headers: Headers,
    pub body: Vec<u8>,
    /// The headers as the script left them, secrets unresolved, for the log.
    pub shown_headers: Headers,
}

/// Runs the rule's request script (a failing one leaves the request as it was). A rule can inject a secret by
/// writing `{{vault.secret}}` into a header or the body; only references that appear in the script itself are
/// resolved, so a client cannot get a secret out of the proxy by sending a reference of its own. The script
/// runs before any value is put in, so it never sees one.
pub(super) async fn run_request_script(rule: &Value, ws_id: &str, headers: &Headers, body: &[u8]) -> ScriptedRequest {
    let (mut shown_headers, mut shown_body) = (headers.clone(), body.to_vec());
    if let Some(script) = script_of(rule, "requestScript") {
        let result = execute_request_script(script, headers, &String::from_utf8_lossy(body)).await;
        if result.error.is_none() {
            shown_headers = result.headers;
            shown_body = result.body.into_bytes();
        }
    }
    let allowed = crate::vault::references_in(str_of(rule, "requestScript"));
    // What the client sent and the script left alone stays as written; only what the script set is resolved.
    let wire_headers: Headers = shown_headers
        .iter()
        .map(|(name, value)| {
            let from_client = headers.get(name) == Some(value);
            let wire = if from_client { value.clone() } else { crate::vault::resolve_allowed(ws_id, value, &allowed) };
            (name.clone(), wire)
        })
        .collect();
    let wire_body = match std::str::from_utf8(&shown_body) {
        Ok(text) if shown_body != body && text.contains("{{") => {
            crate::vault::resolve_allowed(ws_id, text, &allowed).into_bytes()
        }
        _ => shown_body,
    };
    ScriptedRequest { headers: wire_headers, body: wire_body, shown_headers }
}

fn script_of<'a>(rule: &'a Value, key: &str) -> Option<&'a str> {
    let script = str_of(rule, key);
    (!script.trim().is_empty()).then_some(script)
}

/// Forward to a rule's target, running the rule's request script on the way out and, if it has one, its
/// response script on the way back. With a response script the whole answer is buffered, decompressed,
/// rewritten and written with its own `content-length`.
pub async fn proxy_with_scripts<W: AsyncWrite + Unpin>(
    out: &mut W,
    method: &str,
    target: &str,
    path: &str,
    req_headers: &Headers,
    req_body: &[u8],
    rule: &Value,
    ws_id: &str,
) -> Option<Outcome> {
    let scripted = run_request_script(rule, ws_id, req_headers, req_body).await;
    let (headers, body) = (scripted.headers, scripted.body);
    // The log shows the request as the script made it, with secrets as written, only if that differs.
    let shown = (scripted.shown_headers != *req_headers).then_some(scripted.shown_headers);
    let Some(response_script) = script_of(rule, "responseScript") else {
        let mut outcome = proxy_to_upstream(out, method, target, path, &headers, &body, None).await;
        if let Some(outcome) = outcome.as_mut() {
            outcome.req_headers = shown;
        }
        return outcome;
    };

    let t0 = Instant::now();
    let elapsed = || t0.elapsed().as_millis() as i64;
    let url = http_url(target, path);
    let sent = super::response::send_request(method, &url, &headers, &body).await;
    let mut res = match sent {
        Ok(res) => res,
        Err(_) => {
            send_html(out, 502, &format!("<h1>502 Bad Gateway</h1><p>Could not connect to <code>{target}</code>.</p>"))
                .await;
            return Some(Outcome::bad_gateway(elapsed()));
        }
    };
    let status = res.status();
    let reason = status.canonical_reason().unwrap_or("");
    let (raw_headers, _) = collect_response_headers(res.headers());
    let mut full = Vec::new();
    while let Ok(Some(chunk)) = res.chunk().await {
        full.extend_from_slice(&chunk);
    }

    // Scripts always see plain text, and the body that is forwarded is no longer encoded.
    let encoding = raw_headers.get("content-encoding").cloned().unwrap_or_default();
    let decompressed = decompress_body(&full, &encoding);
    let plain_headers = if encoding.is_empty() { raw_headers } else { strip_content_encoding(&raw_headers) };
    let scripted =
        execute_response_script(response_script, &plain_headers, &String::from_utf8_lossy(&decompressed)).await;
    let (final_headers, final_body) = if scripted.error.is_some() {
        (plain_headers, decompressed)
    } else {
        (scripted.headers, scripted.body.into_bytes())
    };

    let mut head = format!("HTTP/1.1 {} {reason}\r\n", status.as_u16());
    for (name, value) in &final_headers {
        if name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if name.eq_ignore_ascii_case("set-cookie") {
            for cookie in value.split('\n').filter(|c| !c.is_empty()) {
                head.push_str(&format!("{name}: {cookie}\r\n"));
            }
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", final_body.len()));
    if out.write_all(head.as_bytes()).await.is_err() || out.write_all(&final_body).await.is_err() {
        return None;
    }
    let _ = out.shutdown().await;
    Some(Outcome {
        status: status.as_u16(),
        duration_ms: elapsed(),
        res_headers: final_headers,
        res_body: log_body(&final_body),
        req_headers: shown,
    })
}

/// The rule a URL matches and where it sends it.
#[derive(Debug, Clone, Default)]
pub struct RuleMatch<'a> {
    pub matched: bool,
    pub rule: Option<&'a Value>,
    /// The target the rule resolves to; `None` when it names no usable one.
    pub target: Option<String>,
}

/// The first rule whose pattern matches `target_url` (exactly, or as a JavaScript regular expression), with
/// the target it resolves to: the external target, or the target of the mapping it points at.
pub fn match_proxy_rule<'a>(rules: &'a [Value], target_url: &str, mappings: &[Value]) -> RuleMatch<'a> {
    for rule in rules {
        let pattern = str_of(rule, "pattern");
        let matches = if rule.get("useRegex").is_some_and(is_truthy) {
            crate::utils::regex::is_match(pattern, target_url)
        } else {
            target_url == pattern
        };
        if !matches {
            continue;
        }
        let target = if str_of(rule, "targetType") == "external" {
            Some(str_of(rule, "targetExternal").to_string()).filter(|t| !t.is_empty())
        } else {
            let mapping_id = str_of(rule, "targetMappingId");
            mappings
                .iter()
                .find(|m| str_of(m, "id") == mapping_id)
                .and_then(|m| m.get("target")?.as_str())
                .map(str::to_string)
        };
        return RuleMatch { matched: true, rule: Some(rule), target };
    }
    RuleMatch::default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tokio::io::AsyncReadExt;

    use super::*;

    #[test]
    fn rules_match_exactly_or_by_regex() {
        let rules = vec![
            json!({ "id": "bad", "pattern": "(", "useRegex": true }),
            json!({ "id": "ext", "pattern": "http://a.test/x", "useRegex": false, "targetType": "external",
                    "targetExternal": "localhost:3000" }),
            json!({ "id": "map", "pattern": "^http://b\\.test/", "useRegex": true, "targetType": "mapping",
                    "targetMappingId": "m1" }),
            json!({ "id": "dangling", "pattern": "^http://c\\.test/", "useRegex": true, "targetType": "mapping",
                    "targetMappingId": "nope" }),
            json!({ "id": "empty", "pattern": "http://d.test/", "targetType": "external", "targetExternal": "" }),
        ];
        let mappings = vec![json!({ "id": "m1", "target": "localhost:4000" })];

        let hit = match_proxy_rule(&rules, "http://a.test/x", &mappings);
        assert_eq!((hit.matched, hit.target.as_deref()), (true, Some("localhost:3000")));
        assert_eq!(str_of(hit.rule.unwrap(), "id"), "ext");

        let hit = match_proxy_rule(&rules, "http://b.test/anything", &mappings);
        assert_eq!(hit.target.as_deref(), Some("localhost:4000"));

        let hit = match_proxy_rule(&rules, "http://c.test/z", &mappings);
        assert!(hit.matched && hit.target.is_none());

        let hit = match_proxy_rule(&rules, "http://d.test/", &mappings);
        assert!(hit.matched && hit.target.is_none());

        assert!(!match_proxy_rule(&rules, "http://a.test/x/y", &mappings).matched);
    }

    #[test]
    fn rule_targets_split_at_the_last_colon() {
        assert_eq!(http_url("localhost:3000", "/a?b=1"), "http://localhost:3000/a?b=1");
        assert_eq!(http_url("localhost", ""), "http://localhost:80/");
        assert_eq!(http_url("[::1]:8080", "/"), "http://[::1]:8080/");
    }

    #[tokio::test]
    async fn tunnel_copies_both_ways_after_the_preamble() {
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = upstream.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = upstream.accept().await.unwrap();
            let mut buf = [0u8; 4];
            socket.read_exact(&mut buf).await.unwrap();
            socket.write_all(&buf.map(|b| b.to_ascii_uppercase())).await.unwrap();
        });
        let (client, mut far) = tokio::io::duplex(1024);
        let tunnel = tokio::spawn(async move {
            tcp_tunnel(client, "127.0.0.1", port, b"", Some("HTTP/1.1 200 Connection Established\r\n\r\n")).await;
        });
        far.write_all(b"ping").await.unwrap();
        let mut got = vec![0u8; 39 + 4];
        far.read_exact(&mut got).await.unwrap();
        assert_eq!(got, b"HTTP/1.1 200 Connection Established\r\n\r\nPING");
        drop(far);
        let _ = tunnel.await;
    }

    #[tokio::test]
    async fn tunnel_to_a_closed_port_answers_502() {
        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let (client, mut far) = tokio::io::duplex(1024);
        tcp_tunnel(client, "127.0.0.1", port, b"", Some("x")).await;
        let mut text = String::new();
        far.read_to_string(&mut text).await.unwrap();
        assert_eq!(text, "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
    }
}
