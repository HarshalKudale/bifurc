//! `setup.getState`, the `account.*` commands, `workspace.import` and `sync.publish`: what a client needs to
//! walk a user through signing in and getting a workspace, without holding any of the logic itself.

use serde_json::{Value, json};

use super::{
    auto_sync::{start_auto_sync, stop_auto_sync},
    git_state::get_workspace_git_state,
    github,
    login::{cancel_login, login_status, start_login},
    remote::{flag_if_rejected, import_workspace, listed_repo, publish_workspace, remove_workspaces},
    status_tracker::get_workspace_sync_status,
    sync_all::{sync_workspace, track_untracked_workspaces},
    sync_state::get_sync_config,
};
use crate::{
    features::team_sync,
    kernel::{CommandRegistry, bus},
    protocol::EngineError,
    runtime::{
        entity_status::emit_entity_status,
        instances::{notify_config_changed, start_listeners},
    },
    utils::{
        id::now_ms,
        json::{bool_of, str_of},
    },
    workspace::{
        accounts::{find_account, public_accounts, sign_out, workspaces_using},
        git_store::git_version,
        settings::{PRIVACY_POLICY_URL, TERMS_OF_SERVICE_URL, load_settings, save_settings},
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

/// Whether this process is refused a listener on `port` for want of permission, as Linux refuses the ports
/// below 1024. A port that is merely taken is not that.
fn port_denied(port: u16) -> bool {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .is_err_and(|error| error.kind() == std::io::ErrorKind::PermissionDenied)
}

/// Gives this engine's program the capability to listen on ports below 1024. Only root may give it, so
/// polkit asks for the password. A program gets its capabilities when it starts: this one has it from its
/// next start.
async fn allow_low_ports() -> Result<(), String> {
    if !cfg!(target_os = "linux") {
        return Err("Only Linux has this permission to give".to_owned());
    }
    let program = std::env::current_exe().map_err(|error| format!("The engine could not find itself. {error}"))?;
    // Through `sh`, so that setcap is looked for on root's PATH: a user's often has no /sbin.
    let output = crate::utils::process::hidden_command("pkexec")
        .args(["sh", "-c", "setcap cap_net_bind_service=+ep \"$1\"", "sh"])
        .arg(program)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|error| format!("pkexec: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    // 126 is pkexec's own answer for a prompt that was dismissed or a password that was refused.
    if output.status.code() == Some(126) {
        return Err("The password prompt was dismissed".to_owned());
    }
    let said = String::from_utf8_lossy(&output.stderr);
    Err(said
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("pkexec failed without saying why")
        .to_owned())
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
            "teamSync": team_sync(),
            "host": github::host(),
            "needsWorkspace": settings.workspaces().is_empty(),
            "accounts": public_accounts(),
            "termsOfServiceUrl": TERMS_OF_SERVICE_URL,
            "termsAccepted": settings.terms_accepted(),
            "privacyPolicyUrl": PRIVACY_POLICY_URL,
            "privacyAccepted": settings.privacy_accepted(),
            "onboarded": settings.onboarded(),
            "serversConfirmed": settings.servers_confirmed(),
        }))
    });

    // The user accepted the terms of service. When is kept, so later terms can ask again.
    registry.register_sync("setup.acceptTerms", |_, _| {
        let mut settings = load_settings();
        if !settings.terms_accepted() {
            settings.set("termsAcceptedAt", json!(now_ms()));
            save_settings(&settings)?;
            bus().emit("settings.changed", json!({}));
            // With both accepted, usage analytics may begin.
            crate::analytics::resume();
        }
        Ok(json!({ "ok": true }))
    });

    // The user accepted the privacy policy. When is kept, so a later policy can ask again.
    registry.register_sync("setup.acceptPrivacy", |_, _| {
        let mut settings = load_settings();
        if !settings.privacy_accepted() {
            settings.set("privacyAcceptedAt", json!(now_ms()));
            save_settings(&settings)?;
            bus().emit("settings.changed", json!({}));
            // With both accepted, usage analytics may begin.
            crate::analytics::resume();
        }
        Ok(json!({ "ok": true }))
    });

    // The first-run setup reached its end; a client does not walk the user through it again.
    registry.register_sync("setup.complete", |_, _| {
        let mut settings = load_settings();
        if !settings.terms_accepted() {
            return Ok(json!({ "ok": false, "error": "The terms of service have not been accepted yet." }));
        }
        if !settings.privacy_accepted() {
            return Ok(json!({ "ok": false, "error": "The privacy policy has not been accepted yet." }));
        }
        if !settings.onboarded() {
            // A walk that never passed the servers step ends with the proxies started all the same.
            let servers_confirmed = settings.servers_confirmed();
            settings.set("hasSeenWelcome", json!(true));
            save_settings(&settings)?;
            bus().emit("settings.changed", json!({}));
            if !servers_confirmed {
                start_listeners(&settings);
            }
        }
        Ok(json!({ "ok": true }))
    });

    // The first run's servers step: the ports are the user's choice now (`port`, `forwardProxyPort`, each
    // optional), so the two proxies start, and every later run of the engine starts them by itself. The
    // answer says how each fared, so a port that could not be bound can be changed there and then.
    registry.register_sync("setup.startServers", |params, _| {
        let prev = load_settings();
        let mut settings = prev.clone();
        for key in ["port", "forwardProxyPort"] {
            if let Some(port) = params.get(key).filter(|port| !port.is_null()) {
                settings.set(key, port.clone());
            }
        }
        if settings.port() == settings.forward_proxy_port() {
            return Ok(json!({ "ok": false, "error": "The two proxies need different ports." }));
        }
        if !settings.servers_confirmed() {
            settings.set("serversConfirmedAt", json!(now_ms()));
        }
        if settings != prev {
            save_settings(&settings)?;
            notify_config_changed();
            bus().emit("settings.changed", json!({}));
        }
        let started = start_listeners(&settings);
        // Only Linux has a permission the engine can be given (`setup.allowLowPorts`).
        let denied =
            cfg!(target_os = "linux") && [settings.port(), settings.forward_proxy_port()].into_iter().any(port_denied);
        Ok(json!({
            "ok": true,
            "forward": started["forward"],
            "reverse": started["reverse"],
            "lowPortsDenied": denied,
        }))
    });

    // Lets this engine listen on ports below 1024 on Linux, which asks the user for their password. It holds
    // from the engine's next start, so a client restarts it (`engine.shutdown {restart}`) and starts the
    // servers again.
    registry.register("setup.allowLowPorts", |_, _| async move {
        Ok(match allow_low_ports().await {
            Ok(()) => json!({ "ok": true }),
            Err(message) => json!({ "ok": false, "error": message }),
        })
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

    // The repositories the account can have a workspace in: the ones it owns and, with team sync, the ones
    // it was given access to. Each with what the account may do with it (`access`) and whether it is already
    // a workspace on this device.
    registry.register("account.repos", |params, _| async move {
        let id = str_of(&params, "accountId");
        let account = find_account(id).ok_or_else(|| EngineError::not_found("That account is not on this device."))?;
        let Some(token) = account.token.clone() else {
            return Ok(json!({ "ok": false, "error": format!("Sign in to GitHub as {} again first.", account.login) }));
        };
        match github::list_repos(&token).await {
            Ok(repos) => {
                // Only the repositories marked as a Bifurc workspace (`github::WORKSPACE_TOPIC`) are offered.
                let repos: Vec<Value> =
                    repos.iter().filter(|repo| repo.is_workspace).map(|repo| listed_repo(repo, &account)).collect();
                Ok(json!({ "ok": true, "repos": repos }))
            }
            Err(message) => Ok(json!({ "ok": false, "error": flag_if_rejected(id, message) })),
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
            let synced = sync_workspace(ws_id, None).await;
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
