//! The two trees of a workspace: its requests and its mocks.
//!
//! ```text
//! <workspace>/requests/                 every request, whatever its type
//!   index.db                            the tree's index (SQLite), on this device only
//!   <folder>/<folder>/<id>.json         { "type": "rest" | "graphql" | "grpc" | ..., ... }
//! <workspace>/mocks/
//!   index.db
//!   <folder>/<folder>/<id>.json         { "type": "mock", ... }
//! ```
//!
//! The directories are the folders: a folder in a folder is a directory in a directory, and an entity is in
//! the folder its file is in. The files are what git carries. The index holds what a listing and a search
//! need (each entity's type, folder, name and listed fields) and gives every folder an id; it is made from the
//! files and can always be made again.

pub mod db;
mod migrate;
mod scan;

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use parking_lot::Mutex;
use rusqlite::Connection;
use serde_json::{Map, Value, json};

pub use db::{Entry, Folder, INDEX_FILE};
pub use scan::type_of;

use super::fs::{sanitize_dir_name, ws_dir};
use crate::utils::{
    id::{generate_id, now_ms},
    json::{read_json, write_json_pretty},
};

/// One of the two trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Requests,
    Mocks,
}

const REQUEST_TYPES: [(&str, &str); 8] = [
    ("requests", "rest"),
    ("graphqlRequests", "graphql"),
    ("grpcRequests", "grpc"),
    ("soapRequests", "soap"),
    ("sockets", "websocket"),
    ("sseRequests", "sse"),
    ("socketioRequests", "socketio"),
    ("mcpRequests", "mcp"),
];

const MOCK_TYPES: [(&str, &str); 1] = [("mocks", "mock")];

impl Scope {
    pub const ALL: [Scope; 2] = [Scope::Requests, Scope::Mocks];

    /// The tree's directory in the workspace.
    pub fn dir(self) -> &'static str {
        match self {
            Scope::Requests => "requests",
            Scope::Mocks => "mocks",
        }
    }

    /// The entity kinds kept in the tree, each with the `type` its files hold. The first is what a file
    /// without a `type` is.
    pub fn types(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Scope::Requests => &REQUEST_TYPES,
            Scope::Mocks => &MOCK_TYPES,
        }
    }

    pub fn kind_of_type(self, ty: &str) -> Option<&'static str> {
        self.types().iter().find(|(_, known)| *known == ty).map(|(kind, _)| *kind)
    }

    /// The tree an entity kind (the store's name for it) is kept in.
    pub fn of_kind(kind: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|scope| scope.types().iter().any(|(known, _)| *known == kind))
    }

    /// The tree a workspace-relative path is in.
    pub fn of_path(rel_path: &str) -> Option<Scope> {
        let top = rel_path.split(['/', '\\']).next().unwrap_or("");
        Scope::ALL.into_iter().find(|scope| scope.dir() == top)
    }
}

/// Whether entities of `kind` are kept in a tree.
pub fn is_tree_kind(kind: &str) -> bool {
    Scope::of_kind(kind).is_some()
}

/// The `type` the files of `kind` hold.
pub fn type_of_kind(kind: &str) -> Option<&'static str> {
    let scope = Scope::of_kind(kind)?;
    scope.types().iter().find(|(known, _)| *known == kind).map(|(_, ty)| *ty)
}

/// A folder's directory name: its name without what a filesystem rejects or the tree keeps for itself.
pub fn dir_name(name: &str) -> String {
    let cleaned = sanitize_dir_name(name);
    let trimmed = cleaned.trim_start_matches('.').trim_end_matches(['.', ' ']).trim();
    if trimmed.is_empty() { "unnamed".to_string() } else { trimmed.to_string() }
}

/// A folder as clients are given it.
pub fn folder_value(folder: &Folder, ws_id: &str) -> Value {
    json!({
        "name": folder.name,
        "parentId": folder.parent_id,
        "id": folder.id,
        "createdAt": folder.created_at,
        "workspaceId": ws_id,
    })
}

fn tree_dir(ws_id: &str, scope: Scope) -> PathBuf {
    ws_dir(ws_id).join(scope.dir())
}

/// One index is used at a time: every call below is a whole step.
static LOCK: Mutex<()> = Mutex::new(());

/// The index files that were brought in step with their tree since the engine started, or since the tree
/// was last changed from outside.
static FRESH: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Say that the files of a workspace's trees changed without the index being told: a pull, a checkout, an
/// import of files. The next use of each index reads its tree again.
pub fn mark_stale(ws_id: &str) {
    let mut fresh = FRESH.lock();
    if let Some(set) = fresh.as_mut() {
        for scope in Scope::ALL {
            set.remove(&tree_dir(ws_id, scope).join(INDEX_FILE));
        }
    }
}

/// Keep the index out of the repository of a workspace whose `.gitignore` was written before there was one.
pub fn ensure_ignored(ws_id: &str) {
    let info = ws_dir(ws_id).join(".git").join("info");
    if !info.is_dir() {
        return;
    }
    let file = info.join("exclude");
    let current = std::fs::read_to_string(&file).unwrap_or_default();
    let rule = format!("{INDEX_FILE}*");
    if !current.lines().any(|line| line.trim() == rule) {
        let separator = if current.is_empty() || current.ends_with('\n') { "" } else { "\n" };
        let _ = std::fs::write(&file, format!("{current}{separator}{rule}\n"));
    }
}

fn no_workspace() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, "No such workspace.")
}

/// Run `step` on a tree's index, which is first brought in step with the tree when it may be behind.
fn with_index<T>(
    ws_id: &str,
    scope: Scope,
    step: impl FnOnce(&mut Connection, &Path) -> std::io::Result<T>,
) -> std::io::Result<T> {
    if ws_id.is_empty() || !ws_dir(ws_id).is_dir() {
        return Err(no_workspace());
    }
    let _guard = LOCK.lock();
    let dir = tree_dir(ws_id, scope);
    let file = dir.join(INDEX_FILE);
    let fresh = file.exists() && FRESH.lock().as_ref().is_some_and(|set| set.contains(&file));
    let mut conn = db::open(&file)?;
    if !fresh {
        ensure_ignored(ws_id);
        migrate::run(&ws_dir(ws_id), scope);
        scan::reconcile(&mut conn, &dir, scope)?;
        FRESH.lock().get_or_insert_with(HashSet::new).insert(file);
    }
    step(&mut conn, &dir)
}

// ── entities ───────────────────────────────────────────────────────────────────────────────────────────

/// An entity as its file holds it: `type` first, and without what is not the file's to say (whether it is
/// switched on, and the folder, which is where the file is).
fn stored_form(ty: &str, data: &Value) -> Value {
    let mut out = Map::new();
    out.insert("type".into(), json!(ty));
    for (key, value) in data.as_object().into_iter().flatten() {
        if !matches!(key.as_str(), "type" | "enabled" | "folderId") {
            out.insert(key.clone(), value.clone());
        }
    }
    Value::Object(out)
}

/// An entity as it is handed out: what its file holds, with its type and the folder it is in.
fn handed_out(mut entity: Value, entry: &Entry) -> Value {
    if let Some(fields) = entity.as_object_mut() {
        fields.insert("type".into(), json!(entry.ty));
        fields.insert("folderId".into(), json!(entry.folder_id));
    }
    entity
}

/// What the index holds of `id`. An entity whose file is gone is looked for again in the tree first.
fn lookup(conn: &mut Connection, dir: &Path, scope: Scope, id: &str) -> std::io::Result<Option<Entry>> {
    let found = db::entry(conn, id).map_err(db::io)?;
    match found {
        Some(entry) if !dir.join(&entry.path).is_file() => {
            scan::reconcile(conn, dir, scope)?;
            db::entry(conn, id).map_err(db::io)
        }
        other => Ok(other),
    }
}

/// Where an entity of `kind` is: what the index holds of it, and its file.
pub fn locate(ws_id: &str, kind: &str, id: &str) -> Option<(Entry, PathBuf)> {
    let scope = Scope::of_kind(kind)?;
    let ty = type_of_kind(kind)?;
    with_index(ws_id, scope, |conn, dir| {
        Ok(lookup(conn, dir, scope, id)?.filter(|entry| entry.ty == ty).map(|entry| {
            let file = dir.join(&entry.path);
            (entry, file)
        }))
    })
    .ok()
    .flatten()
}

/// An entity's file relative to the workspace, as git names it.
pub fn rel_path(scope: Scope, entry: &Entry) -> String {
    format!("{}/{}", scope.dir(), entry.path)
}

/// Whatever the tree holds under `id`, of any type, with the kind it is.
pub fn find_any(ws_id: &str, scope: Scope, id: &str) -> Option<(&'static str, Entry)> {
    with_index(ws_id, scope, |conn, dir| lookup(conn, dir, scope, id))
        .ok()
        .flatten()
        .and_then(|entry| Some((scope.kind_of_type(&entry.ty)?, entry)))
}

pub fn read_entity(ws_id: &str, kind: &str, id: &str) -> Option<Value> {
    let (entry, file) = locate(ws_id, kind, id)?;
    Some(handed_out(read_json(&file)?, &entry))
}

/// Every entity of `kind`, read from its file. `only` narrows them to the ids it holds.
pub fn read_all(ws_id: &str, kind: &str, only: Option<&dyn Fn(&str) -> bool>) -> Vec<Value> {
    let (Some(scope), Some(ty)) = (Scope::of_kind(kind), type_of_kind(kind)) else { return Vec::new() };
    let listed = with_index(ws_id, scope, |conn, dir| {
        Ok((db::entries_of_type(conn, ty).map_err(db::io)?, dir.to_path_buf()))
    });
    let Ok((entries, dir)) = listed else { return Vec::new() };
    entries
        .into_iter()
        .filter(|entry| only.is_none_or(|wanted| wanted(&entry.id)))
        .filter_map(|entry| Some(handed_out(read_json(&dir.join(&entry.path))?, &entry)))
        .collect()
}

/// What the index holds of every entity of a tree.
pub fn entries(ws_id: &str, scope: Scope) -> Vec<Entry> {
    with_index(ws_id, scope, |conn, _| db::entries(conn).map_err(db::io)).unwrap_or_default()
}

/// The entities of a tree whose name or listed fields contain `query`, by name. `types` narrows them.
pub fn search(ws_id: &str, scope: Scope, query: &str, types: &[String], limit: i64) -> Vec<Entry> {
    let needle = query.trim().to_lowercase();
    with_index(ws_id, scope, |conn, _| db::search(conn, &needle, types, limit).map_err(db::io)).unwrap_or_default()
}

/// Write an entity of `kind` into the folder `folder_id` (the top of the tree when it names none that
/// exists), moving its file when that is another folder than it was in.
pub fn write_entity(ws_id: &str, kind: &str, id: &str, data: &Value, folder_id: Option<&str>) -> std::io::Result<()> {
    let (Some(scope), Some(ty)) = (Scope::of_kind(kind), type_of_kind(kind)) else {
        return Err(std::io::Error::other(format!("\"{kind}\" is not kept in a tree.")));
    };
    with_index(ws_id, scope, |conn, dir| {
        let folder = match folder_id.filter(|id| !id.is_empty()) {
            Some(folder_id) => db::folder(conn, folder_id).map_err(db::io)?,
            None => None,
        };
        let path = match &folder {
            Some(folder) => format!("{}/{id}.json", folder.path),
            None => format!("{id}.json"),
        };
        if let Some(old) = db::entry(conn, id).map_err(db::io)?.filter(|old| old.path != path) {
            let _ = std::fs::remove_file(dir.join(old.path));
        }
        let file = dir.join(&path);
        let stored = stored_form(ty, data);
        write_json_pretty(&file, &stored)?;
        let (entry, text) = scan::describe(scope, id, &stored, folder.map(|folder| folder.id), path, &file);
        db::upsert_entry(conn, &entry, &text).map_err(db::io)
    })
}

/// Remove an entity of `kind`: its file, and what the index holds of it.
pub fn delete_entity(ws_id: &str, kind: &str, id: &str) -> std::io::Result<()> {
    let (Some(scope), Some(ty)) = (Scope::of_kind(kind), type_of_kind(kind)) else { return Ok(()) };
    with_index(ws_id, scope, |conn, dir| {
        let Some(entry) = db::entry(conn, id).map_err(db::io)?.filter(|entry| entry.ty == ty) else { return Ok(()) };
        let file = dir.join(&entry.path);
        if file.exists() {
            std::fs::remove_file(file)?;
        }
        db::delete_entry(conn, id).map_err(db::io)
    })
}

// ── folders ────────────────────────────────────────────────────────────────────────────────────────────

pub fn folders(ws_id: &str, scope: Scope) -> Vec<Folder> {
    with_index(ws_id, scope, |conn, _| db::folders(conn).map_err(db::io)).unwrap_or_default()
}

pub fn folder(ws_id: &str, scope: Scope, id: &str) -> Option<Folder> {
    with_index(ws_id, scope, |conn, _| db::folder(conn, id).map_err(db::io)).ok().flatten()
}

fn join(parent: Option<&Folder>, name: &str) -> String {
    match parent {
        Some(parent) => format!("{}/{name}", parent.path),
        None => name.to_string(),
    }
}

/// The folder at `path`, whatever the case of its letters: two directories that differ only in case are one
/// on some filesystems.
fn folder_like(conn: &Connection, path: &str) -> rusqlite::Result<Option<Folder>> {
    let wanted = path.to_lowercase();
    Ok(db::folders(conn)?.into_iter().find(|folder| folder.path.to_lowercase() == wanted))
}

fn taken(name: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("A folder named \"{name}\" is already there."))
}

fn no_folder() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, "No such folder.")
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

/// Move a directory. On Windows a rename can fail across a lock, so fall back to copy and remove.
fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_dir(from, to)?;
    std::fs::remove_dir_all(from)
}

/// Make the index follow a directory that went from `old` to `new`, with everything under it.
fn repath(conn: &Connection, old: &str, new: &str) -> rusqlite::Result<()> {
    let under = format!("{old}/");
    for mut folder in db::folders(conn)? {
        if let Some(rest) = folder.path.strip_prefix(&under) {
            folder.path = format!("{new}/{rest}");
            db::update_folder(conn, &folder)?;
        }
    }
    for entry in db::entries(conn)? {
        if let Some(rest) = entry.path.strip_prefix(&under) {
            db::move_entry(conn, &entry.id, entry.folder_id.as_deref(), &format!("{new}/{rest}"))?;
        }
    }
    Ok(())
}

/// Make a folder in `parent_id` (the top of the tree when it names none that exists). A folder of that name
/// may be there already: with `unique` false it is the answer, with `unique` true the name gets a number
/// until it is free.
pub fn create_folder(
    ws_id: &str,
    scope: Scope,
    name: &str,
    parent_id: Option<&str>,
    unique: bool,
) -> std::io::Result<Folder> {
    with_index(ws_id, scope, |conn, dir| {
        let parent = match parent_id.filter(|id| !id.is_empty()) {
            Some(parent_id) => db::folder(conn, parent_id).map_err(db::io)?,
            None => None,
        };
        let base = dir_name(name);
        let mut name = base.clone();
        let mut n = 2;
        while let Some(existing) = folder_like(conn, &join(parent.as_ref(), &name)).map_err(db::io)? {
            if !unique {
                return Ok(existing);
            }
            name = format!("{base}-{n}");
            n += 1;
        }
        let folder = Folder {
            id: generate_id(),
            parent_id: parent.as_ref().map(|parent| parent.id.clone()),
            path: join(parent.as_ref(), &name),
            name,
            created_at: now_ms(),
        };
        std::fs::create_dir_all(dir.join(&folder.path))?;
        db::insert_folder(conn, &folder).map_err(db::io)?;
        Ok(folder)
    })
}

/// Give a folder another name: its directory is renamed, and everything in it goes with it.
pub fn rename_folder(ws_id: &str, scope: Scope, id: &str, name: &str) -> std::io::Result<Folder> {
    with_index(ws_id, scope, |conn, dir| {
        let mut folder = db::folder(conn, id).map_err(db::io)?.ok_or_else(no_folder)?;
        let name = dir_name(name);
        let path = match scan::parent_path(&folder.path) {
            Some(parent) => format!("{parent}/{name}"),
            None => name.clone(),
        };
        if path == folder.path {
            return Ok(folder);
        }
        if folder_like(conn, &path).map_err(db::io)?.is_some_and(|other| other.id != folder.id) {
            return Err(taken(&name));
        }
        move_dir(&dir.join(&folder.path), &dir.join(&path))?;
        let tx = conn.transaction().map_err(db::io)?;
        repath(&tx, &folder.path, &path).map_err(db::io)?;
        folder.name = name;
        folder.path = path;
        db::update_folder(&tx, &folder).map_err(db::io)?;
        tx.commit().map_err(db::io)?;
        Ok(folder)
    })
}

/// Put a folder into `parent_id`, or at the top of the tree: its directory is moved there.
pub fn move_folder(ws_id: &str, scope: Scope, id: &str, parent_id: Option<&str>) -> std::io::Result<Folder> {
    with_index(ws_id, scope, |conn, dir| {
        let mut folder = db::folder(conn, id).map_err(db::io)?.ok_or_else(no_folder)?;
        let parent = match parent_id.filter(|id| !id.is_empty()) {
            Some(parent_id) => Some(db::folder(conn, parent_id).map_err(db::io)?.ok_or_else(no_folder)?),
            None => None,
        };
        if let Some(parent) = &parent {
            if parent.id == folder.id || parent.path.starts_with(&format!("{}/", folder.path)) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "A folder cannot be moved into itself.",
                ));
            }
        }
        let path = join(parent.as_ref(), &folder.name);
        if path == folder.path {
            return Ok(folder);
        }
        if folder_like(conn, &path).map_err(db::io)?.is_some_and(|other| other.id != folder.id) {
            return Err(taken(&folder.name));
        }
        move_dir(&dir.join(&folder.path), &dir.join(&path))?;
        let tx = conn.transaction().map_err(db::io)?;
        repath(&tx, &folder.path, &path).map_err(db::io)?;
        folder.parent_id = parent.map(|parent| parent.id);
        folder.path = path;
        db::update_folder(&tx, &folder).map_err(db::io)?;
        tx.commit().map_err(db::io)?;
        Ok(folder)
    })
}

/// Remove a folder with every folder and file under it.
pub fn delete_folder(ws_id: &str, scope: Scope, id: &str) -> std::io::Result<()> {
    with_index(ws_id, scope, |conn, dir| {
        let Some(folder) = db::folder(conn, id).map_err(db::io)? else { return Ok(()) };
        let full = dir.join(&folder.path);
        if full.exists() {
            std::fs::remove_dir_all(full)?;
        }
        let under = format!("{}/", folder.path);
        let tx = conn.transaction().map_err(db::io)?;
        for other in db::folders(&tx).map_err(db::io)? {
            if other.id == folder.id || other.path.starts_with(&under) {
                db::delete_folder(&tx, &other.id).map_err(db::io)?;
            }
        }
        for entry in db::entries(&tx).map_err(db::io)? {
            if entry.path.starts_with(&under) {
                db::delete_entry(&tx, &entry.id).map_err(db::io)?;
            }
        }
        tx.commit().map_err(db::io)
    })
}

/// Make a folder that was removed a folder again, under the id it had: its directory at `path` is made, and
/// so is every directory above it. When a folder is at `path` already, that one is the answer.
pub fn restore_folder(ws_id: &str, scope: Scope, id: &str, path: &str) -> std::io::Result<Folder> {
    with_index(ws_id, scope, |conn, dir| {
        if let Some(live) = db::folder_at(conn, path).map_err(db::io)? {
            return Ok(live);
        }
        std::fs::create_dir_all(dir.join(path))?;
        // The directories above it become folders the usual way, then this one takes its id back.
        scan::reconcile(conn, dir, scope)?;
        let mut made = db::folder_at(conn, path).map_err(db::io)?.ok_or_else(no_folder)?;
        if db::folder(conn, id).map_err(db::io)?.is_none() {
            let tx = conn.transaction().map_err(db::io)?;
            let ids = [made.id.as_str(), id];
            tx.execute("UPDATE folders SET id = ?2 WHERE id = ?1", ids).map_err(db::io)?;
            tx.execute("UPDATE folders SET parent_id = ?2 WHERE parent_id = ?1", ids).map_err(db::io)?;
            tx.execute("UPDATE entities SET folder_id = ?2 WHERE folder_id = ?1", ids).map_err(db::io)?;
            tx.commit().map_err(db::io)?;
            made.id = id.to_string();
        }
        Ok(made)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch_data_root;

    fn request(name: &str) -> Value {
        json!({ "id": "ignored", "name": name, "method": "GET", "url": format!("http://a/{name}"), "folderId": "x" })
    }

    #[test]
    fn folders_are_directories_and_every_request_type_shares_them() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let outer = create_folder(ws, Scope::Requests, "Outer", None, false).unwrap();
        let inner = create_folder(ws, Scope::Requests, "In/ner", Some(&outer.id), false).unwrap();
        assert_eq!(inner.path, "Outer/Inner");
        assert_eq!(inner.parent_id.as_deref(), Some(outer.id.as_str()));
        assert!(ws_dir(ws).join("requests/Outer/Inner").is_dir());
        // The same name in the same place is the same folder, unless a new one is asked for.
        assert_eq!(create_folder(ws, Scope::Requests, "outer", None, false).unwrap().id, outer.id);
        assert_eq!(create_folder(ws, Scope::Requests, "Outer", None, true).unwrap().name, "Outer-2");

        write_entity(ws, "requests", "r1", &request("Ping"), Some(&inner.id)).unwrap();
        write_entity(ws, "sockets", "s1", &json!({ "name": "Feed", "url": "ws://a" }), Some(&inner.id)).unwrap();
        let file = ws_dir(ws).join("requests/Outer/Inner/r1.json");
        let stored = read_json(&file).unwrap();
        assert_eq!(stored["type"], "rest");
        assert!(stored.get("folderId").is_none(), "the folder is where the file is");
        assert_eq!(read_json(&ws_dir(ws).join("requests/Outer/Inner/s1.json")).unwrap()["type"], "websocket");

        let loaded = read_entity(ws, "requests", "r1").unwrap();
        assert_eq!(loaded["folderId"], inner.id.as_str());
        assert!(read_entity(ws, "sockets", "r1").is_none(), "a request is of one type");
        assert_eq!(read_all(ws, "sockets", None).len(), 1);
        assert_eq!(entries(ws, Scope::Requests).len(), 2);
    }

    #[test]
    fn renaming_and_moving_a_folder_takes_what_is_in_it() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let a = create_folder(ws, Scope::Requests, "A", None, false).unwrap();
        let b = create_folder(ws, Scope::Requests, "B", Some(&a.id), false).unwrap();
        let c = create_folder(ws, Scope::Requests, "C", None, false).unwrap();
        write_entity(ws, "requests", "r1", &request("Ping"), Some(&b.id)).unwrap();

        assert_eq!(rename_folder(ws, Scope::Requests, &a.id, "Alpha").unwrap().path, "Alpha");
        assert!(ws_dir(ws).join("requests/Alpha/B/r1.json").is_file());
        assert_eq!(locate(ws, "requests", "r1").unwrap().0.path, "Alpha/B/r1.json");
        assert!(rename_folder(ws, Scope::Requests, &c.id, "alpha").is_err(), "the name is taken");

        assert!(move_folder(ws, Scope::Requests, &a.id, Some(&b.id)).is_err(), "not into itself");
        let moved = move_folder(ws, Scope::Requests, &b.id, Some(&c.id)).unwrap();
        assert_eq!(moved.path, "C/B");
        assert_eq!(moved.parent_id.as_deref(), Some(c.id.as_str()));
        assert_eq!(read_entity(ws, "requests", "r1").unwrap()["folderId"], b.id.as_str());

        delete_folder(ws, Scope::Requests, &c.id).unwrap();
        assert!(!ws_dir(ws).join("requests/C").exists());
        assert!(read_entity(ws, "requests", "r1").is_none());
        assert_eq!(folders(ws, Scope::Requests).len(), 1);
    }

    #[test]
    fn the_index_follows_files_that_arrive_and_leave_behind_its_back() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        assert!(entries(ws, Scope::Mocks).is_empty());
        let dir = ws_dir(ws).join("mocks/Pulled/Deep");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("m1.json"), r#"{ "id": "m1", "name": "Users", "method": "GET", "urlPattern": "/users" }"#)
            .unwrap();
        assert!(entries(ws, Scope::Mocks).is_empty(), "nothing said the tree changed");
        mark_stale(ws);
        let listed = entries(ws, Scope::Mocks);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].ty, "mock");
        assert_eq!(listed[0].details["url"], "/users");
        let deep = folders(ws, Scope::Mocks).into_iter().find(|f| f.path == "Pulled/Deep").unwrap();
        assert_eq!(listed[0].folder_id.as_deref(), Some(deep.id.as_str()));
        assert_eq!(search(ws, Scope::Mocks, "USER", &[], 10).len(), 1);
        assert!(search(ws, Scope::Mocks, "nope", &[], 10).is_empty());

        // A file that is gone is noticed when it is asked for.
        std::fs::remove_file(dir.join("m1.json")).unwrap();
        assert!(read_entity(ws, "mocks", "m1").is_none());
        assert!(entries(ws, Scope::Mocks).is_empty());

        // The index itself can go: it is made again from the files, though folders get new ids.
        write_entity(ws, "mocks", "m2", &json!({ "name": "M", "method": "GET", "urlPattern": "/m" }), Some(&deep.id))
            .unwrap();
        std::fs::remove_file(ws_dir(ws).join("mocks").join(INDEX_FILE)).unwrap();
        assert_eq!(entries(ws, Scope::Mocks)[0].path, "Pulled/Deep/m2.json");
    }

    #[test]
    fn a_removed_folder_comes_back_under_its_id() {
        let root = scratch_data_root();
        let ws = root.ws_id;
        let a = create_folder(ws, Scope::Requests, "A", None, false).unwrap();
        let b = create_folder(ws, Scope::Requests, "B", Some(&a.id), false).unwrap();
        delete_folder(ws, Scope::Requests, &a.id).unwrap();
        let back = restore_folder(ws, Scope::Requests, &b.id, "A/B").unwrap();
        assert_eq!(back.id, b.id);
        assert!(ws_dir(ws).join("requests/A/B").is_dir());
        let parent = folders(ws, Scope::Requests).into_iter().find(|f| f.path == "A").unwrap();
        assert_eq!(back.parent_id.as_deref(), Some(parent.id.as_str()));
        assert_eq!(restore_folder(ws, Scope::Requests, &a.id, "A").unwrap().id, parent.id, "it is a folder already");
    }
}
