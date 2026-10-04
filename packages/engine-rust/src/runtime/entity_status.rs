//! Publish a workspace's git state after something changed in it (bus event `sync.entityStatus`).

use serde_json::json;

use crate::{
    gitops::{get_workspace_git_state, get_workspace_sync_status, reconcile_deletions},
    kernel::bus,
};

/// Compute the workspace's entity statuses and git state in the background, then emit them.
pub fn emit_entity_status(ws_id: &str) {
    let ws_id = ws_id.to_string();
    tokio::spawn(async move {
        let mut status = get_workspace_sync_status(&ws_id).await;
        // Clients re-read the workspace on this event, so the deletion records are settled before it.
        if reconcile_deletions(&ws_id, &status).await {
            status = get_workspace_sync_status(&ws_id).await;
        }
        let git = get_workspace_git_state(&ws_id).await;
        bus().emit("sync.entityStatus", json!({ "wsId": ws_id, "status": status, "git": git }));
    });
}
