//! The two listeners the engine runs — the forward proxy and the edge (reverse proxy) — as the rest of the
//! engine sees them: something to start, stop, ask about, and tell that the config changed.

use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::Value;

use crate::workspace::settings::AppSettings;

pub trait Listener: Send + Sync {
    /// Bind `port` on loopback and serve. Replaces a running listener. Failure is reported by `error()`.
    fn start(&self, port: u16);
    fn stop(&self);
    fn is_running(&self) -> bool;
    /// The port last asked for; `0` before the first start.
    fn port(&self) -> u16;
    fn error(&self) -> Option<String>;
    /// Re-read the routing config from disk.
    fn reload_config(&self) {}
    /// The config the listener routes with (the forward proxy only).
    fn dispatch_config(&self) -> Option<Value> {
        None
    }
}

static FORWARD_PROXY: RwLock<Option<Arc<dyn Listener>>> = RwLock::new(None);
static EDGE: RwLock<Option<Arc<dyn Listener>>> = RwLock::new(None);

pub fn bind_forward_proxy(instance: Option<Arc<dyn Listener>>) {
    *FORWARD_PROXY.write() = instance;
}

pub fn forward_proxy() -> Option<Arc<dyn Listener>> {
    FORWARD_PROXY.read().clone()
}

pub fn bind_edge(instance: Option<Arc<dyn Listener>>) {
    *EDGE.write() = instance;
}

pub fn edge() -> Option<Arc<dyn Listener>> {
    EDGE.read().clone()
}

/// Tell the proxies that entities or settings changed on disk.
pub fn notify_config_changed() {
    if let Some(proxy) = forward_proxy() {
        proxy.reload_config();
    }
}

fn tls_changed(next: &AppSettings, prev: &AppSettings) -> bool {
    next.tls_enabled() != prev.tls_enabled()
        || next.tls_ca_cert_path() != prev.tls_ca_cert_path()
        || next.tls_ca_key_path() != prev.tls_ca_key_path()
}

/// Whether the edge has to be restarted for the new settings: its port, or what decides whether it serves HTTPS.
fn edge_changed(next: &AppSettings, prev: &AppSettings) -> bool {
    next.port() != prev.port() || tls_changed(next, prev)
}

/// After a whole config was saved: make sure both listeners run, on the ports now configured.
pub fn rebind_runtime_ports(incoming: &AppSettings, prev: &AppSettings) {
    // Before the first run's servers step nothing is started that was not asked for.
    if !incoming.servers_confirmed() {
        return rebind_changed_listeners(incoming, prev);
    }
    if let Some(edge) = edge() {
        if !edge.is_running() || edge_changed(incoming, prev) {
            edge.start(incoming.port());
        }
    }
    if let Some(proxy) = forward_proxy() {
        if !proxy.is_running()
            || incoming.forward_proxy_port() != prev.forward_proxy_port()
            || tls_changed(incoming, prev)
        {
            proxy.start(incoming.forward_proxy_port());
        }
    }
}

/// After a settings patch: restart only a listener whose own settings changed, and only if it was running
/// or had failed to bind (a listener the user stopped stays stopped).
pub fn rebind_changed_listeners(next: &AppSettings, prev: &AppSettings) {
    if let Some(edge) = edge() {
        if edge_changed(next, prev) && (edge.is_running() || edge.error().is_some()) {
            edge.start(next.port());
        }
    }
    if let Some(proxy) = forward_proxy() {
        let changed = next.forward_proxy_port() != prev.forward_proxy_port() || tls_changed(next, prev);
        if changed && (proxy.is_running() || proxy.error().is_some()) {
            proxy.start(next.forward_proxy_port());
        }
    }
}

fn listener_status(listener: &dyn Listener) -> Value {
    serde_json::json!({ "running": listener.is_running(), "port": listener.port(), "error": listener.error() })
}

/// Starts each of the two listeners that is not already serving its configured port, and says how each fared
/// (`{forward, reverse}`, each `{running, port, error}` or `null` for a listener this engine does not have).
pub fn start_listeners(settings: &AppSettings) -> Value {
    let reverse = edge().map(|edge| {
        if !edge.is_running() || edge.port() != settings.port() {
            edge.start(settings.port());
        }
        listener_status(edge.as_ref())
    });
    let forward = forward_proxy().map(|proxy| {
        if !proxy.is_running() || proxy.port() != settings.forward_proxy_port() {
            proxy.start(settings.forward_proxy_port());
        }
        listener_status(proxy.as_ref())
    });
    serde_json::json!({ "forward": forward, "reverse": reverse })
}

/// A listener that cannot serve; `error()` says so.
#[derive(Default)]
pub struct InertListener;

impl Listener for InertListener {
    fn start(&self, _port: u16) {}
    fn stop(&self) {}
    fn is_running(&self) -> bool {
        false
    }
    fn port(&self) -> u16 {
        0
    }
    fn error(&self) -> Option<String> {
        Some("this listener is not available in this build".to_string())
    }
}
