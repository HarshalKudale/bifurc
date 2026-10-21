//! The forward proxy: HTTP proxying with mocks and rules, CONNECT tunnelling and TLS interception, and the
//! commands that control the listeners.

mod commands;
mod handler;
pub(crate) mod listener;
mod response;
mod routing;
mod server;
#[cfg(test)]
mod tests;
pub(crate) mod tls_intercept;
pub(crate) mod wire;

use std::sync::Arc;

use crate::{kernel::CommandRegistry, runtime::instances::Listener};

pub(crate) use handler::proxy_to_upstream;
pub(crate) use response::{fetch_upstream_response, forward_streaming, log_body, send_html};
pub(crate) use routing::{error_entry, finished_entry, log_outcome};

pub fn register(registry: &mut CommandRegistry) {
    commands::register(registry);
    crate::mocks::register(registry);
}

/// The forward proxy: the port a browser's proxy setting points at.
pub fn create_proxy_server() -> Arc<dyn Listener> {
    Arc::new(server::ProxyServer::new())
}
