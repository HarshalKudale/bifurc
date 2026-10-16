//! The SQLite file that indexes one tree: its folders and what a listing or a search needs of each entity.
//!
//! Everything in it can be made again from the tree's files, so it is written without waiting for the disk and
//! is thrown away when it cannot be read or is of a version this engine cannot bring up to its own. Making it
//! again gives every folder a new id, so an index of an earlier version is changed in place instead.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use crate::workspace::stamp;

/// The index's file name inside its tree. Git never sees it.
pub const INDEX_FILE: &str = "index.db";

const SCHEMA_VERSION: i64 = 2;

/// What version 2 added to version 1: the stamp of each entity. Every file is read again for it.
const FROM_VERSION_1: &str = "
    ALTER TABLE entities ADD COLUMN engine_version TEXT;
    UPDATE entities SET mtime = 0;
";

const SCHEMA: &str = "
    DROP TABLE IF EXISTS folders;
    DROP TABLE IF EXISTS entities;
    CREATE TABLE folders (
        id TEXT PRIMARY KEY,
        parent_id TEXT,
        name TEXT NOT NULL,
        path TEXT NOT NULL UNIQUE,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE entities (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        id TEXT NOT NULL UNIQUE,
        type TEXT NOT NULL,
        folder_id TEXT,
        name TEXT NOT NULL,
        search TEXT NOT NULL,
        details TEXT NOT NULL,
        path TEXT NOT NULL,
        mtime INTEGER NOT NULL,
        size INTEGER NOT NULL,
        engine_version TEXT
    );
    CREATE INDEX entities_type ON entities(type);
    CREATE INDEX entities_folder ON entities(folder_id);
";

/// A folder of a tree. `path` is its directory relative to the tree, with forward slashes.
#[derive(Debug, Clone, PartialEq)]
pub struct Folder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub path: String,
    pub created_at: i64,
}

/// What the index keeps of an entity. `path` is its file relative to the tree, with forward slashes.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    /// `rest`, `graphql`, ... or `mock`: the `type` its file holds.
    pub ty: String,
    pub folder_id: Option<String>,
    pub name: String,
    /// The fields a listing shows (`name`, `method`, `url`, ...), by the names its file gives them.
    pub details: Value,
    pub path: String,
    pub mtime: i64,
    pub size: i64,
    /// The version of the engine that last wrote its file (`workspace::stamp`), when the file says.
    pub engine_version: Option<String>,
}

impl Entry {
    /// The version the entity needs when this engine is older than the one that last wrote it.
    pub fn lock(&self) -> Option<String> {
        stamp::lock_of_stamp(self.engine_version.as_deref())
    }
}

pub fn io(err: rusqlite::Error) -> std::io::Error {
    std::io::Error::other(err)
}

fn prepare(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "synchronous", "OFF")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version != SCHEMA_VERSION {
        conn.execute_batch(if version == 1 { FROM_VERSION_1 } else { SCHEMA })?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

/// Open a tree's index, making it when it is missing and making it again when it cannot be used.
pub fn open(file: &Path) -> std::io::Result<Connection> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let attempt = |file: &Path| -> rusqlite::Result<Connection> {
        let conn = Connection::open(file)?;
        prepare(&conn)?;
        Ok(conn)
    };
    match attempt(file) {
        Ok(conn) => Ok(conn),
        Err(_) => {
            let _ = std::fs::remove_file(file);
            attempt(file).map_err(io)
        }
    }
}

fn folder_of(row: &Row) -> rusqlite::Result<Folder> {
    Ok(Folder {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        name: row.get(2)?,
        path: row.get(3)?,
        created_at: row.get(4)?,
    })
}

const FOLDER_COLUMNS: &str = "id, parent_id, name, path, created_at";

pub fn folders(conn: &Connection) -> rusqlite::Result<Vec<Folder>> {
    let mut statement = conn.prepare(&format!("SELECT {FOLDER_COLUMNS} FROM folders ORDER BY path"))?;
    let rows = statement.query_map([], folder_of)?;
    rows.collect()
}

pub fn folder(conn: &Connection, id: &str) -> rusqlite::Result<Option<Folder>> {
    conn.query_row(&format!("SELECT {FOLDER_COLUMNS} FROM folders WHERE id = ?1"), [id], folder_of).optional()
}

pub fn folder_at(conn: &Connection, path: &str) -> rusqlite::Result<Option<Folder>> {
    conn.query_row(&format!("SELECT {FOLDER_COLUMNS} FROM folders WHERE path = ?1"), [path], folder_of).optional()
}

pub fn insert_folder(conn: &Connection, folder: &Folder) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO folders (id, parent_id, name, path, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![folder.id, folder.parent_id, folder.name, folder.path, folder.created_at],
    )?;
    Ok(())
}

pub fn update_folder(conn: &Connection, folder: &Folder) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE folders SET parent_id = ?2, name = ?3, path = ?4 WHERE id = ?1",
        params![folder.id, folder.parent_id, folder.name, folder.path],
    )?;
    Ok(())
}

pub fn delete_folder(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM folders WHERE id = ?1", [id])?;
    Ok(())
}

fn entry_of(row: &Row) -> rusqlite::Result<Entry> {
    let details: String = row.get(4)?;
    Ok(Entry {
        id: row.get(0)?,
        ty: row.get(1)?,
        folder_id: row.get(2)?,
        name: row.get(3)?,
        details: serde_json::from_str(&details).unwrap_or(Value::Null),
        path: row.get(5)?,
        mtime: row.get(6)?,
        size: row.get(7)?,
        engine_version: row.get(8)?,
    })
}

const ENTRY_COLUMNS: &str = "id, type, folder_id, name, details, path, mtime, size, engine_version";

/// Every entity of the tree, oldest in the index first.
pub fn entries(conn: &Connection) -> rusqlite::Result<Vec<Entry>> {
    let mut statement = conn.prepare(&format!("SELECT {ENTRY_COLUMNS} FROM entities ORDER BY seq"))?;
    let rows = statement.query_map([], entry_of)?;
    rows.collect()
}

pub fn entries_of_type(conn: &Connection, ty: &str) -> rusqlite::Result<Vec<Entry>> {
    let mut statement = conn.prepare(&format!("SELECT {ENTRY_COLUMNS} FROM entities WHERE type = ?1 ORDER BY seq"))?;
    let rows = statement.query_map([ty], entry_of)?;
    rows.collect()
}

pub fn entry(conn: &Connection, id: &str) -> rusqlite::Result<Option<Entry>> {
    conn.query_row(&format!("SELECT {ENTRY_COLUMNS} FROM entities WHERE id = ?1"), [id], entry_of).optional()
}

/// The entities whose name or listed fields contain `needle`, which is in lower case. `types` narrows them
/// when it names any.
pub fn search(conn: &Connection, needle: &str, types: &[String], limit: i64) -> rusqlite::Result<Vec<Entry>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM entities WHERE instr(search, ?1) > 0 ORDER BY name COLLATE NOCASE, seq"
    ))?;
    let rows = statement.query_map([needle], entry_of)?;
    let mut found = Vec::new();
    for row in rows {
        let row = row?;
        if types.is_empty() || types.contains(&row.ty) {
            found.push(row);
            if found.len() as i64 >= limit {
                break;
            }
        }
    }
    Ok(found)
}

/// Add an entity, or replace what the index holds of it. It keeps its place in the order.
pub fn upsert_entry(conn: &Connection, entry: &Entry, search: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO entities (id, type, folder_id, name, search, details, path, mtime, size, engine_version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(id) DO UPDATE SET type = ?2, folder_id = ?3, name = ?4, search = ?5, details = ?6,
             path = ?7, mtime = ?8, size = ?9, engine_version = ?10",
        params![
            entry.id,
            entry.ty,
            entry.folder_id,
            entry.name,
            search,
            entry.details.to_string(),
            entry.path,
            entry.mtime,
            entry.size,
            entry.engine_version
        ],
    )?;
    Ok(())
}

pub fn delete_entry(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM entities WHERE id = ?1", [id])?;
    Ok(())
}

/// Point an entity at the file it moved to, with its folder.
pub fn move_entry(conn: &Connection, id: &str, folder_id: Option<&str>, path: &str) -> rusqlite::Result<()> {
    conn.execute("UPDATE entities SET folder_id = ?2, path = ?3 WHERE id = ?1", params![id, folder_id, path])?;
    Ok(())
}
