//! Bringing a tree's index in step with its files: what a pull, a checkout or a hand brought in, moved or
//! took away.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use rusqlite::Connection;
use serde_json::Value;

use super::{
    Scope,
    db::{self, Entry, Folder},
};
use crate::{
    utils::{
        id::{generate_id, now_ms},
        json::{read_json, str_of},
    },
    workspace::kinds::kind_name_entry,
};

/// Files in a tree that are not entities.
const SKIP_FILES: [&str; 4] = ["index.json", "enabled.json", "names.json", "pending-deletions.json"];

/// Directories in a tree that are not folders. Those whose name starts with a dot are not either.
const SKIP_DIRS: [&str; 2] = ["drafts", "capture"];

/// An entity file found in a tree.
struct DiskFile {
    id: String,
    path: String,
    mtime: i64,
    size: i64,
}

/// When a file was last written, in milliseconds, and how long it is.
pub fn stamp(file: &Path) -> (i64, i64) {
    let Ok(meta) = std::fs::metadata(file) else { return (0, 0) };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0);
    (mtime, meta.len() as i64)
}

fn walk(dir: &Path, prefix: &str, folders: &mut Vec<String>, files: &mut Vec<DiskFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else { continue };
        if name.starts_with('.') {
            continue;
        }
        let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        if file_type.is_dir() {
            if !SKIP_DIRS.contains(&name.as_str()) {
                walk(&entry.path(), &path, folders, files);
                folders.push(path);
            }
        } else if let Some(id) = name.strip_suffix(".json").filter(|_| !SKIP_FILES.contains(&name.as_str())) {
            let (mtime, size) = stamp(&entry.path());
            files.push(DiskFile { id: id.to_string(), path, mtime, size });
        }
    }
}

/// The directory a path is in, or `None` at the top of the tree.
pub fn parent_path(path: &str) -> Option<&str> {
    path.rsplit_once('/').map(|(parent, _)| parent)
}

/// The `type` an entity file of `scope` holds: its own when it names one of the tree's, else the tree's first.
pub fn type_of(scope: Scope, entity: &Value) -> &'static str {
    let named = str_of(entity, "type");
    scope.types().iter().find(|(_, ty)| *ty == named).unwrap_or(&scope.types()[0]).1
}

/// What the index keeps of `entity`, and the lower-case text a search looks through.
pub fn describe(scope: Scope, id: &str, entity: &Value, folder_id: Option<String>, path: String, file: &Path) -> (Entry, String) {
    let ty = type_of(scope, entity);
    let kind = scope.kind_of_type(ty).unwrap_or(scope.dir());
    let details = kind_name_entry(kind, entity).unwrap_or_else(|| serde_json::json!({}));
    let mut search = String::new();
    for value in details.as_object().into_iter().flat_map(|fields| fields.values()) {
        if let Some(text) = value.as_str() {
            search.push_str(&text.to_lowercase());
            search.push('\n');
        }
    }
    let (mtime, size) = stamp(file);
    let entry = Entry {
        id: id.to_string(),
        ty: ty.to_string(),
        folder_id,
        name: str_of(entity, "name").to_string(),
        details,
        path,
        mtime,
        size,
    };
    (entry, search)
}

/// Make the index say what the tree at `dir` holds. A file is read only when it is new to the index or was
/// written since; a folder that is new gets an id, one that is gone loses its.
pub fn reconcile(conn: &mut Connection, dir: &Path, scope: Scope) -> std::io::Result<()> {
    let (mut on_disk, mut files) = (Vec::new(), Vec::new());
    walk(dir, "", &mut on_disk, &mut files);
    // Parents before the folders in them.
    on_disk.sort();

    let tx = conn.transaction().map_err(db::io)?;

    let mut folder_ids: HashMap<String, String> =
        db::folders(&tx).map_err(db::io)?.into_iter().map(|folder| (folder.path, folder.id)).collect();
    let present: HashSet<&str> = on_disk.iter().map(String::as_str).collect();
    let gone: Vec<String> = folder_ids.keys().filter(|path| !present.contains(path.as_str())).cloned().collect();
    for path in gone {
        if let Some(id) = folder_ids.remove(&path) {
            db::delete_folder(&tx, &id).map_err(db::io)?;
        }
    }
    for path in &on_disk {
        if folder_ids.contains_key(path) {
            continue;
        }
        let folder = Folder {
            id: generate_id(),
            parent_id: parent_path(path).and_then(|parent| folder_ids.get(parent)).cloned(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.clone(),
            created_at: now_ms(),
        };
        db::insert_folder(&tx, &folder).map_err(db::io)?;
        folder_ids.insert(folder.path, folder.id);
    }

    let known: HashMap<String, Entry> =
        db::entries(&tx).map_err(db::io)?.into_iter().map(|entry| (entry.id.clone(), entry)).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    for file in &files {
        seen.insert(file.id.as_str());
        let folder_id = parent_path(&file.path).and_then(|parent| folder_ids.get(parent)).cloned();
        if let Some(entry) = known.get(&file.id) {
            if entry.path == file.path && entry.mtime == file.mtime && entry.size == file.size && entry.folder_id == folder_id {
                continue;
            }
        }
        let full = dir.join(&file.path);
        match read_json(&full).filter(Value::is_object) {
            Some(entity) => {
                let (entry, search) = describe(scope, &file.id, &entity, folder_id, file.path.clone(), &full);
                db::upsert_entry(&tx, &entry, &search).map_err(db::io)?;
            }
            // Not an entity that can be read: it is not listed.
            None => db::delete_entry(&tx, &file.id).map_err(db::io)?,
        }
    }
    for id in known.keys().filter(|id| !seen.contains(id.as_str())) {
        db::delete_entry(&tx, id).map_err(db::io)?;
    }

    tx.commit().map_err(db::io)
}
