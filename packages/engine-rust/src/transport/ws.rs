use std::sync::Arc;

use super::session::SessionOptions;
use crate::protocol::{EngineError, Result};

pub struct WsServer {
    /// `ws://127.0.0.1:<port>`
    pub url: String,
    pub port: u16,
}

impl WsServer {
    pub async fn close(&self) {}
}

/// Serve sessions over WebSocket on loopback: what the browser extension and a web UI connect to.
pub async fn create_ws_server(_port: u16, _opts: SessionOptions) -> Result<Arc<WsServer>> {
    Err(EngineError::unsupported("the WebSocket transport is not available in this build"))
}
