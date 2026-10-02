//! `application.*`: application definition storage and process control.

use serde_json::{Value, json};

use super::{
    command_generator::generate_resolved_command,
    port_utils::{check_port_in_use, kill_process_on_port},
    process_spawner,
};
use crate::{
    kernel::CommandRegistry,
    utils::{
        id::{generate_id, now_ms},
        json::{i64_of, object_mut, str_of, truthy},
    },
    workspace::fs::{delete_entity_file, read_all_entities, write_entity},
};

/// The platform name the generated command is specific to (a Node `process.platform` value).
fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// `{ ...app, ...resolved }` where an absent debug field removes the app's own.
fn with_resolved(app: &Value) -> Value {
    let resolved = generate_resolved_command(app, platform());
    let mut out = app.clone();
    let object = object_mut(&mut out);
    object.insert("resolvedCommand".into(), json!(resolved.resolved_command));
    object.insert("resolvedCwd".into(), json!(resolved.resolved_cwd));
    object.insert("resolvedEnv".into(), Value::Object(resolved.resolved_env));
    match resolved.resolved_debug_command {
        Some(command) => object.insert("resolvedDebugCommand".into(), json!(command)),
        None => object.shift_remove("resolvedDebugCommand"),
    };
    match resolved.resolved_debug_port {
        Some(port) => object.insert("resolvedDebugPort".into(), port),
        None => object.shift_remove("resolvedDebugPort"),
    };
    out
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("application.list", |params, _| {
        Ok(Value::Array(read_all_entities(str_of(&params, "workspaceId"), "applications")))
    });

    registry.register_sync("application.save", |params, _| {
        let mut app = params.get("application").cloned().unwrap_or(Value::Null);
        if !truthy(&app, "id") {
            let object = object_mut(&mut app);
            object.insert("id".into(), json!(generate_id()));
            object.insert("createdAt".into(), json!(now_ms()));
        }
        let to_save = with_resolved(&app);
        write_entity(str_of(&to_save, "workspaceId"), "applications", str_of(&to_save, "id"), &to_save, None)?;
        Ok(to_save)
    });

    registry.register_sync("application.delete", |params, _| {
        let id = str_of(&params, "id");
        process_spawner::stop(id);
        delete_entity_file(str_of(&params, "workspaceId"), "applications", id)?;
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("application.start", |params, _| {
        Ok(process_spawner::start(str_of(&params, "workspaceId"), str_of(&params, "appId"), str_of(&params, "mode")))
    });

    registry.register_sync("application.stop", |params, _| {
        process_spawner::stop(str_of(&params, "appId"));
        Ok(json!({ "ok": true }))
    });

    registry
        .register_sync("application.getState", |params, _| Ok(process_spawner::get_state(str_of(&params, "appId"))));

    registry.register_sync("application.getAllStates", |_, _| Ok(process_spawner::get_all_states()));

    registry.register_sync("application.getLogs", |params, _| Ok(process_spawner::get_logs(str_of(&params, "appId"))));

    registry.register("application.checkPort", |params, _| async move {
        Ok(check_port_in_use(i64_of(&params, "port", 0)).await?.to_json())
    });

    registry.register("application.killPort", |params, _| async move {
        Ok(json!({ "ok": kill_process_on_port(i64_of(&params, "port", 0)).await }))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{kernel::CommandContext, testing::scratch_data_root};

    async fn call(registry: &Arc<CommandRegistry>, action: &str, payload: Value) -> Value {
        registry.invoke(action, payload, CommandContext::internal(registry.clone())).await.unwrap()
    }

    fn registry() -> Arc<CommandRegistry> {
        let mut registry = CommandRegistry::new();
        register(&mut registry);
        Arc::new(registry)
    }

    #[tokio::test]
    async fn save_list_delete_round_trip() {
        let root = scratch_data_root();
        let registry = registry();
        let saved = call(
            &registry,
            "application.save",
            json!({ "application": {
                "name": "Api", "type": "npm", "workspaceId": root.ws_id, "workingDirectory": "/srv",
                "args": [], "npmConfig": { "scriptName": "dev", "packageManager": "yarn" },
                "resolvedDebugCommand": "stale",
            } }),
        )
        .await;
        let id = saved["id"].as_str().unwrap().to_string();
        assert!(saved["createdAt"].is_i64() || saved["createdAt"].is_u64());
        assert_eq!(saved["resolvedCommand"], "yarn dev");
        assert_eq!(saved["resolvedCwd"], "/srv");
        assert_eq!(saved["resolvedEnv"], json!({}));
        assert!(saved.get("resolvedDebugCommand").is_none());

        let on_disk = std::fs::read_to_string(
            crate::workspace::fs::entity_dir(root.ws_id, "applications").join(format!("{id}.json")),
        )
        .unwrap();
        assert!(on_disk.contains("\n  \"name\": \"Api\""));

        let listed = call(&registry, "application.list", json!({ "workspaceId": root.ws_id })).await;
        assert_eq!(listed, json!([saved.clone()]));

        // Saving again with the id keeps it and its creation time.
        let mut edited = saved.clone();
        edited["npmConfig"]["packageManager"] = json!("pnpm");
        let again = call(&registry, "application.save", json!({ "application": edited })).await;
        assert_eq!(again["id"], saved["id"]);
        assert_eq!(again["createdAt"], saved["createdAt"]);
        assert_eq!(again["resolvedCommand"], "pnpm run dev");

        let deleted = call(&registry, "application.delete", json!({ "workspaceId": root.ws_id, "id": id })).await;
        assert_eq!(deleted, json!({ "ok": true }));
        assert_eq!(call(&registry, "application.list", json!({ "workspaceId": root.ws_id })).await, json!([]));
    }

    #[tokio::test]
    async fn state_commands_for_unknown_application() {
        let _root = scratch_data_root();
        let registry = registry();
        assert_eq!(call(&registry, "application.getState", json!({ "appId": "ghost" })).await, Value::Null);
        assert_eq!(call(&registry, "application.getLogs", json!({ "appId": "ghost" })).await, json!([]));
        assert_eq!(call(&registry, "application.stop", json!({ "appId": "ghost" })).await, json!({ "ok": true }));
        assert!(call(&registry, "application.getAllStates", json!({})).await.is_array());
    }

    #[tokio::test]
    async fn check_port_through_the_registry() {
        let registry = registry();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let busy = call(&registry, "application.checkPort", json!({ "port": port })).await;
        assert_eq!(busy["inUse"], true);
        drop(listener);
        let free = call(&registry, "application.checkPort", json!({ "port": port })).await;
        assert_eq!(free, json!({ "inUse": false }));
    }
}
