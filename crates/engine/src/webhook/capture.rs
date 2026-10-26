//! `webhook.capture.*`: wait a few minutes for one webhook from the outside. A listener of its own is opened
//! and put on the internet with a quick tunnel; the first request it receives is handed back as a payload, the
//! headers worth keeping and the name of the header it is signed in. Nothing is kept once it ends.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use serde_json::{Map, Value, json};
use tokio::time::timeout;

use super::{Slot, announce, signing, start_listener, stop};
use crate::{
    kernel::CommandRegistry,
    protocol::{EngineError, Result},
    tunnel::{self, Tunnel},
    utils::{id::mk_id, json::str_of},
};

/// How long a capture stays open when no one ends it.
const LIFETIME: Duration = Duration::from_secs(600);

/// How long the tunnel stays open after a capture, so the answer to the sender gets back through it.
const TUNNEL_GRACE: Duration = Duration::from_secs(3);

/// The longest one `webhook.capture.wait` holds on for.
const MAX_WAIT_MS: u64 = 120_000;

struct Capture {
    listener_id: String,
    slot: Arc<Slot>,
    cancelled: AtomicBool,
    closed: AtomicBool,
    tunnel: Mutex<Option<Tunnel>>,
}

static CAPTURES: Mutex<Vec<(String, Arc<Capture>)>> = Mutex::new(Vec::new());

fn find(id: &str) -> Option<Arc<Capture>> {
    CAPTURES.lock().iter().find(|(known, _)| known == id).map(|(_, capture)| capture.clone())
}

/// Closes the capture: its listener, its tunnel, and whoever waits on it. The tunnel stays up for
/// [`TUNNEL_GRACE`] when `grace` is on (a request was captured and is being answered).
fn end(id: &str, grace: bool) -> Option<Arc<Capture>> {
    let capture = {
        let mut captures = CAPTURES.lock();
        let at = captures.iter().position(|(known, _)| known == id)?;
        captures.swap_remove(at).1
    };
    capture.closed.store(true, Ordering::SeqCst);
    let _ = stop(&json!({ "id": capture.listener_id }));
    announce();
    let tunnel = capture.tunnel.lock().take();
    match (tunnel, grace, tokio::runtime::Handle::try_current()) {
        (Some(tunnel), true, Ok(runtime)) => {
            runtime.spawn(async move {
                tokio::time::sleep(TUNNEL_GRACE).await;
                drop(tunnel);
            });
        }
        (tunnel, _, _) => drop(tunnel),
    }
    capture.slot.arrived.notify_one();
    Some(capture)
}

/// Opens the listener, and the tunnel to it unless `use_tunnel` is off (the address is then the local one).
async fn begin(workspace_id: &str, use_tunnel: bool) -> Result<Value> {
    let id = mk_id();
    let slot = Arc::new(Slot::default());
    let (listener_id, port) = start_listener(
        workspace_id.to_string(),
        0,
        Some(format!("capture-{id}")),
        None,
        None,
        None,
        Some(slot.clone()),
    )?;
    let (url, tunnel) = if use_tunnel {
        match tunnel::open(port).await {
            Ok(tunnel) => (tunnel.url.clone(), Some(tunnel)),
            Err(error) => {
                let _ = stop(&json!({ "id": listener_id }));
                announce();
                return Err(error);
            }
        }
    } else {
        (format!("http://127.0.0.1:{port}"), None)
    };
    let capture = Arc::new(Capture {
        listener_id,
        slot,
        cancelled: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        tunnel: Mutex::new(tunnel),
    });
    CAPTURES.lock().push((id.clone(), capture));
    let watchdog = id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(LIFETIME).await;
        end(&watchdog, false);
    });
    Ok(json!({ "captureId": id, "url": url }))
}

/// Headers that say how the request travelled, not what the sender sent.
fn travelled(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("cf-")
        || [
            "host",
            "content-length",
            "connection",
            "accept-encoding",
            "transfer-encoding",
            "keep-alive",
            "x-forwarded-for",
            "x-forwarded-proto",
            "x-forwarded-host",
            "x-real-ip",
            "cdn-loop",
        ]
        .contains(&name.as_str())
}

/// What the UI gets for a request that arrived.
fn captured(request: &Value) -> Value {
    use base64::Engine as _;
    let body = request["body"]
        .as_str()
        .and_then(|body| base64::engine::general_purpose::STANDARD.decode(body).ok())
        .unwrap_or_default();
    let text = String::from_utf8_lossy(&body).into_owned();
    // The body is signed again when it is sent, so it can be laid out for reading.
    let payload = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|json| serde_json::to_string_pretty(&json).ok())
        .unwrap_or(text);
    let kept: Vec<(&String, &str)> = request["headers"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, _)| !travelled(name))
        .filter_map(|(name, value)| value.as_str().map(|value| (name, value)))
        .collect();
    let signature = signing::signature_header_of(kept.iter().map(|(name, _)| name.as_str())).map(str::to_string);
    let headers: Map<String, Value> = kept
        .into_iter()
        .filter(|(name, _)| Some(name.as_str()) != signature.as_deref())
        .map(|(name, value)| (name.clone(), json!(value)))
        .collect();
    json!({
        "captured": true,
        "method": request["method"],
        "payload": payload,
        "headers": headers,
        "signatureHeader": signature.unwrap_or_default(),
    })
}

/// The request, once it has arrived; otherwise why not: `waiting` (ask again), `cancelled` or `expired`.
async fn wait(id: &str, wait_ms: u64) -> Result<Value> {
    let capture = find(id).ok_or_else(|| EngineError::not_found("no capture is waiting (it may have ended)"))?;
    if capture.slot.request.lock().is_none() {
        let _ = timeout(Duration::from_millis(wait_ms.min(MAX_WAIT_MS)), capture.slot.arrived.notified()).await;
    }
    let request = capture.slot.request.lock().take();
    Ok(match request {
        Some(request) => {
            end(id, true);
            captured(&request)
        }
        None if capture.cancelled.load(Ordering::SeqCst) => json!({ "captured": false, "reason": "cancelled" }),
        None if capture.closed.load(Ordering::SeqCst) => json!({ "captured": false, "reason": "expired" }),
        None => json!({ "captured": false, "reason": "waiting" }),
    })
}

fn cancel(id: &str) -> Value {
    let found = find(id);
    if let Some(capture) = &found {
        capture.cancelled.store(true, Ordering::SeqCst);
    }
    end(id, false);
    json!({ "cancelled": found.is_some() })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register(
        "webhook.capture.start",
        |params, _| async move { begin(str_of(&params, "workspaceId"), true).await },
    );
    registry.register("webhook.capture.wait", |params, _| async move {
        wait(str_of(&params, "captureId"), params["timeoutMs"].as_u64().unwrap_or(30_000)).await
    });
    registry.register_sync("webhook.capture.cancel", |params, _| Ok(cancel(str_of(&params, "captureId"))));
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
    };

    use super::*;

    /// POSTs `body` to a local address with `headers`; returns the status line.
    async fn post(url: &str, headers: &[(&str, &str)], body: &str) -> String {
        let address = url.trim_start_matches("http://");
        let mut socket = TcpStream::connect(address).await.unwrap();
        let mut request = format!("POST /hook HTTP/1.1\r\nHost: {address}\r\nContent-Length: {}\r\n", body.len());
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        socket.write_all(format!("{request}\r\n{body}").as_bytes()).await.unwrap();
        let mut answer = String::new();
        socket.read_to_string(&mut answer).await.unwrap();
        answer.lines().next().unwrap_or_default().to_string()
    }

    fn port_of(url: &str) -> u16 {
        url.rsplit(':').next().unwrap().parse().unwrap()
    }

    #[tokio::test]
    async fn the_first_request_is_captured_with_its_payload_headers_and_signature_header() {
        let started = begin("ws1", false).await.unwrap();
        let (id, url) =
            (started["captureId"].as_str().unwrap().to_string(), started["url"].as_str().unwrap().to_string());
        let waiting = tokio::spawn({
            let id = id.clone();
            async move { wait(&id, 5_000).await.unwrap() }
        });
        let sent = post(
            &url,
            &[
                ("Content-Type", "application/json"),
                ("X-GitHub-Event", "push"),
                ("X-Hub-Signature-256", "sha256=abc"),
                ("CF-Connecting-IP", "1.2.3.4"),
                ("X-Forwarded-For", "1.2.3.4"),
            ],
            "{\"ref\":\"main\",\"n\":1}",
        )
        .await;
        assert!(sent.contains("200"), "{sent}");
        let got = waiting.await.unwrap();
        assert_eq!(got["captured"], true);
        assert_eq!(got["method"], "POST");
        let payload: Value = serde_json::from_str(got["payload"].as_str().unwrap()).unwrap();
        assert_eq!(payload, json!({ "ref": "main", "n": 1 }));
        assert!(got["payload"].as_str().unwrap().contains('\n'), "laid out for reading");
        assert_eq!(got["signatureHeader"].as_str().unwrap().to_lowercase(), "x-hub-signature-256");
        let names: Vec<String> = got["headers"].as_object().unwrap().keys().map(|k| k.to_lowercase()).collect();
        assert!(
            names.contains(&"x-github-event".to_string()) && names.contains(&"content-type".to_string()),
            "{names:?}"
        );
        for dropped in ["host", "content-length", "cf-connecting-ip", "x-forwarded-for", "x-hub-signature-256"] {
            assert!(!names.contains(&dropped.to_string()), "{dropped} is not kept: {names:?}");
        }
        assert_eq!(wait(&id, 100).await.unwrap_err().code, crate::protocol::ErrorCode::NotFound);
        assert!(TcpStream::connect(("127.0.0.1", port_of(&url))).await.is_err(), "the listener is closed");
    }

    #[tokio::test]
    async fn only_the_first_request_counts_and_a_wait_with_nothing_yet_says_so() {
        let started = begin("ws1", false).await.unwrap();
        let (id, url) =
            (started["captureId"].as_str().unwrap().to_string(), started["url"].as_str().unwrap().to_string());
        assert_eq!(wait(&id, 50).await.unwrap(), json!({ "captured": false, "reason": "waiting" }));
        post(&url, &[], "first").await;
        post(&url, &[], "second").await;
        let got = wait(&id, 1_000).await.unwrap();
        assert_eq!(got["payload"], "first");
    }

    #[tokio::test]
    async fn cancelling_ends_a_wait_and_closes_the_listener() {
        let started = begin("ws1", false).await.unwrap();
        let (id, url) =
            (started["captureId"].as_str().unwrap().to_string(), started["url"].as_str().unwrap().to_string());
        let waiting = tokio::spawn({
            let id = id.clone();
            async move { wait(&id, 30_000).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(cancel(&id), json!({ "cancelled": true }));
        assert_eq!(waiting.await.unwrap().unwrap(), json!({ "captured": false, "reason": "cancelled" }));
        assert!(TcpStream::connect(("127.0.0.1", port_of(&url))).await.is_err());
        assert_eq!(cancel(&id), json!({ "cancelled": false }));
    }

    /// Run by hand: `cargo test -p bifurc-engine --lib a_real_tunnel -- --ignored`.
    #[tokio::test]
    #[ignore = "downloads cloudflared and opens a real quick tunnel"]
    async fn a_real_tunnel_carries_a_webhook_to_the_capture() {
        let _root = crate::testing::scratch_data_root();
        let installed = tunnel::install().await.unwrap();
        assert_eq!(installed["installed"], true, "{installed}");
        let started = begin("ws1", true).await.unwrap();
        let (id, url) =
            (started["captureId"].as_str().unwrap().to_string(), started["url"].as_str().unwrap().to_string());
        assert!(url.starts_with("https://") && url.ends_with(".trycloudflare.com"), "{url}");
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap();
        let waiting = tokio::spawn({
            let id = id.clone();
            async move { wait(&id, 100_000).await.unwrap() }
        });
        // A new quick tunnel name takes a few seconds to exist; asking sooner makes this computer cache "no such host".
        tokio::time::sleep(Duration::from_secs(12)).await;
        let mut sent = None;
        for _ in 0..30 {
            // A new quick tunnel needs a few seconds before its name resolves.
            match client.post(&url).header("X-Test", "1").body("{\"a\":1}").send().await {
                Ok(response) => {
                    let status = response.status();
                    // The capture finishes while this answer is on its way back through the tunnel.
                    sent = Some((status, response.text().await.unwrap_or_default()));
                    break;
                }
                Err(error) => {
                    eprintln!("not yet: {error:?}");
                    tokio::time::sleep(Duration::from_secs(2)).await
                }
            }
        }
        assert_eq!(sent.map(|(status, body)| (status.as_u16(), body)), Some((200, "{\"ok\":true}".to_string())));
        let got = waiting.await.unwrap();
        assert_eq!(got["captured"], true, "{got}");
        assert_eq!(serde_json::from_str::<Value>(got["payload"].as_str().unwrap()).unwrap(), json!({ "a": 1 }));
        assert!(got["headers"].as_object().unwrap().keys().any(|name| name.eq_ignore_ascii_case("x-test")));
        assert!(find(&id).is_none());
    }

    #[tokio::test]
    async fn starting_without_cloudflared_fails_and_leaves_no_listener() {
        // The test machine may have cloudflared; without it the start must still clean up after itself.
        if tunnel::find().is_some() {
            return;
        }
        let error = begin("ws1", true).await.unwrap_err();
        assert_eq!(error.code, crate::protocol::ErrorCode::NotFound);
        assert!(CAPTURES.lock().iter().all(|(_, capture)| !capture.closed.load(Ordering::SeqCst)));
    }
}
