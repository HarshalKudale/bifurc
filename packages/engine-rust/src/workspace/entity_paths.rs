//! Where every entity and folder of the active workspace is in its repository, by id, and which kind of
//! entity a path of the repository is.

use indexmap::IndexMap;

use super::{
    config::load_config,
    fs::{entity_rel_path, flat_entity_rel_path, get_pending_deletions, sanitize_dir_name},
    kinds::crud_kind,
    tree::{self, Scope},
};
use crate::utils::json::{array_of, opt_str, str_of};

/// The kinds outside the trees, in the order they are listed in.
const KIND_ORDER: [&str; 4] = ["mappings", "rules", "environments", "flows"];

#[derive(Debug, Default)]
pub struct EntityPathMaps {
    /// entity id -> the path of its file
    pub entities: IndexMap<String, String>,
    /// folder id -> the path of its directory, ending in `/`
    pub folders: IndexMap<String, String>,
}

/// A deleted entity is where it was when it was deleted, whatever became of its folder since.
fn add_deleted(maps: &mut EntityPathMaps, ws_id: &str, kind: &str) {
    for record in get_pending_deletions(ws_id, kind) {
        if let Some(path) = opt_str(&record, "path") {
            maps.entities.insert(str_of(&record, "id").to_string(), path.to_string());
        }
    }
}

pub fn load_entity_path_maps() -> EntityPathMaps {
    let cfg = load_config();
    let ws_id = str_of(&cfg, "activeWorkspaceId");
    let mut maps = EntityPathMaps::default();
    for scope in Scope::ALL {
        for folder in tree::folders(ws_id, scope) {
            maps.folders.insert(folder.id, format!("{}/{}/", scope.dir(), folder.path));
        }
        for entry in tree::entries(ws_id, scope) {
            let path = tree::rel_path(scope, &entry);
            maps.entities.insert(entry.id, path);
        }
        for (kind, _) in scope.types() {
            add_deleted(&mut maps, ws_id, kind);
        }
    }
    for kind in KIND_ORDER.iter().filter_map(|name| crud_kind(name)) {
        let folder_list = kind.folder_config_key.map(|key| array_of(&cfg, key)).unwrap_or(&[]);
        for folder in folder_list {
            maps.folders.insert(
                str_of(folder, "id").to_string(),
                format!("{}/{}/", kind.kind, sanitize_dir_name(str_of(folder, "name"))),
            );
        }
        for entity in array_of(&cfg, kind.config_key) {
            let id = str_of(entity, "id");
            let path = if kind.is_flat {
                flat_entity_rel_path(kind.kind, id)
            } else {
                let folder_name = opt_str(entity, "folderId")
                    .filter(|f| !f.is_empty())
                    .and_then(|folder_id| folder_list.iter().find(|f| str_of(f, "id") == folder_id))
                    .map(|f| str_of(f, "name"));
                entity_rel_path(kind.kind, id, folder_name)
            };
            maps.entities.insert(id.to_string(), path);
        }
        add_deleted(&mut maps, ws_id, kind.kind);
    }
    maps
}

/// The kind of the entity whose file is, or was, at `rel_path`. Outside the trees that is the directory at the
/// top of the path. In a tree it is what the entity's `type` says: the index knows it of an entity that is
/// there, its deletion record of one that is deleted here, and the tree's first kind stands in for the rest.
pub fn kind_of_path(ws_id: &str, rel_path: &str) -> String {
    let normalized = rel_path.replace('\\', "/");
    let top = normalized.split('/').next().unwrap_or("").to_string();
    let Some(scope) = Scope::of_path(&normalized) else { return top };
    let id = normalized.rsplit('/').next().and_then(|base| base.strip_suffix(".json")).unwrap_or("");
    if let Some((kind, _)) = tree::find_any(ws_id, scope, id) {
        return kind.to_string();
    }
    scope
        .types()
        .iter()
        .map(|(kind, _)| *kind)
        .find(|kind| get_pending_deletions(ws_id, kind).iter().any(|record| str_of(record, "id") == id))
        .unwrap_or(scope.types()[0].0)
        .to_string()
}
