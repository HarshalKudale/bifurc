use std::sync::Arc;

use serde_json::Value;

use crate::runtime::instances::{InertListener, Listener};

/// The edge: the reverse proxy for `*.localhost` mappings and the home page. `resolve_config` yields the
/// routing config of the active workspace, or `None` when this engine does not serve it.
pub fn create_edge_router(_resolve_config: Arc<dyn Fn() -> Option<Value> + Send + Sync>) -> Arc<dyn Listener> {
    Arc::new(InertListener::default())
}
