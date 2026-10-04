//! `setup.getState`, the `account.*` commands, `workspace.import` and `sync.publish`: what a client needs to
//! walk a user through signing in and getting a workspace, without holding any of the logic itself.

use serde_json::{Value, json};

use super::{
    auto_sync::{start_auto_sync, stop_auto_sync},
    git_state::get_workspace_git_state,
    github,
    login::{cancel_login, login_status, start_login},
    remote::{import_workspace, publish_workspace, remove_workspaces, workspace_for_repo},
    status_tracker::get_workspace_sync_status,
    sync_all::{sync_workspace, track_untracked_workspaces},
    sync_state::get_sync_config,
};
use crate::{
    kernel::{CommandRegistry, bus},
    protocol::EngineError,
    runtime::{entity_status::emit_entity_status, instances::notify_config_changed},
    utils::{
        id::now_ms,
        json::{bool_of, str_of},
    },
    workspace::{
        accounts::{find_account, public_accounts, sign_out, workspaces_using},
        git_store::git_version,
        settings::{PRIVACY_POLICY_URL, load_settings, save_settings},
    },
};

fn succeeded(result: &Value) -> bool {
    result.get("ok") == Some(&Value::Bool(true))
}

/// A workspace as an account's listing shows it: its repository, and how much of it only this device has
/// (`changes`: files that differ from the last sync; `unpushed`: commits the remote has not got).
async fn workspace_summary(ws_id: &str) -> Value {
    let settings = load_settings();
    let name = settings.workspace(ws_id).map(|ws| str_of(ws, "name").to_string()).unwrap_or_default();
    let repo = get_sync_config(ws_id).map(|config| str_of(&config, "repo").to_string()).filter(|repo| !repo.is_empty());
    let changes =
        get_workspace_sync_status(ws_id).await.values().filter(|status| status.as_str() != Some("clean")).count();
    let unpushed = get_workspace_git_state(ws_id).await.get("ahead") == Some(&Value::Bool(true));
    json!({ "id": ws_id, "name": name, "repo": repo, "changes": changes, "unpushed": unpushed })
}

pub fn register(registry: &mut CommandRegistry) {
    // What a first run has to deal with: is git here, can the user sign in, is there a workspace yet.
    // Asking is also how a git that was installed since is noticed: the workspaces that were plain files
    // become repositories then, with everything in them waiting to be synced.
    registry.register("setup.getState", |_, _| async move {
        let version = git_version().await;
        if version.is_some() {
            for ws_id in track_untracked_workspaces().await {
                emit_entity_status(&ws_id);
            }
        }
        let settings = load_settings();
        Ok(json!({
            "gitInstalled": version.is_some(),
            "gitVersion": version,
            "signInAvailable": github::client_id().is_some(),
            "host": github::host(),
            "needsWorkspace": settings.workspaces().is_empty(),
            "accounts": public_accounts(),
            "privacyPolicyUrl": PRIVACY_POLICY_URL,
            "privacyAccepted": settings.privacy_accepted(),
            "onboarded": settings.onboarded(),
        }))
    });

    // The user accepted the privacy policy. When is kept, so a later policy can ask again.
    registry.register_sync("setup.acceptPrivacy", |_, _| {
        let mut settings = load_settings();
        if !settings.privacy_accepted() {
            settings.set("privacyAcceptedAt", json!(now_ms()));
            save_settings(&settings)?;
            bus().emit("settings.changed", json!({}));
        }
        Ok(json!({ "ok": true }))
    });

    // The first-run setup reached its end; a client does not walk the user through it again.
    registry.register_sync("setup.complete", |_, _| {
        let mut settings = load_settings();
        if !settings.privacy_accepted() {
            return Ok(json!({ "ok": false, "error": "The privacy policy has not been accepted yet." }));
        }
        if !settings.onboarded() {
            settings.set("hasSeenWelcome", json!(true));
            save_settings(&settings)?;
            bus().emit("settings.changed", json!({}));
        }
        Ok(json!({ "ok": true }))
    });

    registry.register("account.login.start", |_, _| async move {
        match start_login().await {
            Ok(login) => Ok(login),
            Err(message) => Ok(json!({ "status": "failed", "error": message })),
        }
    });

    registry.register_sync("account.login.status", |params, _| Ok(login_status(str_of(&params, "loginId"))));

    registry.register_sync("account.login.cancel", |params, _| {
        cancel_login(str_of(&params, "loginId"));
        Ok(json!({ "ok": true }))
    });

    // Each account with the workspaces that use it, and what each of those holds that its remote does not.
    registry.register("account.list", |_, _| async move {
        let mut accounts = public_accounts();
        for account in &mut accounts {
            let mut workspaces = Vec::new();
            for ws_id in workspaces_using(str_of(account, "id")) {
                workspaces.push(workspace_summary(&ws_id).await);
            }
            account["workspaces"] = Value::Array(workspaces);
        }
        Ok(json!({ "accounts": accounts }))
    });

    // Forgets the account and takes the workspaces that use it off this device. What they held that was not
    // synced goes with them, so a client asks first (`account.list` says what there is).
    registry.register_sync("account.logout", |params, _| {
        let id = str_of(&params, "accountId");
        let removed = workspaces_using(id);
        for ws_id in &removed {
            stop_auto_sync(ws_id);
        }
        remove_workspaces(&removed)?;
        sign_out(id)?;
        bus().emit("account.changed", json!({ "accountId": id, "status": "signedOut" }));
        if !removed.is_empty() {
            notify_config_changed();
            bus().emit("settings.changed", json!({}));
        }
        Ok(json!({ "ok": true, "removedWorkspaces": removed }))
    });

    // The repositories the account owns, each with whether it is already a workspace on this device.
    registry.register("account.repos", |params, _| async move {
        let id = str_of(&params, "accountId");
        let account = find_account(id).ok_or_else(|| EngineError::not_found("That account is not on this device."))?;
        let Some(token) = account.token else {
            return Ok(json!({ "ok": false, "error": format!("Sign in to GitHub as {} again first.", account.login) }));
        };
        match github::list_owned_repos(&token).await {
            Ok(repos) => {
                let repos: Vec<Value> = repos
                    .iter()
                    .map(|repo| {
                        let mut listed = repo.public();
                        listed["workspaceId"] = json!(workspace_for_repo(&repo.full_name));
                        listed
                    })
                    .collect();
                Ok(json!({ "ok": true, "repos": repos }))
            }
            Err(message) => Ok(json!({ "ok": false, "error": message })),
        }
    });

    registry.register("workspace.import", |params, _| async move {
        let result = import_workspace(str_of(&params, "accountId"), str_of(&params, "repo")).await;
        if succeeded(&result) {
            let ws_id = result.pointer("/workspace/id").and_then(Value::as_str).unwrap_or_default();
            start_auto_sync(ws_id);
            notify_config_changed();
            bus().emit("settings.changed", json!({}));
            emit_entity_status(ws_id);
        }
        Ok(result)
    });

    // With `syncAll`, what the workspace holds that was never synced goes to the new repository too, an
    // entity to a commit; without it only what was already committed is pushed.
    registry.register("sync.publish", |params, _| async move {
        let ws_id = str_of(&params, "workspaceId");
        let mut result = publish_workspace(
            ws_id,
            str_of(&params, "accountId"),
            str_of(&params, "repoName"),
            bool_of(&params, "private", true),
        )
        .await;
        if succeeded(&result) && bool_of(&params, "syncAll", false) {
            let synced = sync_workspace(ws_id).await;
            if !succeeded(&synced) {
                result["ok"] = json!(false);
                result["error"] = json!(format!(
                    "{} was created, but not everything was synced: {}",
                    result.pointer("/repo/fullName").and_then(Value::as_str).unwrap_or("The repository"),
                    str_of(&synced, "error")
                ));
            }
            result["sync"] = synced;
        }
        // The remote may be assigned even when the first push failed, so clients re-read either way.
        start_auto_sync(ws_id);
        notify_config_changed();
        bus().emit("settings.changed", json!({}));
        emit_entity_status(ws_id);
        Ok(result)
    });
}
