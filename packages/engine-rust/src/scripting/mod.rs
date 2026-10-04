//! The JavaScript sandbox for user scripts (request pre/post/test scripts and proxy rule scripts) and the
//! `script.execute` command.

mod sandbox;

pub use sandbox::{
    ProxyScriptResult, SCRIPT_TIMEOUT_MS, execute_ipc_script, execute_request_script, execute_response_script,
};

use crate::kernel::CommandRegistry;

pub fn register(registry: &mut CommandRegistry) {
    registry.register("script.execute", |params, _| async move { Ok(execute_ipc_script(params).await) });
}
