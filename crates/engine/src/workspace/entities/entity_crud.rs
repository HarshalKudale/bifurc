//! `entity.create`, `entity.update`, `entity.delete` and `entity.list`.
//!
//! A config snapshot (`load_config`) holds full entities for mocks, mappings and environments but only stubs for
//! the request-like kinds, so each path below reads the snapshot or the entity file exactly where it must. Enabled
//! state is never part of an entity file: it is written through `enabled.json`.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::{
    env::effective_environment_id,
    kernel::{
        CommandRegistry, bus,
        kinds::{to_engine_kind, to_protocol_kind},
    },
    protocol::{EngineError, Result},
    runtime::{
        entity_status::{emit_entity_status, emit_entity_status_after_change},
        instances::notify_config_changed,
    },
    utils::{
        id::{generate_id, now_ms},
        json::{array_mut, array_of, is_truthy, merged, object_mut, opt_str, str_of, truthy},
    },
    workspace::{
        config::{load_config, save_config},
        deletions::{pending_entity, remove_entity},
        fs::{
            delete_entity_file, entity_lock, find_entity_file, read_all_entities, resolve_folder, sync_enabled_set,
            upsert_name_entry, write_entity, write_flat_entity,
        },
        kinds::{CrudKind, crud_kind, is_simple_kind, requires_folder},
        settings::{load_settings, save_settings},
        stamp,
        status_cache::invalidate_cache,
        tree,
    },
};

fn not_routed(command: &str, kind: &str) -> EngineError {
    EngineError::engine(format!("{command}: kind \"{kind}\" is not routed through the CommandRegistry yet"))
        .keyed("entity.not_routed", json!({ "command": command, "kind": kind }))
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

/// Settle the folder of an entity of a kind that keeps all of them in folders: the one it names, else the Default
/// folder, which is made when it is missing. `folderId` of the entity is rewritten to match.
fn place_in_folder(kind: &CrudKind, ws_id: &str, entity: &mut Value) -> Result<()> {
    if !requires_folder(kind) {
        return Ok(());
    }
    let (folder_id, _) = resolve_folder(ws_id, kind.kind, opt_str(entity, "folderId"))?;
    object_mut(entity).insert("folderId".into(), json!(folder_id));
    Ok(())
}

fn write_entity_file(cfg: &Value, kind: &CrudKind, ws_id: &str, id: &str, entity: &Value) -> Result<()> {
    if kind.is_flat {
        write_flat_entity(ws_id, kind.kind, id, entity)?;
    } else if tree::is_tree_kind(kind.kind) {
        // A tree's folder is named by its id: its name is only the last part of where it is.
        write_entity(ws_id, kind.kind, id, entity, opt_str(entity, "folderId"))?;
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
    let ws_id = workspace_id.map(str::to_string).unwrap_or_else(|| workspace_of(entity, &load_config()));

    let mut new_entity = merged(entity, &json!({ "id": generate_id(), "createdAt": now_ms(), "workspaceId": ws_id }));
    if let Some(ty) = tree::type_of_kind(kind.kind) {
        new_entity["type"] = json!(ty);
    }
    let id = str_of(&new_entity, "id").to_string();
    // After the folder is settled, so the snapshot lists a Default folder this call has just made.
    place_in_folder(kind, &ws_id, &mut new_entity)?;
    let mut cfg = load_config();
    if let Some(validate) = kind.validate {
        validate(&new_entity, &siblings_of(&cfg, kind, &ws_id, &id))?;
    }

    if kind.has_enabled_state && new_entity.get("enabled").is_none() {
        new_entity["enabled"] = json!(true);
    }

    array_mut(&mut cfg, kind.config_key);

    if let Some(on_conflict) = kind.on_add_conflict
        && may_resolve_conflicts
    {
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

    if let Some(name_entry) = kind.name_entry
        && !kind.is_flat
    {
        upsert_name_entry(&ws_id, kind.kind, &id, name_entry(&new_entity))?;
    }

    notify_config_changed();
    emit_entity_status(&ws_id);

    if kind.kind == "mocks" || kind.kind == "requests" {
        bus().emit("entity.changed", json!({ "wsId": ws_id, "kind": kind.kind, "id": id, "action": "created" }));
    }
    Ok(new_entity)
}

fn update_entity_core(kind: &CrudKind, entity: &Value) -> Result<Value> {
    let ws_id = workspace_of(entity, &load_config());
    let id = opt_str(entity, "id")
        .ok_or_else(|| EngineError::bad_request("entity.id is required").keyed("entity.id_required", json!({})))?
        .to_string();
    if let Some(version) = entity_lock(&ws_id, kind.kind, &id) {
        return Err(stamp::update_required(&version));
    }
    // Writing it would bring its file back while it is still listed as deleted.
    if pending_entity(&ws_id, kind.kind, &id).is_some() && find_entity_file(&ws_id, kind.kind, &id).is_none() {
        return Err(EngineError::conflict("It is deleted. Revert the delete to change it.")
            .keyed("entity.deleted_revert", json!({})));
    }
    let mut entity = entity.clone();
    place_in_folder(kind, &ws_id, &mut entity)?;
    let mut cfg = load_config();
    if let Some(validate) = kind.validate {
        validate(&entity, &siblings_of(&cfg, kind, &ws_id, &id))?;
    }

    let list = array_mut(&mut cfg, kind.config_key);
    if let Some(slot) = list.iter_mut().find(|e| str_of(e, "id") == id) {
        *slot = entity.clone();
    }

    save_config(&cfg)?;
    write_entity_file(&cfg, kind, &ws_id, &id, &entity)?;

    if let Some(name_entry) = kind.name_entry
        && !kind.is_flat
    {
        upsert_name_entry(&ws_id, kind.kind, &id, name_entry(&entity))?;
    }

    notify_config_changed();
    bus().emit(
        "entity.changed",
        json!({
            "wsId": ws_id, "kind": to_protocol_kind(kind.kind), "id": id, "action": "updated",
            "listing": listing_of(kind.kind, &entity),
        }),
    );
    emit_entity_status_after_change(&ws_id);
    Ok(json!({ "ok": true }))
}

/// The fields of an entity that `config.get` lists for it, so a client can patch its snapshot from the event.
fn listing_of(kind: &str, entity: &Value) -> Value {
    const FIELDS: [&str; 12] = [
        "id",
        "name",
        "folderId",
        "method",
        "url",
        "endpointUrl",
        "serverAddress",
        "serviceName",
        "methodName",
        "urlPattern",
        "domain",
        "target",
    ];
    let mut listing = serde_json::Map::new();
    for field in FIELDS.into_iter().chain(["label"]).chain((kind == "environments").then_some("variables")) {
        if let Some(value) = entity.get(field) {
            listing.insert(field.into(), value.clone());
        }
    }
    // A request listing has a method even when the request does not say one.
    if kind == "requests" && !listing.contains_key("method") {
        listing.insert("method".into(), json!("GET"));
    }
    Value::Object(listing)
}

async fn delete_entity_core(kind: &CrudKind, id: &str) -> Result<Value> {
    if kind.kind == "environments" && id == "__global__" {
        return Ok(json!({
            "ok": false, "error": "cannot_delete_global", "errorKey": "env.cannot_delete_global", "errorParams": {},
        }));
    }

    let mut cfg = load_config();
    let ws_id = str_of(&cfg, "activeWorkspaceId").to_string();
    if let Some(version) = entity_lock(&ws_id, kind.kind, id) {
        return Err(stamp::update_required(&version));
    }

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
    bus().emit(
        "entity.changed",
        json!({ "wsId": ws_id, "kind": to_protocol_kind(kind.kind), "id": id, "action": "deleted", "pending": pending }),
    );
    emit_entity_status_after_change(&ws_id);
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

    // What a tree's index lists of the requests (or mocks) whose name, address or other listed field contains
    // `query`, by name. No file is read. `kinds` narrows the requests to some types.
    registry.register_sync("entity.search", |params, _| {
        let ws_id = str_of(&params, "workspaceId");
        let scope = if str_of(&params, "scope") == "mocks" { tree::Scope::Mocks } else { tree::Scope::Requests };
        let types: Vec<String> = array_of(&params, "kinds")
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|kind| tree::type_of_kind(to_engine_kind(kind)))
            .map(str::to_string)
            .collect();
        let limit = params.get("limit").and_then(Value::as_i64).unwrap_or(200).clamp(1, 1000);
        let results: Vec<Value> = tree::search(ws_id, scope, str_of(&params, "query"), &types, limit)
            .iter()
            .map(|entry| {
                let mut found = entry.details.as_object().cloned().unwrap_or_default();
                let kind = scope.kind_of_type(&entry.ty).unwrap_or(scope.dir());
                found.insert("id".into(), json!(entry.id));
                found.insert("kind".into(), json!(to_protocol_kind(kind)));
                found.insert("type".into(), json!(entry.ty));
                found.insert("folderId".into(), json!(entry.folder_id));
                found.insert("path".into(), json!(tree::rel_path(scope, entry)));
                Value::Object(found)
            })
            .collect();
        Ok(json!({ "results": results }))
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
        utils::json::{read_json, write_json_pretty},
        workspace::{
            fs::{entity_dir, read_enabled_set, read_index, read_names_index, ws_dir},
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

    /// Leave an entity's file as another engine would have: stamped `version` with a field this engine does
    /// not know, or with no stamp at all, as an engine from before the stamp wrote it.
    fn written_by(kind: &str, id: &str, version: Option<&str>) -> std::path::PathBuf {
        let file = find_entity_file(WS, kind, id).expect("the entity has a file");
        let mut entity = read_json(&file).expect("the file is an entity");
        match version {
            Some(version) => {
                entity[stamp::VERSION_FIELD] = json!(version);
                entity["somethingNewer"] = json!({ "kept": true });
            }
            None => {
                entity.as_object_mut().unwrap().shift_remove(stamp::VERSION_FIELD);
            }
        }
        write_json_pretty(&file, &entity).unwrap();
        tree::mark_stale(WS);
        file
    }

    fn stamp_of(kind: &str, id: &str) -> Value {
        let file = find_entity_file(WS, kind, id).expect("the entity has a file");
        read_json(&file).unwrap().get(stamp::VERSION_FIELD).cloned().unwrap_or(Value::Null)
    }

    async fn create(ctx: &CommandContext, kind: &str, entity: Value) -> Value {
        call(ctx, "entity.create", json!({ "kind": kind, "entity": entity })).await.unwrap()["entity"].clone()
    }

    fn update_required(result: Result<Value>) -> Value {
        let error = result.expect_err("a locked entity is left alone");
        assert_eq!(error.code, ErrorCode::Unsupported);
        let details = error.details.expect("the error says why");
        assert_eq!(details["reason"], stamp::UPDATE_REQUIRED);
        details["requiredVersion"].clone()
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

        let kept = crate::workspace::response_history::failed(&EngineError::engine("x"), "GET".into(), "u".into());
        crate::workspace::response_history::record(WS, &id, kept);
        assert_eq!(
            call(&ctx, "entity.delete", json!({ "kind": "requests", "id": id })).await.unwrap(),
            json!({ "ok": true, "pending": false })
        );
        assert!(crate::workspace::response_history::entries(WS, &id).is_empty());
        assert!(!entity_dir(WS, "requests").join("Auth").join(format!("{id}.json")).exists());
        assert!(read_names_index(WS, "requests").get(&id).is_none());
        let gone = call(&ctx, "entity.load", json!({ "workspaceId": WS, "kind": "requests", "id": id })).await.unwrap();
        assert_eq!(gone, json!({ "ok": false }));
    }

    #[tokio::test]
    async fn an_update_and_a_delete_announce_what_a_client_needs_to_patch_its_listing() {
        let _root = scratch_data_root();
        let ctx = context();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let listener = bus().on("entity.changed", move |payload| sink.lock().unwrap().push(payload.clone()));

        let entity = json!({ "name": "Login", "method": "POST", "url": "http://a/login", "body": "big" });
        let created = create(&ctx, "requests", entity).await;
        let id = created["id"].as_str().unwrap().to_string();
        let mut changed = created.clone();
        changed["name"] = json!("Sign in");
        call(&ctx, "entity.update", json!({ "kind": "requests", "entity": changed })).await.unwrap();
        call(&ctx, "entity.delete", json!({ "kind": "requests", "id": id })).await.unwrap();

        bus().off(listener);
        let mine: Vec<Value> = events.lock().unwrap().iter().filter(|e| e["id"] == id.as_str()).cloned().collect();
        assert_eq!(mine.len(), 3);
        assert_eq!(mine[1]["action"], "updated");
        assert_eq!(mine[1]["kind"], "requests");
        let listing = &mine[1]["listing"];
        assert_eq!(
            *listing,
            json!({
                "id": id, "name": "Sign in", "folderId": created["folderId"], "method": "POST", "url": "http://a/login",
            })
        );
        assert_eq!(mine[2]["action"], "deleted");
        assert_eq!(mine[2]["pending"], false);
    }

    #[tokio::test]
    async fn nothing_is_saved_outside_a_folder() {
        let _root = scratch_data_root();
        let ctx = context();
        let default_id = |kind: &str| -> Value {
            let folders = tree::folders(WS, tree::Scope::of_kind(kind).unwrap());
            let found: Vec<_> = folders.iter().filter(|f| f.name == "Default").collect();
            assert_eq!(found.len(), 1, "one Default folder per tree");
            json!(found[0].id)
        };

        // No folder, an empty one and one that does not exist all land in Default, which is made on the way.
        let request = |folder: Value| json!({ "name": "R", "method": "GET", "url": "http://a", "folderId": folder });
        let loose =
            call(&ctx, "entity.create", json!({ "kind": "requests", "entity": request(Value::Null) })).await.unwrap();
        let empty =
            call(&ctx, "entity.create", json!({ "kind": "requests", "entity": request(json!("")) })).await.unwrap();
        let dangling =
            call(&ctx, "entity.create", json!({ "kind": "requests", "entity": request(json!("gone")) })).await.unwrap();
        for created in [&loose, &empty, &dangling] {
            assert_eq!(created["entity"]["folderId"], default_id("requests"));
            let id = created["id"].as_str().unwrap();
            assert!(entity_dir(WS, "requests").join("Default").join(format!("{id}.json")).exists());
            assert!(!entity_dir(WS, "requests").join(format!("{id}.json")).exists());
        }

        let mock = call(&ctx, "entity.create", json!({ "kind": "mocks", "entity": mock("/a") })).await.unwrap();
        assert_eq!(mock["entity"]["folderId"], default_id("mocks"));
        let id = mock["id"].as_str().unwrap();
        assert!(entity_dir(WS, "mocks").join("Default").join(format!("{id}.json")).exists());
        let listed = load_config();
        assert_eq!(listed["mocks"][0]["folderId"], default_id("mocks"));

        // A folder of the caller's own is kept, and moving out of it goes to Default, never to the top level.
        let own =
            call(&ctx, "folder.add", json!({ "kind": "request", "name": "Auth", "parentId": null })).await.unwrap();
        let kept = call(&ctx, "entity.create", json!({ "kind": "requests", "entity": request(own["id"].clone()) }))
            .await
            .unwrap();
        assert_eq!(kept["entity"]["folderId"], own["id"]);
        let mut moved = kept["entity"].clone();
        moved["folderId"] = Value::Null;
        call(&ctx, "entity.update", json!({ "kind": "requests", "entity": moved })).await.unwrap();
        let kept_id = kept["id"].as_str().unwrap();
        assert!(entity_dir(WS, "requests").join("Default").join(format!("{kept_id}.json")).exists());
        assert!(!entity_dir(WS, "requests").join("Auth").join(format!("{kept_id}.json")).exists());
        let reloaded =
            call(&ctx, "entity.load", json!({ "workspaceId": WS, "kind": "requests", "id": kept_id })).await.unwrap();
        assert_eq!(reloaded["entity"]["folderId"], default_id("requests"));

        // A duplicate is saved beside its original.
        let copy = call(&ctx, "entity.duplicate", json!({ "workspaceId": WS, "kind": "requests", "id": kept_id }))
            .await
            .unwrap();
        assert_eq!(copy["entity"]["folderId"], default_id("requests"));

        // Only Auth and Default exist: the Default folder is made once and reused.
        assert_eq!(tree::folders(WS, tree::Scope::Requests).len(), 2);

        // A request of another type has no folders of its own: it goes into the same Default folder, and its
        // file says what it is.
        let socket = json!({ "name": "Feed", "url": "ws://a" });
        let socket = call(&ctx, "entity.create", json!({ "kind": "wsConnections", "entity": socket })).await.unwrap();
        assert_eq!(socket["entity"]["folderId"], default_id("requests"));
        assert_eq!(socket["entity"]["type"], "websocket");
        let id = socket["id"].as_str().unwrap();
        assert!(entity_dir(WS, "requests").join("Default").join(format!("{id}.json")).exists());
        assert!(!ws_dir(WS).join("sockets").exists(), "no directory per request type");
        let cfg = load_config();
        assert_eq!(cfg["wsConnections"][0]["id"], id);
        assert_eq!(cfg["wsConnections"][0]["folderId"], default_id("requests"));
        assert!(cfg.get("wsFolders").is_none(), "the request folders are every request type's");
        assert_eq!(cfg["requests"].as_array().unwrap().len(), 5);
        let listed = call(&ctx, "entity.list", json!({ "workspaceId": WS, "kind": "wsConnections" })).await.unwrap();
        assert_eq!(listed["entities"].as_array().unwrap().len(), 1);

        // Into a folder in a folder: the file is in the directory in the directory.
        let inner = call(&ctx, "folder.add", json!({ "kind": "request", "name": "Tokens", "parentId": own["id"] }))
            .await
            .unwrap();
        let mut deep = socket["entity"].clone();
        deep["folderId"] = inner["id"].clone();
        call(&ctx, "entity.update", json!({ "kind": "wsConnections", "entity": deep })).await.unwrap();
        assert!(entity_dir(WS, "requests").join("Auth").join("Tokens").join(format!("{id}.json")).exists());
        assert!(!entity_dir(WS, "requests").join("Default").join(format!("{id}.json")).exists());
    }

    #[tokio::test]
    async fn requests_are_searched_by_what_the_index_lists_of_them() {
        let _root = scratch_data_root();
        let ctx = context();
        let users = json!({ "name": "List users", "method": "GET", "url": "http://a/people" });
        let users = call(&ctx, "entity.create", json!({ "kind": "requests", "entity": users })).await.unwrap();
        let feed = json!({ "name": "Feed", "url": "ws://a/users/live" });
        call(&ctx, "entity.create", json!({ "kind": "wsConnections", "entity": feed })).await.unwrap();
        let other = json!({ "name": "Ping", "method": "GET", "url": "http://a/ping" });
        call(&ctx, "entity.create", json!({ "kind": "requests", "entity": other })).await.unwrap();

        let search = |query: &str, kinds: Value| {
            call(
                &ctx,
                "entity.search",
                json!({ "workspaceId": WS, "scope": "requests", "query": query, "kinds": kinds }),
            )
        };
        // By name or by address, whatever the case, across every request type.
        let found = search("USERS", json!([])).await.unwrap();
        let names: Vec<&str> = found["results"].as_array().unwrap().iter().map(|r| str_of(r, "name")).collect();
        assert_eq!(names, ["Feed", "List users"]);
        assert_eq!(found["results"][0]["kind"], "wsConnections");
        assert_eq!(found["results"][1]["id"], users["id"]);
        assert_eq!(found["results"][1]["path"], format!("requests/Default/{}.json", str_of(&users, "id")));
        assert_eq!(found["results"][1]["folderId"], users["entity"]["folderId"]);

        let only = search("users", json!(["wsConnections"])).await.unwrap();
        assert_eq!(only["results"].as_array().unwrap().len(), 1);
        assert!(search("nothing", json!([])).await.unwrap()["results"].as_array().unwrap().is_empty());

        // A rename is found at once: the index is written with the file.
        let mut renamed = users["entity"].clone();
        renamed["name"] = json!("People");
        call(&ctx, "entity.update", json!({ "kind": "requests", "entity": renamed })).await.unwrap();
        let found = search("people", json!(["requests"])).await.unwrap();
        assert_eq!(found["results"][0]["name"], "People");
        let none = call(&ctx, "entity.search", json!({ "workspaceId": WS, "scope": "mocks", "query": "people" })).await;
        assert!(none.unwrap()["results"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn rules_and_flat_kinds_are_not_put_in_a_folder() {
        let _root = scratch_data_root();
        let ctx = context();
        let rule = json!({ "name": "R", "pattern": "api", "useRegex": false, "targetType": "mapping" });
        let created = call(&ctx, "entity.create", json!({ "kind": "proxyRules", "entity": rule })).await.unwrap();
        assert!(created["entity"].get("folderId").is_none_or(Value::is_null));
        assert!(read_index(WS, "rules").folders.is_empty());
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
        let details = bad.details.unwrap();
        assert_eq!((&details["field"], &details["key"]), (&json!("domain"), &json!("mapping.domain_not_localhost")));

        let good = json!({ "domain": "app.localhost", "target": "localhost:3000" });
        let created = call(&ctx, "entity.create", json!({ "kind": "mappings", "entity": good })).await.unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        assert!(entity_dir(WS, "mappings").join(format!("{id}.json")).exists());
        assert!(!entity_dir(WS, "mappings").join("names.json").exists(), "flat kinds keep no names index");
        assert_eq!(enabled("mappings"), vec![id.clone()]);

        let dup = call(&ctx, "entity.create", json!({ "kind": "mappings", "entity": good })).await.unwrap_err();
        assert_eq!(dup.details.unwrap()["key"], "mapping.domain_taken");

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
        assert_eq!(
            out,
            json!({
                "ok": false, "error": "cannot_delete_global", "errorKey": "env.cannot_delete_global",
                "errorParams": {},
            })
        );
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
        {
            let events = events.lock().unwrap();
            assert_eq!(events.len(), 3);
            assert_eq!(events[0], json!({ "wsId": WS, "kind": "mocks", "id": first_id, "action": "created" }));
        }

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
    async fn requests_mocks_and_rules_are_stamped_with_the_engine_that_writes_them() {
        let _root = scratch_data_root();
        let ctx = context();
        let request = create(&ctx, "requests", json!({ "name": "Ping", "method": "GET", "url": "http://a/" })).await;
        let stream = create(&ctx, "sseRequests", json!({ "name": "Feed", "url": "http://a/feed" })).await;
        let made_mock = create(&ctx, "mocks", mock("/a")).await;
        let rule = create(&ctx, "proxyRules", json!({ "name": "R", "pattern": "/x" })).await;
        for (kind, entity) in
            [("requests", &request), ("sseRequests", &stream), ("mocks", &made_mock), ("rules", &rule)]
        {
            assert_eq!(stamp_of(kind, str_of(entity, "id")), crate::ENGINE_VERSION, "{kind}");
        }

        // What a client sends as the stamp is not what is written.
        let mut forged = request.clone();
        forged[stamp::VERSION_FIELD] = json!("0.0.1");
        forged["name"] = json!("Pong");
        call(&ctx, "entity.update", json!({ "kind": "requests", "entity": forged })).await.unwrap();
        assert_eq!(stamp_of("requests", str_of(&request, "id")), crate::ENGINE_VERSION);
    }

    /// A flow, a mapping and an environment, as `entity.create` takes them.
    fn flow_mapping_environment() -> [(&'static str, &'static str, Value); 3] {
        [
            ("flows", "flows", json!({ "name": "F", "nodes": [], "edges": [] })),
            ("mappings", "mappings", json!({ "domain": "app.localhost", "target": "localhost:3000" })),
            ("environments", "environments", json!({ "name": "Dev", "variables": [] })),
        ]
    }

    #[tokio::test]
    async fn flows_mappings_and_environments_are_stamped_and_keep_an_unchanged_stamp() {
        let _root = scratch_data_root();
        let ctx = context();
        for (kind, store, entity) in flow_mapping_environment() {
            let id = str_of(&create(&ctx, kind, entity).await, "id").to_string();
            assert_eq!(stamp_of(store, &id), crate::ENGINE_VERSION, "{kind}");

            // Saved as it is, and saved with everything else when the snapshot is: an earlier stamp stays.
            let file = find_entity_file(WS, store, &id).unwrap();
            let mut earlier = read_json(&file).unwrap();
            earlier[stamp::VERSION_FIELD] = json!("0.0.1");
            write_json_pretty(&file, &earlier).unwrap();
            let loaded = call(&ctx, "entity.load", json!({ "workspaceId": WS, "kind": kind, "id": id })).await.unwrap();
            call(&ctx, "entity.update", json!({ "kind": kind, "entity": loaded["entity"] })).await.unwrap();
            save_config(&load_config()).unwrap();
            assert_eq!(stamp_of(store, &id), "0.0.1", "{kind}: nothing changed");
        }
    }

    #[tokio::test]
    async fn a_flow_mapping_or_environment_a_newer_engine_wrote_is_listed_and_left_alone() {
        let _root = scratch_data_root();
        let ctx = context();
        for (kind, store, entity) in flow_mapping_environment() {
            let entity = create(&ctx, kind, entity).await;
            let id = str_of(&entity, "id").to_string();
            let file = written_by(store, &id, Some("999.0.0"));
            let before = std::fs::read(&file).unwrap();

            let cfg = load_config();
            let listed = array_of(&cfg, kind).iter().find(|e| str_of(e, "id") == id).cloned().expect("it is listed");
            assert_eq!(listed["locked"], true, "{kind}");
            assert_eq!(listed["requiredVersion"], "999.0.0", "{kind}");
            assert!(listed.get("somethingNewer").is_none(), "{kind}");
            assert_eq!(array_of(&cfg, kind).iter().filter(|e| str_of(e, "id") == id).count(), 1, "{kind}");
            assert_eq!(listed["folderId"], entity["folderId"], "{kind}: listed in its folder");
            assert!(read_all_entities(WS, store).iter().all(|e| str_of(e, "id") != id), "{kind}: nothing reads it");

            let load = json!({ "workspaceId": WS, "kind": kind, "id": id });
            assert_eq!(
                call(&ctx, "entity.load", load.clone()).await.unwrap(),
                json!({ "ok": false, "locked": true, "requiredVersion": "999.0.0" }),
                "{kind}"
            );
            let mut changed = entity.clone();
            changed["name"] = json!("Mine now");
            for result in [
                call(&ctx, "entity.update", json!({ "kind": kind, "entity": changed })).await,
                call(&ctx, "entity.delete", json!({ "kind": kind, "id": id })).await,
            ] {
                assert_eq!(update_required(result), "999.0.0", "{kind}");
            }
            let stored = if store == "flows" {
                write_entity(WS, store, &id, &entity, None)
            } else {
                crate::workspace::fs::write_flat_entity(WS, store, &id, &entity)
            };
            assert!(stored.is_err(), "{kind}: the store itself refuses");
            save_config(&load_config()).unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), before, "{kind}: its file is as the newer engine left it");
        }
    }

    #[tokio::test]
    async fn the_global_environment_stays_when_a_newer_engine_wrote_it() {
        let _root = scratch_data_root();
        let _ctx = context();
        let _ = load_config();
        let file = find_entity_file(WS, "environments", crate::workspace::config::GLOBAL_ENV_ID).unwrap();
        let mut global = read_json(&file).unwrap();
        global[stamp::VERSION_FIELD] = json!("999.0.0");
        write_json_pretty(&file, &global).unwrap();
        let before = std::fs::read(&file).unwrap();
        let cfg = load_config();
        let is_global = |e: &&Value| str_of(e, "id") == crate::workspace::config::GLOBAL_ENV_ID;
        let listed: Vec<_> = array_of(&cfg, "environments").iter().filter(is_global).collect();
        assert_eq!((listed.len(), listed[0]["locked"].clone()), (1, json!(true)));
        save_config(&cfg).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }

    #[tokio::test]
    async fn an_entity_from_before_the_stamp_is_stamped_when_it_changes_and_not_before() {
        let _root = scratch_data_root();
        let ctx = context();
        for (kind, store, entity) in [
            ("requests", "requests", json!({ "name": "Ping", "method": "GET", "url": "http://a/" })),
            ("mocks", "mocks", mock("/a")),
            ("proxyRules", "rules", json!({ "name": "R", "pattern": "/x" })),
        ] {
            let id = str_of(&create(&ctx, kind, entity).await, "id").to_string();
            written_by(store, &id, None);
            let load = json!({ "workspaceId": WS, "kind": kind, "id": id });
            let loaded = call(&ctx, "entity.load", load).await.unwrap();
            assert_eq!(loaded["ok"], true, "{kind}: an entity without a stamp is read as it always was");

            // Saved as it is, and saved with everything else when the snapshot is: it stays the older engine's.
            let same = loaded["entity"].clone();
            call(&ctx, "entity.update", json!({ "kind": kind, "entity": same })).await.unwrap();
            save_config(&load_config()).unwrap();
            assert_eq!(stamp_of(store, &id), Value::Null, "{kind}: nothing changed");

            let mut changed = loaded["entity"].clone();
            changed["name"] = json!("Renamed");
            call(&ctx, "entity.update", json!({ "kind": kind, "entity": changed })).await.unwrap();
            assert_eq!(stamp_of(store, &id), crate::ENGINE_VERSION, "{kind}: it was changed here");
        }
    }

    #[tokio::test]
    async fn an_entity_a_newer_engine_wrote_is_listed_and_otherwise_left_alone() {
        let _root = scratch_data_root();
        let ctx = context();
        let folder_of = |kind: &'static str| {
            let ctx = ctx.clone();
            async move {
                let folder = call(&ctx, "folder.add", json!({ "kind": kind, "name": "Held", "parentId": null })).await;
                str_of(&folder.unwrap(), "id").to_string()
            }
        };
        let (request_folder, mock_folder, rule_folder) =
            (folder_of("request").await, folder_of("mock").await, folder_of("rule").await);
        let request = json!({ "name": "Ping", "method": "POST", "url": "http://a/secret", "folderId": request_folder });
        let mut held_mock = mock("/held");
        held_mock["folderId"] = json!(mock_folder);
        let rule = json!({ "name": "R", "pattern": "/x", "folderId": rule_folder });
        let cases = [
            ("requests", "requests", "request", request_folder, create(&ctx, "requests", request).await),
            ("mocks", "mocks", "mock", mock_folder, create(&ctx, "mocks", held_mock).await),
            ("proxyRules", "rules", "rule", rule_folder, create(&ctx, "proxyRules", rule).await),
        ];

        for (kind, store, folder_kind, folder_id, entity) in cases {
            let id = str_of(&entity, "id").to_string();
            let file = written_by(store, &id, Some("999.0.0"));
            let before = std::fs::read(&file).unwrap();

            // Listed, by name, with the version it needs; what else its file holds is not shown.
            let cfg = load_config();
            let listed = array_of(&cfg, kind).iter().find(|e| str_of(e, "id") == id).cloned().expect("it is listed");
            assert_eq!(listed["locked"], true, "{kind}");
            assert_eq!(listed["requiredVersion"], "999.0.0", "{kind}");
            assert_eq!(listed["name"], entity["name"], "{kind}");
            assert_eq!(listed["folderId"], folder_id.as_str(), "{kind}");
            assert!(!listed["enabled"].as_bool().unwrap_or(false), "{kind}: it is not in use");
            assert!(listed.get("somethingNewer").is_none(), "{kind}");
            if kind == "requests" {
                assert_eq!(listed["url"], "", "the address is not read out of it");
                assert!(tree::search(WS, tree::Scope::Requests, "secret", &[], 10).is_empty());
            }

            // Nothing that uses entities is handed it.
            assert!(read_all_entities(WS, store).is_empty(), "{kind}");
            let load = json!({ "workspaceId": WS, "kind": kind, "id": id });
            assert_eq!(
                call(&ctx, "entity.load", load.clone()).await.unwrap(),
                json!({ "ok": false, "locked": true, "requiredVersion": "999.0.0" }),
                "{kind}"
            );

            // Nothing changes it, and each refusal names the version.
            let mut changed = entity.clone();
            changed["name"] = json!("Mine now");
            let mut refused = vec![
                call(&ctx, "entity.update", json!({ "kind": kind, "entity": changed })).await,
                call(&ctx, "entity.delete", json!({ "kind": kind, "id": id })).await,
                call(&ctx, "entity.duplicate", load.clone()).await,
                call(&ctx, "folder.delete", json!({ "kind": folder_kind, "id": folder_id })).await,
            ];
            // A request is not something that is switched on.
            if store != "requests" {
                let switch = json!({ "workspaceId": WS, "kind": store, "id": id, "enabled": true });
                refused.push(call(&ctx, "entity.setEnabled", switch).await);
            }
            for result in refused {
                assert_eq!(update_required(result), "999.0.0", "{kind}");
            }
            assert!(write_entity(WS, store, &id, &entity, None).is_err(), "{kind}: the store itself refuses");
            save_config(&load_config()).unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), before, "{kind}: its file is as the newer engine left it");
            assert_eq!(array_of(&load_config(), kind).len(), 1, "{kind}: and it is still the one entity there");
        }
    }

    #[tokio::test]
    async fn an_entity_is_open_again_once_the_engine_is_as_new_as_its_stamp() {
        let _root = scratch_data_root();
        let ctx = context();
        let request = create(&ctx, "requests", json!({ "name": "Ping", "method": "GET", "url": "http://a/" })).await;
        let id = str_of(&request, "id").to_string();
        let load = json!({ "workspaceId": WS, "kind": "requests", "id": id });
        written_by("requests", &id, Some("999.0.0"));
        assert_eq!(call(&ctx, "entity.load", load.clone()).await.unwrap()["locked"], true);
        // The index keeps the stamp, not the verdict: the same file is read by the engine that is new enough.
        written_by("requests", &id, Some(crate::ENGINE_VERSION));
        let loaded = call(&ctx, "entity.load", load).await.unwrap();
        assert_eq!(loaded["ok"], true);
        assert_eq!(loaded["entity"]["somethingNewer"]["kept"], true);
        assert!(load_config()["requests"][0].get("locked").is_none());
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
