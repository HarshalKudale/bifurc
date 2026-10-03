//! Connecting a workspace to a remote and moving commits between them: `setRemote`, `disconnect`, push, pull,
//! auto-sync and the remote head query.

use std::path::Path;

use serde_json::{Value, json};

use super::{
    git_lock::workspace_lock,
    git_sync_ops::{clone_branch, fetch_remote_head, is_remote_empty, perform_git_pull, set_local_identity},
    sync_state::{
        clear_state, emit, get_sync_config, get_sync_meta, save_sync_config, save_sync_meta, set_state, sync_meta_value,
    },
};
use crate::{
    utils::{id::now_ms, json::str_of},
    workspace::{
        fs::ws_dir,
        git_store::{git, init_workspace_repo},
        settings::{load_settings, save_settings},
    },
};

/// The entity id a workspace-relative path names (`requests/folder/abc.json` -> `abc`), if it is an entity file.
fn entity_id_from_path(path: &str) -> Option<String> {
    let base = path.rsplit('/').next()?;
    let id = base.strip_suffix(".json")?;
    if id.is_empty() || ["index", "enabled", "names"].contains(&id) { None } else { Some(id.to_string()) }
}

/// Whether the workspace holds no entities yet.
fn is_workspace_empty(ws_id: &str) -> bool {
    let dir = ws_dir(ws_id);
    let is_entity_file = |name: &str| name.ends_with(".json") && name != "index.json" && name != "enabled.json";
    for kind in ["mappings", "rules", "environments", "mocks", "requests", "sockets"] {
        let Ok(entries) = std::fs::read_dir(dir.join(kind)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_file() && is_entity_file(&name) {
                return false;
            }
            if file_type.is_dir() && name != "drafts" && name != "capture" {
                let has_entity = std::fs::read_dir(entry.path())
                    .map(|sub| sub.flatten().any(|f| is_entity_file(&f.file_name().to_string_lossy())))
                    .unwrap_or(false);
                if has_entity {
                    return false;
                }
            }
        }
    }
    true
}

fn status_idle(ws_id: &str) {
    set_state(ws_id, |s| {
        s.status = "idle".into();
        s.error = None;
        s.progress_message = None;
    });
    emit(ws_id);
}

fn progress(ws_id: &str, message: &str) {
    set_state(ws_id, |s| s.progress_message = Some(message.to_string()));
    emit(ws_id);
}

fn fail(ws_id: &str, message: String) -> Value {
    set_state(ws_id, |s| {
        s.status = "error".into();
        s.error = Some(message.clone());
        s.progress_message = None;
    });
    emit(ws_id);
    json!({ "ok": false, "error": message })
}

/// Point `origin` at `remote` and push the workspace's history to `branch`.
async fn publish_to_empty_remote(ws_id: &str, remote: &str, branch: &str) -> Result<(), String> {
    init_workspace_repo(ws_id).await.map_err(|e| e.to_string())?;
    let _ = git(ws_id, &["remote", "remove", "origin"]).await;
    git(ws_id, &["remote", "add", "origin", remote]).await.map_err(|e| e.to_string())?;
    git(ws_id, &["push", "--set-upstream", "origin", &format!("HEAD:{branch}")]).await.map_err(|e| e.to_string())?;
    save_sync_config(ws_id, Some(json!({ "remote": remote, "branch": branch, "autoSync": false })));
    save_sync_meta(ws_id, Some(sync_meta_value(Some(now_ms()), None, None)));
    status_idle(ws_id);
    Ok(())
}

/// Replace the workspace directory's content with a fresh clone of the remote, adopting the remote
/// workspace's identity when it differs. Returns the id the workspace has afterwards.
async fn clone_into_workspace(ws_id: &str, remote: &str, branch: &str) -> Result<String, String> {
    let dir = ws_dir(ws_id);
    let temp_name = format!("{}_clone_tmp", dir.file_name().map(|n| n.to_string_lossy()).unwrap_or_default());
    let temp_dir = dir.with_file_name(temp_name);
    if temp_dir.exists() {
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
    if let Some(parent) = temp_dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    clone_branch(remote, branch, &temp_dir).await.map_err(|e| e.to_string())?;

    progress(ws_id, "Unpacking files…");
    swap_in_clone(&dir, &temp_dir).map_err(|e| e.to_string())?;
    set_local_identity(&dir).await.map_err(|e| e.to_string())?;

    // The remote workspace's identity is in its workspace.json.
    let mut adopted_id = ws_id.to_string();
    let mut adopted_name: Option<String> = None;
    if let Some(info) = crate::utils::json::read_json(&dir.join("workspace.json")) {
        let remote_id = str_of(&info, "id");
        if !remote_id.is_empty() && remote_id != ws_id {
            let new_dir = ws_dir(remote_id);
            if !new_dir.exists() {
                let _ = std::fs::rename(&dir, &new_dir);
            }
            adopted_id = remote_id.to_string();
            adopted_name = info.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
        }
    }

    let mut settings = load_settings();
    if adopted_id != ws_id {
        if let Some(workspace) = settings.workspace_mut(ws_id) {
            workspace["id"] = json!(adopted_id);
            if let Some(name) = &adopted_name {
                workspace["name"] = json!(name);
            }
        }
        if settings.active_workspace_id() == ws_id {
            settings.set("activeWorkspaceId", json!(adopted_id));
        }
    }
    let _ = save_settings(&settings);

    save_sync_config(&adopted_id, Some(json!({ "remote": remote, "branch": branch, "autoSync": false })));
    save_sync_meta(&adopted_id, Some(sync_meta_value(None, Some(now_ms()), None)));
    if adopted_id != ws_id {
        clear_state(ws_id);
    }
    Ok(adopted_id)
}

fn swap_in_clone(dir: &Path, temp_dir: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if entry.file_type()?.is_dir() { std::fs::remove_dir_all(&path)? } else { std::fs::remove_file(&path)? }
    }
    for entry in std::fs::read_dir(temp_dir)?.flatten() {
        std::fs::rename(entry.path(), dir.join(entry.file_name()))?;
    }
    std::fs::remove_dir_all(temp_dir)
}

/// Connect the workspace to `remote`: push an empty remote the workspace's content, or clone a non-empty
/// remote into an empty workspace.
pub async fn set_remote(ws_id: &str, remote: &str, branch: &str) -> Value {
    set_state(ws_id, |s| {
        s.status = "cloning".into();
        s.error = None;
        s.progress_message = Some("Connecting…".into());
    });
    emit(ws_id);

    if remote.starts_with('-') || branch.starts_with('-') {
        return fail(ws_id, "Invalid remote or branch".to_string());
    }

    let _guard = workspace_lock(ws_id).lock_owned().await;
    let remote_empty = match is_remote_empty(remote).await {
        Ok(empty) => empty,
        Err(e) => return fail(ws_id, e),
    };

    if is_workspace_empty(ws_id) {
        if remote_empty {
            progress(ws_id, "Pushing initial commit…");
            return match publish_to_empty_remote(ws_id, remote, branch).await {
                Ok(()) => json!({ "ok": true, "cloned": false }),
                Err(e) => fail(ws_id, e),
            };
        }
        progress(ws_id, "Cloning remote…");
        return match clone_into_workspace(ws_id, remote, branch).await {
            Ok(adopted_id) => {
                set_state(&adopted_id, |s| {
                    s.status = "idle".into();
                    s.error = None;
                    s.progress_message = None;
                });
                emit(&adopted_id);
                json!({ "ok": true, "cloned": true, "adoptedId": adopted_id })
            }
            Err(e) => fail(ws_id, e),
        };
    }

    if !remote_empty {
        set_state(ws_id, |s| {
            s.status = "idle".into();
            s.error = Some("Remote is not empty".into());
            s.progress_message = None;
        });
        emit(ws_id);
        return json!({
            "ok": false,
            "error": "Remote is not empty. Connect an empty workspace to clone from a non-empty remote, or use an empty remote for pushing local data."
        });
    }
    progress(ws_id, "Pushing to remote…");
    match publish_to_empty_remote(ws_id, remote, branch).await {
        Ok(()) => json!({ "ok": true, "cloned": false }),
        Err(e) => fail(ws_id, e),
    }
}

/// Forget the remote: drop `origin` and the sync configuration. Always succeeds.
pub async fn disconnect(ws_id: &str) -> Value {
    let _ = git(ws_id, &["remote", "remove", "origin"]).await;
    save_sync_config(ws_id, None);
    save_sync_meta(ws_id, None);
    set_state(ws_id, |s| {
        s.status = "idle".into();
        s.error = None;
        s.last_pushed_at = None;
        s.last_pulled_at = None;
    });
    emit(ws_id);
    json!({ "ok": true })
}

pub async fn sync_push(ws_id: &str) -> Value {
    let Some(config) = get_sync_config(ws_id) else { return json!({ "ok": false, "error": "No remote configured" }) };
    set_state(ws_id, |s| {
        s.status = "pushing".into();
        s.error = None;
    });
    emit(ws_id);

    let _guard = workspace_lock(ws_id).lock_owned().await;
    match git(ws_id, &["push", "origin", &format!("HEAD:{}", str_of(&config, "branch"))]).await {
        Ok(_) => {
            let now = now_ms();
            let pulled = get_sync_meta(ws_id).and_then(|m| m.get("lastPulledAt").and_then(Value::as_i64));
            save_sync_meta(ws_id, Some(sync_meta_value(Some(now), pulled, None)));
            set_state(ws_id, |s| {
                s.status = "idle".into();
                s.error = None;
                s.last_pushed_at = Some(now);
            });
            emit(ws_id);
            json!({ "ok": true })
        }
        Err(e) => {
            set_state(ws_id, |s| {
                s.status = "error".into();
                s.error = Some(e.0.clone());
            });
            emit(ws_id);
            json!({ "ok": false, "error": e.0 })
        }
    }
}

pub async fn sync_pull(ws_id: &str) -> Value {
    let Some(config) = get_sync_config(ws_id) else { return json!({ "ok": false, "error": "No remote configured" }) };
    set_state(ws_id, |s| {
        s.status = "pulling".into();
        s.error = None;
        s.updated_ids = None;
    });
    emit(ws_id);

    let _guard = workspace_lock(ws_id).lock_owned().await;
    match perform_git_pull(ws_id, str_of(&config, "branch")).await {
        Ok(outcome) => {
            let now = now_ms();
            let meta = get_sync_meta(ws_id);
            let field = |key: &str| meta.as_ref().and_then(|m| m.get(key)).and_then(Value::as_i64);
            let synced_commit = meta.as_ref().and_then(|m| m.get("lastSyncedCommit")).and_then(Value::as_str);
            save_sync_meta(ws_id, Some(sync_meta_value(field("lastPushedAt"), Some(now), synced_commit)));

            if !outcome.updated {
                set_state(ws_id, |s| {
                    s.status = "idle".into();
                    s.error = None;
                    s.last_pulled_at = Some(now);
                });
                emit(ws_id);
                return json!({ "ok": true, "updated": false });
            }
            let updated_ids: Vec<String> =
                outcome.changed_paths.iter().filter_map(|p| entity_id_from_path(p)).collect();
            set_state(ws_id, |s| {
                s.status = "idle".into();
                s.error = None;
                s.last_pulled_at = Some(now);
                s.updated_ids = Some(updated_ids.clone());
            });
            emit(ws_id);
            json!({ "ok": true, "updated": true, "updatedIds": updated_ids })
        }
        Err(e) => {
            set_state(ws_id, |s| {
                s.status = "error".into();
                s.error = Some(e.0.clone());
            });
            emit(ws_id);
            json!({ "ok": false, "error": e.0 })
        }
    }
}

/// Persist the auto-sync switch. `{ ok: false }` when no remote is configured.
pub fn set_auto_sync(ws_id: &str, enabled: bool) -> Value {
    let Some(mut config) = get_sync_config(ws_id) else { return json!({ "ok": false }) };
    config["autoSync"] = json!(enabled);
    save_sync_config(ws_id, Some(config));
    json!({ "ok": true })
}

/// The remote branch's tip, read with `ls-remote` (no fetch).
pub async fn get_remote_head(ws_id: &str) -> Option<String> {
    let config = get_sync_config(ws_id)?;
    fetch_remote_head(str_of(&config, "remote"), str_of(&config, "branch")).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ids_come_from_entity_paths_only() {
        assert_eq!(entity_id_from_path("requests/folder/abc.json").as_deref(), Some("abc"));
        assert_eq!(entity_id_from_path("mocks/index.json"), None);
        assert_eq!(entity_id_from_path("workspace.txt"), None);
    }
}
