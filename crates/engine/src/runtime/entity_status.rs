//! Publish a workspace's git state after something changed in it (bus event `sync.entityStatus`).

use serde_json::json;

use crate::{
    gitops::{get_workspace_git_state, get_workspace_sync_status, reconcile_deletions},
    kernel::bus,
};

/// Compute the workspace's entity statuses and git state in the background, then emit them.
/// Clients re-read the workspace on this event (`configChanged: true`).
pub fn emit_entity_status(ws_id: &str) {
    emit(ws_id, false);
}

/// Like [`emit_entity_status`], for a change an `entity.changed` event already described in full (a listing
/// update or a delete): the event says `configChanged: false`, so clients only refresh the git state. A
/// reconciled deletion still says `true`.
pub fn emit_entity_status_after_change(ws_id: &str) {
    emit(ws_id, true);
}

fn emit(ws_id: &str, announced: bool) {
    let ws_id = ws_id.to_string();
    tokio::spawn(async move {
        let mut status = get_workspace_sync_status(&ws_id).await;
        // Clients re-read the workspace on this event, so the deletion records are settled before it.
        let reconciled = reconcile_deletions(&ws_id, &status).await;
        if reconciled {
            status = get_workspace_sync_status(&ws_id).await;
        }
        let git = get_workspace_git_state(&ws_id).await;
        let config_changed = reconciled || !announced;
        bus().emit(
            "sync.entityStatus",
            json!({ "wsId": ws_id, "status": status, "git": git, "configChanged": config_changed }),
        );
    });
}
