//! The browser companion's way in. The extension catches a page's request before the browser encrypts it,
//! and the engine routes it exactly as the proxy port routes one: faults, the cassette being replayed, mocks,
//! then proxy rules. The answer tells the extension to hand the page a response, or to let the browser send
//! the request itself.

// The `browser.*` commands that call this come in the next chunk.
#![cfg_attr(not(test), allow(dead_code))]

use tokio::io::{DuplexStream, duplex};

use super::{
    routing::{Exchange, handle_proxy_routing},
    server::{Shared, base_entry},
    wire::{ResponseBody, raw_header, read_head},
};
use crate::{
    kernel::traffic::{Headers, RequestLogEntry, forwarded_headers},
    utils::{
        cache_policy::strip_cache_headers,
        http::is_hop_by_hop,
        id::{mk_id, now_ms},
    },
};

/// The largest answer handed over whole; a bigger one, or one of unknown length, is relayed as it arrives.
const MAX_WHOLE_BYTES: usize = 1024 * 1024;

/// What the extension does with a request it caught.
pub(super) enum Decision {
    /// Nothing claimed it: the browser sends it. `base` is the log entry the extension's mirror completes.
    Continue(Box<RequestLogEntry>),
    /// A whole answer of known length, already read: a mock, a preflight, a fault, most rule answers.
    Respond { status: u16, headers: Vec<(String, String)>, body: Vec<u8> },
    /// An answer whose body is still arriving: a streaming mock, a rule's target streaming or sending a lot.
    Relay { status: u16, headers: Vec<(String, String)>, body: ResponseBody<DuplexStream> },
    /// Claimed but never answered (a dropped or reset fault, an upstream that broke off): a network error.
    Failed,
}

/// Route a request the extension caught. `None` when `url` is not an `http(s)` URL.
pub(super) async fn decide(
    shared: &Shared,
    method: &str,
    url: &str,
    headers: &Headers,
    body: Vec<u8>,
) -> Option<Decision> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let hostname = parsed.host_str()?.to_string();
    // `port()` is `None` for the scheme's default, which neither the URL nor `Host` spells out.
    let host = match parsed.port() {
        Some(port) => format!("{hostname}:{port}"),
        None => hostname.clone(),
    };
    let req_path = format!("{}{}", parsed.path(), parsed.query().map(|q| format!("?{q}")).unwrap_or_default());
    let url = format!("{}://{host}{req_path}", parsed.scheme());
    // The page never sees `Host`; a rule's target gets the production host, as with TLS interception.
    let mut headers: Headers =
        headers.iter().filter(|(name, _)| !is_hop_by_hop(name)).map(|(n, v)| (n.to_lowercase(), v.clone())).collect();
    strip_cache_headers(&mut headers);
    headers.entry("host".to_string()).or_insert(host);

    let snapshot = shared.snapshot();
    let t0 = now_ms();
    let (mut base, req_body_b64) =
        base_entry(mk_id(), t0, method, url.clone(), hostname, headers, &body, &snapshot.routing.workspace_id);
    let method = method.to_string();
    // The routing writes an HTTP/1.1 answer as it would to a socket; it runs on its own so a streamed body
    // keeps flowing after the head is read here.
    let (mut out, mut answer) = duplex(64 * 1024);
    let routed = tokio::spawn(async move {
        if crate::faults::apply(&mut out, &mut base, t0).await {
            return None;
        }
        let forwarded = forwarded_headers(&base.req_headers, &base.id);
        let exchange = Exchange {
            routing: &snapshot.routing,
            method: &method,
            url: &url,
            rule_match_target: &url,
            req_path: &req_path,
            headers: &forwarded,
            body: &body,
            req_body_b64: &req_body_b64,
            base: &base,
            t0,
        };
        let claimed = handle_proxy_routing(&mut out, &exchange).await;
        (!claimed).then_some(base)
    });

    let Some((head, leftover)) = read_head(&mut answer).await else {
        return Some(match routed.await {
            Ok(Some(base)) => Decision::Continue(Box::new(base)),
            _ => Decision::Failed,
        });
    };
    let status = head.target.parse().unwrap_or(502);
    let headers = head
        .header_lines
        .iter()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_string()))
        .filter(|(name, _)| !is_hop_by_hop(name))
        .collect();
    let body = ResponseBody::new(answer, leftover, &head.header_lines);
    let length = raw_header(&head.header_lines, "content-length").and_then(|v| v.parse::<usize>().ok());
    if length.is_none_or(|n| n > MAX_WHOLE_BYTES) {
        return Some(Decision::Relay { status, headers, body });
    }
    Some(match body.read_to_end().await {
        Ok(body) => Decision::Respond { status, headers, body },
        Err(_) => Decision::Failed,
    })
}
