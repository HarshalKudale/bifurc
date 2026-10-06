//! Deleting saved entities and folders: one rule for every kind.
//!
//! An entity git knows (it was synced) is not gone when it is deleted. Its file is removed, so nothing uses
//! it any more, and a record of it is kept in `pending-deletions.json`: it stays listed, marked `deleted`,
//! until the deletion is either confirmed (committed, and pushed when there is a remote) or reverted (the
//! file is put back from `HEAD`). An entity git has never seen is simply removed.
//!
//! A folder follows what is in it: it stays, marked `deleted`, while anything under it is waiting, and is
//! gone otherwise.

use serde_json::{Value, json};

use super::{
    fs::{
        FLAT_ENTITY_KINDS, add_pending_deletion, clear_pending_deletions, delete_entity_file, delete_flat_entity_file, entity_dir,
        find_entity_rel_path, flat_entity_rel_path, get_pending_deletions, pending_folders_key, read_index,
        remove_name_entry, remove_pending_deletion, sanitize_dir_name, sync_enabled_set, write_index,
    },
    git_store::git,
    kinds::CrudKind,
    tree::{self, Scope},
};
use crate::utils::json::{merged, opt_str, str_of};

pub async fn is_git_tracked(ws_id: &str, rel_path: &str) -> bool {
    match git(ws_id, &["ls-files", "--error-unmatch", "--", rel_path]).await {
        Ok(out) => !out.trim().is_empty(),
        Err(_) => false,
    }
}

/// The record of an entity that is deleted here and waiting to be confirmed or reverted.
pub fn pending_entity(ws_id: &str, kind: &str, id: &str) -> Option<Value> {
    get_pending_deletions(ws_id, kind).into_iter().find(|record| str_of(record, "id") == id)
}

/// A kind's folders that are deleted here and still hold something waiting.
pub fn pending_folders(ws_id: &str, kind: &str) -> Vec<Value> {
    get_pending_deletions(ws_id, &pending_folders_key(kind))
}

/// Take one entity out of the workspace. `true` when git knows it, so it stays listed as deleted until that
/// is confirmed or reverted; `false` when it is gone for good.
pub async fn remove_entity(ws_id: &str, kind: &CrudKind, entity: &Value) -> std::io::Result<bool> {
    let id = str_of(entity, "id");
    // Where the file is has to be asked before it is removed.
    let rel_path =
        if kind.is_flat { Some(flat_entity_rel_path(kind.kind, id)) } else { find_entity_rel_path(ws_id, kind.kind, id) };
    let tracked = match &rel_path {
        Some(path) => is_git_tracked(ws_id, path).await,
        None => false,
    };

    if kind.is_flat {
        delete_flat_entity_file(ws_id, kind.kind, id)?;
    } else {
        delete_entity_file(ws_id, kind.kind, id)?;
    }

    if tracked {
        let folder_id = entity.get("folderId").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
        let mut record = json!({ "id": id, "folderId": folder_id, "path": rel_path });
        if let Some(name_entry) = kind.name_entry {
            record = merged(&record, &name_entry(entity));
        }
        add_pending_deletion(ws_id, kind.kind, record)?;
    } else if kind.name_entry.is_some() && !kind.is_flat {
        remove_name_entry(ws_id, kind.kind, id)?;
    }

    if kind.has_enabled_state {
        sync_enabled_set(ws_id, kind.kind, id, false)?;
    }
    Ok(tracked)
}

/// Where a deleted entity's file was, as git names it.
pub fn record_path(kind: &str, record: &Value) -> String {
    opt_str(record, "path").map(str::to_string).unwrap_or_else(|| flat_entity_rel_path(kind, str_of(record, "id")))
}

/// Stop listing a deleted entity: its deletion is final.
pub fn forget_deleted(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    remove_pending_deletion(ws_id, kind, id)?;
    if !FLAT_ENTITY_KINDS.contains(&kind) {
        remove_name_entry(ws_id, kind, id)?;
    }
    Ok(())
}

/// The ids of the folder `root` and every folder under it, parents before their children.
pub fn folder_subtree(folders: &[Value], root: &str) -> Vec<String> {
    let mut ids = vec![root.to_string()];
    let mut next = 0;
    while next < ids.len() {
        let parent = ids[next].clone();
        for folder in folders {
            let id = str_of(folder, "id");
            if opt_str(folder, "parentId") == Some(parent.as_str()) && !ids.iter().any(|seen| seen == id) {
                ids.push(id.to_string());
            }
        }
        next += 1;
    }
    ids
}

/// The kinds whose entities are in the folders of `kind`: every kind of its tree, or the kind alone.
pub fn sharing_kinds(kind: &str) -> Vec<&str> {
    match Scope::of_kind(kind) {
        Some(scope) => scope.types().iter().map(|(kind, _)| *kind).collect(),
        None => vec![kind],
    }
}

/// Point what was waiting in the deleted folder `from` at the folder `to`.
fn retarget(ws_id: &str, kind: &str, from: &str, to: &str) -> std::io::Result<()> {
    let mut keys: Vec<(String, &str)> =
        sharing_kinds(kind).into_iter().map(|kind| (kind.to_string(), "folderId")).collect();
    keys.push((pending_folders_key(kind), "parentId"));
    for (key, field) in keys {
        let records = get_pending_deletions(ws_id, &key);
        if !records.iter().any(|record| opt_str(record, field) == Some(from)) {
            continue;
        }
        clear_pending_deletions(ws_id, &key)?;
        for mut record in records {
            if opt_str(&record, field) == Some(from) {
                record[field] = json!(to);
            }
            add_pending_deletion(ws_id, &key, record)?;
        }
    }
    Ok(())
}

/// Make the deleted folder `folder_id`, and every deleted folder above it, a folder again: something in it
/// is coming back.
pub fn restore_folder_chain(ws_id: &str, kind: &str, folder_id: &str) -> std::io::Result<()> {
    let key = pending_folders_key(kind);
    let mut next = Some(folder_id.to_string());
    while let Some(id) = next.take() {
        let Some(folder) = pending_folders(ws_id, kind).into_iter().find(|f| str_of(f, "id") == id) else { break };
        remove_pending_deletion(ws_id, &key, &id)?;
        if let Some(scope) = Scope::of_kind(kind) {
            // Its directory is made where it was. A folder made there since takes what comes back.
            let live = tree::restore_folder(ws_id, scope, &id, str_of(&folder, "path"))?;
            if live.id != id {
                retarget(ws_id, kind, &id, &live.id)?;
            }
            next = opt_str(&folder, "parentId").map(str::to_string);
            continue;
        }
        let mut index = read_index(ws_id, kind);
        let dir_name = sanitize_dir_name(str_of(&folder, "name"));
        let same_directory =
            index.folders.iter().find(|f| sanitize_dir_name(str_of(f, "name")) == dir_name).map(|f| str_of(f, "id").to_string());
        match same_directory {
            // A folder of that name was made since: it has the directory, so it takes what comes back.
            Some(live) => retarget(ws_id, kind, &id, &live)?,
            None => {
                std::fs::create_dir_all(entity_dir(ws_id, kind).join(&dir_name))?;
                index.folders.push(folder.clone());
                write_index(ws_id, kind, &index)?;
            }
        }
        next = opt_str(&folder, "parentId").map(str::to_string);
    }
    Ok(())
}

/// Forget the deleted folders that have nothing waiting under them any more.
pub fn prune_pending_folders(ws_id: &str, kind: &str) -> std::io::Result<()> {
    let key = pending_folders_key(kind);
    loop {
        let folders = pending_folders(ws_id, kind);
        let entities: Vec<Value> =
            sharing_kinds(kind).into_iter().flat_map(|kind| get_pending_deletions(ws_id, kind)).collect();
        let empty = folders.iter().map(|f| str_of(f, "id")).find(|id| {
            !entities.iter().any(|e| opt_str(e, "folderId") == Some(id))
                && !folders.iter().any(|f| opt_str(f, "parentId") == Some(id))
        });
        match empty {
            Some(id) => remove_pending_deletion(ws_id, &key, id)?,
            None => return Ok(()),
        }
    }
}
