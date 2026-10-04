//! `entity.create`, `entity.update`, `entity.delete` and `entity.list`.
//!
//! A config snapshot (`load_config`) holds full entities for mocks, mappings and environments but only stubs for
//! the request-like kinds, so each path below reads the snapshot or the entity file exactly where it must. Enabled
//! state is never part of an entity file: it is written through `enabled.json`.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{
    env::effective_environment_id,
    kernel::{CommandRegistry, bus, kinds::to_engine_kind},
    protocol::{EngineError, Result},
    runtime::{entity_status::emit_entity_status, instances::notify_config_changed},
    utils::{
        id::{generate_id, now_ms},
        json::{array_mut, array_of, is_truthy, merged, object_mut, opt_str, str_of, truthy},
    },
    workspace::{
        config::{load_config, save_config},
        deletions::{pending_entity, remove_entity},
        fs::{
            delete_entity_file, find_entity_file, read_all_entities, sync_enabled_set, upsert_name_entry, write_entity,
            write_flat_entity,
        },
        kinds::{CrudKind, crud_kind, is_simple_kind},
        settings::{load_settings, save_settings},
        status_cache::invalidate_cache,
    },
};

fn not_routed(command: &str, kind: &str) -> EngineError {
    EngineError::engine(format!("{command}: kind \"{kind}\" is not routed through the CommandRegistry yet"))
}

/// The workspace an entity belongs to: its own `workspaceId`, else the active one.
fn workspace_of(entity: &Value, cfg: &Value) -> String {
    opt_str(entity, "workspaceId").unwrap_or_else(|| str_of(cfg, "activeWorkspaceId")).to_string()
}

/// The name of the folder an entity sits in, for a kind that has folders.
fn folder_name_of(cfg: &Value, kind: &CrudKind, entity: &Value) -> Option<String> {
    let folders_key = kind.folder_config_key?;
    let folder_id = opt_str(entity, "folderId").filter(|id| !id.is_empty())?;
    array_of(cfg, folders_key)
        .iter()
        .find(|folder| str_of(folder, "id") == folder_id)
        .and_then(|folder| opt_str(folder, "name"))
        .map(str::to_string)
}

/// The kind's other entities in the workspace: what `validate` checks an entity against.
fn siblings_of(cfg: &Value, kind: &CrudKind, ws_id: &str, id: &str) -> Vec<Value> {
    array_of(cfg, kind.config_key)
        .iter()
        .filter(|e| str_of(e, "id") != id && opt_str(e, "workspaceId").unwrap_or(ws_id) == ws_id)
        .filter(|e| !truthy(e, "deleted"))
        .cloned()
        .collect()
}

fn write_entity_file(cfg: &Value, kind: &CrudKind, ws_id: &str, id: &str, entity: &Value) -> Result<()> {
    if kind.is_flat {
        write_flat_entity(ws_id, kind.kind, id, entity)?;
    } else {
        write_entity(ws_id, kind.kind, id, entity, folder_name_of(cfg, kind, entity).as_deref())?;
    }
    Ok(())
}

/// A lone environment is selected implicitly. Adding a second one would end that, so the implicit selection is
/// written down first.
fn pin_implicit_environment(workspace_id: Option<&str>) -> Result<()> {
    let mut settings = load_settings();
    let ws_id = workspace_id.map(str::to_string).unwrap_or_else(|| settings.active_workspace_id());
    let Some(stored) = settings.workspace(&ws_id).map(|_| settings.stored_environment_id(&ws_id)) else {
        return Ok(());
    };
    let implicit = effective_environment_id(&read_all_entities(&ws_id, "environments"), stored.as_deref());
    let Some(implicit) = implicit else { return Ok(()) };
    if Some(&implicit) == stored.as_ref() {
        return Ok(());
    }
    if let Some(ws) = settings.workspace_mut(&ws_id) {
        ws["activeEnvironmentId"] = json!(implicit);
    }
    save_settings(&settings)?;
    Ok(())
}

fn create_entity_core(
    kind: &CrudKind,
    entity: &Value,
    workspace_id: Option<&str>,
    may_resolve_conflicts: bool,
) -> Result<Value> {
    let mut cfg = load_config();
    let ws_id = workspace_id.map(str::to_string).unwrap_or_else(|| workspace_of(entity, &cfg));

    let mut new_entity = merged(entity, &json!({ "id": generate_id(), "createdAt": now_ms(), "workspaceId": ws_id }));
    let id = str_of(&new_entity, "id").to_string();
    if let Some(validate) = kind.validate {
        validate(&new_entity, &siblings_of(&cfg, kind, &ws_id, &id))?;
    }

    if kind.has_enabled_state && new_entity.get("enabled").is_none() {
        new_entity["enabled"] = json!(true);
    }

    array_mut(&mut cfg, kind.config_key);

    if let Some(on_conflict) = kind.on_add_conflict {
        if may_resolve_conflicts {
            let before: HashMap<String, Option<Value>> = array_of(&cfg, kind.config_key)
                .iter()
                .map(|e| (str_of(e, "id").to_string(), e.get("enabled").cloned()))
                .collect();
            on_conflict(&mut cfg, &new_entity);
            if kind.has_enabled_state {
                for e in array_of(&cfg, kind.config_key) {
                    let other = str_of(e, "id");
                    if before.get(other).is_some_and(|was| *was != e.get("enabled").cloned()) {
                        sync_enabled_set(&ws_id, kind.kind, other, e.get("enabled").is_some_and(is_truthy))?;
                    }
                }
            }
        }
    }

    let list = array_mut(&mut cfg, kind.config_key);
    if kind.kind == "rules" || kind.kind == "mocks" {
        list.insert(0, new_entity.clone());
    } else {
        list.push(new_entity.clone());
    }

    save_config(&cfg)?;
    write_entity_file(&cfg, kind, &ws_id, &id, &new_entity)?;

    if kind.has_enabled_state {
        sync_enabled_set(&ws_id, kind.kind, &id, new_entity.get("enabled").is_some_and(is_truthy))?;
    }

    if let Some(name_entry) = kind.name_entry {
        if !kind.is_flat {
            upsert_name_entry(&ws_id, kind.kind, &id, name_entry(&new_entity))?;
        }
    }

    notify_config_changed();
    emit_entity_status(&ws_id);

    if kind.kind == "mocks" || kind.kind == "requests" {
        bus().emit("entity.changed", json!({ "wsId": ws_id, "kind": kind.kind, "id": id, "action": "created" }));
    }
    Ok(new_entity)
}

fn update_entity_core(kind: &CrudKind, entity: &Value) -> Result<Value> {
    let mut cfg = load_config();
    let ws_id = workspace_of(entity, &cfg);
    let id = opt_str(entity, "id").ok_or_else(|| EngineError::bad_request("entity.id is required"))?.to_string();
    // Writing it would bring its file back while it is still listed as deleted.
    if pending_entity(&ws_id, kind.kind, &id).is_some() && find_entity_file(&ws_id, kind.kind, &id).is_none() {
        return Err(EngineError::conflict("It is deleted. Revert the delete to change it."));
    }
    if let Some(validate) = kind.validate {
        validate(entity, &siblings_of(&cfg, kind, &ws_id, &id))?;
    }

    let list = array_mut(&mut cfg, kind.config_key);
    if let Some(slot) = list.iter_mut().find(|e| str_of(e, "id") == id) {
        *slot = entity.clone();
    }

    save_config(&cfg)?;
    write_entity_file(&cfg, kind, &ws_id, &id, entity)?;

    if let Some(name_entry) = kind.name_entry {
        if !kind.is_flat {
            upsert_name_entry(&ws_id, kind.kind, &id, name_entry(entity))?;
        }
    }

    notify_config_changed();
    emit_entity_status(&ws_id);
    Ok(json!({ "ok": true }))
}

async fn delete_entity_core(kind: &CrudKind, id: &str) -> Result<Value> {
    if kind.kind == "environments" && id == "__global__" {
        return Ok(json!({ "ok": false, "error": "cannot_delete_global" }));
    }

    let mut cfg = load_config();
    let ws_id = str_of(&cfg, "activeWorkspaceId").to_string();

    let Some(entity) = array_of(&cfg, kind.config_key).iter().find(|e| str_of(e, "id") == id).cloned() else {
        return Ok(json!({ "ok": true, "pending": false }));
    };
    // Already deleted, and waiting to be confirmed or reverted.
    if truthy(&entity, "deleted") {
        return Ok(json!({ "ok": true, "pending": true }));
    }
    array_mut(&mut cfg, kind.config_key).retain(|e| str_of(e, "id") != id);

    if kind.kind == "mappings" {
        array_mut(&mut cfg, "proxyRules").retain(|rule| {
            let target_type =
                rule.get("targetType").filter(|v| !v.is_null()).and_then(Value::as_str).unwrap_or("mapping");
            target_type != "mapping" || str_of(rule, "targetMappingId") != id
        });
    }

    if kind.kind == "environments" && opt_str(&cfg, "activeEnvironmentId") == Some(id) {
        object_mut(&mut cfg).insert("activeEnvironmentId".into(), Value::Null);
    }

    save_config(&cfg)?;

    let pending = remove_entity(&ws_id, kind, &entity).await?;

    notify_config_changed();
    invalidate_cache(&ws_id);
    emit_entity_status(&ws_id);
    Ok(json!({ "ok": true, "pending": pending }))
}

fn create_simple_entity(kind: &str, entity: &Value, workspace_id: Option<&str>) -> Result<Value> {
    let cfg = load_config();
    let ws_id = workspace_id.map(str::to_string).unwrap_or_else(|| workspace_of(entity, &cfg));
    let new_entity = merged(entity, &json!({ "id": generate_id(), "createdAt": now_ms(), "workspaceId": ws_id }));
    write_entity(&ws_id, kind, str_of(&new_entity, "id"), &new_entity, None)?;
    Ok(new_entity)
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register("entity.create", |params, ctx| async move {
        let wire_kind = str_of(&params, "kind");
        let engine_kind = to_engine_kind(wire_kind);
        let mut entity = params.get("entity").cloned().unwrap_or_else(|| json!({}));
        let workspace_id =
            opt_str(&params, "workspaceId").or_else(|| opt_str(&entity, "workspaceId")).map(str::to_string);
        // `workspaceId` takes its place right after the caller's own fields, ahead of `id` and `createdAt`.
        if let Some(fields) = entity.as_object_mut() {
            fields.insert("workspaceId".to_string(), json!(workspace_id));
        }
        let workspace_id = workspace_id.as_deref();
        if let Some(kind) = crud_kind(engine_kind) {
            if kind.kind == "environments" {
                pin_implicit_environment(workspace_id)?;
            }
            let created = create_entity_core(kind, &entity, workspace_id, ctx.may_affect_unnamed_entities())?;
            return Ok(json!({ "id": str_of(&created, "id"), "entity": created }));
        }
        if is_simple_kind(engine_kind) {
            let created = create_simple_entity(engine_kind, &entity, workspace_id)?;
            return Ok(json!({ "id": str_of(&created, "id"), "entity": created }));
        }
        Err(not_routed("entity.create", wire_kind))
    });

    registry.register_sync("entity.update", |params, _| {
        let wire_kind = str_of(&params, "kind");
        let Some(kind) = crud_kind(to_engine_kind(wire_kind)) else {
            return Err(not_routed("entity.update", wire_kind));
        };
        update_entity_core(kind, params.get("entity").unwrap_or(&Value::Null))
    });

    registry.register("entity.delete", |params, _| async move {
        let wire_kind = str_of(&params, "kind");
        let engine_kind = to_engine_kind(wire_kind);
        let id = str_of(&params, "id");
        if let Some(kind) = crud_kind(engine_kind) {
            return delete_entity_core(kind, id).await;
        }
        if is_simple_kind(engine_kind) {
            let cfg = load_config();
            delete_entity_file(str_of(&cfg, "activeWorkspaceId"), engine_kind, id)?;
            return Ok(json!({ "ok": true }));
        }
        Err(not_routed("entity.delete", wire_kind))
    });

    registry.register_sync("entity.list", |params, _| {
        let wire_kind = str_of(&params, "kind");
        let engine_kind = to_engine_kind(wire_kind);
        if crud_kind(engine_kind).is_none() && !is_simple_kind(engine_kind) {
            return Err(not_routed("entity.list", wire_kind));
        }
        Ok(json!({ "entities": read_all_entities(str_of(&params, "workspaceId"), engine_kind) }))
    });
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::{
        kernel::{CommandContext, Scope, SessionIdentity},
        protocol::ErrorCode,
        testing::scratch_data_root,
        workspace::{
            fs::{entity_dir, read_enabled_set, read_names_index, ws_dir},
            git_store::{CommitMutation, check_git_installed, commit_mutation, init_workspace_repo},
        },
    };

    const WS: &str = "ws1";

    fn context() -> CommandContext {
        let mut registry = CommandRegistry::new();
        crate::workspace::entities::register(&mut registry);
        CommandContext::internal(Arc::new(registry))
    }

    fn write_only(ctx: &CommandContext) -> CommandContext {
        let session = SessionIdentity { scopes: Some(HashSet::from([Scope::Write])), ..Default::default() };
        CommandContext { registry: ctx.registry.clone(), session: Some(session) }
    }

    async fn call(ctx: &CommandContext, action: &str, payload: Value) -> Result<Value> {
        ctx.registry.invoke(action, payload, ctx.clone()).await
    }

    fn enabled(kind: &str) -> Vec<String> {
        read_enabled_set(WS, kind).map(|set| set.into_iter().collect()).unwrap_or_default()
    }

    fn mock(url: &str) -> Value {
        json!({ "name": "M", "method": "GET", "urlPattern": url, "responseStatus": 200, "responseBody": "{}" })
    }

    #[tokio::test]
    async fn requests_are_created_listed_loaded_updated_and_deleted() {
        let _root = scratch_data_root();
        let ctx = context();
        let folder =
            call(&ctx, "folder.add", json!({ "kind": "request", "name": "Auth", "parentId": null })).await.unwrap();

        let entity = json!({ "name": "Login", "method": "POST", "url": "http://a/login", "folderId": folder["id"] });
        let created = call(&ctx, "entity.create", json!({ "kind": "requests", "entity": entity })).await.unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        assert_eq!(created["entity"]["workspaceId"], WS);
        assert!(entity_dir(WS, "requests").join("Auth").join(format!("{id}.json")).exists());
        assert_eq!(read_names_index(WS, "requests")[&id]["url"], "http://a/login");

        let listed = call(&ctx, "entity.list", json!({ "workspaceId": WS, "kind": "requests" })).await.unwrap();
        assert_eq!(listed["entities"].as_array().unwrap().len(), 1);

        let loaded =
            call(&ctx, "entity.load", json!({ "workspaceId": WS, "kind": "requests", "id": id })).await.unwrap();
        assert_eq!(loaded["ok"], true);
        assert_eq!(loaded["entity"]["name"], "Login");
        assert!(loaded["entity"].get("enabled").is_none());

        let mut changed = created["entity"].clone();
        changed["name"] = json!("Sign in");
        assert_eq!(
            call(&ctx, "entity.update", json!({ "kind": "requests", "entity": changed })).await.unwrap(),
            json!({ "ok": true })
        );
        assert_eq!(read_names_index(WS, "requests")[&id]["name"], "Sign in");

        assert_eq!(
            call(&ctx, "entity.delete", json!({ "kind": "requests", "id": id })).await.unwrap(),
            json!({ "ok": true, "pending": false })
        );
        assert!(!entity_dir(WS, "requests").join("Auth").join(format!("{id}.json")).exists());
        assert!(read_names_index(WS, "requests").get(&id).is_none());
        let gone = call(&ctx, "entity.load", json!({ "workspaceId": WS, "kind": "requests", "id": id })).await.unwrap();
        assert_eq!(gone, json!({ "ok": false }));
    }

    #[tokio::test]
    async fn a_request_without_a_url_is_rejected() {
        let _root = scratch_data_root();
        let ctx = context();
        let err = call(&ctx, "entity.create", json!({ "kind": "requests", "entity": { "method": "GET" } }))
            .await
            .unwrap_err();
        assert_eq!(err.message, "url is required for requests");
    }

    #[tokio::test]
    async fn mappings_are_flat_validated_and_tracked_in_the_enabled_set() {
        let _root = scratch_data_root();
        let ctx = context();

        let bad = call(
            &ctx,
            "entity.create",
            json!({ "kind": "mappings", "entity": { "domain": "x.example", "target": "a:1" } }),
        )
        .await
        .unwrap_err();
        assert_eq!(bad.code, ErrorCode::BadRequest);
        assert_eq!(bad.details, Some(json!({ "field": "domain" })));

        let good = json!({ "domain": "app.localhost", "target": "localhost:3000" });
        let created = call(&ctx, "entity.create", json!({ "kind": "mappings", "entity": good })).await.unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        assert!(entity_dir(WS, "mappings").join(format!("{id}.json")).exists());
        assert!(!entity_dir(WS, "mappings").join("names.json").exists(), "flat kinds keep no names index");
        assert_eq!(enabled("mappings"), vec![id.clone()]);

        let dup = call(&ctx, "entity.create", json!({ "kind": "mappings", "entity": good })).await.unwrap_err();
        assert_eq!(dup.details, Some(json!({ "field": "domain" })));

        let loaded =
            call(&ctx, "entity.load", json!({ "workspaceId": WS, "kind": "mappings", "id": id })).await.unwrap();
        assert_eq!(loaded["entity"]["enabled"], true);

        call(&ctx, "entity.delete", json!({ "kind": "mappings", "id": id })).await.unwrap();
        assert!(!entity_dir(WS, "mappings").join(format!("{id}.json")).exists());
        assert!(enabled("mappings").is_empty());
        assert!(!ws_dir(WS).join("pending-deletions.json").exists());
    }

    #[tokio::test]
    async fn deleting_a_published_mapping_records_a_pending_deletion() {
        if !check_git_installed().await {
            return;
        }
        let _root = scratch_data_root();
        init_workspace_repo(WS).await.unwrap();
        let ctx = context();
        let good = json!({ "domain": "app.localhost", "target": "localhost:3000", "label": "App" });
        let id = call(&ctx, "entity.create", json!({ "kind": "mappings", "entity": good })).await.unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let rel_path = format!("mappings/{id}.json");
        let commit = CommitMutation {
            action: "create",
            entity: "mapping",
            entity_id: &id,
            entity_name: "App",
            workspace_id: WS,
            rel_path: &rel_path,
            actor: None,
            changed_fields: &[],
        };
        assert!(!commit_mutation(commit).await.unwrap().is_empty());

        call(&ctx, "entity.delete", json!({ "kind": "mappings", "id": id })).await.unwrap();
        let pending = crate::workspace::fs::get_pending_deletions(WS, "mappings");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["id"], id.as_str());
        assert_eq!(pending[0]["name"], "App");
        assert_eq!(pending[0]["url"], "app.localhost");
        assert_eq!(pending[0]["folderId"], Value::Null);
        assert_eq!(pending[0]["path"], rel_path.as_str());

        // It stays listed, marked as deleted, and no later write brings its file back or trips over it.
        let listed = load_config()["mappings"].as_array().unwrap().clone();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["deleted"], true);
        call(&ctx, "entity.create", json!({ "kind": "mappings", "entity": good })).await.unwrap();
        assert!(!entity_dir(WS, "mappings").join(format!("{id}.json")).exists());
        assert_eq!(load_config()["mappings"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_global_environment_cannot_be_deleted() {
        let _root = scratch_data_root();
        let ctx = context();
        let out = call(&ctx, "entity.delete", json!({ "kind": "environments", "id": "__global__" })).await.unwrap();
        assert_eq!(out, json!({ "ok": false, "error": "cannot_delete_global" }));
    }

    #[tokio::test]
    async fn a_second_environment_pins_the_implicitly_selected_one() {
        let _root = scratch_data_root();
        let ctx = context();
        let first = call(
            &ctx,
            "entity.create",
            json!({ "kind": "environments", "entity": { "name": "Dev", "variables": [] } }),
        )
        .await
        .unwrap();
        assert_eq!(load_settings().stored_environment_id(WS), None);
        call(&ctx, "entity.create", json!({ "kind": "environments", "entity": { "name": "Prod", "variables": [] } }))
            .await
            .unwrap();
        assert_eq!(load_settings().stored_environment_id(WS).as_deref(), first["id"].as_str());
    }

    #[tokio::test]
    async fn mocks_are_enabled_on_create_and_shadow_a_conflicting_one() {
        let _root = scratch_data_root();
        let ctx = context();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let listener = bus().on("entity.changed", move |payload| sink.lock().unwrap().push(payload.clone()));

        let first = call(&ctx, "entity.create", json!({ "kind": "mocks", "entity": mock("/a") })).await.unwrap();
        let first_id = first["id"].as_str().unwrap().to_string();
        assert_eq!(enabled("mocks"), vec![first_id.clone()]);

        let second = call(&ctx, "entity.create", json!({ "kind": "mocks", "entity": mock("/a") })).await.unwrap();
        let second_id = second["id"].as_str().unwrap().to_string();
        assert_eq!(enabled("mocks"), vec![second_id.clone()], "the older mock is switched off");
        assert_eq!(load_config()["mocks"].as_array().unwrap().len(), 2);

        let third =
            call(&write_only(&ctx), "entity.create", json!({ "kind": "mocks", "entity": mock("/a") })).await.unwrap();
        let third_id = third["id"].as_str().unwrap().to_string();
        assert_eq!(enabled("mocks"), vec![second_id.clone(), third_id], "a caller without admin leaves others alone");

        bus().off(listener);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], json!({ "wsId": WS, "kind": "mocks", "id": first_id, "action": "created" }));

        let invalid =
            call(&ctx, "entity.create", json!({ "kind": "mocks", "entity": { "method": "GET" } })).await.unwrap_err();
        assert_eq!(invalid.message, "urlPattern is required for mocks");
    }

    #[tokio::test]
    async fn rules_use_the_protocol_kind_name_and_are_stored_as_rules() {
        let _root = scratch_data_root();
        let ctx = context();
        let rule = json!({ "name": "R", "pattern": "api", "useRegex": false, "targetType": "mapping" });
        let created = call(&ctx, "entity.create", json!({ "kind": "proxyRules", "entity": rule })).await.unwrap();
        let id = created["id"].as_str().unwrap();
        assert!(entity_dir(WS, "rules").join(format!("{id}.json")).exists());
        assert_eq!(read_names_index(WS, "rules")[id]["url"], "api");
        assert_eq!(enabled("rules"), vec![id.to_string()]);
        let listed = call(&ctx, "entity.list", json!({ "workspaceId": WS, "kind": "proxyRules" })).await.unwrap();
        assert_eq!(listed["entities"][0]["id"], id);
    }

    #[tokio::test]
    async fn simple_kinds_are_written_straight_to_disk() {
        let _root = scratch_data_root();
        let ctx = context();
        let created = call(&ctx, "entity.create", json!({ "kind": "protoFiles", "entity": { "name": "a.proto" } }))
            .await
            .unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        let listed = call(&ctx, "entity.list", json!({ "workspaceId": WS, "kind": "protoFiles" })).await.unwrap();
        assert_eq!(listed["entities"].as_array().unwrap().len(), 1);
        assert_eq!(
            call(&ctx, "entity.delete", json!({ "kind": "protoFiles", "id": id })).await.unwrap(),
            json!({ "ok": true })
        );
        let listed = call(&ctx, "entity.list", json!({ "workspaceId": WS, "kind": "protoFiles" })).await.unwrap();
        assert!(listed["entities"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_duplicate_is_a_renamed_copy_and_starts_disabled_when_it_can_be_enabled() {
        let _root = scratch_data_root();
        let ctx = context();
        let request = json!({ "name": "Ping", "method": "GET", "url": "http://a/ping" });
        let original = call(&ctx, "entity.create", json!({ "kind": "requests", "entity": request })).await.unwrap();
        let copy =
            call(&ctx, "entity.duplicate", json!({ "workspaceId": WS, "kind": "requests", "id": original["id"] }))
                .await
                .unwrap();
        assert_eq!(copy["entity"]["name"], "Ping copy");
        assert_ne!(copy["entity"]["id"], original["id"]);

        let original = call(&ctx, "entity.create", json!({ "kind": "mocks", "entity": mock("/a") })).await.unwrap();
        let copy = call(&ctx, "entity.duplicate", json!({ "workspaceId": WS, "kind": "mocks", "id": original["id"] }))
            .await
            .unwrap();
        assert_eq!(copy["entity"]["enabled"], false);
        assert_eq!(enabled("mocks"), vec![original["id"].as_str().unwrap().to_string()]);

        let missing = call(&ctx, "entity.duplicate", json!({ "workspaceId": WS, "kind": "requests", "id": "nope" }))
            .await
            .unwrap_err();
        assert_eq!(missing.code, ErrorCode::NotFound);
    }

    #[tokio::test]
    async fn enabling_switches_off_what_it_shadows_for_a_caller_that_may() {
        let _root = scratch_data_root();
        let ctx = context();
        let a = call(&ctx, "entity.create", json!({ "kind": "mocks", "entity": mock("/a") })).await.unwrap();
        let a_id = a["id"].as_str().unwrap().to_string();
        let mut other = mock("/a");
        other["enabled"] = json!(false);
        let b = call(&ctx, "entity.create", json!({ "kind": "mocks", "entity": other })).await.unwrap();
        let b_id = b["id"].as_str().unwrap().to_string();
        assert_eq!(enabled("mocks"), vec![a_id.clone()]);

        let params = json!({ "workspaceId": WS, "kind": "mocks", "id": b_id, "enabled": true });
        call(&write_only(&ctx), "entity.setEnabled", params.clone()).await.unwrap();
        assert_eq!(
            enabled("mocks"),
            vec![a_id.clone(), b_id.clone()],
            "the toggle is unconditional, the shadowing is not"
        );

        call(&ctx, "entity.setEnabled", json!({ "workspaceId": WS, "kind": "mocks", "id": a_id, "enabled": false }))
            .await
            .unwrap();
        call(&ctx, "entity.setEnabled", json!({ "workspaceId": WS, "kind": "mocks", "id": a_id, "enabled": true }))
            .await
            .unwrap();
        assert_eq!(enabled("mocks"), vec![a_id]);
    }

    #[tokio::test]
    async fn setting_the_active_workspace_returns_the_new_config() {
        let _root = scratch_data_root();
        let ctx = context();
        assert_eq!(call(&ctx, "workspace.setActive", json!({ "id": "nope" })).await.unwrap(), json!({ "ok": false }));
        let out = call(&ctx, "workspace.setActive", json!({ "id": WS })).await.unwrap();
        assert_eq!(out["ok"], true);
        assert_eq!(out["config"]["activeWorkspaceId"], WS);
    }
}
