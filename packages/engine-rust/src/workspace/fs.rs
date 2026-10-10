//! The on-disk layout of a workspace, and every read and write of it.
//!
//! ```text
//! <dataDir>/data/<wsId>/            a git repository
//!   workspace.json  .gitignore  pending-deletions.json
//!   requests/  mocks/               the two trees: see `workspace::tree`
//!   rules/<id>.json                 a proxy rule at the top level
//!   rules/<folder name>/<id>.json   a proxy rule in a folder (one level deep)
//!   rules/index.json                { folders: [...], order: [...] }
//!   rules/names.json                id -> { name, url } for cheap listings
//!   <kind>/enabled.json             ids that are switched on (not in git's entity files)
//! ```
//!
//! Every request, whatever its type, is in the `requests` tree, and every mock in the `mocks` tree; the
//! functions here hand those kinds over to `workspace::tree`. `mappings` and `environments` are flat: no
//! folders, no names index.

use std::path::{Path, PathBuf};

use indexmap::IndexSet;
use serde_json::{Map, Value, json};

use super::{config::GLOBAL_ENV_ID, paths::data_dir, stamp, tree};
use crate::utils::{
    id::{generate_id, now_ms},
    json::{read_json, str_of, write_json_pretty},
};

pub const ALL_ENTITY_KINDS: [&str; 12] = [
    "requests",
    "mocks",
    "sockets",
    "rules",
    "mappings",
    "graphqlRequests",
    "soapRequests",
    "grpcRequests",
    "sseRequests",
    "socketioRequests",
    "mcpRequests",
    "environments",
];

pub const FLAT_ENTITY_KINDS: [&str; 2] = ["mappings", "environments"];

/// Files in a kind's directory that are not entities.
const SKIP_FILES: [&str; 4] = ["index.json", "enabled.json", "names.json", "pending-deletions.json"];

/// Directories in a kind's directory that are not folders.
pub const SKIP_DIRS: [&str; 3] = ["drafts", "capture", ".runs"];

/// `<dataDir>/data`: the parent of every workspace directory.
pub fn data_root() -> PathBuf {
    data_dir().join("data")
}

pub fn ws_dir(ws_id: &str) -> PathBuf {
    data_root().join(ws_id)
}

/// The directory the entities of `kind` are in: a tree's for the kinds kept in one.
pub fn entity_dir(ws_id: &str, kind: &str) -> PathBuf {
    ws_dir(ws_id).join(tree::Scope::of_kind(kind).map_or(kind, |scope| scope.dir()))
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

/// The file at a workspace's root that says the directory (and the repository it is pushed to) is a workspace.
pub const WORKSPACE_FILE: &str = "workspace.json";
/// What `kind` in `workspace.json` holds.
pub const WORKSPACE_KIND: &str = "bifurc-workspace";
pub const WORKSPACE_VERSION: u64 = 1;

/// The id and name a `workspace.json` gives, when it is a workspace's. A file from before `kind` was written
/// is known by the fields every one of them has.
pub fn workspace_identity(info: &Value) -> Option<(String, String)> {
    let marked = match info.get("kind") {
        Some(kind) => kind.as_str() == Some(WORKSPACE_KIND),
        None => info.get("createdAt").is_some_and(Value::is_number) && info.get("name").is_some_and(Value::is_string),
    };
    let id = info.get("id").and_then(Value::as_str).filter(|id| !id.is_empty())?;
    marked.then(|| (id.to_string(), info.get("name").and_then(Value::as_str).unwrap_or_default().to_string()))
}

/// Create a workspace's directories, `workspace.json` and `.gitignore`. Safe to repeat.
pub fn init_workspace_dir(ws_id: &str, name: &str) -> std::io::Result<()> {
    let root = ws_dir(ws_id);
    std::fs::create_dir_all(&root)?;
    for kind in ["mappings", "rules", "environments", "mocks", "requests", "capture", "applications"] {
        std::fs::create_dir_all(root.join(kind))?;
    }
    let ws_file = root.join("workspace.json");
    if !ws_file.exists() {
        write_json_pretty(
            &ws_file,
            &json!({
                "kind": WORKSPACE_KIND, "version": WORKSPACE_VERSION,
                "id": ws_id, "name": name, "createdAt": now_ms(), "activeEnvironmentId": null,
            }),
        )?;
    }
    let gitignore = root.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(gitignore, format!("capture/\n*.tmp\n{}*\n", tree::INDEX_FILE))?;
    }
    ensure_global_environment(ws_id)?;
    Ok(())
}

/// Workspace-relative path of the environment every workspace has.
pub const GLOBAL_ENV_PATH: &str = "environments/__global__.json";

/// The Global environment as a new workspace gets it. The same on every device, so two of them that each
/// had to make it agree on the file.
pub fn default_global_environment(ws_id: &str) -> Value {
    json!({ "id": GLOBAL_ENV_ID, "name": "Global", "variables": [], "createdAt": 0, "workspaceId": ws_id })
}

/// Write the Global environment when the workspace has none. Returns it when it was written.
pub fn ensure_global_environment(ws_id: &str) -> std::io::Result<Option<Value>> {
    if ws_dir(ws_id).join(GLOBAL_ENV_PATH).exists() {
        return Ok(None);
    }
    let global = default_global_environment(ws_id);
    write_flat_entity(ws_id, "environments", GLOBAL_ENV_ID, &global)?;
    Ok(Some(global))
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
    if tree::is_tree_kind(kind) {
        return tree::locate(ws_id, kind, id).map(|(_, file)| file);
    }
    let dir = entity_dir(ws_id, kind);
    let root_file = dir.join(format!("{id}.json"));
    if root_file.exists() {
        return Some(root_file);
    }
    folder_dirs(&dir).into_iter().map(|(_, path)| path.join(format!("{id}.json"))).find(|file| file.exists())
}

/// The path of an entity's file relative to the workspace, with forward slashes (as git names it).
pub fn find_entity_rel_path(ws_id: &str, kind: &str, id: &str) -> Option<String> {
    if let Some(scope) = tree::Scope::of_kind(kind) {
        return tree::locate(ws_id, kind, id).map(|(entry, _)| tree::rel_path(scope, &entry));
    }
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

/// An entity of `kind` as stored: of a stamped kind, with the stamp of the engine writing it
/// (`workspace::stamp`).
fn stamped(kind: &str, data: &Value) -> Value {
    let mut out = without_enabled(data);
    if stamp::is_stamped_kind(kind) {
        out[stamp::VERSION_FIELD] = stamp::stamp_for(data);
    }
    out
}

/// The version an entity is locked to, when a newer engine than this one last wrote it (`workspace::stamp`).
pub fn entity_lock(ws_id: &str, kind: &str, id: &str) -> Option<String> {
    if tree::is_tree_kind(kind) {
        return tree::lock_of(ws_id, kind, id);
    }
    if !stamp::is_stamped_kind(kind) {
        return None;
    }
    stamp::lock_of(&read_json(&find_entity_file(ws_id, kind, id)?)?)
}

/// What an entity file holds, unless it is of a stamped kind and locked.
fn read_unlocked(kind: &str, path: &Path) -> Option<Value> {
    read_json(path).filter(|entity| !stamp::is_stamped_kind(kind) || stamp::lock_of(entity).is_none())
}

/// Write an entity, moving its file if its folder changed. `folder` is the folder's id for a kind kept in a
/// tree, and its name for the others. A locked entity is not written over.
pub fn write_entity(ws_id: &str, kind: &str, id: &str, data: &Value, folder: Option<&str>) -> std::io::Result<()> {
    if tree::is_tree_kind(kind) {
        return tree::write_entity(ws_id, kind, id, data, folder);
    }
    let folder_name = folder;
    let on_disk = find_entity_file(ws_id, kind, id).and_then(|file| read_json(&file));
    delete_entity_file(ws_id, kind, id)?;
    write_json_pretty(&entity_file(ws_id, kind, id, folder_name), &stamp::settled(on_disk, stamped(kind, data)))
}

/// Remove an entity's file. A locked entity stays.
pub fn delete_entity_file(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    if tree::is_tree_kind(kind) {
        return tree::delete_entity(ws_id, kind, id);
    }
    if let Some(version) = entity_lock(ws_id, kind, id) {
        return Err(stamp::update_required_io(&version));
    }
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

/// Every entity of a kind, parsed. Unreadable files are skipped, and so are locked entities.
pub fn read_all_entities(ws_id: &str, kind: &str) -> Vec<Value> {
    if tree::is_tree_kind(kind) {
        return tree::read_all(ws_id, kind, None);
    }
    entity_files(ws_id, kind).into_iter().filter_map(|(_, path, _)| read_unlocked(kind, &path)).collect()
}

/// The entities of a kind whose id is in `enabled_ids`; all of them when it is `None`.
pub fn read_enabled_entities(ws_id: &str, kind: &str, enabled_ids: Option<&IndexSet<String>>) -> Vec<Value> {
    let Some(enabled) = enabled_ids else { return read_all_entities(ws_id, kind) };
    if enabled.is_empty() {
        return Vec::new();
    }
    if tree::is_tree_kind(kind) {
        return tree::read_all(ws_id, kind, Some(&|id| enabled.contains(id)));
    }
    entity_files(ws_id, kind)
        .into_iter()
        .filter(|(id, _, _)| enabled.contains(id))
        .filter_map(|(_, path, _)| read_unlocked(kind, &path))
        .collect()
}

/// One entity, from its file. One that is locked is not read: `entity_lock` says that it is.
pub fn read_entity(ws_id: &str, kind: &str, id: &str) -> Option<Value> {
    if tree::is_tree_kind(kind) {
        return tree::read_entity(ws_id, kind, id);
    }
    read_unlocked(kind, &find_entity_file(ws_id, kind, id)?)
}

/// What a listing needs of an entity without opening its file.
#[derive(Debug, Clone, PartialEq)]
pub struct EntityStub {
    pub id: String,
    pub folder_id: Option<String>,
    /// The pending-deletion record of an entity that was deleted here and whose deletion is not synced yet.
    pub deleted: Option<Value>,
    /// The version the entity is locked to, when a newer engine than this one last wrote it.
    pub locked: Option<String>,
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
        .map(|(id, path, folder_dir)| EntityStub {
            id,
            folder_id: folder_dir.as_deref().and_then(folder_id_of),
            deleted: None,
            // The one thing a listing opens a file of a stamped kind for.
            locked: stamp::is_stamped_kind(kind).then(|| read_json(&path)).flatten().as_ref().and_then(stamp::lock_of),
        })
        .collect();
    for pending in get_pending_deletions(ws_id, kind) {
        let id = str_of(&pending, "id").to_string();
        if !stubs.iter().any(|stub| stub.id == id) {
            let folder_id = pending.get("folderId").and_then(Value::as_str).map(str::to_string);
            stubs.push(EntityStub { id, folder_id, deleted: Some(pending), locked: None });
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

/// The folder that takes what is saved without one.
pub const DEFAULT_FOLDER_NAME: &str = "Default";

/// The `(id, name)` of the kind's Default folder, which is created when the kind has none.
pub fn ensure_default_folder(ws_id: &str, kind: &str) -> std::io::Result<(String, String)> {
    create_folder(ws_id, kind, DEFAULT_FOLDER_NAME, None, false)
}

/// Make a folder of a kind and return its `(id, name)`. Folders of one name share a directory, so with `unique` false a
/// folder of that name that exists is the answer; with `unique` true the name gets a number until it is free.
pub fn create_folder(
    ws_id: &str,
    kind: &str,
    name: &str,
    parent_id: Option<&str>,
    unique: bool,
) -> std::io::Result<(String, String)> {
    if let Some(scope) = tree::Scope::of_kind(kind) {
        return tree::create_folder(ws_id, scope, name, parent_id, unique).map(|folder| (folder.id, folder.name));
    }
    let mut index = auto_sync_fs_directories(ws_id, kind, generate_id);
    let taken = |name: &str| index.folders.iter().find(|f| sanitize_dir_name(str_of(f, "name")) == sanitize_dir_name(name));
    let mut name = name.to_string();
    if unique {
        let base = name.clone();
        let mut n = 2;
        while taken(&name).is_some() {
            name = format!("{base}-{n}");
            n += 1;
        }
    } else if let Some(folder) = taken(&name) {
        return Ok((str_of(folder, "id").to_string(), str_of(folder, "name").to_string()));
    }
    let folder = json!({
        "name": name,
        "parentId": parent_id,
        "id": generate_id(),
        "createdAt": now_ms(),
        "workspaceId": ws_id,
    });
    std::fs::create_dir_all(entity_dir(ws_id, kind).join(sanitize_dir_name(&name)))?;
    index.folders.push(folder.clone());
    write_index(ws_id, kind, &index)?;
    Ok((str_of(&folder, "id").to_string(), name))
}

/// The `(id, name)` of the folder `folder_id` names, or of the Default folder when it names none that exists.
pub fn resolve_folder(ws_id: &str, kind: &str, folder_id: Option<&str>) -> std::io::Result<(String, String)> {
    if let (Some(scope), Some(folder_id)) = (tree::Scope::of_kind(kind), folder_id.filter(|id| !id.is_empty())) {
        if let Some(folder) = tree::folder(ws_id, scope, folder_id) {
            return Ok((folder.id, folder.name));
        }
    } else if let Some(folder_id) = folder_id.filter(|id| !id.is_empty()) {
        let index = auto_sync_fs_directories(ws_id, kind, generate_id);
        if let Some(folder) = index.folders.iter().find(|f| str_of(f, "id") == folder_id) {
            return Ok((folder_id.to_string(), str_of(folder, "name").to_string()));
        }
    }
    ensure_default_folder(ws_id, kind)
}

// ── names.json ─────────────────────────────────────────────────────────────────────────────────────────

/// A kind's names index: entity id -> `{name, method?, url?, ...}`. For a kind kept in a tree it is what the
/// tree's index lists of each entity, and the three functions that write one have nothing to do.
pub fn read_names_index(ws_id: &str, kind: &str) -> Map<String, Value> {
    if let (Some(scope), Some(ty)) = (tree::Scope::of_kind(kind), tree::type_of_kind(kind)) {
        return tree::entries(ws_id, scope)
            .into_iter()
            .filter(|entry| entry.ty == ty)
            .map(|entry| (entry.id, entry.details))
            .collect();
    }
    read_json(&entity_dir(ws_id, kind).join("names.json")).and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn write_names_index(ws_id: &str, kind: &str, names: &Map<String, Value>) -> std::io::Result<()> {
    write_json_pretty(&entity_dir(ws_id, kind).join("names.json"), &Value::Object(names.clone()))
}

pub fn upsert_name_entry(ws_id: &str, kind: &str, id: &str, entry: Value) -> std::io::Result<()> {
    if tree::is_tree_kind(kind) {
        return Ok(());
    }
    let mut names = read_names_index(ws_id, kind);
    names.insert(id.to_string(), entry);
    write_names_index(ws_id, kind, &names)
}

pub fn remove_name_entry(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    if tree::is_tree_kind(kind) {
        return Ok(());
    }
    let mut names = read_names_index(ws_id, kind);
    names.shift_remove(id);
    write_names_index(ws_id, kind, &names)
}

/// Rebuild a names index from the entity files (`name`, and `method` / `url` when non-empty).
pub fn bootstrap_names_index(ws_id: &str, kind: &str) -> Map<String, Value> {
    if tree::is_tree_kind(kind) {
        return read_names_index(ws_id, kind);
    }
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
    // No workspace (nothing is active before the first one is made): nothing is on, and nothing is written.
    if ws_id.is_empty() {
        return IndexSet::new();
    }
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
/// `{id, folderId, path, name, method?, url?, urlSuffix?}`. A kind's deleted folders are kept the same way,
/// under `pending_folders_key(kind)`, as the folder objects `index.json` held.
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

/// The key a kind's deleted folders are kept under in `pending-deletions.json`. The kinds of a tree share its
/// folders, so they share the key.
pub fn pending_folders_key(kind: &str) -> String {
    format!("{}.folders", tree::Scope::of_kind(kind).map_or(kind, |scope| scope.dir()))
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
        assert_eq!(entity_rel_path("rules", "m1", Some("a/b")), "rules/ab/m1.json");
        assert_eq!(entity_rel_path("rules", "m1", None), "rules/m1.json");
    }
}
