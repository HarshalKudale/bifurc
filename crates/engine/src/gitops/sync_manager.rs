//! Moving commits between a workspace and its remote: push, pull and the remote head query.

use serde_json::{Value, json};

use super::{
    git_lock::workspace_lock,
    git_sync_ops::{fetch_remote_head, perform_git_pull},
    remote::{Need, ensure_access, git_net, workspace_auth},
    sync_state::{emit, get_sync_config, get_sync_meta, no_remote, save_sync_meta, set_state, sync_meta_value},
};
use crate::{
    protocol::EngineError,
    runtime::failed,
    utils::{id::now_ms, json::str_of},
};

/// The entity id a workspace-relative path names (`requests/folder/abc.json` -> `abc`), if it is an entity file.
fn entity_id_from_path(path: &str) -> Option<String> {
    let base = path.rsplit('/').next()?;
    let id = base.strip_suffix(".json")?;
    if id.is_empty() || ["index", "enabled", "names"].contains(&id) { None } else { Some(id.to_string()) }
}

pub async fn sync_push(ws_id: &str) -> Value {
    let Some(config) = get_sync_config(ws_id) else { return failed(&no_remote()) };
    set_state(ws_id, |s| {
        s.status = "pushing".into();
        s.clear_error();
    });
    emit(ws_id);

    let _guard = workspace_lock(ws_id).lock_owned().await;
    let pushed = match ensure_access(ws_id, Need::Push).await {
        Ok(()) => git_net(ws_id, &["push", "origin", &format!("HEAD:{}", str_of(&config, "branch"))]).await,
        Err(e) => Err(e),
    };
    match pushed {
        Ok(_) => {
            let now = now_ms();
            let pulled = get_sync_meta(ws_id).and_then(|m| m.get("lastPulledAt").and_then(Value::as_i64));
            save_sync_meta(ws_id, Some(sync_meta_value(Some(now), pulled, None)));
            set_state(ws_id, |s| {
                s.status = "idle".into();
                s.clear_error();
                s.last_pushed_at = Some(now);
            });
            emit(ws_id);
            json!({ "ok": true })
        }
        Err(e) => {
            let error = EngineError::from(e);
            set_state(ws_id, |s| s.fail(&error));
            emit(ws_id);
            failed(&error)
        }
    }
}

pub async fn sync_pull(ws_id: &str) -> Value {
    let Some(config) = get_sync_config(ws_id) else { return failed(&no_remote()) };
    set_state(ws_id, |s| {
        s.status = "pulling".into();
        s.clear_error();
        s.updated_ids = None;
    });
    emit(ws_id);

    let _guard = workspace_lock(ws_id).lock_owned().await;
    let pulled = match ensure_access(ws_id, Need::Pull).await {
        Ok(()) => perform_git_pull(ws_id, str_of(&config, "branch")).await,
        Err(e) => Err(e),
    };
    match pulled {
        Ok(outcome) => {
            let now = now_ms();
            let meta = get_sync_meta(ws_id);
            let field = |key: &str| meta.as_ref().and_then(|m| m.get(key)).and_then(Value::as_i64);
            let synced_commit = meta.as_ref().and_then(|m| m.get("lastSyncedCommit")).and_then(Value::as_str);
            save_sync_meta(ws_id, Some(sync_meta_value(field("lastPushedAt"), Some(now), synced_commit)));

            if !outcome.updated {
                set_state(ws_id, |s| {
                    s.status = "idle".into();
                    s.clear_error();
                    s.last_pulled_at = Some(now);
                });
                emit(ws_id);
                return json!({ "ok": true, "updated": false });
            }
            let updated_ids: Vec<String> =
                outcome.changed_paths.iter().filter_map(|p| entity_id_from_path(p)).collect();
            set_state(ws_id, |s| {
                s.status = "idle".into();
                s.clear_error();
                s.last_pulled_at = Some(now);
                s.updated_ids = Some(updated_ids.clone());
            });
            emit(ws_id);
            json!({ "ok": true, "updated": true, "updatedIds": updated_ids })
        }
        Err(e) => {
            let error = EngineError::from(e);
            set_state(ws_id, |s| s.fail(&error));
            emit(ws_id);
            failed(&error)
        }
    }
}

/// The remote branch's tip, read with `ls-remote` (no fetch).
pub async fn get_remote_head(ws_id: &str) -> Option<String> {
    let config = get_sync_config(ws_id)?;
    let auth = workspace_auth(ws_id).ok()?;
    fetch_remote_head(str_of(&config, "remote"), str_of(&config, "branch"), auth.as_ref()).await
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
