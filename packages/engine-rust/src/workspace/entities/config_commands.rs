//! `config.*`, `settings.update` and `workspace.*`.

use serde_json::{Value, json};

use crate::{
    kernel::{CommandRegistry, bus},
    protocol::EngineError,
    runtime::instances::{notify_config_changed, rebind_changed_listeners, rebind_runtime_ports},
    utils::{
        id::{generate_id, now_ms},
        json::{array_mut, opt_str, str_of},
        random_names::generate_random_workspace_name,
    },
    workspace::{
        accounts::find_account,
        config::{load_config, save_config},
        fs::init_workspace_dir,
        git_store::{check_git_installed, init_workspace_repo},
        settings::{load_settings, save_settings},
    },
};

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("config.get", |_, _| Ok(load_config()));

    registry.register_sync("config.save", |params, _| {
        let prev = load_settings();
        save_config(params.get("config").unwrap_or(&Value::Null))?;
        notify_config_changed();
        bus().emit("settings.changed", json!({}));
        rebind_runtime_ports(&load_settings(), &prev);
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("settings.update", |params, _| {
        let prev = load_settings();
        let mut next = prev.clone();
        for (key, value) in params.as_object().into_iter().flatten() {
            next.set(key, value.clone());
        }
        save_settings(&next)?;
        notify_config_changed();
        bus().emit("settings.changed", json!({}));
        rebind_changed_listeners(&next, &prev);
        Ok(json!({ "ok": true }))
    });

    // A new, local workspace. With `accountId` it commits as that account from its first commit, which makes
    // the account its owner. The first workspace of a data directory becomes the active one.
    registry.register("workspace.add", |params, _| async move {
        let name = str_of(&params, "name").trim();
        let name = if name.is_empty() { generate_random_workspace_name() } else { name.to_string() };
        let account_id = opt_str(&params, "accountId").filter(|id| !id.is_empty());
        if let Some(account_id) = account_id {
            if find_account(account_id).is_none() {
                return Err(EngineError::not_found("That account is not signed in on this device."));
            }
        }
        let id = generate_id();
        let mut settings = load_settings();
        let mut meta = json!({ "id": id, "name": name, "activeEnvironmentId": null });
        if let Some(account_id) = account_id {
            meta["accountId"] = json!(account_id);
        }
        settings.workspaces_mut().push(meta);
        let first = settings.workspace(&settings.active_workspace_id()).is_none();
        if first {
            settings.set("activeWorkspaceId", json!(id));
        }
        save_settings(&settings)?;

        init_workspace_dir(&id, &name)?;
        // Without git the workspace is plain files: no history, no audit log, no sync.
        let git_tracked = check_git_installed().await && init_workspace_repo(&id).await.is_ok();
        if first {
            notify_config_changed();
        }
        bus().emit("settings.changed", json!({}));
        Ok(json!({
            "id": id, "name": name, "createdAt": now_ms(), "activeEnvironmentId": null,
            "accountId": account_id, "gitTracked": git_tracked,
        }))
    });

    registry.register_sync("workspace.rename", |params, _| {
        let mut cfg = load_config();
        let (id, name) = (str_of(&params, "id"), str_of(&params, "name").trim());
        if let Some(ws) = array_mut(&mut cfg, "workspaces").iter_mut().find(|w| str_of(w, "id") == id) {
            if !name.is_empty() {
                ws["name"] = json!(name);
            }
        }
        save_config(&cfg)?;
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("workspace.delete", |params, _| {
        let mut cfg = load_config();
        let id = str_of(&params, "id");
        array_mut(&mut cfg, "workspaces").retain(|w| str_of(w, "id") != id);
        if str_of(&cfg, "activeWorkspaceId") == id {
            match array_mut(&mut cfg, "workspaces").first().cloned() {
                Some(first) => {
                    cfg["activeWorkspaceId"] = first.get("id").cloned().unwrap_or(Value::Null);
                    cfg["activeEnvironmentId"] = first.get("activeEnvironmentId").cloned().unwrap_or(Value::Null);
                }
                None => cfg["activeWorkspaceId"] = json!(""),
            }
        }
        save_config(&cfg)?;
        notify_config_changed();
        Ok(json!({ "ok": true }))
    });
}
