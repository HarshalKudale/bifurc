//! `config.*`, `settings.update` and `workspace.*`.

use serde_json::{Value, json};

use crate::{
    kernel::{CommandRegistry, bus},
    runtime::instances::{notify_config_changed, rebind_changed_listeners, rebind_runtime_ports},
    utils::{
        id::{generate_id, now_ms},
        json::{array_mut, str_of},
        random_names::generate_random_workspace_name,
    },
    workspace::{
        config::{load_config, save_config},
        fs::init_workspace_dir,
        git_store::init_workspace_repo,
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

    registry.register("workspace.add", |params, _| async move {
        let mut cfg = load_config();
        let name = str_of(&params, "name").trim();
        let name = if name.is_empty() { generate_random_workspace_name() } else { name.to_string() };
        let workspace =
            json!({ "id": generate_id(), "name": name, "createdAt": now_ms(), "activeEnvironmentId": null });
        array_mut(&mut cfg, "workspaces").push(workspace.clone());
        save_config(&cfg)?;
        let id = str_of(&workspace, "id");
        if init_workspace_dir(id, &name).is_ok() {
            let _ = init_workspace_repo(id).await;
        }
        Ok(workspace)
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
            if let Some(first) = array_mut(&mut cfg, "workspaces").first().cloned() {
                cfg["activeWorkspaceId"] = first.get("id").cloned().unwrap_or(Value::Null);
                cfg["activeEnvironmentId"] = first.get("activeEnvironmentId").cloned().unwrap_or(Value::Null);
            }
        }
        save_config(&cfg)?;
        notify_config_changed();
        Ok(json!({ "ok": true }))
    });
}
