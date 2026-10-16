//! The edge: the reverse proxy for domain mappings (and any `*.localhost` name) and the home page. A request
//! meets the mocks first, then the mapping its `Host` names.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use base64::Engine as _;
use serde_json::Value;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
};
use tokio_rustls::TlsAcceptor;

use super::pages::{build_home_page, build_not_mapped_page, build_unserved_page};
use crate::{
    certificates::{apply_settings, listener_server_config},
    kernel::traffic::{RequestLogEntry, emit_log},
    mocks::{
        UpstreamResponse, is_fully_mocked, is_streaming_mock, match_mock, merge_mock_with_upstream,
        resolve_mock_only_response, serve_mock, serve_resolved_response,
    },
    proxy::{
        error_entry,
        listener::{ConnectionHandler, ListenerCore},
        log_body, log_outcome, proxy_to_upstream, send_html,
        wire::{Duplicates, parse_header_lines, read_body, read_head},
    },
    runtime::instances::Listener,
    utils::{
        cache_policy::strip_cache_headers,
        id::{mk_id, now_ms},
        json::str_of,
        net::hostname_of,
    },
    workspace::{models::Routing, settings::load_settings},
};

/// The routing config of the workspace the edge serves, or `None` when this engine does not serve it.
pub type ResolveConfig = Arc<dyn Fn() -> Option<Value> + Send + Sync>;

struct Shared {
    core: ListenerCore,
    /// Whether the last start put the edge in HTTPS mode: its one port then speaks TLS.
    secure: AtomicBool,
    resolve_config: ResolveConfig,
}

impl Shared {
    fn is_secure(&self) -> bool {
        self.secure.load(Ordering::Relaxed)
    }

    fn scheme(&self) -> &'static str {
        if self.is_secure() { "https" } else { "http" }
    }
}

/// The edge as a [`Listener`].
pub struct EdgeRouter {
    shared: Arc<Shared>,
}

impl EdgeRouter {
    pub fn new(resolve_config: ResolveConfig) -> Self {
        Self {
            shared: Arc::new(Shared { core: ListenerCore::default(), secure: AtomicBool::new(false), resolve_config }),
        }
    }
}

impl Listener for EdgeRouter {
    fn start(&self, port: u16) {
        let secure = apply_settings(&load_settings());
        self.shared.secure.store(secure, Ordering::Relaxed);

        let shared = self.shared.clone();
        let handler: ConnectionHandler = Arc::new(move |socket| Box::pin(handle_connection(shared.clone(), socket)));
        self.shared.core.start(port, handler);
    }

    fn stop(&self) {
        self.shared.core.stop();
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
}

/// The first byte of a TLS handshake record.
const TLS_HANDSHAKE: u8 = 0x16;

/// With HTTPS on the port speaks TLS, presenting a certificate for the name the client asked for. A client
/// that spoke plain HTTP to it is sent on to the same address on HTTPS.
async fn handle_connection(shared: Arc<Shared>, mut socket: TcpStream) {
    if !shared.is_secure() {
        return serve(shared, socket).await;
    }
    let mut first = [0u8; 1];
    if !matches!(socket.peek(&mut first).await, Ok(1)) {
        return;
    }
    if first[0] == TLS_HANDSHAKE {
        let Ok(config) = listener_server_config() else { return };
        let Ok(tls) = TlsAcceptor::from(config).accept(socket).await else { return };
        return serve(shared, tls).await;
    }
    let Some((head, _)) = read_head(&mut socket).await else { return };
    let host = hostname_of(&crate::proxy::wire::raw_header(&head.header_lines, "host").unwrap_or_default());
    let host = if host.is_empty() { "localhost".to_string() } else { host };
    let port = shared.core.port();
    let port = if port == 443 { String::new() } else { format!(":{port}") };
    let path = if head.target.starts_with('/') { head.target.as_str() } else { "/" };
    let answer = format!(
        "HTTP/1.1 308 Permanent Redirect\r\nLocation: https://{host}{port}{path}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = socket.write_all(answer.as_bytes()).await;
    let _ = socket.shutdown().await;
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(shared: Arc<Shared>, mut socket: S) {
    let Some((head, leftover)) = read_head(&mut socket).await else { return };
    let Ok(body) = read_body(&mut socket, leftover, &head.header_lines).await else { return };
    let mut headers = parse_header_lines(&head.header_lines, Duplicates::Joined);
    strip_cache_headers(&mut headers);
    let host_header = crate::proxy::wire::raw_header(&head.header_lines, "host").unwrap_or_default().to_lowercase();
    route(&shared, &mut socket, &head.method, &head.target, &host_header, headers, &body).await;
}

async fn route<W: AsyncRead + AsyncWrite + Unpin>(
    shared: &Shared,
    out: &mut W,
    method: &str,
    req_path: &str,
    host_header: &str,
    headers: crate::kernel::traffic::Headers,
    body: &[u8],
) {
    let t0 = now_ms();
    let host = hostname_of(host_header);
    let scheme = shared.scheme();
    let url =
        if req_path.starts_with("http") { req_path.to_string() } else { format!("{scheme}://{host_header}{req_path}") };
    let req_body = if body.is_empty() { String::new() } else { base64::engine::general_purpose::STANDARD.encode(body) };
    let current_port = shared.core.port();

    let cfg = (shared.resolve_config)();
    let mut base = RequestLogEntry {
        id: mk_id(),
        ts: t0,
        method: method.to_string(),
        url: url.clone(),
        host: host.clone(),
        req_headers: headers.clone(),
        req_body,
        origin: Some("reverse".to_string()),
        ..Default::default()
    };
    let Some(cfg) = cfg else {
        send_html(out, 503, &build_unserved_page()).await;
        return;
    };
    base.workspace_id = Some(str_of(&cfg, "activeWorkspaceId").to_string());

    let routing = Routing::from_cfg(&cfg);
    let env = routing.env.as_ref();
    let mapping = routing.mappings.iter().find(|m| m.enabled && m.domain == host);
    let is_localhost_domain = host.ends_with(".localhost");

    if mapping.is_none() && !is_localhost_domain {
        send_html(out, 200, &build_home_page(&cfg, scheme, current_port)).await;
        return;
    }

    // Mocks written for a name's `http://` address keep applying once the name is served over HTTPS.
    let mocks = &routing.mocks;
    let http_url = format!("http://{host_header}{req_path}");
    let matched = match_mock(mocks, method, &url, env)
        .or_else(|| if shared.is_secure() { match_mock(mocks, method, &http_url, env) } else { None });
    if let Some(mock) = matched {
        let mock_target = Some(format!("mock:{}", mock.id));
        if is_fully_mocked(mock) {
            let resolved = resolve_mock_only_response(mock, env);
            emit_log(&crate::proxy::finished_entry(
                &base,
                resolved.status,
                "mock",
                mock_target,
                now_ms() - t0 + resolved.delay_ms as i64,
                resolved.headers.clone(),
                log_body(&resolved.body),
            ));
            let _ = if is_streaming_mock(mock) {
                serve_mock(out, mock, env).await
            } else {
                serve_resolved_response(out, &resolved).await
            };
            return;
        }

        let Some(mapping) = mapping else {
            send_html(
                out,
                502,
                &format!(
                    "<h1>502 Bad Gateway</h1><p>No downstream mapping exists for <code>{host}</code>, so this partial \
                     mock cannot fetch live headers.</p>"
                ),
            )
            .await;
            emit_log(&error_entry(&base, 502, now_ms() - t0));
            return;
        };
        let target = mapping.target.as_deref().unwrap_or("");
        let upstream: Result<UpstreamResponse, String> =
            crate::proxy::fetch_upstream_response(method, target, req_path, &headers, body, None, "").await;
        match upstream {
            Ok(upstream) => {
                let merged = merge_mock_with_upstream(mock, &upstream, env);
                emit_log(&crate::proxy::finished_entry(
                    &base,
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
                    &format!("<h1>502 Bad Gateway</h1><p>Could not connect to <code>{target}</code>.</p>"),
                )
                .await;
                emit_log(&error_entry(&base, 502, now_ms() - t0));
            }
        }
        return;
    }

    match mapping {
        Some(mapping) => {
            let target = mapping.target.as_deref().unwrap_or("");
            let via = if is_localhost_domain { "rfc6761" } else { "mapping" };
            let outcome = proxy_to_upstream(out, method, target, req_path, &headers, body, Some(&base.id)).await;
            if let Some(outcome) = outcome {
                log_outcome(&base, via, Some(target.to_string()), outcome);
            }
        }
        None => {
            send_html(out, 404, &build_not_mapped_page(&host, scheme, current_port)).await;
            emit_log(&error_entry(&base, 404, now_ms() - t0));
        }
    }
}
