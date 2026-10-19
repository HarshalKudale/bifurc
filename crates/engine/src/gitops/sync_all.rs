//! Syncing a whole workspace at once (`sync.all`): everything that was created or changed and never synced
//! is committed, one entity to a commit as if each had been synced by hand, and with a remote the commits
//! are pushed together at the end.
//!
//! It is also how a workspace that was plain files becomes a repository: one that was made while git was
//! missing is initialised first, and everything in it is then committed as created. What happened to it
//! before that was never recorded, so its history starts here.
//!
//! Deletions are left alone: each waits to be confirmed or reverted, which is a decision of its own.

use serde_json::{Value, json};

use super::{
    git_lock::workspace_lock,
    git_sync_ops::perform_git_pull,
    publish_service::{PublishOptions, commit_paths},
    remote::{Need, ensure_access, git_net},
    status_tracker::{get_workspace_sync_status, invalidate_cache},
    sync_state::{emit, get_sync_config, get_sync_meta, save_sync_meta, set_state, sync_meta_value},
};
use crate::{
    protocol::EngineError,
    runtime::{failed, instances::notify_config_changed, set_failure},
    utils::{
        id::now_ms,
        json::{str_of, truthy},
    },
    workspace::{
        git_store::{GitError, check_git_installed, init_workspace_repo, is_repo},
        settings::load_settings,
    },
};

/// Say what the sync is doing now (`sync.status`, `status: "syncing"`).
fn progress(ws_id: &str, status: &str, message: String) {
    set_state(ws_id, |s| {
        s.status = status.into();
        s.clear_error();
        s.progress_message = Some(message);
    });
    emit(ws_id);
}

/// Make every workspace that is still plain files a repository, now that git can be run. Returns the ones
/// that became one. Nothing in them is committed beyond the workspace itself: their entities are all new.
pub async fn track_untracked_workspaces() -> Vec<String> {
    let mut tracked = Vec::new();
    for ws in load_settings().workspaces().to_vec() {
        let id = str_of(&ws, "id");
        if id.is_empty() || is_repo(id) {
            continue;
        }
        let _guard = workspace_lock(id).lock_owned().await;
        if init_workspace_repo(id).await.is_ok() {
            invalidate_cache(id);
            tracked.push(id.to_string());
        }
    }
    tracked
}

/// Commit everything in the workspace that is new or modified, an entity at a time, then push. A `message`
/// is the subject of every one of those commits. Returns
/// `{ ok, initialized, total, synced, pushed, pendingDeletions, failed: [{ path, error, errorKey?, errorParams? }], error? }`.
pub async fn sync_workspace(ws_id: &str, message: Option<String>) -> Value {
    let _guard = workspace_lock(ws_id).lock_owned().await;
    let result = match sync_locked(ws_id, message).await {
        Ok(result) => result,
        Err(e) => failed(&EngineError::from(e)),
    };
    set_state(ws_id, |s| {
        s.error_from(&result);
        s.status = if s.error.is_some() { "error" } else { "idle" }.into();
        s.progress_message = None;
        if result.get("pushed") == Some(&Value::Bool(true)) {
            s.last_pushed_at = Some(now_ms());
        }
    });
    emit(ws_id);
    invalidate_cache(ws_id);
    result
}

async fn sync_locked(ws_id: &str, message: Option<String>) -> Result<Value, GitError> {
    if load_settings().workspace(ws_id).is_none() {
        return Err(GitError::keyed("sync.no_workspace", "No such workspace.", json!({})));
    }
    if !check_git_installed().await {
        return Err(GitError::keyed(
            "sync.git_missing",
            "Git is not installed, so this workspace is not tracked and has nothing to sync with. Install Git, then sync again.",
            json!({}),
        ));
    }
    progress(ws_id, "syncing", "Preparing…".to_string());
    let initialized = !is_repo(ws_id);
    init_workspace_repo(ws_id).await?;

    // The commits go on top of what the remote has now, so the one push at the end is never behind it. An
    // account that may not push commits nothing: a commit that cannot leave would part the workspace from
    // its remote.
    let remote = get_sync_config(ws_id).filter(|c| truthy(c, "remote"));
    if let Some(config) = &remote {
        ensure_access(ws_id, Need::Push).await?;
        if perform_git_pull(ws_id, str_of(config, "branch")).await.is_ok_and(|pulled| pulled.updated) {
            notify_config_changed();
        }
    }

    invalidate_cache(ws_id);
    let status = get_workspace_sync_status(ws_id).await;
    let state_of = |wanted: &[&str]| -> Vec<String> {
        status.iter().filter(|(_, s)| s.as_str().is_some_and(|s| wanted.contains(&s))).map(|(p, _)| p.clone()).collect()
    };
    let mut paths = state_of(&["new", "modified"]);
    paths.sort();
    let pending_deletions = state_of(&["deleted"]).len();

    let total = paths.len();
    let mut synced = 0;
    let mut failed = Vec::new();
    for (index, path) in paths.into_iter().enumerate() {
        progress(ws_id, "syncing", format!("Syncing {} of {total}…", index + 1));
        let opts = PublishOptions { ws_id: ws_id.to_string(), paths: vec![path.clone()], message: message.clone() };
        match commit_paths(&opts).await {
            Ok(result) if result.get("ok") == Some(&Value::Bool(true)) => synced += 1,
            Ok(result) => {
                let mut item = json!({ "path": path, "error": str_of(&result, "error") });
                for field in ["errorKey", "errorParams"] {
                    if let Some(value) = result.get(field) {
                        item[field] = value.clone();
                    }
                }
                failed.push(item);
            }
            Err(e) => {
                let error = EngineError::from(e);
                let mut item = json!({ "path": path });
                crate::kernel::streams::put_error(item.as_object_mut().expect("an object"), Some(&error));
                failed.push(item);
            }
        }
    }

    let failure = failed.first().map(|first: &Value| {
        let (count, path, detail) = (failed.len(), str_of(first, "path"), str_of(first, "error"));
        EngineError::engine(format!("{count} of {total} could not be synced. {path}: {detail}"))
            .keyed("sync.items_failed", json!({ "count": count, "total": total, "path": path, "detail": detail }))
    });
    let mut result = json!({
        "ok": failure.is_none(), "initialized": initialized, "total": total, "synced": synced, "pushed": false,
        "pendingDeletions": pending_deletions, "failed": failed,
    });
    if let Some(failure) = failure {
        set_failure(&mut result, &failure);
    }

    // What was committed is pushed even when something else could not be: it is synced, and says so.
    if let Some(config) = remote {
        progress(ws_id, "pushing", "Pushing…".to_string());
        match git_net(ws_id, &["push", "origin", &format!("HEAD:{}", str_of(&config, "branch"))]).await {
            Ok(_) => {
                let pulled = get_sync_meta(ws_id).and_then(|m| m.get("lastPulledAt").and_then(Value::as_i64));
                save_sync_meta(ws_id, Some(sync_meta_value(Some(now_ms()), pulled, None)));
                result["pushed"] = json!(true);
            }
            Err(e) => {
                result["ok"] = json!(false);
                let error =
                    EngineError::engine(format!("{synced} of {total} were committed, but the push failed: {}", e.0))
                        .keyed(
                            "sync.push_failed_after_commit",
                            json!({ "synced": synced, "total": total, "detail": e.0 }),
                        );
                set_failure(&mut result, &error);
            }
        }
    }
    Ok(result)
}
