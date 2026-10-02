//! `workspace.setActive`, `entity.load`, `entity.duplicate` and `entity.setEnabled`.

use serde_json::{Value, json};

use crate::{
    kernel::{CommandRegistry, kinds::to_engine_kind},
    protocol::EngineError,
    runtime::{entity_status::emit_entity_status, instances::notify_config_changed},
    utils::json::{array_of, merged, str_of, truthy},
    workspace::{
        config::{load_config, load_entity, save_config},
        fs::{read_enabled_set, sync_enabled_set},
    },
};

/// The kinds whose enabled flag lives in `enabled.json`, so `entity.load` adds it back.
const ENABLED_STATE_KINDS: [&str; 3] = ["mocks", "mappings", "rules"];

fn mock_signature(mock: &Value) -> String {
    format!("{}|{}|{}", str_of(mock, "method").to_uppercase(), str_of(mock, "urlPattern"), str_of(mock, "capturedBody"))
}

fn rule_signature(rule: &Value) -> String {
    format!("{}|{}", if truthy(rule, "useRegex") { "re" } else { "exact" }, str_of(rule, "pattern"))
}

/// Switch off every other enabled entity in the snapshot's `config_key` list that has the target's signature.
fn disable_same_signature(
    ws_id: &str,
    kind: &str,
    config_key: &str,
    id: &str,
    signature: fn(&Value) -> String,
) -> std::io::Result<()> {
    let cfg = load_config();
    let list = array_of(&cfg, config_key);
    let Some(target) = list.iter().find(|e| str_of(e, "id") == id) else { return Ok(()) };
    let wanted = signature(target);
    for other in list {
        if str_of(other, "id") != id && truthy(other, "enabled") && signature(other) == wanted {
            sync_enabled_set(ws_id, kind, str_of(other, "id"), false)?;
        }
    }
    Ok(())
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("workspace.setActive", |params, _| {
        let mut cfg = load_config();
        let id = str_of(&params, "id");
        let Some(ws) = array_of(&cfg, "workspaces").iter().find(|w| str_of(w, "id") == id).cloned() else {
            return Ok(json!({ "ok": false }));
        };
        cfg["activeWorkspaceId"] = json!(id);
        cfg["activeEnvironmentId"] = ws.get("activeEnvironmentId").cloned().unwrap_or(Value::Null);
        save_config(&cfg)?;
        notify_config_changed();
        Ok(json!({ "ok": true, "config": load_config() }))
    });

    registry.register_sync("entity.load", |params, _| {
        let engine_kind = to_engine_kind(str_of(&params, "kind"));
        let (ws_id, id) = (str_of(&params, "workspaceId"), str_of(&params, "id"));
        let Some(entity) = load_entity(ws_id, engine_kind, id) else { return Ok(json!({ "ok": false })) };
        if ENABLED_STATE_KINDS.contains(&engine_kind) {
            let enabled = read_enabled_set(ws_id, engine_kind).is_some_and(|set| set.contains(id));
            return Ok(json!({ "ok": true, "entity": merged(&entity, &json!({ "enabled": enabled })) }));
        }
        Ok(json!({ "ok": true, "entity": entity }))
    });

    registry.register("entity.duplicate", |params, ctx| async move {
        let (ws_id, kind, id) = (str_of(&params, "workspaceId"), str_of(&params, "kind"), str_of(&params, "id"));
        let loaded = ctx
            .registry
            .invoke("entity.load", json!({ "workspaceId": ws_id, "kind": kind, "id": id }), ctx.clone())
            .await?;
        let Some(mut copy) = loaded.get("entity").filter(|e| e.is_object()).cloned() else {
            return Err(EngineError::not_found(format!("No {kind} entity \"{id}\" in workspace \"{ws_id}\".")));
        };
        if let Some(object) = copy.as_object_mut() {
            for key in ["id", "createdAt", "updatedAt"] {
                object.shift_remove(key);
            }
            // A copy of something that can be switched on starts switched off: it matches exactly what its
            // original matches, and creating it enabled would switch the original off.
            if matches!(kind, "mocks" | "proxyRules" | "mappings") {
                object.insert("enabled".into(), json!(false));
            } else {
                object.shift_remove("enabled");
            }
            let name = object.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).map(str::to_string);
            if let Some(name) = name {
                object.insert("name".into(), json!(format!("{name} copy")));
            }
        }
        let created = ctx
            .registry
            .invoke("entity.create", json!({ "kind": kind, "entity": copy, "workspaceId": ws_id }), ctx.clone())
            .await?;
        match created.get("entity") {
            Some(entity) if !entity.is_null() => Ok(json!({ "entity": entity })),
            _ => {
                let reason = created.get("error").and_then(Value::as_str).unwrap_or("create failed");
                Err(EngineError::conflict(format!("Could not duplicate {kind} entity \"{id}\": {reason}.")))
            }
        }
    });

    registry.register_sync("entity.setEnabled", |params, ctx| {
        let (ws_id, kind, id) = (str_of(&params, "workspaceId"), str_of(&params, "kind"), str_of(&params, "id"));
        let enabled = truthy(&params, "enabled");

        if enabled && ctx.may_affect_unnamed_entities() {
            match kind {
                "mocks" => disable_same_signature(ws_id, "mocks", "mocks", id, mock_signature)?,
                "rules" => disable_same_signature(ws_id, "rules", "proxyRules", id, rule_signature)?,
                _ => {}
            }
        }

        sync_enabled_set(ws_id, kind, id, enabled)?;
        notify_config_changed();
        emit_entity_status(ws_id);
        Ok(json!({ "ok": true }))
    });
}
