//! `folder.add`, `folder.rename`, `folder.move` and `folder.delete`.
//!
//! Folders live in two places that every command keeps in step: the config snapshot's folder list and the kind's
//! `index.json`, which the on-disk tree is derived from.

use std::path::Path;

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
        config::{load_config, save_config},
        deletions::{folder_subtree, remove_entity},
        fs::{
            add_pending_deletion, delete_entity_dir, pending_folders_key, read_index, sanitize_dir_name, write_index,
            ws_dir,
        },
        kinds::crud_kind,
        status_cache::invalidate_cache,
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

/// `fs_kind` is not derivable from the folder kind: `ws` is `sockets`, `rule` is `rules`.
fn folder_config(kind: &str) -> Result<FolderConfig> {
    let (config_key, entity_config_key, fs_kind) = match kind {
        "mock" => ("mockFolders", "mocks", "mocks"),
        "ws" => ("wsFolders", "wsConnections", "sockets"),
        "rule" => ("ruleFolders", "proxyRules", "rules"),
        "graphqlRequest" => ("graphqlRequestFolders", "graphqlRequests", "graphqlRequests"),
        "grpcRequest" => ("grpcRequestFolders", "grpcRequests", "grpcRequests"),
        "sseRequest" => ("sseRequestFolders", "sseRequests", "sseRequests"),
        "soapRequest" => ("soapRequestFolders", "soapRequests", "soapRequests"),
        "request" => ("requestFolders", "requests", "requests"),
        other => return Err(EngineError::engine(format!("Unknown folder kind \"{other}\"."))),
    };
    Ok(FolderConfig { config_key, entity_config_key, fs_kind })
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
        Ok(new_folder)
    });

    registry.register_sync("folder.rename", |params, _| {
        let conf = folder_config(str_of(&params, "kind"))?;
        let (id, name) = (str_of(&params, "id"), str_of(&params, "name"));
        let mut cfg = load_config();

        let renamed = find_folder_mut(&mut cfg, conf.config_key, id).map(|folder| {
            let old_name = str_of(folder, "name").to_string();
            folder["name"] = json!(name);
            (old_name, folder.clone())
        });
        save_config(&cfg)?;

        if let Some((old_name, folder)) = renamed {
            if !old_name.is_empty() {
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
        }
        Ok(json!({ "ok": true }))
    });

    registry.register_sync("folder.move", |params, _| {
        let conf = folder_config(str_of(&params, "kind"))?;
        let id = str_of(&params, "id");
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
        let crud = crud_kind(conf.fs_kind)
            .ok_or_else(|| EngineError::engine(format!("Unknown folder kind \"{kind}\".")))?;

        let live: Vec<Value> = all_folders.iter().filter(|f| !truthy(f, "deleted")).cloned().collect();
        let subtree = folder_subtree(&live, id);
        let in_subtree = |folder_id: &str| subtree.iter().any(|f| f == folder_id);

        // The folders that keep something waiting: the one an entity was in, and every folder above it.
        let mut waiting: Vec<String> = Vec::new();
        let keep = |folder_id: &str, waiting: &mut Vec<String>| {
            let mut at = Some(folder_id.to_string());
            while let Some(current) = at.take().filter(|f| in_subtree(f) && !waiting.contains(f)) {
                at = live.iter().find(|f| str_of(f, "id") == current).and_then(|f| opt_str(f, "parentId")).map(str::to_string);
                waiting.push(current);
            }
        };

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
            let shared = live.iter().any(|other| {
                !in_subtree(str_of(other, "id")) && sanitize_dir_name(str_of(other, "name")) == dir_name
            });
            if !shared {
                delete_entity_dir(&ws_id, conf.fs_kind, &dir_name)?;
            }

            // A request folder owns its run history, keyed by folder id rather than by name.
            if kind == "request" {
                let runs = ws_dir(&ws_id).join("requests").join(".runs").join(folder_id);
                if runs.exists() {
                    std::fs::remove_dir_all(runs)?;
                }
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
        workspace::fs::{EntityIndex, entity_dir, read_all_entities},
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
        assert_eq!(
            inner.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["name", "parentId", "id", "createdAt", "workspaceId"]
        );
        let requests = entity_dir(WS, "requests");
        assert!(requests.join("Outer").is_dir());
        assert!(requests.join("Inner1").is_dir());
        assert_eq!(read_index(WS, "requests").folders.len(), 2);

        let request = json!({ "name": "A", "method": "GET", "url": "http://a", "folderId": outer["id"] });
        let created = call(&ctx, "entity.create", json!({ "kind": "requests", "entity": request })).await.unwrap();
        let id = created["id"].as_str().unwrap().to_string();
        write_index(
            WS,
            "requests",
            &EntityIndex { folders: read_index(WS, "requests").folders, order: vec![json!(id), json!("keep")] },
        )
        .unwrap();

        assert_eq!(
            call(&ctx, "folder.rename", json!({ "kind": "request", "id": outer["id"], "name": "Renamed" }))
                .await
                .unwrap(),
            json!({ "ok": true })
        );
        assert!(!requests.join("Outer").exists());
        assert!(requests.join("Renamed").join(format!("{id}.json")).exists());
        let index = read_index(WS, "requests");
        assert!(index.folders.iter().any(|f| f["name"] == "Renamed"));

        call(&ctx, "folder.move", json!({ "kind": "request", "id": inner["id"], "parentId": outer["id"] }))
            .await
            .unwrap();
        let index = read_index(WS, "requests");
        let moved = index.folders.iter().find(|f| f["id"] == inner["id"]).unwrap();
        assert_eq!(moved["parentId"], outer["id"]);

        let unknown =
            call(&ctx, "folder.rename", json!({ "kind": "request", "id": "nope", "name": "x" })).await.unwrap();
        assert_eq!(unknown, json!({ "ok": true }));

        let runs = requests.join(".runs").join(outer["id"].as_str().unwrap());
        std::fs::create_dir_all(&runs).unwrap();
        // Nothing here was ever synced, so the folder, the folder under it and the request are simply gone.
        let deleted = call(&ctx, "folder.delete", json!({ "kind": "request", "id": outer["id"] })).await.unwrap();
        assert_eq!(deleted, json!({ "ok": true, "pending": false }));
        assert!(!requests.join("Renamed").exists());
        assert!(!requests.join("Inner1").exists(), "a folder goes with the folder it is in");
        assert!(!runs.exists());
        assert!(read_all_entities(WS, "requests").is_empty());
        assert!(crate::workspace::fs::read_names_index(WS, "requests").get(&id).is_none());
        let index = read_index(WS, "requests");
        assert!(index.folders.is_empty());
        assert_eq!(index.order, vec![json!("keep")]);
        let cfg = load_config();
        assert!(array_of(&cfg, "requests").is_empty() && array_of(&cfg, "requestFolders").is_empty());
    }

    #[tokio::test]
    async fn rule_and_websocket_folders_live_under_their_store_names() {
        let _root = scratch_data_root();
        let ctx = context();
        call(&ctx, "folder.add", json!({ "kind": "rule", "name": "Rules", "parentId": null })).await.unwrap();
        call(&ctx, "folder.add", json!({ "kind": "ws", "name": "Sockets", "parentId": null })).await.unwrap();
        assert!(entity_dir(WS, "rules").join("Rules").is_dir());
        assert!(entity_dir(WS, "sockets").join("Sockets").is_dir());
    }
}
