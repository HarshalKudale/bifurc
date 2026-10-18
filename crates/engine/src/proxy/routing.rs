//! The forward proxy's routing: a request meets the mocks first, then the proxy rules; anything neither
//! claims is left for the caller to pass through.

use tokio::io::AsyncWrite;

use super::{
    handler::{match_proxy_rule, proxy_to_upstream, proxy_with_scripts},
    response::{BufferedProxyResponse, Outcome, fetch_upstream_response, log_body, send_html},
};
use crate::{
    kernel::traffic::{Headers, RequestLogEntry, emit_log},
    mocks::{
        is_fully_mocked, is_streaming_mock, match_mock, merge_mock_with_upstream, resolve_mock_only_response,
        serve_mock, serve_resolved_response,
    },
    utils::{
        id::now_ms,
        replay::{decode_base64_lenient, replay_request},
    },
    workspace::models::Routing,
};

/// A request as the routing sees it.
pub struct Exchange<'a> {
    /// The mocks, mappings and rules, already narrowed to what is switched on, and the environment.
    pub routing: &'a Routing,
    pub method: &'a str,
    pub url: &'a str,
    /// What a rule's pattern is matched against.
    pub rule_match_target: &'a str,
    pub req_path: &'a str,
    pub headers: &'a Headers,
    pub body: &'a [u8],
    pub req_body_b64: &'a str,
    /// The log entry with everything known before the answer.
    pub base: &'a RequestLogEntry,
    /// When the request arrived (ms since the epoch).
    pub t0: i64,
}

/// `base` completed with how the exchange ended.
pub fn finished_entry(
    base: &RequestLogEntry,
    status: u16,
    via: &str,
    target: Option<String>,
    duration_ms: i64,
    res_headers: Headers,
    res_body: String,
) -> RequestLogEntry {
    RequestLogEntry {
        status: Some(status),
        via: via.to_string(),
        target,
        duration_ms: Some(duration_ms),
        res_headers,
        res_body,
        res_status: Some(status),
        ..base.clone()
    }
}

/// An exchange that ended in a proxy error page.
pub fn error_entry(base: &RequestLogEntry, status: u16, duration_ms: i64) -> RequestLogEntry {
    finished_entry(base, status, "error", None, duration_ms, Headers::new(), String::new())
}

pub fn log_outcome(base: &RequestLogEntry, via: &str, target: Option<String>, outcome: Outcome) {
    let mut entry =
        finished_entry(base, outcome.status, via, target, outcome.duration_ms, outcome.res_headers, outcome.res_body);
    if let Some(shown) = outcome.req_headers {
        entry.req_headers = shown;
        // The capture shows the request as the client sent it, without the trace header a script saw.
        if let Some(name) = crate::capture::trace_header() {
            entry.req_headers.shift_remove(&name);
        }
    }
    if base.fault.is_some() {
        // An injected delay came before the upstream call, which the outcome's own timing does not cover.
        entry.duration_ms = Some(now_ms() - base.ts);
    }
    mask_secrets(&mut entry);
    emit_log(&entry);
}

/// What a log entry holds never includes a vault secret's value, whatever the server echoed back.
fn mask_secrets(entry: &mut RequestLogEntry) {
    let Some(redactor) = crate::vault::Redactor::for_workspace(entry.workspace_id.as_deref().unwrap_or("")) else {
        return;
    };
    for value in entry.req_headers.values_mut().chain(entry.res_headers.values_mut()) {
        *value = redactor.text(value);
    }
    let mut res_body = serde_json::Value::String(std::mem::take(&mut entry.res_body));
    redactor.base64_body(&mut res_body);
    entry.res_body = res_body.as_str().unwrap_or_default().to_string();
}

const RULE_WITHOUT_TARGET: &str = "<h1>502 Bad Gateway</h1><p>Proxy rule matched but the target is not configured.</p>";

/// Route one request. `true` when a mock or a rule took it (and the response was written to `out`); `false`
/// when nothing matched and the caller should pass it through.
pub async fn handle_proxy_routing<W: AsyncWrite + Unpin>(out: &mut W, ex: &Exchange<'_>) -> bool {
    // A cassette being replayed answers first, for its hosts.
    if crate::cassettes::replay::serve(out, ex.base, true, ex.req_path, ex.body, ex.t0).await {
        return true;
    }
    let routing = ex.routing;
    let env = routing.env.as_ref();
    let rule = match_proxy_rule(&routing.rules, ex.rule_match_target, &routing.mappings);
    let rule_target = rule.target.clone().filter(|t| !t.is_empty());

    // Mocks come first.
    if let Some(mock) = match_mock(&routing.mocks, ex.method, ex.url, env) {
        let mock_target = Some(format!("mock:{}", mock.id));
        if is_fully_mocked(mock) {
            let resolved = resolve_mock_only_response(mock, env);
            emit_log(&finished_entry(
                ex.base,
                resolved.status,
                "mock",
                mock_target,
                now_ms() - ex.t0 + resolved.delay_ms as i64,
                resolved.headers.clone(),
                log_body(&resolved.body),
            ));
            let _ = if is_streaming_mock(mock) {
                serve_mock(out, mock, env).await
            } else {
                serve_resolved_response(out, &resolved).await
            };
            return true;
        }

        if rule.matched && (rule.rule.is_none() || rule_target.is_none()) {
            send_html(out, 502, RULE_WITHOUT_TARGET).await;
            emit_log(&error_entry(ex.base, 502, now_ms() - ex.t0));
            return true;
        }
        let upstream: Result<BufferedProxyResponse, String> = match (rule.rule, &rule_target) {
            (Some(matched_rule), Some(target)) => {
                fetch_upstream_response(
                    ex.method,
                    target,
                    ex.req_path,
                    ex.headers,
                    ex.body,
                    Some(matched_rule),
                    &routing.workspace_id,
                )
                .await
            }
            _ => {
                replay_request(ex.method, ex.url, ex.headers, ex.req_body_b64).await.map(|res| BufferedProxyResponse {
                    status: res.status,
                    headers: res.headers,
                    body: decode_base64_lenient(&res.body),
                    duration_ms: now_ms() - ex.t0,
                })
            }
        };
        match upstream {
            Ok(upstream) => {
                let merged = merge_mock_with_upstream(mock, &upstream, env);
                emit_log(&finished_entry(
                    ex.base,
                    merged.status,
                    "mock",
                    mock_target,
                    upstream.duration_ms + merged.delay_ms as i64,
                    merged.headers.clone(),
                    log_body(&merged.body),
                ));
                let _ = serve_resolved_response(out, &merged).await;
            }
            Err(_) => {
                send_html(
                    out,
                    502,
                    &format!("<h1>502 Bad Gateway</h1><p>Could not connect to <code>{}</code>.</p>", ex.url),
                )
                .await;
                emit_log(&error_entry(ex.base, 502, now_ms() - ex.t0));
            }
        }
        return true;
    }

    // Then the proxy rules.
    if !rule.matched {
        return false;
    }
    let (Some(matched_rule), Some(target)) = (rule.rule, rule_target) else {
        send_html(out, 502, RULE_WITHOUT_TARGET).await;
        emit_log(&error_entry(ex.base, 502, now_ms() - ex.t0));
        return true;
    };
    let has_scripts = !matched_rule.request_script.trim().is_empty() || !matched_rule.response_script.trim().is_empty();
    let outcome = if has_scripts {
        proxy_with_scripts(
            out,
            ex.method,
            &target,
            ex.req_path,
            ex.headers,
            ex.body,
            matched_rule,
            &routing.workspace_id,
        )
        .await
    } else {
        proxy_to_upstream(out, ex.method, &target, ex.req_path, ex.headers, ex.body, Some(&ex.base.id)).await
    };
    if let Some(outcome) = outcome {
        log_outcome(ex.base, "rule", Some(target), outcome);
    }
    true
}
