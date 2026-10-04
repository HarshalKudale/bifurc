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
    remote::{ensure_owner, git_net},
    status_tracker::{get_workspace_sync_status, invalidate_cache},
    sync_state::{emit, get_sync_config, get_sync_meta, save_sync_meta, set_state, sync_meta_value},
};
use crate::{
    runtime::instances::notify_config_changed,
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
        s.error = None;
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

/// Commit everything in the workspace that is new or modified, an entity at a time, then push. Returns
/// `{ ok, initialized, total, synced, pushed, pendingDeletions, failed: [{ path, error }], error? }`.
pub async fn sync_workspace(ws_id: &str) -> Value {
    let _guard = workspace_lock(ws_id).lock_owned().await;
    let result = match sync_locked(ws_id).await {
        Ok(result) => result,
        Err(e) => json!({ "ok": false, "error": e.0 }),
    };
    let error = result.get("error").and_then(Value::as_str).map(str::to_string);
    set_state(ws_id, |s| {
        s.status = if error.is_some() { "error" } else { "idle" }.into();
        s.error = error;
        s.progress_message = None;
        if result.get("pushed") == Some(&Value::Bool(true)) {
            s.last_pushed_at = Some(now_ms());
        }
    });
    emit(ws_id);
    invalidate_cache(ws_id);
    result
}

async fn sync_locked(ws_id: &str) -> Result<Value, GitError> {
    if load_settings().workspace(ws_id).is_none() {
        return Err(GitError("No such workspace.".to_string()));
    }
    if !check_git_installed().await {
        return Err(GitError(
            "Git is not installed, so this workspace is not tracked and has nothing to sync with. Install Git, then sync again."
                .to_string(),
        ));
    }
    progress(ws_id, "syncing", "Preparing…".to_string());
    let initialized = !is_repo(ws_id);
    init_workspace_repo(ws_id).await?;

    // The commits go on top of what the remote has now, so the one push at the end is never behind it.
    let remote = get_sync_config(ws_id).filter(|c| truthy(c, "remote"));
    if let Some(config) = &remote {
        ensure_owner(ws_id).await?;
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
        let opts = PublishOptions { ws_id: ws_id.to_string(), paths: vec![path.clone()], message: None };
        match commit_paths(&opts).await {
            Ok(result) if result.get("ok") == Some(&Value::Bool(true)) => synced += 1,
            Ok(result) => failed.push(json!({ "path": path, "error": str_of(&result, "error") })),
            Err(e) => failed.push(json!({ "path": path, "error": e.0 })),
        }
    }

    let failure = failed.first().map(|first: &Value| {
        format!("{} of {total} could not be synced. {}: {}", failed.len(), str_of(first, "path"), str_of(first, "error"))
    });
    let mut result = json!({
        "ok": failure.is_none(), "initialized": initialized, "total": total, "synced": synced, "pushed": false,
        "pendingDeletions": pending_deletions, "failed": failed,
    });
    if let Some(failure) = failure {
        result["error"] = json!(failure);
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
                result["error"] = json!(format!("{synced} of {total} were committed, but the push failed: {}", e.0));
            }
        }
    }
    Ok(result)
}
