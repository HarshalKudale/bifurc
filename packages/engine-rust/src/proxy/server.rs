//! The forward proxy: the port a browser's proxy setting points at. It serves absolute-form `http://`
//! requests (mocks, proxy rules, passthrough) and CONNECT, which it tunnels or, with a loaded CA, intercepts.
//! Relative requests belong to the edge and are refused here.

use std::{path::PathBuf, sync::Arc};

use parking_lot::Mutex;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};

use super::{
    handler::tcp_tunnel,
    listener::{ConnectionHandler, ListenerCore},
    response::{passthrough_to_upstream, passthrough_to_upstream_https, send_html},
    routing::{Exchange, error_entry, handle_proxy_routing, log_outcome},
    tls_intercept::intercept_tls,
    wire::{Duplicates, Prefixed, RequestHead, parse_header_lines, read_body, read_head},
};
use crate::{
    certificates::{ca_cert_path, ca_key_path, clear_cert_cache, is_ca_loaded, load_ca, unload_ca},
    kernel::telemetry::{Headers, RequestLogEntry, emit_log},
    runtime::instances::Listener,
    utils::{
        cache_policy::strip_cache_headers,
        id::{mk_id, now_ms},
        json::{bool_of, opt_str, str_of},
    },
    workspace::{
        config::{load_config, load_workspace_dispatch_slice},
        dispatch::{active_env, load_enabled_sets, workspace_cfg},
        fs::read_all_entities,
    },
};

/// Everything a request needs of the config, composed when the proxy starts and whenever it is told to reload.
struct Snapshot {
    tls_enabled: bool,
    tls_cert_path: Option<PathBuf>,
    tls_key_path: Option<PathBuf>,
    /// The served workspace's mocks, mappings, rules and environments, narrowed to what is switched on.
    dispatch: Value,
}

fn compose_snapshot() -> Snapshot {
    let cfg = load_config();
    let ws_id = str_of(&cfg, "activeWorkspaceId").to_string();
    let rules = read_all_entities(&ws_id, "rules");
    let enabled = load_enabled_sets(&ws_id);
    let slice = load_workspace_dispatch_slice(&ws_id);
    Snapshot {
        tls_enabled: bool_of(&cfg, "tlsEnabled", false),
        tls_cert_path: opt_str(&cfg, "tlsCaCertPath").map(PathBuf::from),
        tls_key_path: opt_str(&cfg, "tlsCaKeyPath").map(PathBuf::from),
        dispatch: workspace_cfg(&cfg, &rules, &enabled, &slice),
    }
}

#[derive(Default)]
pub(super) struct Shared {
    core: ListenerCore,
    /// Whether `start` was called and `stop` not yet: the CA is unloaded by the stop that ends that.
    active: Mutex<bool>,
    snapshot: Mutex<Option<Arc<Snapshot>>>,
}

impl Shared {
    fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.lock().clone().unwrap_or_else(|| Arc::new(compose_snapshot()))
    }
}

/// The forward proxy as a [`Listener`].
pub struct ProxyServer {
    shared: Arc<Shared>,
}

impl ProxyServer {
    pub fn new() -> Self {
        Self { shared: Arc::new(Shared::default()) }
    }
}

impl Default for ProxyServer {
    fn default() -> Self {
        Self::new()
    }
}

impl Listener for ProxyServer {
    fn start(&self, port: u16) {
        self.stop();
        let snapshot = Arc::new(compose_snapshot());
        // A CA made or imported through `tls.*` lives at the engine's own CA paths and is never written into
        // the settings, so those paths are the fallback when the settings name none.
        if snapshot.tls_enabled {
            let cert = snapshot.tls_cert_path.clone().unwrap_or_else(ca_cert_path);
            let key = snapshot.tls_key_path.clone().unwrap_or_else(ca_key_path);
            load_ca(&cert, &key);
        } else {
            unload_ca();
        }
        *self.shared.snapshot.lock() = Some(snapshot);
        *self.shared.active.lock() = true;
        let shared = self.shared.clone();
        let handler: ConnectionHandler = Arc::new(move |socket| Box::pin(handle_connection(shared.clone(), socket)));
        self.shared.core.start(port, handler);
    }

    fn stop(&self) {
        self.shared.core.stop();
        if std::mem::take(&mut *self.shared.active.lock()) {
            unload_ca();
            clear_cert_cache();
        }
    }

    fn is_running(&self) -> bool {
        self.shared.core.is_running()
    }

    fn port(&self) -> u16 {
        self.shared.core.port()
    }

    fn error(&self) -> Option<String> {
        self.shared.core.error()
    }

    fn reload_config(&self) {
        *self.shared.snapshot.lock() = Some(Arc::new(compose_snapshot()));
    }

    fn dispatch_config(&self) -> Option<Value> {
        Some(self.shared.snapshot().dispatch.clone())
    }
}

/// `host:port` of a CONNECT target; the port is 443 when none is given.
fn connect_target(raw: &str) -> (String, u16) {
    match raw.rfind(':') {
        Some(colon) => (raw[..colon].to_string(), raw[colon + 1..].parse().unwrap_or(443)),
        None => (raw.to_string(), 443),
    }
}

async fn handle_connection(shared: Arc<Shared>, mut socket: TcpStream) {
    let Some((head, leftover)) = read_head(&mut socket).await else { return };
    if head.method == "CONNECT" {
        let (host, port) = connect_target(&head.target);
        let snapshot = shared.snapshot();
        if snapshot.tls_enabled && is_ca_loaded() {
            let client = Prefixed::new(leftover, socket);
            if let Some((mut tls, request, body)) = intercept_tls(client, &host).await {
                dispatch_https(&shared, &mut tls, &request, &body, &host, port).await;
            }
        } else {
            tcp_tunnel(socket, &host, port, &leftover, Some("HTTP/1.1 200 Connection Established\r\n\r\n")).await;
        }
        return;
    }
    let Ok(body) = read_body(&mut socket, leftover, &head.header_lines).await else { return };
    dispatch(&shared, &mut socket, &head, &body).await;
}

/// The request's headers, lowercased, without hop-by-hop headers and, unless caching is on, without the
/// conditional-request ones.
fn request_headers(head: &RequestHead) -> Headers {
    let mut headers = parse_header_lines(&head.header_lines, Duplicates::LastWins);
    strip_cache_headers(&mut headers);
    headers
}

fn base_entry(
    id: String,
    t0: i64,
    head: &RequestHead,
    url: String,
    host: String,
    headers: Headers,
    body: &[u8],
    cfg: &Value,
) -> (RequestLogEntry, String) {
    use base64::Engine as _;
    let req_body = if body.is_empty() { String::new() } else { base64::engine::general_purpose::STANDARD.encode(body) };
    let entry = RequestLogEntry {
        id,
        ts: t0,
        method: head.method.clone(),
        url,
        host,
        req_headers: headers,
        req_body: req_body.clone(),
        workspace_id: Some(str_of(cfg, "activeWorkspaceId").to_string()),
        origin: Some("forward".to_string()),
        ..Default::default()
    };
    (entry, req_body)
}

/// A request that arrived in absolute form on the plain port.
async fn dispatch<W: AsyncRead + AsyncWrite + Unpin>(shared: &Shared, out: &mut W, head: &RequestHead, body: &[u8]) {
    let cfg = shared.snapshot().dispatch.clone();
    let t0 = now_ms();
    let id = mk_id();
    let headers = request_headers(head);
    let host_header = headers.get("host").map(|h| h.to_lowercase()).unwrap_or_default();
    let host = host_header.split(':').next().unwrap_or("").to_string();

    let raw_target = head.target.as_str();
    let absolute = raw_target.starts_with("http://") || raw_target.starts_with("https://");
    let req_path = if absolute {
        match url::Url::parse(raw_target) {
            Ok(u) => format!("{}{}", u.path(), u.query().map(|q| format!("?{q}")).unwrap_or_default()),
            Err(_) => "/".to_string(),
        }
    } else if raw_target.is_empty() {
        "/".to_string()
    } else {
        raw_target.to_string()
    };
    let url =
        if raw_target.starts_with("http") { raw_target.to_string() } else { format!("http://{host_header}{req_path}") };
    let (base, req_body_b64) = base_entry(id, t0, head, url.clone(), host.clone(), headers.clone(), body, &cfg);

    if absolute {
        let env = active_env(&cfg);
        let exchange = Exchange {
            cfg: &cfg,
            env: env.as_ref(),
            method: &head.method,
            url: &url,
            rule_match_target: raw_target,
            req_path: &req_path,
            headers: &headers,
            body,
            req_body_b64: &req_body_b64,
            base: &base,
            t0,
        };
        if handle_proxy_routing(out, &exchange).await {
            return;
        }
        let outcome =
            passthrough_to_upstream(out, &head.method, raw_target, &req_path, &headers, body, Some(&base.id)).await;
        if let Some(outcome) = outcome {
            log_outcome(&base, "proxy", Some(host), outcome);
        }
        return;
    }

    // A relative request is not a proxy request: the edge serves domain mappings and the home page.
    send_html(
        out,
        400,
        "<h1>400 Bad Request</h1>\
         <p>This is Bifurc's <strong>forward-proxy</strong> port, and this request did not arrive as a proxy request.</p>\
         <p>Domain mappings (<code>*.localhost</code>) and the home page are served by the \
         <strong>edge</strong> — the port Bifurc shows as your mapping port.</p>",
    )
    .await;
    emit_log(&error_entry(&base, 400, now_ms() - t0));
}

/// A request read from an intercepted TLS connection, whose target is a path on `hostname:port`.
pub(super) async fn dispatch_https<W: AsyncRead + AsyncWrite + Unpin>(
    shared: &Shared,
    out: &mut W,
    head: &RequestHead,
    body: &[u8],
    hostname: &str,
    port: u16,
) {
    let cfg = shared.snapshot().dispatch.clone();
    let t0 = now_ms();
    let id = mk_id();
    let headers = request_headers(head);
    let req_path = if head.target.is_empty() { "/".to_string() } else { head.target.clone() };
    let port_suffix = if port == 443 { String::new() } else { format!(":{port}") };
    let url = format!("https://{hostname}{port_suffix}{req_path}");
    let (base, req_body_b64) = base_entry(id, t0, head, url.clone(), hostname.to_string(), headers.clone(), body, &cfg);

    let env = active_env(&cfg);
    let exchange = Exchange {
        cfg: &cfg,
        env: env.as_ref(),
        method: &head.method,
        url: &url,
        rule_match_target: &url,
        req_path: &req_path,
        headers: &headers,
        body,
        req_body_b64: &req_body_b64,
        base: &base,
        t0,
    };
    if handle_proxy_routing(out, &exchange).await {
        return;
    }
    let outcome =
        passthrough_to_upstream_https(out, &head.method, hostname, port, &req_path, &headers, body, Some(&base.id))
            .await;
    if let Some(outcome) = outcome {
        log_outcome(&base, "proxy", Some(hostname.to_string()), outcome);
    }
}
