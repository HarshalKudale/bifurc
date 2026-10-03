//! The on-disk layout of a workspace, and every read and write of it.
//!
//! ```text
//! <dataDir>/data/<wsId>/            a git repository
//!   workspace.json  .gitignore  pending-deletions.json
//!   <kind>/<id>.json                an entity at the top level
//!   <kind>/<folder name>/<id>.json  an entity in a folder (one level deep)
//!   <kind>/index.json               { folders: [...], order: [...] }
//!   <kind>/names.json               id -> { name, method?, url?, ... } for cheap listings
//!   <kind>/enabled.json             ids that are switched on (not in git's entity files)
//! ```
//!
//! `mappings` and `environments` are flat: no folders, no names index.

use std::path::{Path, PathBuf};

use indexmap::IndexSet;
use serde_json::{Map, Value, json};

use super::paths::data_dir;
use crate::utils::{
    id::now_ms,
    json::{read_json, str_of, write_json_pretty},
};

pub const ALL_ENTITY_KINDS: [&str; 9] = [
    "requests",
    "mocks",
    "sockets",
    "rules",
    "mappings",
    "graphqlRequests",
    "soapRequests",
    "grpcRequests",
    "environments",
];

pub const FLAT_ENTITY_KINDS: [&str; 2] = ["mappings", "environments"];

/// Files in a kind's directory that are not entities.
const SKIP_FILES: [&str; 4] = ["index.json", "enabled.json", "names.json", "pending-deletions.json"];

/// Directories in a kind's directory that are not folders.
const SKIP_DIRS: [&str; 3] = ["drafts", "capture", ".runs"];

/// `<dataDir>/data`: the parent of every workspace directory.
pub fn data_root() -> PathBuf {
    data_dir().join("data")
}

pub fn ws_dir(ws_id: &str) -> PathBuf {
    data_root().join(ws_id)
}

pub fn entity_dir(ws_id: &str, kind: &str) -> PathBuf {
    ws_dir(ws_id).join(kind)
}

/// A folder's directory name: its display name without the characters a filesystem rejects.
pub fn sanitize_dir_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') && *c >= ' ')
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() { "unnamed".to_string() } else { trimmed.to_string() }
}

fn entity_file(ws_id: &str, kind: &str, id: &str, folder_name: Option<&str>) -> PathBuf {
    let base = entity_dir(ws_id, kind);
    match folder_name.filter(|name| !name.is_empty()) {
        Some(folder) => base.join(sanitize_dir_name(folder)).join(format!("{id}.json")),
        None => base.join(format!("{id}.json")),
    }
}

/// Create a workspace's directories, `workspace.json` and `.gitignore`. Safe to repeat.
pub fn init_workspace_dir(ws_id: &str, name: &str) -> std::io::Result<()> {
    let root = ws_dir(ws_id);
    std::fs::create_dir_all(&root)?;
    for kind in [
        "mappings",
        "rules",
        "environments",
        "mocks",
        "requests",
        "sockets",
        "capture",
        "graphqlRequests",
        "soapRequests",
        "grpcRequests",
        "applications",
    ] {
        std::fs::create_dir_all(root.join(kind))?;
    }
    let ws_file = root.join("workspace.json");
    if !ws_file.exists() {
        write_json_pretty(
            &ws_file,
            &json!({ "id": ws_id, "name": name, "createdAt": now_ms(), "activeEnvironmentId": null }),
        )?;
    }
    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(gitignore, "capture/\n*.tmp\n")?;
    }
    Ok(())
}

/// The sub-directories of a kind that are folders.
fn folder_dirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
        .filter(|(name, _)| !SKIP_DIRS.contains(&name.as_str()))
        .collect()
}

/// The absolute path of an entity's file, wherever in its kind it is.
pub fn find_entity_file(ws_id: &str, kind: &str, id: &str) -> Option<PathBuf> {
    let dir = entity_dir(ws_id, kind);
    let root_file = dir.join(format!("{id}.json"));
    if root_file.exists() {
        return Some(root_file);
    }
    folder_dirs(&dir).into_iter().map(|(_, path)| path.join(format!("{id}.json"))).find(|file| file.exists())
}

/// The path of an entity's file relative to the workspace, with forward slashes (as git names it).
pub fn find_entity_rel_path(ws_id: &str, kind: &str, id: &str) -> Option<String> {
    let dir = entity_dir(ws_id, kind);
    if dir.join(format!("{id}.json")).exists() {
        return Some(format!("{kind}/{id}.json"));
    }
    folder_dirs(&dir)
        .into_iter()
        .find(|(_, path)| path.join(format!("{id}.json")).exists())
        .map(|(name, _)| format!("{kind}/{name}/{id}.json"))
}

pub fn entity_rel_path(kind: &str, id: &str, folder_name: Option<&str>) -> String {
    match folder_name.filter(|name| !name.is_empty()) {
        Some(folder) => format!("{kind}/{}/{id}.json", sanitize_dir_name(folder)),
        None => format!("{kind}/{id}.json"),
    }
}

pub fn flat_entity_rel_path(kind: &str, id: &str) -> String {
    format!("{kind}/{id}.json")
}

/// An entity as stored: without `enabled`, which lives in `enabled.json`.
fn without_enabled(data: &Value) -> Value {
    let mut out: Map<String, Value> = data.as_object().cloned().unwrap_or_default();
    out.shift_remove("enabled");
    Value::Object(out)
}

/// Write an entity, moving its file if its folder changed.
pub fn write_entity(ws_id: &str, kind: &str, id: &str, data: &Value, folder_name: Option<&str>) -> std::io::Result<()> {
    delete_entity_file(ws_id, kind, id)?;
    write_json_pretty(&entity_file(ws_id, kind, id, folder_name), &without_enabled(data))
}

pub fn delete_entity_file(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    match find_entity_file(ws_id, kind, id) {
        Some(file) => std::fs::remove_file(file),
        None => Ok(()),
    }
}

pub fn write_flat_entity(ws_id: &str, kind: &str, id: &str, data: &Value) -> std::io::Result<()> {
    write_json_pretty(&entity_dir(ws_id, kind).join(format!("{id}.json")), &without_enabled(data))
}

pub fn delete_flat_entity_file(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    let file = entity_dir(ws_id, kind).join(format!("{id}.json"));
    if file.exists() { std::fs::remove_file(file) } else { Ok(()) }
}

fn is_entity_file_name(name: &str) -> bool {
    name.ends_with(".json") && !SKIP_FILES.contains(&name)
}

/// `(id, path, folder directory name)` of every entity file of a kind.
fn entity_files(ws_id: &str, kind: &str) -> Vec<(String, PathBuf, Option<String>)> {
    let dir = entity_dir(ws_id, kind);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_file() && is_entity_file_name(&name) {
            out.push((name[..name.len() - 5].to_string(), entry.path(), None));
        } else if file_type.is_dir() && !SKIP_DIRS.contains(&name.as_str()) {
            let Ok(children) = std::fs::read_dir(entry.path()) else { continue };
            for child in children.flatten() {
                let child_name = child.file_name().to_string_lossy().into_owned();
                if is_entity_file_name(&child_name) {
                    out.push((child_name[..child_name.len() - 5].to_string(), child.path(), Some(name.clone())));
                }
            }
        }
    }
    out
}

/// Every entity of a kind, parsed. Unreadable files are skipped.
pub fn read_all_entities(ws_id: &str, kind: &str) -> Vec<Value> {
    entity_files(ws_id, kind).into_iter().filter_map(|(_, path, _)| read_json(&path)).collect()
}

/// The entities of a kind whose id is in `enabled_ids`; all of them when it is `None`.
pub fn read_enabled_entities(ws_id: &str, kind: &str, enabled_ids: Option<&IndexSet<String>>) -> Vec<Value> {
    let Some(enabled) = enabled_ids else { return read_all_entities(ws_id, kind) };
    if enabled.is_empty() {
        return Vec::new();
    }
    entity_files(ws_id, kind)
        .into_iter()
        .filter(|(id, _, _)| enabled.contains(id))
        .filter_map(|(_, path, _)| read_json(&path))
        .collect()
}

pub fn read_entity(ws_id: &str, kind: &str, id: &str) -> Option<Value> {
    read_json(&find_entity_file(ws_id, kind, id)?)
}

/// What a listing needs of an entity without opening its file.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityStub {
    pub id: String,
    pub folder_id: Option<String>,
}

/// The ids and folders of a kind's entities, plus those deleted locally but not yet published.
pub fn read_entity_stubs(ws_id: &str, kind: &str) -> Vec<EntityStub> {
    let index = read_index(ws_id, kind);
    let folder_id_of = |dir_name: &str| -> Option<String> {
        index
            .folders
            .iter()
            .find(|folder| sanitize_dir_name(str_of(folder, "name")) == dir_name)
            .map(|folder| str_of(folder, "id").to_string())
    };
    let mut stubs: Vec<EntityStub> = entity_files(ws_id, kind)
        .into_iter()
        .map(|(id, _, folder_dir)| EntityStub { id, folder_id: folder_dir.as_deref().and_then(folder_id_of) })
        .collect();
    for pending in get_pending_deletions(ws_id, kind) {
        let id = str_of(&pending, "id").to_string();
        if !stubs.iter().any(|stub| stub.id == id) {
            let folder_id = pending.get("folderId").and_then(Value::as_str).map(str::to_string);
            stubs.push(EntityStub { id, folder_id });
        }
    }
    stubs
}

// ── index.json ─────────────────────────────────────────────────────────────────────────────────────────

/// A kind's folders (`{id, name, parentId, createdAt, workspaceId}`) and entity order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EntityIndex {
    pub folders: Vec<Value>,
    pub order: Vec<Value>,
}

pub fn read_index(ws_id: &str, kind: &str) -> EntityIndex {
    let parsed = read_json(&entity_dir(ws_id, kind).join("index.json")).unwrap_or(Value::Null);
    EntityIndex {
        folders: parsed.get("folders").and_then(Value::as_array).cloned().unwrap_or_default(),
        order: parsed.get("order").and_then(Value::as_array).cloned().unwrap_or_default(),
    }
}

pub fn write_index(ws_id: &str, kind: &str, index: &EntityIndex) -> std::io::Result<()> {
    write_json_pretty(
        &entity_dir(ws_id, kind).join("index.json"),
        &json!({ "folders": index.folders, "order": index.order }),
    )
}

pub fn delete_entity_dir(ws_id: &str, kind: &str, folder_name: &str) -> std::io::Result<()> {
    let dir = entity_dir(ws_id, kind).join(sanitize_dir_name(folder_name));
    if dir.exists() { std::fs::remove_dir_all(dir) } else { Ok(()) }
}

/// Adopt directories that appeared on disk (a pull, a manual copy) as folders, and return the index.
pub fn auto_sync_fs_directories(ws_id: &str, kind: &str, make_id: impl Fn() -> String) -> EntityIndex {
    let mut index = read_index(ws_id, kind);
    let mut changed = false;
    for (dir_name, _) in folder_dirs(&entity_dir(ws_id, kind)) {
        if !index.folders.iter().any(|folder| sanitize_dir_name(str_of(folder, "name")) == dir_name) {
            index.folders.push(json!({
                "id": make_id(),
                "name": dir_name,
                "parentId": null,
                "createdAt": now_ms(),
                "workspaceId": ws_id,
            }));
            changed = true;
        }
    }
    if changed {
        let _ = write_index(ws_id, kind, &index);
    }
    index
}

// ── names.json ─────────────────────────────────────────────────────────────────────────────────────────

/// A kind's names index: entity id -> `{name, method?, url?, ...}`.
pub fn read_names_index(ws_id: &str, kind: &str) -> Map<String, Value> {
    read_json(&entity_dir(ws_id, kind).join("names.json")).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn write_names_index(ws_id: &str, kind: &str, names: &Map<String, Value>) -> std::io::Result<()> {
    write_json_pretty(&entity_dir(ws_id, kind).join("names.json"), &Value::Object(names.clone()))
}

pub fn upsert_name_entry(ws_id: &str, kind: &str, id: &str, entry: Value) -> std::io::Result<()> {
    let mut names = read_names_index(ws_id, kind);
    names.insert(id.to_string(), entry);
    write_names_index(ws_id, kind, &names)
}

pub fn remove_name_entry(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    let mut names = read_names_index(ws_id, kind);
    names.shift_remove(id);
    write_names_index(ws_id, kind, &names)
}

/// Rebuild a names index from the entity files (`name`, and `method` / `url` when non-empty).
pub fn bootstrap_names_index(ws_id: &str, kind: &str) -> Map<String, Value> {
    let mut names = Map::new();
    for entity in read_all_entities(ws_id, kind) {
        let mut entry = Map::new();
        entry.insert("name".into(), entity.get("name").cloned().unwrap_or(Value::Null));
        for key in ["method", "url"] {
            if let Some(value) = entity.get(key).filter(|v| crate::utils::json::is_truthy(v)) {
                entry.insert(key.into(), value.clone());
            }
        }
        names.insert(str_of(&entity, "id").to_string(), Value::Object(entry));
    }
    let _ = write_names_index(ws_id, kind, &names);
    names
}

// ── enabled.json ───────────────────────────────────────────────────────────────────────────────────────

/// The ids switched on for a kind, or `None` when the file has never been written.
pub fn read_enabled_set(ws_id: &str, kind: &str) -> Option<IndexSet<String>> {
    let parsed = read_json(&entity_dir(ws_id, kind).join("enabled.json"))?;
    Some(parsed.as_array()?.iter().filter_map(Value::as_str).map(str::to_string).collect())
}

pub fn write_enabled_set(ws_id: &str, kind: &str, ids: &IndexSet<String>) -> std::io::Result<()> {
    write_json_pretty(&entity_dir(ws_id, kind).join("enabled.json"), &json!(ids.iter().collect::<Vec<_>>()))
}

/// Create `enabled.json` from the entity files: everything not explicitly `enabled: false` is on.
pub fn bootstrap_enabled_set(ws_id: &str, kind: &str) -> IndexSet<String> {
    let enabled: IndexSet<String> = read_all_entities(ws_id, kind)
        .iter()
        .filter(|entity| entity.get("enabled") != Some(&Value::Bool(false)))
        .map(|entity| str_of(entity, "id").to_string())
        .collect();
    let _ = write_enabled_set(ws_id, kind, &enabled);
    enabled
}

/// The enabled set, created from the entity files the first time.
pub fn load_enabled_set(ws_id: &str, kind: &str) -> IndexSet<String> {
    read_enabled_set(ws_id, kind).unwrap_or_else(|| bootstrap_enabled_set(ws_id, kind))
}

/// Switch one entity on or off.
pub fn sync_enabled_set(ws_id: &str, kind: &str, id: &str, enabled: bool) -> std::io::Result<()> {
    let mut set = load_enabled_set(ws_id, kind);
    if enabled {
        set.insert(id.to_string());
    } else {
        set.shift_remove(id);
    }
    write_enabled_set(ws_id, kind, &set)
}

// ── pending-deletions.json ─────────────────────────────────────────────────────────────────────────────

fn pending_deletions_file(ws_id: &str) -> PathBuf {
    ws_dir(ws_id).join("pending-deletions.json")
}

fn read_pending_deletions(ws_id: &str) -> Map<String, Value> {
    read_json(&pending_deletions_file(ws_id)).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn write_pending_deletions(ws_id: &str, data: &Map<String, Value>) -> std::io::Result<()> {
    std::fs::write(pending_deletions_file(ws_id), serde_json::to_string_pretty(data)?)
}

/// Remember an entity deleted locally until the deletion is published:
/// `{id, folderId, name, method?, url?, urlSuffix?}`.
pub fn add_pending_deletion(ws_id: &str, kind: &str, entry: Value) -> std::io::Result<()> {
    let mut data = read_pending_deletions(ws_id);
    let list = data.entry(kind.to_string()).or_insert_with(|| json!([]));
    if !list.is_array() {
        *list = json!([]);
    }
    let list = list.as_array_mut().expect("just made an array");
    if !list.iter().any(|e| str_of(e, "id") == str_of(&entry, "id")) {
        list.push(entry);
    }
    write_pending_deletions(ws_id, &data)
}

pub fn remove_pending_deletion(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    let mut data = read_pending_deletions(ws_id);
    if let Some(list) = data.get_mut(kind).and_then(Value::as_array_mut) {
        list.retain(|e| str_of(e, "id") != id);
    }
    write_pending_deletions(ws_id, &data)
}

pub fn get_pending_deletions(ws_id: &str, kind: &str) -> Vec<Value> {
    read_pending_deletions(ws_id).get(kind).and_then(Value::as_array).cloned().unwrap_or_default()
}

pub fn clear_pending_deletions(ws_id: &str, kind: &str) -> std::io::Result<()> {
    let mut data = read_pending_deletions(ws_id);
    data.shift_remove(kind);
    write_pending_deletions(ws_id, &data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_are_made_safe() {
        assert_eq!(sanitize_dir_name(" a/b:c?* "), "abc");
        assert_eq!(sanitize_dir_name("???"), "unnamed");
        assert_eq!(sanitize_dir_name("Auth \u{1}flows"), "Auth flows");
    }

    #[test]
    fn relative_paths_use_the_sanitized_folder() {
        assert_eq!(entity_rel_path("mocks", "m1", Some("a/b")), "mocks/ab/m1.json");
        assert_eq!(entity_rel_path("mocks", "m1", None), "mocks/m1.json");
    }
}
