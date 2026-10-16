//! The in-memory sync state of each workspace (what the UI's sync indicator shows) and the sync configuration
//! persisted in `app.json`.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::{
    kernel::bus,
    workspace::settings::{load_settings, save_settings},
};

/// `idle` | `pushing` | `pulling` | `cloning` | `error`, plus the last error, progress line and the entities
/// the most recent pull changed.
#[derive(Debug, Clone)]
pub struct SyncState {
    pub status: String,
    pub error: Option<String>,
    pub last_pushed_at: Option<i64>,
    pub last_pulled_at: Option<i64>,
    pub progress_message: Option<String>,
    pub updated_ids: Option<Vec<String>>,
}

impl Default for SyncState {
    fn default() -> Self {
        SyncState {
            status: "idle".to_string(),
            error: None,
            last_pushed_at: None,
            last_pulled_at: None,
            progress_message: None,
            updated_ids: None,
        }
    }
}

impl SyncState {
    pub fn to_value(&self) -> Value {
        let mut out = Map::new();
        out.insert("status".into(), json!(self.status));
        out.insert("error".into(), json!(self.error));
        out.insert("lastPushedAt".into(), json!(self.last_pushed_at));
        out.insert("lastPulledAt".into(), json!(self.last_pulled_at));
        out.insert("progressMessage".into(), json!(self.progress_message));
        if let Some(ids) = &self.updated_ids {
            out.insert("updatedIds".into(), json!(ids));
        }
        Value::Object(out)
    }
}

static STATES: Mutex<Option<HashMap<String, SyncState>>> = Mutex::new(None);

pub fn get_state(ws_id: &str) -> SyncState {
    STATES.lock().get_or_insert_with(HashMap::new).entry(ws_id.to_string()).or_default().clone()
}

/// Change the state through `patch` (fields it leaves alone keep their value).
pub fn set_state(ws_id: &str, patch: impl FnOnce(&mut SyncState)) {
    let mut guard = STATES.lock();
    patch(guard.get_or_insert_with(HashMap::new).entry(ws_id.to_string()).or_default());
}

/// Publish the workspace's current state as a `sync.status` event.
pub fn emit(ws_id: &str) {
    let mut payload = Map::new();
    payload.insert("wsId".into(), json!(ws_id));
    if let Value::Object(state) = get_state(ws_id).to_value() {
        payload.extend(state);
    }
    bus().emit("sync.status", Value::Object(payload));
}

/// The state as `sync.getState` reports it: the push and pull times come from the persisted metadata.
pub fn get_sync_state(ws_id: &str) -> Value {
    let meta = get_sync_meta(ws_id);
    let time =
        |key: &str| meta.as_ref().and_then(|m| m.get(key)).cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null);
    let mut state = get_state(ws_id).to_value();
    state["lastPushedAt"] = time("lastPushedAt");
    state["lastPulledAt"] = time("lastPulledAt");
    state
}

fn workspace_field(ws_id: &str, key: &str) -> Option<Value> {
    load_settings().workspace(ws_id)?.get(key).filter(|v| !v.is_null()).cloned()
}

/// The workspace's `{ remote, branch, repo, owner }`, if a remote is configured.
pub fn get_sync_config(ws_id: &str) -> Option<Value> {
    workspace_field(ws_id, "syncConfig")
}

/// The workspace's `{ lastPushedAt, lastPulledAt, lastSyncedCommit }`.
pub fn get_sync_meta(ws_id: &str) -> Option<Value> {
    workspace_field(ws_id, "syncMeta")
}

fn save_workspace_field(ws_id: &str, key: &str, value: Option<Value>) {
    let mut settings = load_settings();
    if let Some(workspace) = settings.workspace_mut(ws_id) {
        workspace[key] = value.unwrap_or(Value::Null);
    }
    let _ = save_settings(&settings);
}

pub fn save_sync_config(ws_id: &str, config: Option<Value>) {
    save_workspace_field(ws_id, "syncConfig", config);
}

pub fn save_sync_meta(ws_id: &str, meta: Option<Value>) {
    save_workspace_field(ws_id, "syncMeta", meta);
}

pub fn sync_meta_value(
    last_pushed_at: Option<i64>,
    last_pulled_at: Option<i64>,
    last_synced_commit: Option<&str>,
) -> Value {
    json!({ "lastPushedAt": last_pushed_at, "lastPulledAt": last_pulled_at, "lastSyncedCommit": last_synced_commit })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_data_root;

    #[test]
    fn state_defaults_to_idle_and_omits_updated_ids() {
        let state = SyncState::default().to_value();
        assert_eq!(state["status"], "idle");
        assert!(state.get("updatedIds").is_none());
        assert_eq!(state["progressMessage"], Value::Null);
    }

    #[test]
    fn config_and_meta_persist_in_app_json() {
        let root = scratch_data_root();
        assert!(get_sync_config(root.ws_id).is_none());
        save_sync_config(root.ws_id, Some(json!({ "remote": "r", "branch": "main" })));
        save_sync_meta(root.ws_id, Some(sync_meta_value(Some(5), None, None)));
        assert_eq!(get_sync_config(root.ws_id).unwrap()["branch"], "main");
        let state = get_sync_state(root.ws_id);
        assert_eq!(state["lastPushedAt"], 5);
        assert_eq!(state["lastPulledAt"], Value::Null);
        save_sync_config(root.ws_id, None);
        assert!(get_sync_config(root.ws_id).is_none());
    }
}
