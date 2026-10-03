//! The `sync.*`, `git.*`, `entity.publish`, `folder.publish`, `entity.restore`, `audit.diff` and `history.*`
//! commands.

use serde_json::{Map, Value, json};

use super::{
    auto_sync::{pull_and_announce, update_last_known_head},
    git_ops::{discard_changes, get_file_diff, get_file_history, sync_changes},
    git_state::get_workspace_git_state,
    remote::workspace_owner,
    status_tracker::get_workspace_sync_status,
    sync_manager::{get_remote_head, sync_push},
    sync_state::get_sync_state,
};
use crate::{
    kernel::CommandRegistry,
    runtime::{entity_status::emit_entity_status, instances::notify_config_changed},
    utils::json::{array_of, opt_str, str_of},
    workspace::{
        accounts::workspace_account,
        config::load_config,
        entity_paths::load_entity_path_maps,
        fs::sanitize_dir_name,
        git_store::{get_commit_changed_files, get_entity_at_commit, is_repo},
    },
};

fn optional_int(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
}

fn string_list(params: &Value, key: &str) -> Vec<String> {
    array_of(params, key).iter().filter_map(Value::as_str).map(str::to_string).collect()
}

/// After a publish: tell clients the entity statuses changed, and remember the remote tip so auto-sync does
/// not pull the push this engine just made. Reading the tip is bookkeeping; its failure is not reported.
async fn after_publish(ws_id: &str) {
    emit_entity_status(ws_id);
    if let Some(sha) = get_remote_head(ws_id).await {
        update_last_known_head(ws_id, &sha);
    }
}

async fn restore(ws_id: &str, rel_path: &str) -> Value {
    let result = discard_changes(ws_id, rel_path).await;
    emit_entity_status(ws_id);
    notify_config_changed();
    result
}

/// An entity's content before and after a commit, or `{ before: null, after: null }`.
async fn before_and_after(commit: &str, ws_id: &str, rel_path: &str) -> Value {
    let after = get_entity_at_commit(commit, ws_id, rel_path).await;
    let before = get_entity_at_commit(&format!("{commit}~1"), ws_id, rel_path).await;
    json!({ "before": before, "after": after })
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("sync.push", |params, _| async move { Ok(sync_push(str_of(&params, "workspaceId")).await) });

    registry.register("sync.pull", |params, _| async move {
        Ok(pull_and_announce(str_of(&params, "workspaceId")).await)
    });

    // How a sync is going, plus who the workspace syncs as, who owns it and where its remote is.
    registry.register_sync("sync.getState", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        let mut state = get_sync_state(ws_id);
        state["account"] = workspace_account(ws_id).map_or(Value::Null, |account| account.public());
        state["owner"] = workspace_owner(ws_id);
        state["gitTracked"] = json!(is_repo(ws_id));
        Ok(state)
    });

    registry.register("sync.getEntityStatus", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId");
        let (files, git) = (get_workspace_sync_status(ws_id).await, get_workspace_git_state(ws_id).await);
        let maps = load_entity_path_maps();
        let to_object = |map: indexmap::IndexMap<String, String>| {
            Value::Object(map.into_iter().map(|(k, v)| (k, Value::String(v))).collect::<Map<String, Value>>())
        };
        Ok(json!({
            "files": files,
            "git": git,
            "paths": { "entities": to_object(maps.entities), "folders": to_object(maps.folders) },
        }))
    });

    registry.register("git.diff", |params, _| async move {
        Ok(get_file_diff(str_of(&params, "workspaceId"), str_of(&params, "relPath")).await)
    });

    registry.register("git.discard", |params, _| async move {
        Ok(restore(str_of(&params, "workspaceId"), str_of(&params, "relPath")).await)
    });

    registry.register("git.sync", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId");
        let result =
            sync_changes(ws_id, &string_list(&params, "paths"), opt_str(&params, "message").map(str::to_string)).await;
        after_publish(ws_id).await;
        Ok(result)
    });

    registry.register("git.history", |params, _| async move {
        Ok(get_file_history(
            str_of(&params, "workspaceId"),
            str_of(&params, "relPath"),
            optional_int(&params, "limit"),
            optional_int(&params, "offset"),
        )
        .await)
    });

    registry.register("entity.publish", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId");
        let result =
            sync_changes(ws_id, &string_list(&params, "paths"), opt_str(&params, "message").map(str::to_string)).await;
        after_publish(ws_id).await;
        Ok(result)
    });

    registry.register("folder.publish", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId");
        let kind = str_of(&params, "kind");
        // The folder was created under the storage layer's sanitised name; the pathspec must match it.
        let folder_path = match opt_str(&params, "folderName").filter(|n| !n.is_empty()) {
            Some(name) => format!("{kind}/{}/", sanitize_dir_name(name)),
            None => format!("{kind}/"),
        };
        let result = sync_changes(ws_id, &[folder_path], None).await;
        after_publish(ws_id).await;
        Ok(result)
    });

    registry.register("entity.restore", |params, _| async move {
        Ok(restore(str_of(&params, "workspaceId"), str_of(&params, "relPath")).await)
    });

    registry.register("audit.diff", |params, _| async move {
        let (commit, entity_id, ws_id) =
            (str_of(&params, "commitHash"), str_of(&params, "entityId"), str_of(&params, "workspaceId"));
        let changed = get_commit_changed_files(commit, ws_id).await;
        let rel_path = changed.iter().find(|f| f.contains(entity_id)).or(changed.first()).cloned().unwrap_or_default();
        if rel_path.is_empty() {
            return Ok(json!({ "before": null, "after": null }));
        }
        Ok(before_and_after(commit, ws_id, &rel_path).await)
    });

    registry.register("history.list", |params, _| async move {
        let ws_id = match opt_str(&params, "workspaceId") {
            Some(id) => id.to_string(),
            None => str_of(&load_config(), "activeWorkspaceId").to_string(),
        };
        Ok(get_file_history(
            &ws_id,
            str_of(&params, "filePath"),
            optional_int(&params, "limit"),
            optional_int(&params, "offset"),
        )
        .await)
    });

    registry.register("history.diff", |params, _| async move {
        let normalized = str_of(&params, "filePath").replace('\\', "/");
        Ok(before_and_after(str_of(&params, "commitHash"), str_of(&params, "workspaceId"), &normalized).await)
    });
}
