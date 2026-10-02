use serde_json::{Map, Value};

use crate::kernel::CommandRegistry;

pub fn register(_registry: &mut CommandRegistry) {}

/// Entity id -> `clean` | `modified` | `new` | `deleted`, for every entity of the workspace.
pub async fn get_workspace_sync_status(_ws_id: &str) -> Map<String, Value> {
    Map::new()
}

/// `{ hasRemote, branch, ahead, behind }` of the workspace's repository.
pub async fn get_workspace_git_state(_ws_id: &str) -> Value {
    Value::Null
}

/// The workspace's `{ remote, branch, autoSync }`, if a remote is configured.
pub fn get_sync_config(_ws_id: &str) -> Option<Value> {
    None
}

pub fn start_auto_sync(_ws_id: &str) {}

pub fn stop_all_auto_sync() {}
