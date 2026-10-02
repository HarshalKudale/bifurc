use std::sync::Arc;

use crate::{
    kernel::CommandRegistry,
    runtime::instances::{InertListener, Listener},
};

pub fn register(_registry: &mut CommandRegistry) {}

/// The forward proxy: the port a browser's proxy setting points at.
pub fn create_proxy_server() -> Arc<dyn Listener> {
    Arc::new(InertListener::default())
}
