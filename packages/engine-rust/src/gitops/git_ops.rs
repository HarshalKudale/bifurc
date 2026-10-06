//! Per-file git operations: diff against `HEAD`, discard, publish-and-clean-up and history.

use serde_json::{Value, json};

use super::{
    git_lock::{with_index_lock_retry, workspace_lock},
    publish_service::{PublishOptions, publish_entities},
    status_tracker::{get_workspace_sync_status, invalidate_cache},
};
use crate::{
    utils::json::{opt_str, str_of, truthy},
    workspace::{
        deletions::{forget_deleted, pending_entity, prune_pending_folders, record_path, restore_folder_chain},
        fs::{
            ALL_ENTITY_KINDS, FLAT_ENTITY_KINDS, find_entity_rel_path, get_pending_deletions, read_entity,
            remove_name_entry, remove_pending_deletion, upsert_name_entry, ws_dir,
        },
        git_store::{QueryLogOptions, git, query_log},
        kinds::kind_name_entry,
    },
};

/// The name entry a kind wants recorded for an entity: the kind's own registration first, then a fallback
/// for kinds that registered none.
pub fn extract_name_entry(kind: &str, entity: &Value) -> Option<Value> {
    if let Some(entry) = kind_name_entry(kind, entity) {
        return Some(entry);
    }
    if !entity.is_object() {
        return None;
    }
    let text = |key: &str| str_of(entity, key).to_string();
    let text_or = |key: &str, default: &str| entity.get(key).and_then(Value::as_str).unwrap_or(default).to_string();
    Some(match kind {
        "rules" => json!({ "name": text("name"), "url": text("pattern") }),
        "mocks" => json!({ "name": text("name"), "method": text_or("method", "GET"), "url": text("urlPattern") }),
        "requests" => json!({ "name": text("name"), "method": text_or("method", "GET"), "url": text("url") }),
        "sockets" => json!({ "name": text("name"), "url": text("url") }),
        "socketioRequests" => json!({ "name": text("name"), "url": text("url"), "namespace": text("namespace") }),
        "mcpRequests" => json!({ "name": text("name"), "transport": text("transport"), "url": text("url") }),
        "soapRequests" => {
            json!({ "name": text("name"), "endpointUrl": text("endpointUrl"), "soapAction": text("soapAction") })
        }
        "graphqlRequests" => json!({ "name": text("name"), "endpointUrl": text("endpointUrl") }),
        "sseRequests" => json!({ "name": text("name"), "method": text_or("method", "GET"), "url": text("url") }),
        "grpcRequests" | "environments" => json!({ "name": text("name") }),
        "mappings" => {
            let name = if truthy(entity, "label") { text("label") } else { text("domain") };
            json!({ "name": name, "url": text("domain") })
        }
        _ => {
            if truthy(entity, "name") {
                json!({ "name": text("name") })
            } else {
                return None;
            }
        }
    })
}

/// The file's status and, where there is one, its difference from `HEAD`.
pub async fn get_file_diff(ws_id: &str, rel_path: &str) -> Value {
    let normalized = rel_path.replace('\\', "/");
    let all = get_workspace_sync_status(ws_id).await;
    let status = all.get(&normalized).and_then(Value::as_str).unwrap_or("clean").to_string();

    match status.as_str() {
        "clean" => json!({ "hasDiff": false, "status": "clean", "diff": "" }),
        "modified" => match git(ws_id, &["diff", "HEAD", "--", &normalized]).await {
            Ok(diff) => json!({ "hasDiff": true, "status": "modified", "diff": diff }),
            Err(_) => json!({ "hasDiff": true, "status": "modified" }),
        },
        "new" => {
            let current = std::fs::read_to_string(ws_dir(ws_id).join(&normalized)).ok();
            json!({ "hasDiff": true, "status": "new", "current": current })
        }
        _ => json!({ "hasDiff": true, "status": status }),
    }
}

/// Undo the file's uncommitted changes: restore it from `HEAD`, or delete it if git does not know it. Also
/// clears its pending deletion, re-syncs `names.json` and invalidates the status cache.
pub async fn discard_changes(ws_id: &str, rel_path: &str) -> Value {
    let _guard = workspace_lock(ws_id).lock_owned().await;
    discard_locked(ws_id, rel_path).await
}

async fn discard_locked(ws_id: &str, rel_path: &str) -> Value {
    let mut normalized = rel_path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').collect();
    let kind = parts[0].to_string();
    let base = parts.last().copied().unwrap_or("").to_string();
    let entity_id = base.strip_suffix(".json").map(str::to_string);

    // A deleted entity comes back into its folder, so a deleted folder is a folder again first: the
    // directory the checkout makes must be that folder's, not a new one's.
    if let Some(record) = entity_id.as_deref().and_then(|id| pending_entity(ws_id, &kind, id)) {
        if let Some(folder_id) = opt_str(&record, "folderId") {
            if let Err(e) = restore_folder_chain(ws_id, &kind, folder_id) {
                return json!({ "ok": false, "error": e.to_string() });
            }
        }
    }

    let mut checkout_succeeded =
        with_index_lock_retry(ws_id, &["checkout", "HEAD", "--", normalized.as_str()]).await.is_ok();

    // Maybe git tracks it under a different subfolder path.
    if !checkout_succeeded && base.ends_with(".json") {
        if let Ok(listed) = git(ws_id, &["ls-files", &format!("*{base}")]).await {
            if let Some(tracked) = listed.lines().map(str::trim).find(|l| !l.is_empty()) {
                normalized = tracked.replace('\\', "/");
                checkout_succeeded =
                    with_index_lock_retry(ws_id, &["checkout", "HEAD", "--", normalized.as_str()]).await.is_ok();
            }
        }
    }

    if !checkout_succeeded {
        let mut full_path = ws_dir(ws_id).join(&normalized);
        if !full_path.exists() {
            if let Some(id) = &entity_id {
                if let Some(disk_rel) = find_entity_rel_path(ws_id, &kind, id) {
                    full_path = ws_dir(ws_id).join(disk_rel);
                }
            }
        }
        if full_path.exists() {
            if let Err(e) = std::fs::remove_file(&full_path) {
                return json!({ "ok": false, "error": e.to_string() });
            }
        }
    }

    if let Some(id) = &entity_id {
        for k in ALL_ENTITY_KINDS {
            if get_pending_deletions(ws_id, k).iter().any(|e| str_of(e, "id") == id) {
                if let Err(e) = remove_pending_deletion(ws_id, k, id) {
                    return json!({ "ok": false, "error": e.to_string() });
                }
            }
        }

        // Foldered kinds only: a flat kind has no names index, and writing one would leave a stray
        // `names.json` that the next publish commits.
        if !FLAT_ENTITY_KINDS.contains(&kind.as_str()) {
            let result = match read_entity(ws_id, &kind, id) {
                Some(restored) => match extract_name_entry(&kind, &restored) {
                    Some(entry) => upsert_name_entry(ws_id, &kind, id, entry),
                    None => Ok(()),
                },
                None => remove_name_entry(ws_id, &kind, id),
            };
            if let Err(e) = result {
                return json!({ "ok": false, "error": e.to_string() });
            }
        }
    }

    invalidate_cache(ws_id);
    json!({ "ok": true })
}

/// Commit and push the given files or folders, then clear the pending deletions they published.
pub async fn sync_changes(ws_id: &str, paths: &[String], message: Option<String>) -> Value {
    let normalized: Vec<String> = paths.iter().map(|p| p.replace('\\', "/")).collect();
    let result =
        publish_entities(PublishOptions { ws_id: ws_id.to_string(), paths: normalized.clone(), message }).await;
    if result.get("ok") == Some(&Value::Bool(true)) {
        // A deletion that was just committed is final: by its file, or by a folder it was under.
        let published = |path: &str| {
            normalized.iter().any(|p| p == path || (p.ends_with('/') && path.starts_with(p.as_str())))
        };
        for kind in ALL_ENTITY_KINDS {
            for record in get_pending_deletions(ws_id, kind) {
                if published(&record_path(kind, &record)) {
                    let _ = forget_deleted(ws_id, kind, str_of(&record, "id"));
                }
            }
            let _ = prune_pending_folders(ws_id, kind);
        }
    }
    invalidate_cache(ws_id);
    result
}

/// The audit entries that touched `rel_path`, newest first: `{ entries, total }`. Limit defaults to 100,
/// offset to 0.
pub async fn get_file_history(ws_id: &str, rel_path: &str, limit: Option<i64>, offset: Option<i64>) -> Value {
    let (entries, total) = query_log(&QueryLogOptions {
        workspace_id: ws_id.to_string(),
        file_path: Some(rel_path.replace('\\', "/")),
        limit: Some(limit.unwrap_or(100).max(0) as usize),
        offset: Some(offset.unwrap_or(0).max(0) as usize),
        ..Default::default()
    })
    .await;
    json!({ "entries": entries, "total": total })
}
