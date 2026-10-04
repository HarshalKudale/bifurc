//! What happens to a deletion after it is made (see `workspace::deletions`): `entity.confirmDelete` and
//! `folder.confirmDelete` commit it (and push it, with a remote), after which it is gone for good;
//! `entity.revertDelete` and `folder.revertDelete` put the files back from `HEAD`.

use serde_json::{Map, Value, json};

use super::{
    git_ops::{discard_changes, extract_name_entry, sync_changes},
    publish_service::singular_kind,
    sync_commands::after_publish,
};
use crate::{
    kernel::{CommandRegistry, kinds::to_engine_kind},
    protocol::{EngineError, Result},
    runtime::{entity_status::emit_entity_status, instances::notify_config_changed},
    utils::{
        id::{generate_id, now_ms},
        json::{merged, opt_str, str_of},
    },
    workspace::{
        accounts::commit_identity,
        deletions::{
            folder_subtree, forget_deleted, is_git_tracked, pending_entity, pending_folders, prune_pending_folders,
            record_path, restore_folder_chain,
        },
        fs::{
            ALL_ENTITY_KINDS, FLAT_ENTITY_KINDS, SKIP_DIRS, add_pending_deletion, find_entity_file,
            get_pending_deletions, pending_folders_key, read_index, read_names_index, remove_pending_deletion,
            sanitize_dir_name,
        },
        git_store::get_entity_at_commit,
        kinds::folder_store_kind,
        settings::load_settings,
        status_cache::invalidate_cache,
    },
};

/// The id of the folder a deleted file was in, by its directory: the folder that has that directory, else
/// the deleted folder that had it, else a deleted folder made for it now.
fn folder_of_directory(ws_id: &str, kind: &str, dir_name: &str) -> std::io::Result<String> {
    let has_directory = |folder: &&Value| sanitize_dir_name(str_of(folder, "name")) == dir_name;
    if let Some(folder) = read_index(ws_id, kind).folders.iter().find(has_directory) {
        return Ok(str_of(folder, "id").to_string());
    }
    if let Some(folder) = pending_folders(ws_id, kind).iter().find(has_directory) {
        return Ok(str_of(folder, "id").to_string());
    }
    let id = generate_id();
    let folder = json!({ "name": dir_name, "parentId": null, "id": id, "createdAt": now_ms(), "workspaceId": ws_id });
    add_pending_deletion(ws_id, &pending_folders_key(kind), folder)?;
    Ok(id)
}

/// Make the deletion records agree with the repository. An entity file git knows that is missing from the
/// workspace without a record (it was removed by hand, or by an engine that kept no record) gets one, so it
/// is listed as deleted and can be confirmed or reverted like any other. A record whose file is back is
/// dropped. `status` is the workspace's per-file status. Returns whether anything changed.
pub async fn reconcile_deletions(ws_id: &str, status: &Map<String, Value>) -> bool {
    let mut changed = false;
    for kind in ALL_ENTITY_KINDS {
        for record in get_pending_deletions(ws_id, kind) {
            let id = str_of(&record, "id");
            if find_entity_file(ws_id, kind, id).is_some() && remove_pending_deletion(ws_id, kind, id).is_ok() {
                changed = true;
            }
        }
    }

    for (path, state) in status {
        if state != "deleted" {
            continue;
        }
        let parts: Vec<&str> = path.split('/').collect();
        let (kind, folder_dir, file) = match parts.as_slice() {
            [kind, file] => (*kind, None, *file),
            [kind, dir, file] => (*kind, Some(*dir), *file),
            _ => continue,
        };
        let Some(id) = file.strip_suffix(".json") else { continue };
        if !ALL_ENTITY_KINDS.contains(&kind)
            || folder_dir.is_some_and(|dir| SKIP_DIRS.contains(&dir) || FLAT_ENTITY_KINDS.contains(&kind))
            || pending_entity(ws_id, kind, id).is_some()
            || find_entity_file(ws_id, kind, id).is_some()
        {
            continue;
        }
        let folder_id = match folder_dir {
            Some(dir) => match folder_of_directory(ws_id, kind, dir) {
                Ok(folder_id) => json!(folder_id),
                Err(_) => continue,
            },
            None => Value::Null,
        };
        let listed = match read_names_index(ws_id, kind).get(id) {
            Some(entry) => Some(entry.clone()),
            None => match get_entity_at_commit("HEAD", ws_id, path).await {
                Some(entity) => extract_name_entry(kind, &entity),
                None => None,
            },
        };
        let record = merged(&json!({ "id": id, "folderId": folder_id, "path": path }), &listed.unwrap_or(json!({})));
        if add_pending_deletion(ws_id, kind, record).is_ok() {
            changed = true;
        }
    }
    changed
}

fn active_workspace(params: &Value) -> String {
    opt_str(params, "workspaceId").map(str::to_string).unwrap_or_else(|| load_settings().active_workspace_id())
}

fn audit_body(ws_id: &str, entity_id: &str) -> String {
    format!("entity-id: {entity_id}\nworkspace-id: {ws_id}\nactor: {}", commit_identity(ws_id).actor)
}

/// Commit the deletion of `records` (and push it, with a remote). A record git no longer knows has nothing
/// left to commit and is simply forgotten.
async fn confirm(ws_id: &str, kind: &str, records: &[Value], message: String) -> Value {
    let mut paths = Vec::new();
    for record in records {
        let path = record_path(kind, record);
        if is_git_tracked(ws_id, &path).await {
            paths.push(path);
        } else if let Err(e) = forget_deleted(ws_id, kind, str_of(record, "id")) {
            return json!({ "ok": false, "error": e.to_string() });
        }
    }
    let result = if paths.is_empty() {
        json!({ "ok": true, "commit": null })
    } else {
        sync_changes(ws_id, &paths, Some(message)).await
    };
    let _ = prune_pending_folders(ws_id, kind);
    invalidate_cache(ws_id);
    notify_config_changed();
    after_publish(ws_id).await;
    result
}

/// Put the files of `records` back from `HEAD`. Reports the first that could not be put back.
async fn revert(ws_id: &str, kind: &str, records: &[Value]) -> Value {
    let mut failure = None;
    for record in records {
        let path = record_path(kind, record);
        let result = if is_git_tracked(ws_id, &path).await {
            discard_changes(ws_id, &path).await
        } else {
            json!({ "ok": false, "error": "Its deletion is already committed. Confirm the delete to finish it." })
        };
        if result.get("ok") != Some(&Value::Bool(true)) {
            failure.get_or_insert(result);
        }
    }
    invalidate_cache(ws_id);
    notify_config_changed();
    emit_entity_status(ws_id);
    failure.unwrap_or_else(|| json!({ "ok": true }))
}

fn deleted_entity(ws_id: &str, kind: &str, id: &str) -> Result<Value> {
    pending_entity(ws_id, kind, id).ok_or_else(|| EngineError::not_found("It is not waiting to be deleted."))
}

/// The deleted folder `id` with the deleted folders under it, and the deleted entities in them.
fn deleted_folder(ws_id: &str, kind: &str, id: &str) -> Result<(Value, Vec<String>, Vec<Value>)> {
    let folders = pending_folders(ws_id, kind);
    let folder = folders
        .iter()
        .find(|f| str_of(f, "id") == id)
        .cloned()
        .ok_or_else(|| EngineError::not_found("The folder is not waiting to be deleted."))?;
    let subtree = folder_subtree(&folders, id);
    let records = get_pending_deletions(ws_id, kind)
        .into_iter()
        .filter(|record| opt_str(record, "folderId").is_some_and(|f| subtree.iter().any(|s| s == f)))
        .collect();
    Ok((folder, subtree, records))
}

fn store_kind_of_folder(params: &Value) -> Result<&'static str> {
    let kind = str_of(params, "kind");
    folder_store_kind(kind).ok_or_else(|| EngineError::engine(format!("Unknown folder kind \"{kind}\".")))
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("entity.confirmDelete", |params, _| async move {
        let (ws_id, kind, id) = (active_workspace(&params), to_engine_kind(str_of(&params, "kind")), str_of(&params, "id"));
        let record = deleted_entity(&ws_id, kind, id)?;
        let name = opt_str(&record, "name").filter(|name| !name.is_empty()).unwrap_or(id);
        let message = format!("delete {} {name}\n\n{}", singular_kind(kind), audit_body(&ws_id, id));
        Ok(confirm(&ws_id, kind, &[record], message).await)
    });

    registry.register("entity.revertDelete", |params, _| async move {
        let (ws_id, kind, id) = (active_workspace(&params), to_engine_kind(str_of(&params, "kind")), str_of(&params, "id"));
        let record = deleted_entity(&ws_id, kind, id)?;
        Ok(revert(&ws_id, kind, &[record]).await)
    });

    registry.register("folder.confirmDelete", |params, _| async move {
        let (ws_id, kind, id) = (active_workspace(&params), store_kind_of_folder(&params)?, str_of(&params, "id"));
        let (folder, _, records) = deleted_folder(&ws_id, kind, id)?;
        let message = format!("delete folder {}\n\n{}", str_of(&folder, "name"), audit_body(&ws_id, id));
        Ok(confirm(&ws_id, kind, &records, message).await)
    });

    registry.register("folder.revertDelete", |params, _| async move {
        let (ws_id, kind, id) = (active_workspace(&params), store_kind_of_folder(&params)?, str_of(&params, "id"));
        let (_, subtree, records) = deleted_folder(&ws_id, kind, id)?;
        for folder_id in &subtree {
            restore_folder_chain(&ws_id, kind, folder_id)?;
        }
        Ok(revert(&ws_id, kind, &records).await)
    });
}
