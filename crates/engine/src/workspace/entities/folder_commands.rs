//! `folder.add`, `folder.rename`, `folder.move` and `folder.delete`.
//!
//! A `request` or `mock` folder is a directory of its tree (`workspace::tree`): adding, renaming, moving and
//! deleting one is done to the directory, and the tree's index follows. Every request type shares the request
//! folders. A `rule` folder lives in two places that every command keeps in step: the config snapshot's folder
//! list and the kind's `index.json`.

use std::{collections::HashSet, path::Path};

use serde_json::{Value, json};

use crate::{
    kernel::{CommandRegistry, bus},
    protocol::{EngineError, Result},
    runtime::{entity_status::emit_entity_status, instances::notify_config_changed, notice},
    utils::{
        id::{generate_id, now_ms},
        json::{array_mut, array_of, opt_str, str_of, truthy},
    },
    workspace::{
        changes,
        config::{load_config, save_config},
        deletions::{folder_subtree, pending_folders, remove_entity, sharing_kinds},
        fs::{
            add_pending_deletion, delete_entity_dir, get_pending_deletions, pending_folders_key, read_index,
            sanitize_dir_name, write_index, ws_dir,
        },
        kinds::crud_kind,
        settings::load_settings,
        stamp,
        status_cache::invalidate_cache,
        tree::{self, Scope},
    },
};

/// Where a folder kind's folders and entities live.
struct FolderConfig {
    /// The config snapshot's folder list.
    config_key: &'static str,
    /// The config snapshot's entity list: the entities inside the folders.
    entity_config_key: &'static str,
    /// The directory name under the workspace.
    fs_kind: &'static str,
}

/// The folders that are not a tree's: the rule folders.
fn folder_config(kind: &str) -> Result<FolderConfig> {
    let (config_key, entity_config_key, fs_kind) = match kind {
        "rule" => ("ruleFolders", "proxyRules", "rules"),
        other => return Err(unknown_folder_kind(other)),
    };
    Ok(FolderConfig { config_key, entity_config_key, fs_kind })
}

fn unknown_folder_kind(kind: &str) -> EngineError {
    EngineError::engine(format!("Unknown folder kind \"{kind}\"."))
        .keyed("folder.kind_unknown", json!({ "kind": kind }))
}

/// The tree a folder kind's folders are directories of.
fn tree_of(kind: &str) -> Option<Scope> {
    match kind {
        "request" => Some(Scope::Requests),
        "mock" => Some(Scope::Mocks),
        "flow" => Some(Scope::Flows),
        _ => None,
    }
}

fn workspace_of(params: &Value) -> String {
    opt_str(params, "workspaceId").map(str::to_string).unwrap_or_else(|| load_settings().active_workspace_id())
}

/// Why a tree would not do what was asked of a folder, as a client is told it.
fn tree_error(err: std::io::Error) -> EngineError {
    let detail = err.to_string();
    let error = match err.kind() {
        std::io::ErrorKind::AlreadyExists => EngineError::conflict(err.to_string()),
        std::io::ErrorKind::NotFound => EngineError::not_found(err.to_string()),
        std::io::ErrorKind::InvalidInput => EngineError::bad_request(err.to_string()),
        _ => return err.into(),
    };
    error.keyed("folder.failed", json!({ "detail": detail }))
}

/// Tell clients that files of the workspace are somewhere else now.
fn files_moved(ws_id: &str) {
    notify_config_changed();
    invalidate_cache(ws_id);
    emit_entity_status(ws_id);
}

fn add_tree_folder(scope: Scope, kind: &str, params: &Value) -> Result<Value> {
    let ws_id = workspace_of(params);
    let folder = tree::create_folder(&ws_id, scope, str_of(params, "name"), opt_str(params, "parentId"), false)
        .map_err(tree_error)?;
    bus().emit("entity.changed", json!({ "wsId": ws_id, "kind": kind, "id": folder.id, "action": "created" }));
    changes::record(&ws_id, "folder", &folder.id, &folder.name, "created", &[]);
    Ok(tree::folder_value(&folder, &ws_id))
}

/// Rename or move a folder of a tree. One that is not there is left alone, as it always was.
fn change_tree_folder(ws_id: &str, changed: std::io::Result<tree::Folder>) -> Result<Value> {
    match changed {
        Ok(_) => files_moved(ws_id),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(tree_error(err)),
    }
    Ok(json!({ "ok": true }))
}

/// Delete a folder of a tree with the folders under it and every entity in them, each entity by the one rule
/// of `workspace::deletions`. A folder that held something git knows stays listed as deleted with it.
async fn delete_tree_folder(scope: Scope, params: &Value) -> Result<Value> {
    let (ws_id, id) = (workspace_of(params), str_of(params, "id"));
    let all = tree::folders(&ws_id, scope);
    let Some(folder) = all.iter().find(|f| f.id == id) else {
        // Not a folder: gone, or deleted already and waiting to be confirmed or reverted.
        let pending = pending_folders(&ws_id, scope.dir()).iter().any(|f| str_of(f, "id") == id);
        return Ok(json!({ "ok": true, "pending": pending }));
    };
    let under = format!("{}/", folder.path);
    let subtree: Vec<&tree::Folder> = all.iter().filter(|f| f.id == id || f.path.starts_with(&under)).collect();
    let in_subtree = |folder_id: &str| subtree.iter().any(|f| f.id == folder_id);
    // A locked entity cannot be deleted, so neither can a folder that holds one: nothing is taken out of it.
    let held =
        tree::entries(&ws_id, scope).into_iter().filter(|entry| entry.folder_id.as_deref().is_some_and(in_subtree));
    if let Some(version) = held.filter_map(|entry| entry.lock()).next() {
        return Err(stamp::folder_update_required(&version));
    }

    // The folders that keep something waiting: the one an entity was in, and every folder above it.
    let mut waiting: HashSet<String> = HashSet::new();
    let keep = |folder_id: &str, waiting: &mut HashSet<String>| {
        let mut at = subtree.iter().find(|f| f.id == folder_id);
        while let Some(current) = at.filter(|f| waiting.insert(f.id.clone())) {
            at = current.parent_id.as_deref().and_then(|parent| subtree.iter().find(|f| f.id == parent));
        }
    };

    for kind in sharing_kinds(scope.dir()) {
        for record in get_pending_deletions(&ws_id, kind) {
            if let Some(folder_id) = opt_str(&record, "folderId").filter(|f| in_subtree(f)) {
                keep(folder_id, &mut waiting);
            }
        }
    }
    for entry in tree::entries(&ws_id, scope) {
        let Some(folder_id) = entry.folder_id.as_deref().filter(|f| in_subtree(f)) else { continue };
        let Some(crud) = scope.kind_of_type(&entry.ty).and_then(crud_kind) else { continue };
        let Some(entity) = tree::read_entity(&ws_id, crud.kind, &entry.id) else { continue };
        if remove_entity(&ws_id, crud, &entity).await? {
            keep(folder_id, &mut waiting);
        }
    }

    for folder in &subtree {
        // A request folder owns its run history, keyed by folder id.
        if scope == Scope::Requests {
            let runs = ws_dir(&ws_id).join("requests").join(".runs").join(&folder.id);
            if runs.exists() {
                std::fs::remove_dir_all(runs)?;
            }
        }
        if waiting.contains(&folder.id) {
            let mut record = tree::folder_value(folder, &ws_id);
            record["path"] = json!(folder.path);
            add_pending_deletion(&ws_id, &pending_folders_key(scope.dir()), record)?;
        }
    }
    tree::delete_folder(&ws_id, scope, id)?;

    files_moved(&ws_id);
    Ok(json!({ "ok": true, "pending": !waiting.is_empty() }))
}

/// The workspace a folder belongs to: its own `workspaceId`, else the active one.
fn folder_workspace(folder: &Value, cfg: &Value) -> String {
    opt_str(folder, "workspaceId").unwrap_or_else(|| str_of(cfg, "activeWorkspaceId")).to_string()
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Rename a directory. On Windows a rename can fail across a lock, so fall back to copy and remove.
fn move_dir(from: &Path, to: &Path) {
    if std::fs::rename(from, to).is_ok() {
        return;
    }
    if let Err(err) = copy_dir(from, to).and_then(|()| std::fs::remove_dir_all(from)) {
        notice(&format!("[folder:rename] Error: {err}"));
    }
}

fn find_folder_mut<'a>(cfg: &'a mut Value, config_key: &str, id: &str) -> Option<&'a mut Value> {
    array_mut(cfg, config_key).iter_mut().find(|f| str_of(f, "id") == id)
}

pub fn register(registry: &mut CommandRegistry) {
    registry.register_sync("folder.add", |params, _| {
        let kind = str_of(&params, "kind");
        if let Some(scope) = tree_of(kind) {
            return add_tree_folder(scope, kind, &params);
        }
        let conf = folder_config(kind)?;
        let mut cfg = load_config();
        let ws_id = opt_str(&params, "workspaceId").unwrap_or_else(|| str_of(&cfg, "activeWorkspaceId")).to_string();
        let new_folder = json!({
            "name": params.get("name").cloned().unwrap_or(Value::Null),
            "parentId": params.get("parentId").cloned().unwrap_or(Value::Null),
            "id": generate_id(),
            "createdAt": now_ms(),
            "workspaceId": ws_id,
        });

        array_mut(&mut cfg, conf.config_key).push(new_folder.clone());
        save_config(&cfg)?;

        std::fs::create_dir_all(
            ws_dir(&ws_id).join(conf.fs_kind).join(sanitize_dir_name(str_of(&new_folder, "name"))),
        )?;

        let mut index = read_index(&ws_id, conf.fs_kind);
        index.folders.push(new_folder.clone());
        write_index(&ws_id, conf.fs_kind, &index)?;
        bus().emit(
            "entity.changed",
            json!({ "wsId": ws_id, "kind": kind, "id": str_of(&new_folder, "id"), "action": "created" }),
        );
        changes::record(&ws_id, "folder", str_of(&new_folder, "id"), str_of(&new_folder, "name"), "created", &[]);
        Ok(new_folder)
    });

    registry.register_sync("folder.rename", |params, _| {
        let (id, name) = (str_of(&params, "id"), str_of(&params, "name"));
        if let Some(scope) = tree_of(str_of(&params, "kind")) {
            let ws_id = workspace_of(&params);
            return change_tree_folder(&ws_id, tree::rename_folder(&ws_id, scope, id, name));
        }
        let conf = folder_config(str_of(&params, "kind"))?;
        let mut cfg = load_config();

        let renamed = find_folder_mut(&mut cfg, conf.config_key, id).map(|folder| {
            let old_name = str_of(folder, "name").to_string();
            folder["name"] = json!(name);
            (old_name, folder.clone())
        });
        save_config(&cfg)?;

        if let Some((old_name, folder)) = renamed
            && !old_name.is_empty()
        {
            let ws_id = folder_workspace(&folder, &cfg);
            let base = ws_dir(&ws_id).join(conf.fs_kind);
            let old_dir = base.join(sanitize_dir_name(&old_name));
            let new_dir = base.join(sanitize_dir_name(name));
            if old_dir.exists() && old_dir != new_dir {
                move_dir(&old_dir, &new_dir);
            }
            let mut index = read_index(&ws_id, conf.fs_kind);
            if let Some(entry) = index.folders.iter_mut().find(|f| str_of(f, "id") == id) {
                entry["name"] = json!(name);
            }
            write_index(&ws_id, conf.fs_kind, &index)?;
            // The entities' files moved with the directory.
            notify_config_changed();
            invalidate_cache(&ws_id);
            emit_entity_status(&ws_id);
        }
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("folder.move", |params, _| {
        let id = str_of(&params, "id");
        if let Some(scope) = tree_of(str_of(&params, "kind")) {
            let ws_id = workspace_of(&params);
            return change_tree_folder(&ws_id, tree::move_folder(&ws_id, scope, id, opt_str(&params, "parentId")));
        }
        let conf = folder_config(str_of(&params, "kind"))?;
        let parent_id = params.get("parentId").cloned().unwrap_or(Value::Null);
        let mut cfg = load_config();

        let moved = find_folder_mut(&mut cfg, conf.config_key, id).map(|folder| {
            folder["parentId"] = parent_id.clone();
            folder.clone()
        });

        if let Some(folder) = moved {
            let ws_id = folder_workspace(&folder, &cfg);
            let mut index = read_index(&ws_id, conf.fs_kind);
            if let Some(entry) = index.folders.iter_mut().find(|f| str_of(f, "id") == id) {
                entry["parentId"] = parent_id;
            }
            write_index(&ws_id, conf.fs_kind, &index)?;
            emit_entity_status(&ws_id);
        }
        save_config(&cfg)?;
        Ok(json!({ "ok": true }))
    });

    // Deletes the folder, the folders under it and every entity in them, each entity by the one rule of
    // `workspace::deletions`. A folder that held something git knows stays listed as deleted with it.
    registry.register("folder.delete", |params, _| async move {
        let kind = str_of(&params, "kind");
        if let Some(scope) = tree_of(kind) {
            return delete_tree_folder(scope, &params).await;
        }
        let conf = folder_config(kind)?;
        let id = str_of(&params, "id");
        let cfg = load_config();

        let all_folders = array_of(&cfg, conf.config_key);
        let Some(folder) = all_folders.iter().find(|f| str_of(f, "id") == id).cloned() else {
            return Ok(json!({ "ok": true, "pending": false }));
        };
        // Already deleted, and waiting to be confirmed or reverted.
        if truthy(&folder, "deleted") {
            return Ok(json!({ "ok": true, "pending": true }));
        }
        let ws_id = folder_workspace(&folder, &cfg);
        let crud = crud_kind(conf.fs_kind).ok_or_else(|| unknown_folder_kind(kind))?;

        let live: Vec<Value> = all_folders.iter().filter(|f| !truthy(f, "deleted")).cloned().collect();
        let subtree = folder_subtree(&live, id);
        let in_subtree = |folder_id: &str| subtree.iter().any(|f| f == folder_id);

        // The folders that keep something waiting: the one an entity was in, and every folder above it.
        let mut waiting: Vec<String> = Vec::new();
        let keep = |folder_id: &str, waiting: &mut Vec<String>| {
            let mut at = Some(folder_id.to_string());
            while let Some(current) = at.take().filter(|f| in_subtree(f) && !waiting.contains(f)) {
                at = live
                    .iter()
                    .find(|f| str_of(f, "id") == current)
                    .and_then(|f| opt_str(f, "parentId"))
                    .map(str::to_string);
                waiting.push(current);
            }
        };

        // A locked entity cannot be deleted, so neither can a folder that holds one: nothing is taken out of it.
        let held =
            array_of(&cfg, conf.entity_config_key).iter().filter(|e| opt_str(e, "folderId").is_some_and(in_subtree));
        if let Some(version) = held.filter(|e| truthy(e, "locked")).find_map(|e| opt_str(e, "requiredVersion")) {
            return Err(stamp::folder_update_required(version));
        }

        let mut affected: Vec<String> = Vec::new();
        for entity in array_of(&cfg, conf.entity_config_key) {
            let Some(folder_id) = opt_str(entity, "folderId").filter(|f| in_subtree(f)) else { continue };
            if truthy(entity, "deleted") || remove_entity(&ws_id, crud, entity).await? {
                keep(folder_id, &mut waiting);
            }
            affected.push(str_of(entity, "id").to_string());
        }

        for folder in live.iter().filter(|f| in_subtree(str_of(f, "id"))) {
            let folder_id = str_of(folder, "id");
            let dir_name = sanitize_dir_name(str_of(folder, "name"));
            // Two folders of one name share a directory; it goes with the last of them.
            let shared = live
                .iter()
                .any(|other| !in_subtree(str_of(other, "id")) && sanitize_dir_name(str_of(other, "name")) == dir_name);
            if !shared {
                delete_entity_dir(&ws_id, conf.fs_kind, &dir_name)?;
            }

            if waiting.iter().any(|w| w == folder_id) {
                add_pending_deletion(&ws_id, &pending_folders_key(conf.fs_kind), folder.clone())?;
            }
        }

        let mut index = read_index(&ws_id, conf.fs_kind);
        index.folders.retain(|f| !in_subtree(str_of(f, "id")));
        index.order.retain(|entry| !affected.iter().any(|gone| entry.as_str() == Some(gone.as_str())));
        write_index(&ws_id, conf.fs_kind, &index)?;

        notify_config_changed();
        invalidate_cache(&ws_id);
        emit_entity_status(&ws_id);
        Ok(json!({ "ok": true, "pending": !waiting.is_empty() }))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        kernel::CommandContext,
        testing::scratch_data_root,
        workspace::fs::{entity_dir, read_all_entities},
    };

    const WS: &str = "ws1";

    fn context() -> CommandContext {
        let mut registry = CommandRegistry::new();
        crate::workspace::entities::register(&mut registry);
        CommandContext::internal(Arc::new(registry))
    }

    async fn call(ctx: &CommandContext, action: &str, payload: Value) -> Result<Value> {
        ctx.registry.invoke(action, payload, ctx.clone()).await
    }

    #[tokio::test]
    async fn a_folder_is_added_renamed_moved_and_deleted_with_its_entities() {
        let _root = scratch_data_root();
        let ctx = context();

        let outer =
            call(&ctx, "folder.add", json!({ "kind": "request", "name": "Outer", "parentId": null })).await.unwrap();
        let inner =
            call(&ctx, "folder.add", json!({ "kind": "request", "name": "Inner/1", "parentId": null })).await.unwrap();
        assert_eq!(inner["workspaceId"], WS);
        assert_eq!(inner["parentId"], Value::Null);
        assert_eq!(inner["name"], "Inner1", "a folder is named as its directory is");
        assert_eq!(
            inner.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["name", "parentId", "id", "createdAt", "workspaceId"]
        );
        let requests = entity_dir(WS, "requests");
        assert!(requests.join("Outer").is_dir());
        assert!(requests.join("Inner1").is_dir());
        assert_eq!(tree::folders(WS, Scope::Requests).len(), 2);
        // A folder of that name in that place is that folder.
        let same =
            call(&ctx, "folder.add", json!({ "kind": "request", "name": "Outer", "parentId": null })).await.unwrap();
        assert_eq!(same["id"], outer["id"]);

        // The folders are every request type's: a REST request and a WebSocket in the same one.
        let request = json!({ "name": "A", "method": "GET", "url": "http://a", "folderId": outer["id"] });
        let created = call(&ctx, "entity.create", json!({ "kind": "requests", "entity": request })).await.unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        let socket = json!({ "name": "S", "url": "ws://a", "folderId": inner["id"] });
        let socket = call(&ctx, "entity.create", json!({ "kind": "wsConnections", "entity": socket })).await.unwrap();
        let socket_id = socket["id"].as_str().unwrap().to_string();
        assert!(requests.join("Inner1").join(format!("{socket_id}.json")).exists());

        assert_eq!(
            call(&ctx, "folder.rename", json!({ "kind": "request", "id": outer["id"], "name": "Renamed" }))
                .await
                .unwrap(),
            json!({ "ok": true })
        );
        assert!(!requests.join("Outer").exists());
        assert!(requests.join("Renamed").join(format!("{id}.json")).exists());
        let renamed = tree::folder(WS, Scope::Requests, outer["id"].as_str().unwrap()).unwrap();
        assert_eq!(renamed.name, "Renamed");
        let taken =
            call(&ctx, "folder.rename", json!({ "kind": "request", "id": inner["id"], "name": "renamed" })).await;
        assert_eq!(taken.unwrap_err().code, crate::protocol::ErrorCode::Conflict);

        // Moving a folder into another moves its directory into the other's.
        call(&ctx, "folder.move", json!({ "kind": "request", "id": inner["id"], "parentId": outer["id"] }))
            .await
            .unwrap();
        let moved = tree::folder(WS, Scope::Requests, inner["id"].as_str().unwrap()).unwrap();
        assert_eq!(moved.parent_id.as_deref(), outer["id"].as_str());
        assert!(!requests.join("Inner1").exists());
        assert!(requests.join("Renamed").join("Inner1").join(format!("{socket_id}.json")).exists());
        let cfg = load_config();
        assert_eq!(cfg["wsConnections"][0]["folderId"], inner["id"]);
        let into_itself =
            call(&ctx, "folder.move", json!({ "kind": "request", "id": outer["id"], "parentId": inner["id"] })).await;
        assert!(into_itself.is_err());

        let unknown =
            call(&ctx, "folder.rename", json!({ "kind": "request", "id": "nope", "name": "x" })).await.unwrap();
        assert_eq!(unknown, json!({ "ok": true }));

        let runs = requests.join(".runs").join(outer["id"].as_str().unwrap());
        std::fs::create_dir_all(&runs).unwrap();
        // Nothing here was ever synced, so the folder, the folder under it and the request are simply gone.
        let deleted = call(&ctx, "folder.delete", json!({ "kind": "request", "id": outer["id"] })).await.unwrap();
        assert_eq!(deleted, json!({ "ok": true, "pending": false }));
        assert!(!requests.join("Renamed").exists(), "a folder goes with the folder it is in");
        assert!(!runs.exists());
        assert!(read_all_entities(WS, "requests").is_empty());
        assert!(read_all_entities(WS, "sockets").is_empty(), "whatever the type of what was in it");
        assert!(tree::folders(WS, Scope::Requests).is_empty());
        let cfg = load_config();
        assert!(array_of(&cfg, "requests").is_empty() && array_of(&cfg, "requestFolders").is_empty());
        assert!(array_of(&cfg, "wsConnections").is_empty());
    }

    #[tokio::test]
    async fn rule_folders_are_kept_as_they_were_and_mock_folders_are_a_tree() {
        let _root = scratch_data_root();
        let ctx = context();
        call(&ctx, "folder.add", json!({ "kind": "rule", "name": "Rules", "parentId": null })).await.unwrap();
        assert!(entity_dir(WS, "rules").join("Rules").is_dir());
        assert_eq!(read_index(WS, "rules").folders.len(), 1);

        let outer =
            call(&ctx, "folder.add", json!({ "kind": "mock", "name": "Users", "parentId": null })).await.unwrap();
        let inner = call(&ctx, "folder.add", json!({ "kind": "mock", "name": "Admin", "parentId": outer["id"] }))
            .await
            .unwrap();
        assert_eq!(inner["parentId"], outer["id"]);
        assert!(entity_dir(WS, "mocks").join("Users").join("Admin").is_dir());
        let cfg = load_config();
        assert_eq!(array_of(&cfg, "mockFolders").len(), 2);

        // A request type has no folders of its own any more.
        let gone = call(&ctx, "folder.add", json!({ "kind": "ws", "name": "Sockets", "parentId": null })).await;
        assert!(gone.is_err());
    }
}
