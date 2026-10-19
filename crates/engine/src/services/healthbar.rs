//! `healthbar.*`: the per-workspace list of monitored services and the URL probe behind it.

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::{
    kernel::CommandRegistry,
    runtime::entity_status::emit_entity_status,
    utils::json::{read_json, str_of, write_json_pretty},
    workspace::{fs::ws_dir, status_cache::invalidate_cache},
};

const HEALTHBAR_DIR: &str = "healthbar";
const HEALTHBAR_FILE: &str = "services.json";
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_LIMIT_UTF16: usize = 10_000;

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("healthbar.getServices", |params, _| Ok(get_services(str_of(&params, "workspaceId"))));

    registry.register_sync("healthbar.saveServices", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        let services = params.get("services").unwrap_or(&Value::Null);
        let added = added_count(&get_services(ws_id), services);
        save_services(ws_id, services)?;
        invalidate_cache(ws_id);
        emit_entity_status(ws_id);
        Ok(json!({ "ok": true, "added": added }))
    });

    registry.register("healthbar.checkUrl", |params, _| async move { Ok(check_url(str_of(&params, "url")).await) });
}

fn services_file(ws_id: &str) -> std::path::PathBuf {
    ws_dir(ws_id).join(HEALTHBAR_DIR).join(HEALTHBAR_FILE)
}

/// The saved services of a workspace; an empty list when there is no readable file.
fn get_services(ws_id: &str) -> Value {
    read_json(&services_file(ws_id)).unwrap_or_else(|| json!([]))
}

/// How many of `next`'s services are not in `saved`, by id: the ones a save creates.
fn added_count(saved: &Value, next: &Value) -> usize {
    let ids = |list: &Value| -> Vec<String> {
        list.as_array().into_iter().flatten().map(|service| str_of(service, "id").to_string()).collect()
    };
    let before = ids(saved);
    ids(next).iter().filter(|id| !before.contains(id)).count()
}

fn save_services(ws_id: &str, services: &Value) -> std::io::Result<()> {
    write_json_pretty(&services_file(ws_id), services)
}

fn probe_result(
    status: Option<u16>,
    body: Option<String>,
    headers: Option<Value>,
    error: Option<String>,
    started: Instant,
) -> Value {
    json!({
        "ok": error.is_none(),
        "statusCode": status,
        "body": body,
        "headers": headers,
        "error": error,
        "durationMs": started.elapsed().as_millis() as u64,
    })
}

/// The first `limit` UTF-16 code units of `text`, as JavaScript's `slice` would keep them.
fn slice_utf16(text: &str, limit: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for ch in text.chars() {
        used += ch.len_utf16();
        if used > limit {
            break;
        }
        out.push(ch);
    }
    out
}

/// The innermost message of an error chain, which is where the OS reason lives.
fn error_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "Request timed out".to_string();
    }
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(inner) = source {
        message = inner.to_string();
        source = inner.source();
    }
    message
}

/// A single GET with no redirects, no proxy and no certificate verification. Any HTTP status is a successful
/// probe; connection failures and timeouts are reported in the result.
pub async fn check_url(url: &str) -> Value {
    let started = Instant::now();
    let fail = |message: String| probe_result(None, None, None, Some(message), started);
    let parsed = match url::Url::parse(url) {
        Ok(parsed) => parsed,
        Err(_) => return fail("Invalid URL".to_string()),
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return fail(format!("Protocol \"{}:\" not supported. Expected \"http:\"", parsed.scheme()));
    }
    // The probe monitors local services, which commonly serve self-signed certificates.
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(PROBE_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(error) => return fail(error.to_string()),
    };
    let response = match client.get(parsed).send().await {
        Ok(response) => response,
        Err(error) => return fail(error_message(&error)),
    };
    let status = response.status().as_u16();
    let mut headers = serde_json::Map::new();
    for (name, value) in response.headers() {
        let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
        match headers.get_mut(name.as_str()) {
            Some(Value::String(existing)) => {
                existing.push_str(", ");
                existing.push_str(&value);
            }
            _ => {
                headers.insert(name.as_str().to_string(), Value::String(value));
            }
        }
    }
    let bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => return fail(error_message(&error)),
    };
    let body = slice_utf16(&String::from_utf8_lossy(&bytes), BODY_LIMIT_UTF16);
    probe_result(Some(status), Some(body), Some(Value::Object(headers)), None, started)
}

#[cfg(test)]
mod tests {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    use super::*;
    use crate::testing::scratch_data_root;

    /// Serve `response` verbatim to every connection on a loopback port.
    async fn serve(response: &'static str, hold: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = socket.read(&mut buf).await;
                    if hold {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn healthy_and_unhealthy_statuses_are_both_ok_probes() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let port =
            serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nX-Test: a\r\nConnection: close\r\n\r\nhi", false).await;
        let result = check_url(&format!("http://127.0.0.1:{port}/")).await;
        assert_eq!(result["ok"], true);
        assert_eq!(result["statusCode"], 200);
        assert_eq!(result["body"], "hi");
        assert_eq!(result["headers"]["x-test"], "a");
        assert!(result["error"].is_null());

        let port =
            serve("HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", false).await;
        let result = check_url(&format!("http://127.0.0.1:{port}/")).await;
        assert_eq!(result["ok"], true);
        assert_eq!(result["statusCode"], 503);

        let port =
            serve("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\n\r\n", false).await;
        assert_eq!(check_url(&format!("http://127.0.0.1:{port}/")).await["statusCode"], 302);
    }

    #[tokio::test]
    async fn closed_port_and_bad_urls_are_error_results() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let result = check_url(&format!("http://127.0.0.1:{port}/")).await;
        assert_eq!(result["ok"], false);
        assert!(result["statusCode"].is_null() && result["body"].is_null() && result["headers"].is_null());
        assert!(result["error"].as_str().is_some_and(|e| !e.is_empty()));

        assert_eq!(check_url("not a url").await["error"], "Invalid URL");
        assert_eq!(check_url("ftp://127.0.0.1/").await["ok"], false);
    }

    #[tokio::test(start_paused = false)]
    async fn silent_server_times_out_with_short_budget() {
        // The probe's 10s budget is a constant; here only the shape of a hung connection is checked cheaply
        // by confirming the server accepts and the probe is still pending after a moment.
        let port = serve("", true).await;
        let url = format!("http://127.0.0.1:{port}/");
        let probe = check_url(&url);
        assert!(tokio::time::timeout(Duration::from_millis(300), probe).await.is_err());
    }

    #[test]
    fn utf16_slice_matches_javascript() {
        assert_eq!(slice_utf16("abcdef", 3), "abc");
        assert_eq!(slice_utf16("a\u{1F600}b", 2), "a");
        assert_eq!(slice_utf16("short", 100), "short");
    }

    #[test]
    fn services_round_trip_and_default() {
        let root = scratch_data_root();
        assert_eq!(get_services(root.ws_id), json!([]));
        let services = json!([{ "id": "a", "name": "API", "url": "http://localhost:3000/health" }]);
        save_services(root.ws_id, &services).unwrap();
        assert_eq!(get_services(root.ws_id), services);
        let text = std::fs::read_to_string(services_file(root.ws_id)).unwrap();
        assert!(text.contains("\n  {"));
        std::fs::write(services_file(root.ws_id), "{ not json").unwrap();
        assert_eq!(get_services(root.ws_id), json!([]));
    }
}
