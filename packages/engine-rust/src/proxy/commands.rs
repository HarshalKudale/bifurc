//! `server.status`, `server.start`, `server.stop`, `server.restart` and `proxy.status`: control and
//! inspection of the two listeners, the forward proxy and (with `target: "reverse"`) the edge.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::server::ProxyServer;
use crate::{
    kernel::CommandRegistry,
    runtime::instances::{Listener, bind_forward_proxy, edge, forward_proxy},
    utils::json::opt_str,
    workspace::settings::load_settings,
};

/// The `workspaceId` echo for a status result, present only when the caller supplied one, so a multiplexed
/// client can match a response to its request.
fn echo_workspace(params: &Value, out: &mut Map<String, Value>) {
    if let Some(id) = opt_str(params, "workspaceId") {
        out.insert("workspaceId".to_string(), json!(id));
    }
}

fn is_reverse(params: &Value) -> bool {
    opt_str(params, "target") == Some("reverse")
}

/// The forward proxy commands act on, created on first use when nothing bound one.
fn forward_listener() -> Arc<dyn Listener> {
    forward_proxy().unwrap_or_else(|| {
        let created: Arc<dyn Listener> = Arc::new(ProxyServer::new());
        bind_forward_proxy(Some(created.clone()));
        created
    })
}

/// The port to start the forward proxy on: the one it was last started on, else the configured one.
fn forward_port_in_force() -> u16 {
    match forward_proxy().map(|proxy| proxy.port()) {
        Some(port) if port != 0 => port,
        _ => load_settings().forward_proxy_port(),
    }
}

fn reverse_port_in_force(reverse: &dyn Listener) -> u16 {
    match reverse.port() {
        0 => load_settings().port(),
        port => port,
    }
}

fn status_of(running: bool, port: u16, error: Option<String>, params: &Value) -> Value {
    let mut out = Map::new();
    out.insert("running".to_string(), json!(running));
    out.insert("port".to_string(), json!(port));
    out.insert("error".to_string(), json!(error));
    echo_workspace(params, &mut out);
    Value::Object(out)
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("server.status", |params, _| {
        if is_reverse(&params) {
            return Ok(match edge() {
                Some(reverse) => status_of(reverse.is_running(), reverse.port(), reverse.error(), &params),
                None => status_of(false, 0, Some("There is no reverse proxy in this engine.".to_string()), &params),
            });
        }
        Ok(match forward_proxy() {
            Some(proxy) => status_of(proxy.is_running(), proxy.port(), proxy.error(), &params),
            None => status_of(false, 0, None, &params),
        })
    });

    registry.register_sync("server.start", |params, _| {
        if is_reverse(&params) {
            if let Some(reverse) = edge() {
                reverse.start(reverse_port_in_force(reverse.as_ref()));
            }
            return Ok(json!({ "ok": true }));
        }
        let port = forward_port_in_force();
        forward_listener().start(port);
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("server.stop", |params, _| {
        if is_reverse(&params) {
            if let Some(reverse) = edge() {
                reverse.stop();
            }
        } else if let Some(proxy) = forward_proxy() {
            proxy.stop();
        }
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("server.restart", |params, _| {
        if is_reverse(&params) {
            if let Some(reverse) = edge() {
                let port = reverse_port_in_force(reverse.as_ref());
                reverse.stop();
                reverse.start(port);
            }
            return Ok(json!({ "ok": true }));
        }
        let port = forward_port_in_force();
        if let Some(proxy) = forward_proxy() {
            proxy.stop();
        }
        forward_listener().start(port);
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("proxy.status", |_params, _| {
        Ok(json!({ "running": forward_proxy().is_some_and(|proxy| proxy.is_running()) }))
    });
}
