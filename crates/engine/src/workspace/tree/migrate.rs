//! Bringing a workspace of the layout before the trees into them.
//!
//! That layout gave every request type a directory of its own (`sockets/`, `graphqlRequests/`, ...), kept every
//! folder one level deep whatever folder it was shown in, and said which folder was in which in an
//! `index.json`. Here the directories of the other request types are moved into `requests/`, each file is told
//! its `type`, and the folders are put inside one another as `index.json` had them.

use std::{collections::HashMap, path::Path};

use serde_json::{Value, json};

use super::Scope;
use crate::{
    utils::json::{opt_str, read_json, str_of, write_json_pretty},
    workspace::fs::sanitize_dir_name,
};

/// Bookkeeping of the old layout: it is not carried over.
const OLD_FILES: [&str; 2] = ["index.json", "names.json"];

/// Move what `from` holds into `to`, which may exist. An entity file without a `type` is given `ty`.
fn move_into(from: &Path, to: &Path, ty: Option<&str>) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            move_into(&entry.path(), &target, ty)?;
            continue;
        }
        if OLD_FILES.contains(&name.as_str()) {
            continue;
        }
        let typed = ty.filter(|_| name.ends_with(".json")).and_then(|ty| {
            let mut entity = read_json(&entry.path()).filter(Value::is_object)?;
            entity.get("type").is_none().then(|| {
                entity["type"] = json!(ty);
                entity
            })
        });
        match typed {
            Some(entity) => write_json_pretty(&target, &entity)?,
            None => {
                if std::fs::rename(entry.path(), &target).is_err() {
                    std::fs::copy(entry.path(), &target)?;
                }
            }
        }
    }
    std::fs::remove_dir_all(from)
}

/// Where each folder of an old `index.json` belongs in a tree: its directory then, and its path now, parents
/// before the folders in them.
fn nested(index: &Value) -> Vec<(String, String)> {
    let folders = index.get("folders").and_then(Value::as_array).cloned().unwrap_or_default();
    let by_id: HashMap<&str, &Value> = folders.iter().map(|folder| (str_of(folder, "id"), folder)).collect();
    let mut out: Vec<(String, String)> = Vec::new();
    for folder in &folders {
        let mut chain = vec![sanitize_dir_name(str_of(folder, "name"))];
        let mut at = opt_str(folder, "parentId");
        // A parent that names itself, or a ring of them, ends where the ring closes.
        let mut steps = 0;
        while let Some(parent) = at.and_then(|id| by_id.get(id)).filter(|_| steps < folders.len()) {
            chain.push(sanitize_dir_name(str_of(parent, "name")));
            at = opt_str(parent, "parentId");
            steps += 1;
        }
        chain.reverse();
        let dir = chain.last().cloned().unwrap_or_default();
        if !out.iter().any(|(known, _)| *known == dir) {
            out.push((dir, chain.join("/")));
        }
    }
    out.sort_by_key(|(_, path)| path.matches('/').count());
    out
}

/// Move the old directory `from` of one entity kind into the tree at `tree`.
fn bring_in(from: &Path, tree: &Path, ty: Option<&str>) -> std::io::Result<()> {
    let index = read_json(&from.join("index.json")).unwrap_or(Value::Null);
    for (dir, path) in nested(&index) {
        let source = from.join(&dir);
        let target = tree.join(&path);
        if source.is_dir() && source != target {
            move_into(&source, &target, ty)?;
        }
    }
    for name in OLD_FILES {
        let _ = std::fs::remove_file(from.join(name));
    }
    if from != tree {
        // What is left is at the top level, or in a directory the index did not know.
        move_into(from, tree, ty)?;
    }
    Ok(())
}

/// Bring what a workspace keeps the old way into the tree of `scope`. A workspace that is in the trees already
/// is left as it is.
pub fn run(ws: &Path, scope: Scope) {
    let tree = ws.join(scope.dir());
    if tree.join("index.json").is_file() {
        let _ = bring_in(&tree, &tree, None);
    }
    for (kind, ty) in scope.types().iter().filter(|(kind, _)| *kind != scope.dir()) {
        let old = ws.join(kind);
        if old.is_dir() {
            let _ = bring_in(&old, &tree, Some(ty));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        testing::scratch_data_root,
        workspace::{
            fs::ws_dir,
            tree::{entries, folders, read_entity},
        },
    };

    #[test]
    fn a_workspace_of_the_old_layout_is_brought_into_the_trees() {
        let root = scratch_data_root();
        let ws = ws_dir(root.ws_id);
        let write = |path: &str, value: Value| write_json_pretty(&ws.join(path), &value).unwrap();
        write("requests/index.json", json!({ "folders": [
            { "id": "f1", "name": "Users", "parentId": null },
            { "id": "f2", "name": "Admin", "parentId": "f1" },
        ], "order": [] }));
        write("requests/names.json", json!({}));
        write("requests/Users/r1.json", json!({ "id": "r1", "name": "List", "method": "GET", "url": "/users" }));
        write("requests/Admin/r2.json", json!({ "id": "r2", "name": "Ban", "method": "POST", "url": "/ban" }));
        write("sockets/index.json", json!({ "folders": [{ "id": "w1", "name": "Users", "parentId": null }] }));
        write("sockets/Users/s1.json", json!({ "id": "s1", "name": "Feed", "url": "ws://a" }));
        write("graphqlRequests/g1.json", json!({ "id": "g1", "name": "Me", "endpointUrl": "http://a/graphql" }));

        let listed = entries(root.ws_id, Scope::Requests);
        let path_of = |id: &str| listed.iter().find(|entry| entry.id == id).map(|entry| entry.path.clone());
        assert_eq!(path_of("r1").as_deref(), Some("Users/r1.json"));
        assert_eq!(path_of("r2").as_deref(), Some("Users/Admin/r2.json"));
        assert_eq!(path_of("s1").as_deref(), Some("Users/s1.json"), "a folder of that name takes them all");
        assert_eq!(path_of("g1").as_deref(), Some("g1.json"));
        assert_eq!(read_entity(root.ws_id, "sockets", "s1").unwrap()["type"], "websocket");
        assert_eq!(read_entity(root.ws_id, "graphqlRequests", "g1").unwrap()["name"], "Me");
        assert_eq!(folders(root.ws_id, Scope::Requests).len(), 2);
        assert!(!ws.join("sockets").exists() && !ws.join("graphqlRequests").exists());
        assert!(!ws.join("requests/index.json").exists() && !ws.join("requests/Admin").exists());
    }
}
